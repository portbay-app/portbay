//! System metrics — CPU + RAM only.
//!
//! Storage and network panels are explicitly out of scope per
//! `docs/UX_DESIGN.md` and the card #11 outcome: they belong in Activity
//! Monitor, not a dev-env tool. The right rail's spare vertical room is
//! reserved for PortBay-specific telemetry later (e.g. ports-in-use,
//! running-project count).

use std::sync::Mutex;

use serde::Serialize;
use sysinfo::System;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::error::AppResult;

pub const METRICS_CHANNEL: &str = "portbay://metrics";

/// One sample of system load. Mirrors the screenshot's right-rail data.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMetrics {
    pub cpu: CpuMetrics,
    pub memory: MemoryMetrics,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuMetrics {
    /// 0..=100. Aggregate across all cores.
    pub total: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryMetrics {
    pub used_bytes: u64,
    pub total_bytes: u64,
}

/// Shared sysinfo `System` handle. Refreshing in place is cheaper than
/// re-allocating the whole struct each tick.
pub struct MetricsState {
    pub system: Mutex<System>,
}

impl MetricsState {
    pub fn new() -> Self {
        let mut system = System::new();
        system.refresh_cpu();
        system.refresh_memory();
        Self {
            system: Mutex::new(system),
        }
    }

    fn sample(&self) -> SystemMetrics {
        let mut sys = self.system.lock().expect("system mutex poisoned");
        // CPU usage requires two refreshes spaced apart. The poller calls
        // `sample` on a 2s cadence so the gap is already there.
        sys.refresh_cpu();
        sys.refresh_memory();
        let cpu_total = sys.global_cpu_info().cpu_usage();
        let used_bytes = sys.used_memory();
        let total_bytes = sys.total_memory();
        SystemMetrics {
            cpu: CpuMetrics { total: cpu_total },
            memory: MemoryMetrics {
                used_bytes,
                total_bytes,
            },
        }
    }
}

impl Default for MetricsState {
    fn default() -> Self {
        Self::new()
    }
}

/// One-shot sample, on demand. The frontend uses the event stream below
/// for steady-state updates; this command is the fallback when the
/// stream hasn't ticked yet.
#[tauri::command]
pub async fn system_metrics(state: State<'_, MetricsState>) -> AppResult<SystemMetrics> {
    Ok(state.sample())
}

/// Spawn the background metrics poller. Emits `portbay://metrics` every
/// 1 s — fast enough to track CPU spikes during a dev-server hot-reload
/// without overwhelming sysinfo's refresh cost (~5-15 ms per sample on
/// a modern Mac). Matches the cadence reference tools like ServBay use.
pub fn spawn_metrics_poller(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tick.tick().await;
            let state: tauri::State<MetricsState> = app.state();
            let sample = state.sample();
            let _ = app.emit(METRICS_CHANNEL, sample);
        }
    });
}
