//! Local crash capture and explicit opt-in telemetry.

use std::backtrace::Backtrace;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Once;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use regex::Regex;
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
                message: scrub(&message),
                backtrace: Some(scrub(&Backtrace::force_capture().to_string())),
                os: std::env::consts::OS.into(),
                arch: std::env::consts::ARCH.into(),
                app_version: app_version.clone(),
                created_at: now_ms(),
            };
            let _ = write_crash_report(&report);
            // Do NOT call the default panic hook here: Rust's built-in hook
            // writes to stderr via `eprintln!`, which panics with
            // "failed printing to stderr: Broken pipe" when the pipe reader
            // has gone away (e.g. a closed terminal). That secondary panic is
            // then itself captured by this hook, looping. Instead we write
            // the panic location to stderr ourselves using `writeln!` — which
            // returns an `Err` on EPIPE and we silently discard it.
            let loc = info
                .location()
                .map(|l| format!(" at {}:{}", l.file(), l.line()))
                .unwrap_or_default();
            let _ = writeln!(std::io::stderr(), "thread panicked{loc}: {message}");
        }));
    });
}

pub fn write_js_crash(kind: CrashKind, message: String, stack: Option<String>) -> Result<String> {
    let report = CrashReport {
        id: report_id("js"),
        kind,
        message: scrub(&message),
        backtrace: stack.map(|s| scrub(&s)),
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

pub async fn send_crash_report(id: &str) -> Result<()> {
    // No opt-in gate here: a crash report is only ever uploaded in response to
    // an explicit user click ("Send report" on the crash card, or "Send" in
    // Settings). That click is the per-incident consent — it lets someone who
    // keeps automatic diagnostics off still hand us a single crash. Background
    // usage telemetry (`send_telemetry_event`) stays gated on `telemetryEnabled`.
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

/// Append a usage event to the on-disk outbox (`PortBay/events/*.json`).
///
/// Cheap and network-free — this is the hot path for short-lived surfaces like
/// the CLI, where blocking each command on a round-trip to the cloud would be a
/// poor, offline-fragile UX. Delivery happens later via [`flush_outbox`] on the
/// next run, and only when telemetry consent is on. The spooled file carries no
/// PII beyond the existing [`TelemetryEvent`] shape (command name + ok +
/// os/arch/version), so persisting it is always safe; the consent gate governs
/// *delivery*, not capture. Past a cap the event is dropped rather than growing
/// the queue without bound when we're perpetually offline.
pub fn spool_telemetry_event(event: &TelemetryEvent) -> Result<()> {
    const MAX_SPOOLED_EVENTS: usize = 200;
    let dir = events_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| TelemetryError::io(&dir, e))?;
    if let Ok(existing) = list_spooled_event_paths() {
        if existing.len() >= MAX_SPOOLED_EVENTS {
            return Ok(());
        }
    }
    let id = format!("evt-{}-{}", event.created_at, std::process::id());
    let path = dir.join(format!("{id}.json"));
    let body = serde_json::to_vec(event)?;
    std::fs::write(&path, body).map_err(|e| TelemetryError::io(&path, e))
}

/// Best-effort delivery of everything queued on disk — pending crash reports
/// and spooled usage events — gated on standing telemetry consent.
///
/// Built for the CLI: it's short-lived and has no UI to surface crash cards, so
/// the `telemetry_enabled` preference set during `portbay login` (or
/// `portbay telemetry on`) *is* the consent, and queued items flow without a
/// per-incident prompt. Bounded so it can never hang a command — each upload has
/// a timeout and the per-run counts are capped; anything left over is retried on
/// the next run. Every error is swallowed: telemetry must never change a
/// command's outcome or exit code. A no-op (two cheap dir stats) on the common
/// path where consent is off or the queue is empty.
pub async fn flush_outbox(prefs: &Preferences) {
    const PER_ITEM_TIMEOUT: Duration = Duration::from_secs(5);
    const MAX_CRASHES_PER_RUN: usize = 5;
    const MAX_EVENTS_PER_RUN: usize = 20;

    if !prefs.telemetry_enabled {
        return;
    }
    let Some(url) = endpoint() else {
        return;
    };

    // Crash reports first — they're the higher-value signal and the rarer event.
    // `send_crash_report` deletes the file on success; failures keep it for the
    // next run.
    if let Ok(reports) = list_crash_reports() {
        for summary in reports.into_iter().take(MAX_CRASHES_PER_RUN) {
            let _ = tokio::time::timeout(PER_ITEM_TIMEOUT, send_crash_report(&summary.id)).await;
        }
    }

    // Then queued usage events — post, delete on success.
    if let Ok(paths) = list_spooled_event_paths() {
        for path in paths.into_iter().take(MAX_EVENTS_PER_RUN) {
            let Ok(body) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(event) = serde_json::from_str::<TelemetryEvent>(&body) else {
                // Corrupt or foreign file — drop it so it can't wedge the queue.
                let _ = std::fs::remove_file(&path);
                continue;
            };
            let target = format!("{url}/telemetry");
            let send = post_json(&target, &event);
            if let Ok(Ok(())) = tokio::time::timeout(PER_ITEM_TIMEOUT, send).await {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

fn list_spooled_event_paths() -> Result<Vec<PathBuf>> {
    let dir = events_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| TelemetryError::io(&dir, e))? {
        let entry = entry.map_err(|e| TelemetryError::io(&dir, e))?;
        if entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
            paths.push(entry.path());
        }
    }
    // Oldest-first by filename (ids embed a millisecond timestamp), so a capped
    // flush drains the backlog in arrival order.
    paths.sort();
    Ok(paths)
}

fn events_dir() -> Result<PathBuf> {
    let mut dir = dirs::data_dir().ok_or(TelemetryError::NoDataDir)?;
    dir.push("PortBay");
    dir.push("events");
    Ok(dir)
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

/// Production telemetry sink — the PortBay Cloud Worker (portbay-cloud). The
/// app posts crash reports to `{endpoint}/crash` and usage events to
/// `{endpoint}/telemetry`, and only ever talks to this first-party host (never
/// a third-party analytics SDK). Forwarding to the analytics backend happens
/// server-side inside the Worker.
const DEFAULT_TELEMETRY_ENDPOINT: &str = "https://cloud.portbay.app";

fn endpoint() -> Option<String> {
    let raw = option_env!("PORTBAY_TELEMETRY_ENDPOINT")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_TELEMETRY_ENDPOINT);
    Some(raw.trim_end_matches('/').to_string())
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

/// The placeholder substituted for any redacted secret value.
const REDACTED: &str = "[redacted]";

/// Credentials embedded in a URL authority — `scheme://user:password@host`.
/// This is the exact shape a DB or registry connection string takes, the named
/// risk in the assessment: a panic during provisioning can carry the live
/// connection string (password and all) into the message or backtrace. We keep
/// the scheme and username (useful, non-secret context) and redact only the
/// password.
static URL_CREDENTIALS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"([A-Za-z][A-Za-z0-9+.\-]*://[^\s:/@]+):[^\s/@]+@").unwrap());

/// A value keyed by a secret-looking name — `KEY=value`, `key: value`,
/// `"key":"value"` (covers `.env` lines, DB/registry creds, and config dumps).
/// The key and separator (group 1, including the value's opening quote) are
/// preserved; the value (group 2) is redacted. The value cannot start with `:`
/// and stops at whitespace/quote/comma/semicolon/brace, so Rust's `path::seg`
/// symbol separators in a backtrace don't trip it.
static KEYED_SECRET: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?ix)
        (                                       # $1: key + separator (kept)
          ["']?
          [a-z0-9_.\-]*                         # optional prefix (DB_, MYSQL_, …)
          (?: password | passwd | pwd | passphrase | secret | token
            | api[_-]?key | apikey | access[_-]?key | private[_-]?key
            | client[_-]?secret | credentials? | connection[_-]?string | dsn )
          [a-z0-9_.\-]*                         # optional suffix (_hash, …)
          ["']?
          \s* [:=] \s*
          ["']?                                 # opening quote of the value
        )
        ( [^\s:"',;{}] [^\s"',;{}]* )           # $2: the value, redacted
        "#,
    )
    .unwrap()
});

/// Full crash-text scrub applied to every message and backtrace before it is
/// written to disk: filesystem paths first ([`scrub_paths`]), then secret
/// *values* ([`scrub_secrets`]). The path pass strips user-identifying home and
/// volume roots; the secret pass redacts credentials a panic can carry onto the
/// stack (DB/registry creds, `.env` values) that the path pass alone would let
/// through.
fn scrub(input: &str) -> String {
    scrub_secrets(&scrub_paths(input))
}

/// Redact secret *values* from free-form crash text. Conservative by
/// construction: it only fires on URL-embedded credentials and values keyed by
/// a secret-looking name, leaving ordinary backtrace content (types, `file:line`,
/// messages) intact. Where a value is ambiguous it over-redacts rather than risk
/// a leak — a crash report with one fewer field is always better than one that
/// ships a password.
fn scrub_secrets(input: &str) -> String {
    let stage = URL_CREDENTIALS.replace_all(input, |c: &regex::Captures| {
        format!("{}:{REDACTED}@", &c[1])
    });
    KEYED_SECRET
        .replace_all(stage.as_ref(), |c: &regex::Captures| {
            format!("{}{REDACTED}", &c[1])
        })
        .into_owned()
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
            line = scrub_volume_roots(&line);
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Replace every `/Volumes/<name>` prefix with `<volume>` so paths on
/// external drives (whose names are often user-identifying) don't leak into
/// crash reports. Derived at runtime — no machine-specific literals.
fn scrub_volume_roots(line: &str) -> String {
    const MARKER: &str = "/Volumes/";
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(idx) = rest.find(MARKER) {
        out.push_str(&rest[..idx]);
        out.push_str("<volume>");
        let after = &rest[idx + MARKER.len()..];
        rest = after.find('/').map_or("", |sep| &after[sep..]);
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_paths_removes_volume_roots() {
        let input = "/Volumes/SomeDrive/projects/portbay/src/lib.rs";
        assert_eq!(scrub_paths(input), "<volume>/projects/portbay/src/lib.rs");
    }

    #[test]
    fn scrub_paths_handles_bare_volume_and_mid_line() {
        // A volume root with no trailing segment, and one embedded mid-line.
        assert_eq!(scrub_paths("/Volumes/Backup"), "<volume>");
        assert_eq!(
            scrub_paths("at /Volumes/Work/app/main.rs:42"),
            "at <volume>/app/main.rs:42",
        );
    }

    #[test]
    fn scrub_paths_replaces_home_with_tilde() {
        let home = std::env::var("HOME").unwrap();
        let input = format!("{home}/projects/demo/src/main.rs");
        assert_eq!(scrub_paths(&input), "~/projects/demo/src/main.rs");
    }

    #[test]
    fn scrub_secrets_redacts_db_connection_string() {
        // The named threat: a panic during DB provisioning carries the live
        // connection string onto the stack. Scheme + user survive; the password
        // is gone.
        let input = "Error: could not connect to \
            postgres://portbay:s3cr3t-p4ss@localhost:5432/portbay_dev";
        let out = scrub_secrets(input);
        assert!(!out.contains("s3cr3t-p4ss"), "password leaked: {out}");
        assert!(out.contains("postgres://portbay:[redacted]@localhost:5432/portbay_dev"));
    }

    #[test]
    fn scrub_secrets_redacts_keyed_values() {
        // .env line, quoted YAML/log value, and a JSON config dump.
        assert_eq!(
            scrub_secrets("DB_PASSWORD=hunter2"),
            "DB_PASSWORD=[redacted]"
        );
        assert_eq!(
            scrub_secrets("password: \"hunter2\""),
            "password: \"[redacted]\""
        );
        assert_eq!(
            scrub_secrets(r#"{"apiKey":"sk-live-abc123","ok":true}"#),
            r#"{"apiKey":"[redacted]","ok":true}"#
        );
        assert_eq!(
            scrub_secrets("MYSQL_PWD=p@ssw0rd and AWS_SECRET_ACCESS_KEY=abcd/efgh"),
            "MYSQL_PWD=[redacted] and AWS_SECRET_ACCESS_KEY=[redacted]"
        );
    }

    #[test]
    fn scrub_secrets_preserves_ordinary_backtrace_lines() {
        // Path-style symbol separators (`::`) and file:line must survive — the
        // value scrubber is for secrets, not for mangling stack frames.
        let frame = "   3: portbay_lib::context::token::refresh at src/context.rs:42";
        assert_eq!(scrub_secrets(frame), frame);
        let msg = "called `Result::unwrap()` on an `Err` value: NotFound";
        assert_eq!(scrub_secrets(msg), msg);
    }

    #[test]
    fn synthetic_secret_in_panic_message_is_redacted_end_to_end() {
        // The card's acceptance test: a secret in a panic-shaped message is
        // redacted by the same `scrub` the panic hook applies before the report
        // reaches disk.
        let panicked = "thread 'main' panicked at 'provisioning failed: \
            DATABASE_URL=postgres://root:topsecret@db/app', src/db.rs:88";
        let out = scrub(panicked);
        assert!(!out.contains("topsecret"), "secret leaked: {out}");
        assert!(out.contains("[redacted]"));
        // Non-secret structure is preserved.
        assert!(out.contains("src/db.rs:88"));
    }

    #[test]
    fn default_settings_are_opt_out() {
        let prefs = Preferences::default();
        let settings = telemetry_settings(&prefs).unwrap();
        assert!(!settings.enabled);
    }

    #[tokio::test]
    async fn flush_outbox_is_noop_when_consent_off() {
        // With telemetry off (the default), flushing must touch no network and
        // return promptly — the consent gate is the first thing it checks.
        let prefs = Preferences::default();
        assert!(!prefs.telemetry_enabled);
        flush_outbox(&prefs).await;
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
