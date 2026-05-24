//! Local crash capture and explicit opt-in telemetry.

use std::backtrace::Backtrace;
use std::path::PathBuf;
use std::sync::Once;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::preferences::Preferences;

static INSTALL_PANIC_HOOK: Once = Once::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashReport {
    pub id: String,
    pub kind: CrashKind,
    pub message: String,
    pub backtrace: Option<String>,
    pub os: String,
    pub arch: String,
    pub app_version: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrashKind {
    RustPanic,
    JsError,
    JsUnhandledRejection,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashReportSummary {
    pub id: String,
    pub kind: CrashKind,
    pub message: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryEvent {
    pub command_name: String,
    pub ok: bool,
    pub os: String,
    pub arch: String,
    pub app_version: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetrySettings {
    pub enabled: bool,
    pub crash_report_count: usize,
    pub endpoint_configured: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    #[error("platform data dir is unavailable")]
    NoDataDir,

    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("serialisation failed: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("telemetry is disabled")]
    Disabled,

    #[error("telemetry endpoint is not configured")]
    EndpointMissing,

    #[error("upload failed: {0}")]
    Upload(String),
}

impl TelemetryError {
    fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, TelemetryError>;

pub fn install_panic_hook(app_version: impl Into<String>) {
    let app_version = app_version.into();
    INSTALL_PANIC_HOOK.call_once(move || {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let message = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| info.to_string());
            let report = CrashReport {
                id: report_id("rust"),
                kind: CrashKind::RustPanic,
                message: scrub_paths(&message),
                backtrace: Some(scrub_paths(&Backtrace::force_capture().to_string())),
                os: std::env::consts::OS.into(),
                arch: std::env::consts::ARCH.into(),
                app_version: app_version.clone(),
                created_at: now_ms(),
            };
            let _ = write_crash_report(&report);
            default_hook(info);
        }));
    });
}

pub fn write_js_crash(kind: CrashKind, message: String, stack: Option<String>) -> Result<String> {
    let report = CrashReport {
        id: report_id("js"),
        kind,
        message: scrub_paths(&message),
        backtrace: stack.map(|s| scrub_paths(&s)),
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        created_at: now_ms(),
    };
    let id = report.id.clone();
    write_crash_report(&report)?;
    Ok(id)
}

pub fn telemetry_settings(prefs: &Preferences) -> Result<TelemetrySettings> {
    Ok(TelemetrySettings {
        enabled: prefs.telemetry_enabled,
        crash_report_count: list_crash_reports()?.len(),
        endpoint_configured: endpoint().is_some(),
    })
}

pub fn list_crash_reports() -> Result<Vec<CrashReportSummary>> {
    let mut reports = Vec::new();
    let dir = crashes_dir()?;
    if !dir.exists() {
        return Ok(reports);
    }
    for entry in std::fs::read_dir(&dir).map_err(|e| TelemetryError::io(&dir, e))? {
        let entry = entry.map_err(|e| TelemetryError::io(&dir, e))?;
        if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let body =
            std::fs::read_to_string(entry.path()).map_err(|e| TelemetryError::io(&dir, e))?;
        if let Ok(report) = serde_json::from_str::<CrashReport>(&body) {
            reports.push(CrashReportSummary {
                id: report.id,
                kind: report.kind,
                message: report.message,
                created_at: report.created_at,
            });
        }
    }
    reports.sort_by_key(|report| std::cmp::Reverse(report.created_at));
    Ok(reports)
}

pub fn read_crash_report(id: &str) -> Result<CrashReport> {
    let path = crash_path(id)?;
    let body = std::fs::read_to_string(&path).map_err(|e| TelemetryError::io(&path, e))?;
    Ok(serde_json::from_str(&body)?)
}

pub fn discard_crash_report(id: &str) -> Result<()> {
    let path = crash_path(id)?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| TelemetryError::io(&path, e))?;
    }
    Ok(())
}

pub async fn send_crash_report(id: &str, prefs: &Preferences) -> Result<()> {
    if !prefs.telemetry_enabled {
        return Err(TelemetryError::Disabled);
    }
    let url = endpoint().ok_or(TelemetryError::EndpointMissing)?;
    let report = read_crash_report(id)?;
    post_json(&format!("{url}/crash"), &report).await?;
    discard_crash_report(id)?;
    Ok(())
}

pub async fn send_telemetry_event(event: TelemetryEvent, prefs: &Preferences) -> Result<()> {
    if !prefs.telemetry_enabled {
        return Err(TelemetryError::Disabled);
    }
    let url = endpoint().ok_or(TelemetryError::EndpointMissing)?;
    post_json(&format!("{url}/telemetry"), &event).await
}

async fn post_json<T: Serialize>(url: &str, payload: &T) -> Result<()> {
    let response = reqwest::Client::new()
        .post(url)
        .json(payload)
        .send()
        .await
        .map_err(|e| TelemetryError::Upload(e.to_string()))?;
    if !response.status().is_success() {
        return Err(TelemetryError::Upload(format!(
            "server returned {}",
            response.status()
        )));
    }
    Ok(())
}

fn write_crash_report(report: &CrashReport) -> Result<()> {
    let dir = crashes_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| TelemetryError::io(&dir, e))?;
    let path = dir.join(format!("{}.json", report.id));
    let body = serde_json::to_vec_pretty(report)?;
    std::fs::write(&path, body).map_err(|e| TelemetryError::io(&path, e))
}

fn crash_path(id: &str) -> Result<PathBuf> {
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(TelemetryError::Io {
            path: PathBuf::from(id),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid report id"),
        });
    }
    Ok(crashes_dir()?.join(format!("{id}.json")))
}

fn crashes_dir() -> Result<PathBuf> {
    let mut dir = dirs::data_dir().ok_or(TelemetryError::NoDataDir)?;
    dir.push("PortBay");
    dir.push("crashes");
    Ok(dir)
}

fn endpoint() -> Option<String> {
    option_env!("PORTBAY_TELEMETRY_ENDPOINT")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_end_matches('/').to_string())
}

fn report_id(prefix: &str) -> String {
    format!("{prefix}-{}", now_ms())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn scrub_paths(input: &str) -> String {
    let home = std::env::var("HOME").ok();
    input
        .lines()
        .map(|line| {
            let mut line = line.to_string();
            if let Some(home) = home.as_deref() {
                line = line.replace(home, "~");
            }
            line = line.replace("/Volumes/DevSSD/projects/Clients/", "<workspace>/");
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_paths_removes_workspace_roots() {
        let input = "/Volumes/DevSSD/projects/Clients/portbay/src/lib.rs";
        assert_eq!(scrub_paths(input), "<workspace>/portbay/src/lib.rs");
    }

    #[test]
    fn default_settings_are_opt_out() {
        let prefs = Preferences::default();
        let settings = telemetry_settings(&prefs).unwrap();
        assert!(!settings.enabled);
    }

    #[test]
    fn telemetry_event_shape_has_no_project_fields() {
        let event = TelemetryEvent {
            command_name: "start_project".into(),
            ok: true,
            os: "macos".into(),
            arch: "aarch64".into(),
            app_version: "0.1.0".into(),
            created_at: 1,
        };
        let json = serde_json::to_value(event).unwrap();
        assert!(json.get("commandName").is_some());
        assert!(json.get("projectPath").is_none());
        assert!(json.get("hostname").is_none());
        assert!(json.get("env").is_none());
        assert!(json.get("logs").is_none());
    }
}
