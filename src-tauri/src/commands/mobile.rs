//! Mobile run UX commands — destination enumeration, toolchain pre-flight,
//! phase hydration, hot reload/restart, and Open Simulator.
//!
//! Every command that shells out to platform tooling (`simctl`, `adb`,
//! `flutter`) runs on the blocking pool: these are exactly the blocking
//! subprocess class that has starved the async command workers before
//! (see the async-worker-starvation incident notes).

use std::collections::HashMap;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::commands::projects::load_registry;
use crate::error::{AppError, AppResult};
use crate::mobile_phase::MobilePhase;
use crate::mobile_targets::{PreflightCheck, RunTarget};
use crate::registry::{Project, ProjectId, ProjectType};
use crate::state::AppState;

fn mobile_project(state: &State<'_, AppState>, id: &str) -> AppResult<Project> {
    let registry = load_registry(state)?;
    let project = registry
        .get_project(&ProjectId::new(id))
        .ok_or_else(|| AppError::NotFound(id.to_string()))?
        .clone();
    if !crate::mobile::is_mobile_kind(project.kind) {
        return Err(AppError::BadInput(format!("{id} is not a mobile project")));
    }
    Ok(project)
}

/// `list_mobile_run_targets(id)` — enumerate the run destinations for a mobile
/// project: simulators/devices for iOS, devices/emulators/AVDs for Android,
/// flutter's own device list (plus bootable sims/AVDs) for Flutter, and the
/// Metro `ios`/`android` switches for Expo.
#[tauri::command]
pub async fn list_mobile_run_targets(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Vec<RunTarget>> {
    let project = mobile_project(&state, &id)?;
    let kind = project.kind;
    let path = project.path.clone();
    tokio::task::spawn_blocking(move || crate::mobile_targets::list_targets(kind, &path))
        .await
        .map_err(|e| AppError::Internal(format!("target enumeration failed: {e}")))
}

/// `mobile_preflight(id)` — toolchain checks for the rail's Checks section
/// (Xcode tools / simulators / adb / AVDs / flutter on PATH…), replacing the
/// web-shaped cert/port/Caddy checks that don't apply to mobile kinds.
#[tauri::command]
pub async fn mobile_preflight(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Vec<PreflightCheck>> {
    let project = mobile_project(&state, &id)?;
    let kind = project.kind;
    let path = project.path.clone();
    tokio::task::spawn_blocking(move || crate::mobile_targets::preflight(kind, &path))
        .await
        .map_err(|e| AppError::Internal(format!("preflight failed: {e}")))
}

/// Current phase of a mobile run, as the hydration shape for the frontend
/// store (live updates ride `portbay://mobile-phase`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MobilePhaseInfo {
    pub phase: MobilePhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// `get_mobile_phases()` — snapshot of every active mobile run phase, so the
/// frontend can hydrate after a reload instead of waiting for the next
/// transition event.
#[tauri::command]
pub async fn get_mobile_phases() -> AppResult<HashMap<String, MobilePhaseInfo>> {
    Ok(crate::mobile_phase::snapshot()
        .into_iter()
        .map(|(id, (phase, detail))| (id, MobilePhaseInfo { phase, detail }))
        .collect())
}

/// Send a signal to the project's live `flutter run` session (SIGUSR1 = hot
/// reload, SIGUSR2 = hot restart — Flutter's documented headless mechanism).
async fn signal_flutter(state: &State<'_, AppState>, id: &str, signal: &str) -> AppResult<()> {
    let project = mobile_project(state, id)?;
    if project.kind != ProjectType::Flutter {
        return Err(AppError::Unsupported {
            feature: "Hot reload",
            reason:
                "Hot reload is driven through `flutter run` and only applies to Flutter projects.",
        });
    }
    let Some(pid) = crate::mobile::flutter_run_pid(&project.path) else {
        return Err(AppError::BadInput(
            "No running `flutter run` session for this project — press Play first.".into(),
        ));
    };
    let signal = signal.to_string();
    let ok = tokio::task::spawn_blocking(move || {
        std::process::Command::new("kill")
            .args([format!("-{signal}"), pid.to_string()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
    .await
    .map_err(|e| AppError::Internal(format!("signal failed: {e}")))?;
    if !ok {
        return Err(AppError::Internal(
            "The flutter session didn't accept the signal — it may have just exited.".into(),
        ));
    }
    Ok(())
}

/// `mobile_hot_reload(id)` — Flutter hot reload (SIGUSR1 to `flutter run`).
#[tauri::command]
pub async fn mobile_hot_reload(state: State<'_, AppState>, id: String) -> AppResult<()> {
    signal_flutter(&state, &id, "USR1").await
}

/// `mobile_hot_restart(id)` — Flutter hot restart (SIGUSR2 to `flutter run`).
#[tauri::command]
pub async fn mobile_hot_restart(state: State<'_, AppState>, id: String) -> AppResult<()> {
    signal_flutter(&state, &id, "USR2").await
}

/// `open_mobile_simulator(id)` — bring the iOS Simulator app forward for
/// projects that target it. Android has no equivalent `open -a` target (the
/// emulator window is a bare qemu process); the rail hides the action there.
#[tauri::command]
pub async fn open_mobile_simulator(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    let project = mobile_project(&state, &id)?;
    if project.kind == ProjectType::Android {
        return Err(AppError::Unsupported {
            feature: "Open Simulator",
            reason: "Android emulators have no openable app bundle — boot one from the destination picker instead.",
        });
    }
    let _ = app; // AppHandle reserved for future opener-plugin routing.
    tokio::task::spawn_blocking(|| {
        std::process::Command::new("open")
            .args(["-a", "Simulator"])
            .output()
    })
    .await
    .map_err(|e| AppError::Internal(format!("open failed: {e}")))?
    .map_err(|e| AppError::Internal(format!("could not open Simulator: {e}")))?;
    Ok(())
}

/// `android_wifi_pair_start()` — begin a QR pairing session (Android 11+
/// Wireless debugging). Returns the QR SVG + password to render, and spawns
/// the blocking watcher that pairs + connects the phone once it scans,
/// streaming progress on `portbay://adb-pair`. Starting a new session
/// supersedes any previous watcher.
#[tauri::command]
pub async fn android_wifi_pair_start(app: AppHandle) -> AppResult<crate::adb_pair::PairSession> {
    let (session, generation) = crate::adb_pair::new_session().map_err(AppError::Internal)?;
    let name = session.name.clone();
    let password = session.password.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::adb_pair::watch_and_pair(app, generation, name, password);
    });
    Ok(session)
}

/// `android_wifi_pair_manual(host_port, code)` — the phone's "Pair device
/// with pairing code" fallback. Blocking adb work → blocking pool.
#[tauri::command]
pub async fn android_wifi_pair_manual(host_port: String, code: String) -> AppResult<String> {
    let host_port = host_port.trim().to_string();
    let code = code.trim().to_string();
    if !host_port.contains(':') {
        return Err(AppError::BadInput(
            "Enter the IP address and port exactly as the phone shows them, e.g. 192.168.1.7:37123."
                .into(),
        ));
    }
    tokio::task::spawn_blocking(move || crate::adb_pair::pair_manual(&host_port, &code))
        .await
        .map_err(|e| AppError::Internal(format!("pairing task failed: {e}")))?
        .map_err(AppError::BadInput)
}
