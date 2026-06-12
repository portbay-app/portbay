//! Local image generation — the `portbay-imagegen` sidecar client.
//!
//! The diffusion half of PortBay's on-device AI: a bundled Swift sidecar
//! (src-tauri/imagegen/) that runs Stable Diffusion (SD 1.5 / 2.1 / SDXL) with
//! apple/ml-stable-diffusion (Core ML on the Neural Engine / GPU). Prompts and
//! images never leave the machine. Mirrors `stt.rs` exactly — same
//! binary-resolution order and the same line-delimited JSON protocol — with
//! `generate` in place of capture.
//!
//! The Tauri commands live in `commands::imagegen`; this module owns binary
//! resolution and the serve-mode client.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

// Image-gen reuses the STT status shape (available / reason / engines) so the
// frontend renders both with one type and one set of reason strings.
use crate::stt::SttStatus;

/// `--check` output shape (see imagegen/Sources/portbay-imagegen/main.swift).
#[derive(Debug, serde::Deserialize)]
struct CheckOutput {
    available: bool,
    reason: Option<String>,
    #[serde(default)]
    engines: Vec<String>,
}

fn unavailable(reason: &str) -> SttStatus {
    SttStatus {
        available: false,
        reason: Some(reason.to_string()),
        engines: Vec::new(),
    }
}

/// Locate the bundled sidecar. Same search order as `resolve_stt_binary`:
/// plain name next to the running exe (packaged .app and `tauri dev`), then
/// the triple-suffixed name (bare `cargo run`), then the source-tree binaries
/// dir (dev/test runs from a checkout).
pub fn resolve_imagegen_binary() -> Option<std::path::PathBuf> {
    use std::env::consts::{ARCH, OS};

    // Test/diagnostic override — debug builds only (a release .app must not
    // let an env var swap the sidecar binary).
    #[cfg(debug_assertions)]
    if let Ok(path) = std::env::var("PORTBAY_IMAGEGEN_BIN") {
        let p = std::path::PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }

    let triple = match (OS, ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        _ => None,
    };

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let plain = dir.join("portbay-imagegen");
            if plain.exists() {
                return Some(plain);
            }
            if let Some(triple) = triple {
                let suffixed = dir.join(format!("portbay-imagegen-{triple}"));
                if suffixed.exists() {
                    return Some(suffixed);
                }
            }
        }
    }

    // Dev-only fallback (stripped from release): a locally-built sidecar under
    // the source tree. `env!("CARGO_MANIFEST_DIR")` is the build machine's path,
    // which must never be referenced by a shipped binary.
    #[cfg(debug_assertions)]
    if let Some(triple) = triple {
        let dev = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join(format!("portbay-imagegen-{triple}"));
        if dev.exists() {
            return Some(dev);
        }
    }
    None
}

/// Probe the sidecar (`portbay-imagegen --check`). The deployment target is
/// macOS 14 (MLX/Core ML floor), so on older macOS the exec fails by design,
/// mapped to `requires_macos_14`.
#[cfg(target_os = "macos")]
pub async fn check() -> SttStatus {
    let Some(binary) = resolve_imagegen_binary() else {
        return unavailable("sidecar_missing");
    };
    // Zero-byte placeholders (seeded for tauri's externalBin check) = missing.
    if std::fs::metadata(&binary).map(|m| m.len()).unwrap_or(0) == 0 {
        return unavailable("sidecar_missing");
    }
    let output = tokio::process::Command::new(&binary)
        .arg("--check")
        .stdin(std::process::Stdio::null())
        .output()
        .await;
    let output = match output {
        Ok(out) => out,
        Err(_) => return unavailable(exec_failure_reason().await),
    };
    if !output.status.success() {
        return unavailable(exec_failure_reason().await);
    }
    match serde_json::from_slice::<CheckOutput>(&output.stdout) {
        Ok(check) => SttStatus {
            available: check.available,
            reason: check.reason,
            engines: check.engines,
        },
        Err(_) => unavailable("sidecar_failed"),
    }
}

