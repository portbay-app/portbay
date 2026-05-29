//! Bring-your-own **named** Cloudflare tunnel support.
//!
//! Two read-only touches of the user's `~/.cloudflared`:
//!   1. [`detect_named_tunnels`] lists the user's tunnels (one per `<uuid>.json`
//!      credentials file), best-effort enriched with a hostname parsed from
//!      their `config.yml`.
//!   2. nothing is ever written there.
//!
//! [`write_named_config`] generates a **PortBay-owned** ingress config under our
//! own data dir that references the user's tunnel UUID + credentials file, so
//! `cloudflared tunnel run` serves the project at the user's stable hostname
//! without inheriting (or mutating) their `~/.cloudflared/config.yml`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::registry::{CustomTunnelConfig, Project};
use crate::tunnel::error::{Result, TunnelError};

/// A named tunnel discovered under `~/.cloudflared`, offered in the picker.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedTunnel {
    /// Cloudflare tunnel UUID.
    pub uuid: String,
    /// Absolute path to the `<uuid>.json` credentials file.
    pub credentials_file: String,
    /// Hostname prefilled from the user's `config.yml` ingress, if it maps this tunnel.
    pub suggested_hostname: Option<String>,
}

fn cloudflared_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".cloudflared"))
}

/// Credentials-file JSON (subset). cloudflared writes `~/.cloudflared/<UUID>.json`
/// with at least a `TunnelID`.
#[derive(Deserialize)]
struct CredsFile {
    #[serde(rename = "TunnelID")]
    tunnel_id: Option<String>,
}

/// Hints scraped from the user's `config.yml` (best-effort, never required).
struct ConfigHints {
    tunnel: Option<String>,
    hostname: Option<String>,
}

/// Light, permissive parse of `~/.cloudflared/config.yml` for the `tunnel:` id
/// and the first ingress `hostname:`. Returns `None` if the file is absent or
/// unparseable — prefill is a nicety, not a requirement.
fn parse_user_config(path: &Path) -> Option<ConfigHints> {
    let text = std::fs::read_to_string(path).ok()?;
    let val: serde_yaml::Value = serde_yaml::from_str(&text).ok()?;
    let tunnel = val.get("tunnel").and_then(|v| v.as_str()).map(String::from);
    let hostname = val
        .get("ingress")
        .and_then(|i| i.as_sequence())
        .and_then(|seq| {
            seq.iter()
                .find_map(|r| r.get("hostname").and_then(|h| h.as_str()).map(String::from))
        });
    Some(ConfigHints { tunnel, hostname })
}

/// Detect named tunnels in `~/.cloudflared` — one per `<uuid>.json` credentials
/// file. Best-effort hostname prefill from the user's `config.yml`. Read-only.
pub fn detect_named_tunnels() -> Vec<DetectedTunnel> {
    let Some(dir) = cloudflared_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let hints = parse_user_config(&dir.join("config.yml"));

    let mut out: Vec<DetectedTunnel> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        // Prefer the TunnelID inside the creds file; fall back to the filename
        // stem when it's UUID-shaped. Skip non-creds JSON (e.g. cert.json).
        let uuid = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<CredsFile>(&s).ok())
            .and_then(|c| c.tunnel_id)
            .or_else(|| is_uuidish(&stem).then(|| stem.clone()));
        let Some(uuid) = uuid else { continue };

        // Offer the config.yml hostname only when its `tunnel:` maps this one.
        let suggested_hostname = hints
            .as_ref()
            .filter(|h| h.tunnel.as_deref() == Some(uuid.as_str()) || h.tunnel.as_deref() == Some(stem.as_str()))
            .and_then(|h| h.hostname.clone());

        out.push(DetectedTunnel {
            uuid,
            credentials_file: path.to_string_lossy().into_owned(),
            suggested_hostname,
        });
    }
    out.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    out.dedup_by(|a, b| a.uuid == b.uuid);
    out
}

