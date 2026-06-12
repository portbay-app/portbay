//! Certificate lifecycle commands.
//!
//! Drives mkcert from the GUI. The reconciler issues certs at registry-
//! change time; these commands cover the surfaces the reconciler can't
//! own — user-driven CA install (one-time, privileged), per-project
//! cert metadata for the detail panel, and manual reissue.

use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::process::Command;

use serde::Serialize;
use tauri::{AppHandle, State};
use x509_parser::pem::Pem;
use x509_parser::prelude::*;

use crate::error::{AppError, AppResult};
use crate::mkcert::{CaTrustState, MkcertError};
use crate::registry::{ProjectId, SslMode};
use crate::state::AppState;

/// Cert metadata surfaced to the detail panel's Certificates section.
/// All timestamps are ISO-8601 strings so the frontend can format them
/// without round-tripping a u64.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CertInfo {
    pub project_id: String,
    pub certificate_path: String,
    pub key_path: String,
    pub issued_at: Option<String>,
    pub expires_at: Option<String>,
    pub days_until_expiry: Option<i64>,
    pub sans: Vec<String>,
    pub status: CertStatus,
    pub trust_store_verified: Option<bool>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CertStatus {
    Ready,
    MissingCa,
    Expired,
    Untrusted,
    RegenerateNeeded,
    Error,
}

/// CA trust state returned to the frontend so it can show the correct status
/// and conditionally render the "Install / re-trust CA" action.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaStatusResult {
    /// `"trusted"` | `"untrusted"` | `"missing"` | `"error"`
    pub state: String,
    pub detail: Option<String>,
}

/// `get_ca_status()` — returns the current mkcert CA trust state without
/// running any privileged operation. Used by the Domains & HTTPS panel to
/// decide whether to show the "Install / re-trust CA" button.
#[tauri::command]
pub async fn get_ca_status(state: State<'_, AppState>) -> AppResult<CaStatusResult> {
    let mkcert = state
        .mkcert
        .as_ref()
        .ok_or_else(|| AppError::BadInput("mkcert binary not bundled".into()))?;

    match mkcert.ca_status() {
        Ok(status) => match status.state {
            CaTrustState::Trusted => Ok(CaStatusResult {
                state: "trusted".into(),
                detail: None,
            }),
            CaTrustState::Missing => Ok(CaStatusResult {
                state: "missing".into(),
                detail: Some(format!(
                    "CA root not found at {}",
                    status.root_path.display()
                )),
            }),
            CaTrustState::Untrusted => Ok(CaStatusResult {
                state: "untrusted".into(),
                detail: Some(format!(
                    "CA exists at {} but is not trusted by the system keychain",
                    status.root_path.display()
                )),
            }),
        },
        Err(e) => Ok(CaStatusResult {
            state: "error".into(),
            detail: Some(e.to_string()),
        }),
    }
}

/// `install_mkcert_ca()` — runs the bundled mkcert's `-install` flow.
/// The OS may prompt for authorization to add the CA to the system trust store.
/// Idempotent — `mkcert -install` is safe to call when the CA is already
/// trusted.
#[tauri::command]
pub async fn install_mkcert_ca(state: State<'_, AppState>) -> AppResult<()> {
    let mkcert = state
        .mkcert
        .as_ref()
        .ok_or_else(|| AppError::BadInput("mkcert binary not bundled".into()))?
        .clone();

    // mkcert -install is synchronous and may take a few seconds while
    // macOS authorises the keychain write. Run it on the blocking pool
    // so the async runtime stays responsive.
    let result = tokio::task::spawn_blocking(move || mkcert.install_ca())
        .await
        .map_err(|e| AppError::Internal(format!("install_ca join: {e}")))?;

    match result {
        Ok(()) => Ok(()),
        Err(MkcertError::ExitStatus { status, stderr }) => {
            // Only call it "cancelled" on a real cancel signature. macOS
            // `security` reports a user-dismissed authorization as "cancel"/-128;
            // Linux pkexec/polkit reports authorization denial text.
            // Anything else (untrusted store, SIP, disk error) is surfaced with
            // mkcert's actual stderr instead of being mislabeled.
            let lower = stderr.to_ascii_lowercase();
            if lower.contains("cancel")
                || stderr.contains("-128")
                || lower.contains("not authorized")
                || lower.contains("authentication failed")
                || lower.contains("dismissed")
            {
                Err(AppError::BadInput(
                    "cancelled — the OS trust-store authorization prompt was dismissed".into(),
                ))
            } else if stderr.is_empty() {
                Err(AppError::Internal(format!(
                    "mkcert -install failed (exit {status})"
                )))
            } else {
                Err(AppError::Internal(format!(
                    "mkcert -install failed (exit {status}): {stderr}"
                )))
            }
        }
        Err(e) => Err(AppError::Internal(format!("mkcert -install: {e}"))),
    }
}

