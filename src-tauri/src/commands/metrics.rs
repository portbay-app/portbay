//! System metrics — CPU, RAM, and root-volume disk usage.
//!
//! Disk was added back in for the redesigned sidebar footer (CPU / Memory
//! / Disk row, mirroring the design reference). It's sampled from the
//! root mount only; per-volume detail still belongs in Activity Monitor.

use std::sync::Mutex;

use serde::Serialize;
use sysinfo::{Disks, System};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::error::AppResult;

pub const METRICS_CHANNEL: &str = "portbay://metrics";

/// One sample of system load. Mirrors the screenshot's right-rail data.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMetrics {
    pub cpu: CpuMetrics,
    pub memory: MemoryMetrics,
    pub disk: DiskMetrics,
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

/// Root-volume usage. macOS reports per-mount totals; on systems with
/// firmlinks (the standard "Data" + "System" split) the user-visible
/// "/" is the right mount to surface.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskMetrics {
    pub used_bytes: u64,
    pub total_bytes: u64,
}

/// Shared sysinfo handles. Refreshing in place is cheaper than
/// re-allocating each tick.
pub struct MetricsState {
    pub system: Mutex<System>,
    pub disks: Mutex<Disks>,
}

impl MetricsState {
    pub fn new() -> Self {
        let mut system = System::new();
        system.refresh_cpu();
        system.refresh_memory();
        let disks = Disks::new_with_refreshed_list();
        Self {
            system: Mutex::new(system),
            disks: Mutex::new(disks),
        }
    }

    fn sample(&self) -> SystemMetrics {
        let mut sys = self.system.lock().unwrap_or_else(|e| e.into_inner());
        // CPU usage requires two refreshes spaced apart. The poller calls
        // `sample` on a 2s cadence so the gap is already there.
        sys.refresh_cpu();
        sys.refresh_memory();
        let cpu_total = sys.global_cpu_info().cpu_usage();
        let used_bytes = sys.used_memory();
        let total_bytes = sys.total_memory();
        drop(sys);

        let mut disks = self.disks.lock().unwrap_or_else(|e| e.into_inner());
        disks.refresh();
        // Pick the mount with the largest total — on macOS that's the
        // root "Data" firmlink. If we ever support multi-volume hosts
        // formally, this turns into a per-volume DTO.
        let root = disks
            .iter()
            .max_by_key(|d| d.total_space())
            .map(|d| {
                (
                    d.total_space(),
                    d.total_space().saturating_sub(d.available_space()),
                )
            })
            .unwrap_or((0, 0));
        drop(disks);

        SystemMetrics {
            cpu: CpuMetrics { total: cpu_total },
            memory: MemoryMetrics {
                used_bytes,
                total_bytes,
            },
            disk: DiskMetrics {
                used_bytes: root.1,
                total_bytes: root.0,
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