#[cfg(not(target_os = "macos"))]
pub async fn check() -> SttStatus {
    unavailable("unsupported")
}

/// "Mac too old" vs "sidecar broken": the binary targets macOS 14, so on 13
/// and older the exec fails by design. `sw_vers` is authoritative — async
/// Command so the probe never blocks the shared worker.
#[cfg(target_os = "macos")]
async fn exec_failure_reason() -> &'static str {
    let major = tokio::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .await
        .ok()
        .and_then(|out| {
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .split('.')
                .next()
                .and_then(|m| m.parse::<u32>().ok())
        });
    match major {
        Some(m) if m < 14 => "requires_macos_14",
        _ => "sidecar_failed",
    }
}

#[cfg(target_os = "macos")]
pub use client::*;

#[cfg(target_os = "macos")]
mod client {
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use once_cell::sync::Lazy;
    use serde_json::Value;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    use super::resolve_imagegen_binary;

    /// Ceiling for metadata ops — local-disk only, but `installed` walks
    /// multi-GB model directories for sizes.
    const OP_TIMEOUT: Duration = Duration::from_secs(20);

    /// Stdin handles of in-flight downloads, keyed by download id, so
    /// `imagegen_cancel_download` can reach the right process.
    static ACTIVE_DOWNLOADS: Lazy<
        Mutex<HashMap<String, Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>>>,
    > = Lazy::new(|| Mutex::new(HashMap::new()));

    /// Download ids cancelled before their stdin was registered — closes the
    /// spawn→register race (mirrors `stt`'s registry).
    static CANCELLED_DOWNLOADS: Lazy<Mutex<HashSet<String>>> =
        Lazy::new(|| Mutex::new(HashSet::new()));

    /// Cancel signals for in-flight generations, keyed by generate id, so
    /// `imagegen_cancel_generate` can kill the right process — diffusion can
    /// legitimately run for minutes and the Generate button must never wedge
    /// on a hung sidecar. `Notify::notify_one` stores a permit, so a cancel
    /// that races registration still lands.
    static ACTIVE_GENERATES: Lazy<Mutex<HashMap<String, Arc<tokio::sync::Notify>>>> =
        Lazy::new(|| Mutex::new(HashMap::new()));