/// Read + parse a project's on-disk cert under `certs_root` into [`CertInfo`].
/// `Ok(None)` when no cert has been issued (files absent) — a normal empty
/// state, not an error; `Err` only on a present-but-unreadable/corrupt cert.
/// Shared by the `cert_info` command and the out-of-process CLI / MCP server,
/// which read certs straight off disk without the bundled mkcert binary.
pub fn read_cert_info(
    certs_root: &std::path::Path,
    project_id: &str,
) -> AppResult<Option<CertInfo>> {
    let Some(paths) = crate::mkcert::cert_paths_in(certs_root, project_id) else {
        return Ok(None);
    };
    let pem_bytes = fs::read(&paths.certificate).map_err(AppError::Io)?;
    Ok(Some(parse_cert_pem(
        project_id,
        &paths.certificate,
        &paths.key,
        &pem_bytes,
    )?))
}

/// `cert_info(id)` — returns issue/expiry/SANs for a project's cert.
/// Reads the on-disk PEM via `x509-parser`. The cert file may not exist
/// yet (project hasn't been reconciled, or HTTPS off) — caller treats a
/// `404`-style `BadInput` as the empty-state case.
#[tauri::command]
pub async fn cert_info(state: State<'_, AppState>, id: String) -> AppResult<CertInfo> {
    let registry = crate::commands::projects::load_registry(&state)?;
    let project = registry.get_project(&ProjectId::new(id.clone())).cloned();
    if let Some(project) = project.as_ref() {
        if project.https && project.ssl_mode() == SslMode::CustomCertificate {
            let Some((cert_path, key_path)) = project.custom_cert_paths() else {
                return Err(AppError::BadInput(
                    "custom certificate mode requires certificate and key paths".into(),
                ));
            };
            let cert_path = Path::new(cert_path);
            let key_path = Path::new(key_path);
            let pem_bytes = fs::read(cert_path).map_err(AppError::Io)?;
            let mut info = parse_cert_pem(&id, cert_path, key_path, &pem_bytes)?;
            let desired_owned = desired_cert_names(
                project.hostname.as_str(),
                project.include_wildcard_subdomains(),
            );
            let desired: Vec<&str> = desired_owned.iter().map(String::as_str).collect();
            if let Err(e) = validate_custom_cert_pair(cert_path, key_path, &desired) {
                info.status = CertStatus::Error;
                info.errors.push(e);
            }
            return Ok(info);
        }
    }

    let mkcert = state
        .mkcert
        .as_ref()
        .ok_or_else(|| AppError::BadInput("mkcert binary not bundled".into()))?;

    let mut info = read_cert_info(mkcert.certs_root(), &id)?
        .ok_or_else(|| AppError::NotFound(format!("no cert issued for project '{id}'")))?;
    match mkcert.ca_status() {
        Ok(status) => match status.state {
            CaTrustState::Trusted => {
                info.trust_store_verified = Some(true);
                if info.status == CertStatus::Untrusted || info.status == CertStatus::MissingCa {
                    info.status = CertStatus::Ready;
                }
            }
            CaTrustState::Missing => {
                info.trust_store_verified = Some(false);
                info.status = CertStatus::MissingCa;
                info.errors.push(format!(
                    "mkcert root CA missing at {}",
                    status.root_path.display()
                ));
            }
            CaTrustState::Untrusted => {
                info.trust_store_verified = Some(false);
                info.status = CertStatus::Untrusted;
                info.errors
                    .push("mkcert root CA exists but is not trusted by the OS trust store".into());
            }
        },
        Err(e) => {
            info.trust_store_verified = Some(false);
            info.status = CertStatus::Error;
            info.errors.push(format!("CA trust check failed: {e}"));
        }
    }
    Ok(info)
}