/// Loose UUID shape check (8-4-4-4-12 hex). Avoids pulling in a uuid crate just
/// to recognise a filename.
fn is_uuidish(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 5
        && [8, 4, 4, 4, 12]
            .iter()
            .zip(&parts)
            .all(|(&len, p)| p.len() == len && p.chars().all(|c| c.is_ascii_hexdigit()))
}

// --- PortBay-owned ingress config generation -------------------------------

#[derive(Serialize)]
struct OriginRequest {
    #[serde(rename = "httpHostHeader")]
    http_host_header: String,
}

#[derive(Serialize)]
struct IngressRule {
    #[serde(skip_serializing_if = "Option::is_none")]
    hostname: Option<String>,
    service: String,
    #[serde(skip_serializing_if = "Option::is_none", rename = "originRequest")]
    origin_request: Option<OriginRequest>,
}

#[derive(Serialize)]
struct GeneratedConfig {
    tunnel: String,
    #[serde(rename = "credentials-file")]
    credentials_file: String,
    ingress: Vec<IngressRule>,
}

/// Where the generated config for a project lives (under PortBay's data dir,
/// alongside the quick-tunnel marker). Never under `~/.cloudflared`.
fn named_config_path(project_id: &str) -> Result<PathBuf> {
    let mut dir =
        dirs::data_dir().ok_or_else(|| TunnelError::SpawnFailed("no data dir".to_string()))?;
    dir.push("PortBay");
    dir.push("cloudflared");
    std::fs::create_dir_all(&dir)
        .map_err(|e| TunnelError::SpawnFailed(format!("mkdir cloudflared dir: {e}")))?;
    // Sanitise the id for a filename (ids are slug-like already, but be safe).
    let safe: String = project_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    Ok(dir.join(format!("{safe}.yml")))
}

/// The local origin a named tunnel points at: the project's own dev-server port
/// when it has one (so the app sees the *real* custom Host — the point of a
/// stable domain), else Caddy's `:80` with a Host-header rewrite to the
/// project's `.test` hostname so Caddy routes it.
fn upstream_for(project: &Project) -> (String, Option<String>) {
    match project.port {
        Some(port) => (format!("http://127.0.0.1:{port}"), None),
        None => ("http://127.0.0.1:80".to_string(), Some(project.hostname.clone())),
    }
}

/// Generate the PortBay-owned ingress config for `project`'s attached tunnel and
/// return `(config_path, upstream_url)`. `upstream_url` is the local origin for
/// the reachability probe + status. Errors if the tunnel config is incomplete.
pub fn write_named_config(
    project: &Project,
    cfg: &CustomTunnelConfig,
) -> Result<(PathBuf, String)> {
    if !cfg.is_active() {
        return Err(TunnelError::SpawnFailed(
            "custom tunnel is not fully configured".to_string(),
        ));
    }
    let (upstream_url, host_header) = upstream_for(project);

    let generated = GeneratedConfig {
        tunnel: cfg.tunnel_id.clone(),
        credentials_file: cfg.credentials_file.clone(),
        ingress: vec![
            IngressRule {
                hostname: Some(cfg.hostname.clone()),
                service: upstream_url.clone(),
                origin_request: host_header.map(|h| OriginRequest { http_host_header: h }),
            },
            // Catch-all required by cloudflared: anything else 404s.
            IngressRule {
                hostname: None,
                service: "http_status:404".to_string(),
                origin_request: None,
            },
        ],
    };

    let yaml = serde_yaml::to_string(&generated)
        .map_err(|e| TunnelError::SpawnFailed(format!("serialize tunnel config: {e}")))?;
    let path = named_config_path(&project.id.to_string())?;
    let body = format!("# Generated by PortBay — do not edit. Source: project settings.\n{yaml}");
    std::fs::write(&path, body)
        .map_err(|e| TunnelError::SpawnFailed(format!("write tunnel config: {e}")))?;
    Ok((path, upstream_url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuidish_accepts_real_uuid_rejects_other() {
        assert!(is_uuidish("6ff42ae2-765d-4adf-8112-31c55c1551ef"));
        assert!(!is_uuidish("cert"));
        assert!(!is_uuidish("config"));
        assert!(!is_uuidish("not-a-uuid"));
    }
}