    fn spawn_serve() -> Result<tokio::process::Child, String> {
        let binary = resolve_imagegen_binary().ok_or("image-generation sidecar is missing")?;
        if std::fs::metadata(&binary).map(|m| m.len()).unwrap_or(0) == 0 {
            return Err("image-generation sidecar is missing".to_string());
        }
        tokio::process::Command::new(&binary)
            .arg("--serve")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("failed to start image-generation sidecar: {e}"))
    }

    /// Run one metadata op (catalog / installed / delete) and return its
    /// terminal response. Event lines are skipped.
    pub async fn one_shot_op(request: Value) -> Result<Value, String> {
        let op = request
            .get("op")
            .and_then(Value::as_str)
            .ok_or("request is missing op")?
            .to_string();
        let mut child = spawn_serve()?;
        let mut stdin = child.stdin.take().ok_or("sidecar stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("sidecar stdout unavailable")?;

        let mut line = serde_json::to_string(&request).map_err(|e| e.to_string())?;
        line.push('\n');
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("sidecar write failed: {e}"))?;
        drop(stdin); // EOF after one request: sidecar answers, then exits clean.

        let mut lines = BufReader::new(stdout).lines();
        tokio::time::timeout(OP_TIMEOUT, async {
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if value.get("event").is_some() {
                    continue;
                }
                if value.get("op").and_then(Value::as_str) == Some(op.as_str()) {
                    return Ok(value);
                }
            }
            Err(format!("sidecar closed without answering {op}"))
        })
        .await
        .map_err(|_| format!("sidecar timed out answering {op}"))?
    }

    /// What a download reported when it finished.
    pub struct DownloadOutcome {
        pub success: bool,
        pub cancelled: bool,
        pub error: Option<String>,
    }

    /// Engine/variant the sidecar needs to download a model, resolved app-side
    /// from the PortBay Model Catalog (lets the live catalog ship new
    /// same-engine models without a sidecar rebuild).
    pub struct DownloadSpec {
        pub engine: String,
        pub repo_model: String,
        /// Catalog override for the HF glob to fetch; `None` lets the sidecar
        /// derive it from `engine`. See `ImageCatalogModel::compiled_glob`.
        pub compiled_glob: Option<String>,
        /// Expected install-content digest from the signed catalog; the
        /// sidecar verifies the download against it before sealing.
        pub content_digest: Option<String>,
    }

    /// Run one model download in a dedicated sidecar process, relaying each
    /// progress event into `on_progress(fraction, phase)`.
    pub async fn run_download(
        models_dir: &str,
        model: &str,
        download_id: &str,
        spec: Option<DownloadSpec>,
        mut on_progress: impl FnMut(f64, String),
    ) -> Result<DownloadOutcome, String> {
        let mut child = spawn_serve()?;
        let mut stdin = child.stdin.take().ok_or("sidecar stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("sidecar stdout unavailable")?;

        let mut request = serde_json::json!({
            "op": "download",
            "modelsDir": models_dir,
            "model": model,
            "downloadId": download_id,
        });
        if let Some(spec) = spec {
            request["engine"] = Value::String(spec.engine);
            request["repoModel"] = Value::String(spec.repo_model);
            if let Some(d) = spec.content_digest {
                request["contentDigest"] = Value::String(d);
            }
            if let Some(glob) = spec.compiled_glob {
                request["compiledGlob"] = Value::String(glob);
            }
        }
        let mut line = serde_json::to_string(&request).map_err(|e| e.to_string())?;
        line.push('\n');
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("sidecar write failed: {e}"))?;

        // Park stdin where cancel can find it (not dropped — EOF would end the
        // sidecar's read loop and the cancel line still needs a way in).
        let stdin = Arc::new(tokio::sync::Mutex::new(stdin));
        ACTIVE_DOWNLOADS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(download_id.to_string(), Arc::clone(&stdin));

        let pre_cancelled = CANCELLED_DOWNLOADS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(download_id);
        if pre_cancelled {
            let line = format!(
                "{}\n",
                serde_json::json!({ "op": "cancel-download", "downloadId": download_id })
            );
            let _ = stdin.lock().await.write_all(line.as_bytes()).await;
        }

        let mut lines = BufReader::new(stdout).lines();
        let outcome = loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    let Ok(value) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if value.get("event").and_then(Value::as_str) == Some("progress") {
                        let fraction = value.get("fraction").and_then(Value::as_f64).unwrap_or(0.0);
                        let phase = value
                            .get("phase")
                            .and_then(Value::as_str)
                            .unwrap_or("downloading")
                            .to_string();
                        on_progress(fraction, phase);
                        continue;
                    }
                    if value.get("op").and_then(Value::as_str) == Some("download") {
                        let ok = value.get("ok").and_then(Value::as_bool).unwrap_or(false);
                        let code = value.get("code").and_then(Value::as_i64);
                        break Ok(DownloadOutcome {
                            success: ok,
                            cancelled: code == Some(6),
                            error: value
                                .get("error")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                        });
                    }
                }
                Ok(None) => break Err("sidecar exited mid-download".to_string()),
                Err(e) => break Err(format!("sidecar read failed: {e}")),
            }
        };

        ACTIVE_DOWNLOADS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(download_id);
        CANCELLED_DOWNLOADS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(download_id);
        outcome
    }

    /// Ask an in-flight download's sidecar to cancel. No-op for unknown ids.
    pub async fn cancel_download(download_id: &str) {
        CANCELLED_DOWNLOADS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(download_id.to_string());
        let stdin = ACTIVE_DOWNLOADS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(download_id)
            .cloned();
        if let Some(stdin) = stdin {
            let line = format!(
                "{}\n",
                serde_json::json!({ "op": "cancel-download", "downloadId": download_id })
            );
            let _ = stdin.lock().await.write_all(line.as_bytes()).await;
        }
    }

    /// Parameters for one generation, resolved app-side from the playground.
    pub struct GenerateParams {
        pub prompt: String,
        pub negative_prompt: Option<String>,
        pub steps: Option<u32>,
        pub guidance: Option<f64>,
        pub size: Option<u32>,
        pub seed: Option<i64>,
    }

    /// Generate one image in a dedicated sidecar process, relaying each step's
    /// progress into `on_step(fraction, step, total_steps)`. Returns the
    /// base64-encoded PNG on success. No total timeout — diffusion of a large
    /// model legitimately runs for minutes; the process is killed on drop,
    /// and `cancel_generate(generate_id)` kills it on demand.
    pub async fn generate(
        models_dir: &str,
        model: &str,
        generate_id: &str,
        params: GenerateParams,
        mut on_step: impl FnMut(f64, u32, u32),
    ) -> Result<String, String> {
        let cancel = Arc::new(tokio::sync::Notify::new());
        ACTIVE_GENERATES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(generate_id.to_string(), cancel.clone());
        let outcome = generate_in(models_dir, model, &cancel, params, &mut on_step).await;
        ACTIVE_GENERATES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(generate_id);
        outcome
    }

    /// Ask an in-flight generation's sidecar to stop. No-op for unknown ids.
    pub fn cancel_generate(generate_id: &str) {
        if let Some(cancel) = ACTIVE_GENERATES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(generate_id)
        {
            cancel.notify_one();
        }
    }

    async fn generate_in(
        models_dir: &str,
        model: &str,
        cancel: &tokio::sync::Notify,
        params: GenerateParams,
        on_step: &mut impl FnMut(f64, u32, u32),
    ) -> Result<String, String> {
        let mut child = spawn_serve()?;
        let mut stdin = child.stdin.take().ok_or("sidecar stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("sidecar stdout unavailable")?;

        let mut request = serde_json::json!({
            "op": "generate",
            "modelsDir": models_dir,
            "model": model,
            "prompt": params.prompt,
        });
        if let Some(n) = params.negative_prompt {
            request["negativePrompt"] = Value::String(n);
        }
        if let Some(s) = params.steps {
            request["steps"] = Value::from(s);
        }
        if let Some(g) = params.guidance {
            request["guidance"] = Value::from(g);
        }
        if let Some(sz) = params.size {
            request["size"] = Value::from(sz);
        }
        if let Some(seed) = params.seed {
            request["seed"] = Value::from(seed);
        }
        let mut line = serde_json::to_string(&request).map_err(|e| e.to_string())?;
        line.push('\n');
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("sidecar write failed: {e}"))?;
        drop(stdin); // one request per generate process; EOF ends it after the reply.

        let mut lines = BufReader::new(stdout).lines();
        loop {
            let next = tokio::select! {
                _ = cancel.notified() => {
                    // Explicit kill (not just drop) so the diffusion stops
                    // burning the GPU the moment the user cancels.
                    let _ = child.kill().await;
                    return Err("generation cancelled".to_string());
                }
                next = lines.next_line() => next,
            };
            match next {
                Ok(Some(line)) => {
                    let Ok(value) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if value.get("event").and_then(Value::as_str) == Some("progress") {
                        let fraction = value.get("fraction").and_then(Value::as_f64).unwrap_or(0.0);
                        let step = value.get("step").and_then(Value::as_u64).unwrap_or(0) as u32;
                        let total =
                            value.get("totalSteps").and_then(Value::as_u64).unwrap_or(0) as u32;
                        on_step(fraction, step, total);
                        continue;
                    }
                    if value.get("op").and_then(Value::as_str) == Some("generate") {
                        if value.get("ok").and_then(Value::as_bool) == Some(true) {
                            return value
                                .get("imageBase64")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                                .ok_or_else(|| "generation returned no image".to_string());
                        }
                        return Err(value
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("generation failed")
                            .to_string());
                    }
                }
                Ok(None) => return Err("sidecar exited mid-generation".to_string()),
                Err(e) => return Err(format!("sidecar read failed: {e}")),
            }
        }
    }
}