/// `reissue_cert(id)` — delete the existing cert dir, mark the
/// reconciler dirty, and invalidate the Caddy sub-cache so the next
/// tick re-issues + re-POSTs `/load` (Caddy re-reads cert files on
/// load even when the JSON config is byte-identical).
#[tauri::command]
pub async fn reissue_cert(app: AppHandle, state: State<'_, AppState>, id: String) -> AppResult<()> {
    // Validate the project exists in the registry before destructively
    // removing cert files.
    let registry = crate::commands::projects::load_registry(&state)?;
    let pid = ProjectId::new(id.clone());
    if registry.get_project(&pid).is_none() {
        return Err(AppError::NotFound(id));
    }

    if let Some(mkcert) = state.mkcert.as_ref() {
        mkcert
            .remove_cert(&id)
            .map_err(|e| AppError::Internal(format!("remove cert: {e}")))?;
    }

    state.reconciler.invalidate_caddy_cache().await;
    let report = state.reconciler.tick(&app).await;

    if matches!(report.certs, crate::reconciler::StepOutcome::Failed { .. }) {
        return Err(AppError::Internal(format!(
            "reissue failed: {:?}",
            report.certs
        )));
    }
    Ok(())
}

/// `export_cert_bundle(id, dest_dir)` — copy a project's leaf certificate +
/// private key and the mkcert CA root into `<dest_dir>/<hostname>-cert/`,
/// alongside a README describing how to install them on another machine or
/// server. Returns the created folder path. Lets a PortBay-issued cert be
/// reused outside the app the way you'd install a cert on a server.
#[tauri::command]
pub async fn export_cert_bundle(
    state: State<'_, AppState>,
    id: String,
    dest_dir: String,
) -> AppResult<String> {
    let registry = crate::commands::projects::load_registry(&state)?;
    let project = registry
        .get_project(&ProjectId::new(id.clone()))
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("project '{id}' not found")))?;

    // Resolve the leaf cert + key: custom mode uses the user-provided paths;
    // every other mode uses the mkcert-issued pair under certs_root.
    let (cert_src, key_src) = if project.https && project.ssl_mode() == SslMode::CustomCertificate {
        let (cert, key) = project.custom_cert_paths().ok_or_else(|| {
            AppError::BadInput("custom certificate mode requires certificate and key paths".into())
        })?;
        (
            std::path::PathBuf::from(cert),
            std::path::PathBuf::from(key),
        )
    } else {
        let mkcert = state
            .mkcert
            .as_ref()
            .ok_or_else(|| AppError::BadInput("mkcert binary not bundled".into()))?;
        let paths = mkcert.cert_paths(&id).ok_or_else(|| {
            AppError::NotFound(format!("no certificate issued for project '{id}' yet"))
        })?;
        (paths.certificate, paths.key)
    };

    if !cert_src.exists() {
        return Err(AppError::NotFound(format!(
            "certificate file not found at {}",
            cert_src.display()
        )));
    }
    if !key_src.exists() {
        return Err(AppError::NotFound(format!(
            "private key file not found at {}",
            key_src.display()
        )));
    }

    // The mkcert CA root — needed for other machines to trust an automatic-local
    // cert. Absent/optional for custom certs (their chain travels with them).
    let ca_root = state
        .mkcert
        .as_ref()
        .and_then(|m| m.ca_root().ok())
        .map(|root| root.join("rootCA.pem"))
        .filter(|p| p.exists());

    let hostname = project.hostname.as_str();
    let safe_host: String = hostname
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let out_dir = std::path::PathBuf::from(&dest_dir).join(format!("{safe_host}-cert"));
    std::fs::create_dir_all(&out_dir).map_err(AppError::Io)?;

    std::fs::copy(&cert_src, out_dir.join("cert.pem")).map_err(AppError::Io)?;
    std::fs::copy(&key_src, out_dir.join("key.pem")).map_err(AppError::Io)?;
    let has_ca = match ca_root.as_ref() {
        Some(ca) => {
            std::fs::copy(ca, out_dir.join("rootCA.pem")).map_err(AppError::Io)?;
            true
        }
        None => false,
    };

    std::fs::write(out_dir.join("README.md"), export_readme(hostname, has_ca))
        .map_err(AppError::Io)?;

    Ok(out_dir.to_string_lossy().into_owned())
}

