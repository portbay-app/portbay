//! Crash reporting and opt-in telemetry commands.

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::telemetry::{
    self, CrashKind, CrashReport, CrashReportSummary, TelemetryEvent, TelemetrySettings,
};

#[tauri::command]
pub async fn telemetry_settings(state: State<'_, AppState>) -> AppResult<TelemetrySettings> {
    let prefs = state.preferences_snapshot();
    telemetry::telemetry_settings(&prefs).map_err(to_app_error)
}

#[tauri::command]
pub async fn list_crash_reports() -> AppResult<Vec<CrashReportSummary>> {
    telemetry::list_crash_reports().map_err(to_app_error)
}

#[tauri::command]
pub async fn read_crash_report(id: String) -> AppResult<CrashReport> {
    telemetry::read_crash_report(&id).map_err(to_app_error)
}

#[tauri::command]
pub async fn discard_crash_report(id: String) -> AppResult<Noop> {
    telemetry::discard_crash_report(&id).map_err(to_app_error)?;
    Ok(Noop {})
}

#[tauri::command]
pub async fn send_crash_report(state: State<'_, AppState>, id: String) -> AppResult<Noop> {
    let prefs = state.preferences_snapshot();
    telemetry::send_crash_report(&id, &prefs)
        .await
        .map_err(to_app_error)?;
    Ok(Noop {})
}

#[tauri::command]
pub async fn record_js_error(
    kind: String,
    message: String,
    stack: Option<String>,
) -> AppResult<String> {
    let kind = match kind.as_str() {
        "error" => CrashKind::JsError,
        "unhandledrejection" => CrashKind::JsUnhandledRejection,
        _ => {
            return Err(AppError::BadInput(format!(
                "unknown JS error kind `{kind}`"
            )))
        }
    };
    telemetry::write_js_crash(kind, message, stack).map_err(to_app_error)
}

#[tauri::command]
pub async fn record_telemetry_event(
    state: State<'_, AppState>,
    command_name: String,
    ok: bool,
) -> AppResult<Noop> {
    let prefs = state.preferences_snapshot();
    let event = TelemetryEvent {
        command_name,
        ok,
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        created_at: current_ms(),
    };
    telemetry::send_telemetry_event(event, &prefs)
        .await
        .map_err(to_app_error)?;
    Ok(Noop {})
}

fn to_app_error(error: telemetry::TelemetryError) -> AppError {
    match error {
        telemetry::TelemetryError::Disabled => {
            AppError::BadInput("Telemetry is disabled in Settings.".into())
        }
        telemetry::TelemetryError::EndpointMissing => {
            AppError::BadInput("Telemetry endpoint is not configured for this build.".into())
        }
        other => AppError::Internal(other.to_string()),
    }
}

fn current_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Noop {}
