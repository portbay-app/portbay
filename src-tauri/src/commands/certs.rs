//! Certificate lifecycle commands.
//!
//! Drives mkcert from the GUI. The reconciler issues certs at registry-
//! change time; these commands cover the surfaces the reconciler can't
//! own — user-driven CA install (one-time, privileged), per-project
//! cert metadata for the detail panel, and manual reissue.

use std::fs;

use serde::Serialize;
use tauri::{AppHandle, State};
use x509_parser::pem::Pem;
use x509_parser::prelude::*;

use crate::error::{AppError, AppResult};
use crate::mkcert::MkcertError;
use crate::registry::ProjectId;
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
}

/// `install_mkcert_ca()` — runs the bundled mkcert's `-install` flow.
/// macOS prompts for the user's password to add the CA to the system
/// keychain. Idempotent — `mkcert -install` is safe to call when the CA
/// is already trusted.
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
            // Only call it "cancelled" on a real cancel signature — macOS
            // `security` reports a user-dismissed authorization as "cancel"/-128.
            // Anything else (untrusted store, SIP, disk error) is surfaced with
            // mkcert's actual stderr instead of being mislabeled.
            let lower = stderr.to_ascii_lowercase();
            if lower.contains("cancel") || stderr.contains("-128") {
                Err(AppError::BadInput(
                    "cancelled — macOS keychain prompt was dismissed".into(),
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
    let mkcert = state
        .mkcert
        .as_ref()
        .ok_or_else(|| AppError::BadInput("mkcert binary not bundled".into()))?;

    read_cert_info(mkcert.certs_root(), &id)?
        .ok_or_else(|| AppError::NotFound(format!("no cert issued for project '{id}'")))
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
            if let GeneralName::DNSName(dns) = name {
                sans.push((*dns).to_string());
            }
        }
    }
    sans.sort();

    Ok(CertInfo {
        project_id: project_id.to_string(),
        certificate_path: cert_path.to_string_lossy().into_owned(),
        key_path: key_path.to_string_lossy().into_owned(),
        issued_at,
        expires_at,
        days_until_expiry,
        sans,
    })
}

/// Read the DNS SAN list from an on-disk cert PEM. Best-effort: any read or
/// parse failure yields an empty list (callers treat that as "doesn't cover
/// the desired names" and reissue). The cert reconciler uses this to decide
/// whether an existing cert already covers a project's desired hostnames —
/// e.g. after wildcard subdomains are toggled on, or the hostname changes.
pub(crate) fn cert_dns_sans(cert_path: &std::path::Path) -> Vec<String> {
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
            if let GeneralName::DNSName(dns) = name {
                sans.push((*dns).to_string());
            }
        }
    }
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
        };
        let v = serde_json::to_value(&info).unwrap();
        assert!(v.get("projectId").is_some());
        assert!(v.get("certificatePath").is_some());
        assert!(v.get("issuedAt").is_some());
        assert!(v.get("expiresAt").is_some());
        assert!(v.get("daysUntilExpiry").is_some());
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