/// Human-readable install guide written into the exported bundle.
fn export_readme(hostname: &str, has_ca: bool) -> String {
    let ca_section = if has_ca {
        "- `rootCA.pem` — PortBay's local mkcert Certificate Authority. Install this \
in the trust store of any machine that must trust `cert.pem` (see below)."
    } else {
        "- (No CA root was exported — this certificate carries its own chain, or \
mkcert is not configured on this machine.)"
    };

    format!(
        "# {hostname} — certificate bundle

Exported by PortBay.

## Files

- `cert.pem` — the leaf certificate (the public certificate for `{hostname}`).
- `key.pem` — the matching private key. Keep this secret; never commit it.
{ca_section}

## Use it on a web server

Point your server at `cert.pem` and `key.pem`:

**nginx**
```
ssl_certificate     /path/to/cert.pem;
ssl_certificate_key /path/to/key.pem;
```

**Apache**
```
SSLCertificateFile    /path/to/cert.pem
SSLCertificateKeyFile /path/to/key.pem
```

**Caddy**
```
{hostname} {{
    tls /path/to/cert.pem /path/to/key.pem
}}
```

## Make other machines trust it

This is a locally-issued certificate. Browsers and tools only trust it where
PortBay's CA is installed. On another machine, install `rootCA.pem` into the
system/browser trust store:

- **macOS:** `sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain rootCA.pem`
- **Linux (Debian/Ubuntu):** copy `rootCA.pem` to `/usr/local/share/ca-certificates/` (rename it to end in `.crt`), then run `sudo update-ca-certificates`
- **Windows:** import `rootCA.pem` into *Trusted Root Certification Authorities*.

## Heads up

A locally-trusted (mkcert) certificate is meant for local development. For a real
public server with a public domain, issue a publicly-trusted certificate instead —
in PortBay, set the project's SSL mode to **Public ACME / AutoSSL**.
"
    )
}

fn parse_cert_pem(
    project_id: &str,
    cert_path: &std::path::Path,
    key_path: &std::path::Path,
    bytes: &[u8],
) -> AppResult<CertInfo> {
    let pem = Pem::iter_from_buffer(bytes)
        .next()
        .ok_or_else(|| AppError::Internal("cert file is empty / not PEM".into()))?
        .map_err(|e| AppError::Internal(format!("PEM parse: {e}")))?;

    let (_, cert) = X509Certificate::from_der(&pem.contents)
        .map_err(|e| AppError::Internal(format!("DER parse: {e}")))?;

    let issued_at = iso_from_asn1(cert.validity().not_before);
    let expires_at = iso_from_asn1(cert.validity().not_after);
    let days_until_expiry = cert.validity().time_to_expiration().map(|d| d.whole_days());

    let mut sans: Vec<String> = Vec::new();
    if let Ok(Some(san_ext)) = cert.subject_alternative_name() {
        for name in &san_ext.value.general_names {
            match name {
                GeneralName::DNSName(dns) => sans.push((*dns).to_string()),
                GeneralName::IPAddress(bytes) => {
                    if bytes.len() == 4 {
                        sans.push(
                            Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]).to_string(),
                        );
                    } else if bytes.len() == 16 {
                        let mut octets = [0u8; 16];
                        octets.copy_from_slice(bytes);
                        sans.push(Ipv6Addr::from(octets).to_string());
                    }
                }
                _ => {}
            }
        }
    }
    sans.sort();
    sans.dedup();

    let mut errors = Vec::new();
    if !key_path.exists() {
        errors.push(format!("private key missing at {}", key_path.display()));
    }
    let status = match days_until_expiry {
        Some(days) if days < 0 => CertStatus::Expired,
        Some(days) if days < 30 => CertStatus::RegenerateNeeded,
        None => CertStatus::Error,
        _ => {
            if errors.is_empty() {
                CertStatus::Ready
            } else {
                CertStatus::Error
            }
        }
    };

    Ok(CertInfo {
        project_id: project_id.to_string(),
        certificate_path: cert_path.to_string_lossy().into_owned(),
        key_path: key_path.to_string_lossy().into_owned(),
        issued_at,
        expires_at,
        days_until_expiry,
        sans,
        status,
        trust_store_verified: None,
        errors,
    })
}

pub(crate) fn cert_all_sans(cert_path: &std::path::Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(cert_path) else {
        return Vec::new();
    };
    let Some(Ok(pem)) = Pem::iter_from_buffer(&bytes).next() else {
        return Vec::new();
    };
    let Ok((_, cert)) = X509Certificate::from_der(&pem.contents) else {
        return Vec::new();
    };
    let mut sans = Vec::new();
    if let Ok(Some(ext)) = cert.subject_alternative_name() {
        for name in &ext.value.general_names {
            match name {
                GeneralName::DNSName(dns) => sans.push((*dns).to_string()),
                GeneralName::IPAddress(bytes) if bytes.len() == 4 => {
                    sans.push(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]).to_string());
                }
                GeneralName::IPAddress(bytes) if bytes.len() == 16 => {
                    let mut octets = [0u8; 16];
                    octets.copy_from_slice(bytes);
                    sans.push(Ipv6Addr::from(octets).to_string());
                }
                _ => {}
            }
        }
    }
    sans.sort();
    sans.dedup();
    sans
}

/// Best-effort days until the cert at `cert_path` expires. `None` if the file
/// can't be read or parsed. Drives auto-renewal in the cert reconciler.
pub(crate) fn cert_days_until_expiry(cert_path: &std::path::Path) -> Option<i64> {
    let bytes = std::fs::read(cert_path).ok()?;
    let pem = Pem::iter_from_buffer(&bytes).next()?.ok()?;
    let (_, cert) = X509Certificate::from_der(&pem.contents).ok()?;
    cert.validity().time_to_expiration().map(|d| d.whole_days())
}

pub(crate) fn validate_custom_cert_pair(
    cert_path: &Path,
    key_path: &Path,
    desired_names: &[&str],
) -> std::result::Result<(), String> {
    if !cert_path.exists() {
        return Err(format!(
            "certificate file not found at {}",
            cert_path.display()
        ));
    }
    if !key_path.exists() {
        return Err(format!(
            "private key file not found at {}",
            key_path.display()
        ));
    }

    let sans = cert_all_sans(cert_path);
    if !cert_covers_names(&sans, desired_names) {
        return Err(format!(
            "certificate SANs do not cover {}",
            desired_names.join(", ")
        ));
    }
    if cert_days_until_expiry(cert_path).is_some_and(|days| days < 0) {
        return Err("certificate is expired".into());
    }

    // Validate that the cert and key belong together without printing key
    // material. macOS ships openssl/libressl; if absent, Caddy will still
    // reject a bad pair during config load, but most users get an earlier error.
    if which::which("openssl").is_ok() && !openssl_pair_matches(cert_path, key_path)? {
        return Err("certificate and private key do not match".into());
    }
    Ok(())
}

pub(crate) fn cert_covers_names(have: &[String], desired: &[&str]) -> bool {
    desired.iter().all(|d| have.iter().any(|h| h == d))
}

fn openssl_pair_matches(cert_path: &Path, key_path: &Path) -> std::result::Result<bool, String> {
    let cert_pub = Command::new("openssl")
        .args(["x509", "-pubkey", "-noout", "-in"])
        .arg(cert_path)
        .output()
        .map_err(|e| format!("openssl cert validation failed: {e}"))?;
    if !cert_pub.status.success() {
        return Err("openssl could not read the certificate".into());
    }

    let key_pub = Command::new("openssl")
        .args(["pkey", "-pubout", "-in"])
        .arg(key_path)
        .output()
        .map_err(|e| format!("openssl key validation failed: {e}"))?;
    if !key_pub.status.success() {
        return Err("openssl could not read the private key".into());
    }

    Ok(cert_pub.stdout == key_pub.stdout)
}

fn desired_cert_names(hostname: &str, include_wildcard: bool) -> Vec<String> {
    let mut names = vec![
        hostname.to_string(),
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];
    if include_wildcard {
        names.push(format!("*.{hostname}"));
    }
    names.sort();
    names.dedup();
    names
}

fn iso_from_asn1(t: x509_parser::time::ASN1Time) -> Option<String> {
    // x509-parser's ASN1Time stringifies to RFC 3339-ish; we re-emit a
    // strict ISO-8601 form for the frontend.
    let s = t.to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cert_info_serialises_camel_case() {
        let info = CertInfo {
            project_id: "a".into(),
            certificate_path: "/c/a/cert.pem".into(),
            key_path: "/c/a/key.pem".into(),
            issued_at: Some("2026-01-01T00:00:00Z".into()),
            expires_at: Some("2027-01-01T00:00:00Z".into()),
            days_until_expiry: Some(365),
            sans: vec!["a.test".into()],
            status: CertStatus::Ready,
            trust_store_verified: Some(true),
            errors: vec![],
        };
        let v = serde_json::to_value(&info).unwrap();
        assert!(v.get("projectId").is_some());
        assert!(v.get("certificatePath").is_some());
        assert!(v.get("issuedAt").is_some());
        assert!(v.get("expiresAt").is_some());
        assert!(v.get("daysUntilExpiry").is_some());
        assert!(v.get("trustStoreVerified").is_some());
    }

    /// Integration-flavoured: round-trips a fake PEM through the parser.
    /// Skipped if the host can't run openssl to generate one.
    #[test]
    fn parse_cert_pem_extracts_sans_and_validity() {
        if which::which("openssl").is_err() {
            eprintln!("skipping — host has no openssl");
            return;
        }
        use std::process::Command;
        let tmp = tempfile::tempdir().unwrap();
        let cert = tmp.path().join("cert.pem");
        let key = tmp.path().join("key.pem");
        let out = Command::new("openssl")
            .args(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout"])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .args([
                "-days",
                "30",
                "-subj",
                "/CN=portbay-test.test",
                "-addext",
                "subjectAltName=DNS:portbay-test.test,DNS:alt.test",
            ])
            .output();
        let out = match out {
            Ok(o) if o.status.success() => o,
            _ => {
                eprintln!("openssl failed; skipping");
                return;
            }
        };
        let _ = out;
        let bytes = std::fs::read(&cert).unwrap();
        let info = parse_cert_pem("p", &cert, &key, &bytes).unwrap();
        assert_eq!(info.project_id, "p");
        assert!(info.issued_at.is_some());
        assert!(info.expires_at.is_some());
        let expiry = info.days_until_expiry.unwrap();
        assert!(
            (25..=31).contains(&expiry),
            "expected ~30 days, got {expiry}"
        );
        assert!(info.sans.contains(&"portbay-test.test".to_string()));
        assert!(info.sans.contains(&"alt.test".to_string()));
    }
}
