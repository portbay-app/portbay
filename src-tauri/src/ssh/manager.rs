use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::registry::SshForwardKind;
use crate::ssh::backend::{
    equivalent_ssh_command, spawn_tunnel, wait_for_ready, EffectiveSshTunnel, Result, SshError,
    SshProcess,
};
use crate::ssh::interaction::SshInteractor;
use crate::ssh::secret::{secret_str, SecretString};

pub const SSH_STATE_CHANNEL: &str = "portbay://ssh-tunnels";
pub const STATE_FILE: &str = "ssh-tunnels-state.json";

/// Auto-reconnect backoff. The first retry waits [`RECONNECT_BASE_MS`], each
/// subsequent failure doubles the wait, capped at [`RECONNECT_CAP_MS`]. After
/// [`MAX_RECONNECT_ATTEMPTS`] consecutive failures the supervisor gives up and
/// the tunnel is reported `Down` until the user starts it again — so a server
/// that's gone for good never becomes an infinite silent retry loop.
const RECONNECT_BASE_MS: u64 = 1_000;
const RECONNECT_CAP_MS: u64 = 60_000;
const MAX_RECONNECT_ATTEMPTS: u32 = 12;

/// Backoff delay (ms) before the Nth consecutive reconnect attempt. `0` failures
/// means "retry immediately".
fn backoff_ms(consecutive_failures: u32) -> u64 {
    if consecutive_failures == 0 {
        return 0;
    }
    // Shift is bounded so the `1 << shift` can't overflow regardless of how many
    // failures pile up; the result is capped anyway.
    let shift = consecutive_failures.saturating_sub(1).min(20);
    RECONNECT_BASE_MS
        .saturating_mul(1u64 << shift)
        .min(RECONNECT_CAP_MS)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SshTunnelState {
    Live,
    Down,
    Reconnecting,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTunnelRuntimeStatus {
    pub id: String,
    /// Id of the connection this forward rides on — lets the UI open the file
    /// manager (and future SFTP/exec/shell) for the host behind the tunnel.
    /// `#[serde(default)]` keeps pre-v3 state-mirror files parsing.
    #[serde(default)]
    pub connection_id: String,
    pub name: String,
    pub ssh_host: String,
    pub ssh_port: u16,
    pub ssh_user: String,
    #[serde(default)]
    pub auth_kind: crate::registry::SshAuthKind,
    #[serde(default)]
    pub key_path: Option<String>,
    pub local_host: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub forward_kind: SshForwardKind,
    pub proxy_jump: Option<String>,
    #[serde(default)]
    pub keep_alive: bool,
    #[serde(default)]
    pub auto_reconnect: bool,
    pub state: SshTunnelState,
    pub running: bool,
    pub started_at_ms: Option<u64>,
    pub command: String,
}

/// The event-channel projection of [`SshTunnelRuntimeStatus`]: everything a
/// listener needs to render live tunnel state, minus the secrets-adjacent
/// fields — the absolute private-key path is reduced to `has_key` and the
/// full equivalent `ssh` command line is dropped. The complete status still
/// comes back from the `ssh_tunnel_*` command returns, which only the
/// invoking webview receives; events must stay safe to deliver anywhere.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTunnelEventStatus {
    pub id: String,
    pub connection_id: String,
    pub name: String,
    pub ssh_host: String,
    pub ssh_port: u16,
    pub ssh_user: String,
    pub auth_kind: crate::registry::SshAuthKind,
    pub has_key: bool,
    pub local_host: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub forward_kind: SshForwardKind,
    pub proxy_jump: Option<String>,
    pub keep_alive: bool,
    pub auto_reconnect: bool,
    pub state: SshTunnelState,
    pub running: bool,
    pub started_at_ms: Option<u64>,
}

impl From<&SshTunnelRuntimeStatus> for SshTunnelEventStatus {
    fn from(s: &SshTunnelRuntimeStatus) -> Self {
        Self {
            id: s.id.clone(),
            connection_id: s.connection_id.clone(),
            name: s.name.clone(),
            ssh_host: s.ssh_host.clone(),
            ssh_port: s.ssh_port,
            ssh_user: s.ssh_user.clone(),
            auth_kind: s.auth_kind,
            has_key: s.key_path.is_some(),
            local_host: s.local_host.clone(),
            local_port: s.local_port,
            remote_host: s.remote_host.clone(),
            remote_port: s.remote_port,
            forward_kind: s.forward_kind,
            proxy_jump: s.proxy_jump.clone(),
            keep_alive: s.keep_alive,
            auto_reconnect: s.auto_reconnect,
            state: s.state.clone(),
            running: s.running,
            started_at_ms: s.started_at_ms,
        }
    }
}

pub struct RunningSshTunnel {
    profile: EffectiveSshTunnel,
    command: String,
    process: Option<SshProcess>,
    /// Kept for supervisor reconnects (a password tunnel re-authenticates with
    /// it). Zeroized when the running entry drops — this is the longest-lived
    /// in-memory copy of an SSH password in the app.
    password: Option<SecretString>,
    /// Host-key trust prompter, kept so supervisor reconnects ask the user
    /// about an unknown/changed key exactly like the initial start did (a
    /// Trust-Once key is gone from `known_hosts`, so a reconnect re-prompts
    /// rather than silently re-learning).
    interactor: Option<Arc<dyn SshInteractor>>,
    started_at_ms: u64,
    /// Consecutive failed auto-reconnect attempts. Reset to 0 whenever the
    /// session is observed live again.
    consecutive_failures: u32,
    /// Earliest epoch-ms the supervisor may attempt the next reconnect.
    next_retry_at_ms: u64,
    /// Set once [`MAX_RECONNECT_ATTEMPTS`] is hit — stop retrying, report `Down`.
    gave_up: bool,
}

impl Drop for RunningSshTunnel {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            process.stop();
        }
    }
}

impl RunningSshTunnel {
    /// Whether the child/session is currently alive. Nulls a dead process so a
    /// later reconnect spawns into the empty slot.
    fn process_running(&mut self) -> bool {
        let running = match self.process.as_mut() {
            Some(process) => process.is_running(),
            None => false,
        };
        if !running {
            self.process = None;
        }
        running
    }

    /// The state the UI should see, derived purely from liveness + reconnect
    /// bookkeeping. No side effects — spawning is the supervisor's job
    /// ([`Self::supervise`]), never the read path. This is what fixed the
    /// async-worker stall: `list` used to reconnect inline here.
    fn state_label(&mut self) -> SshTunnelState {
        if self.process_running() {
            SshTunnelState::Live
        } else if self.profile.auto_reconnect && !self.gave_up {
            SshTunnelState::Reconnecting
        } else {
            SshTunnelState::Down
        }
    }

    fn status(&mut self) -> SshTunnelRuntimeStatus {
        let state = self.state_label();
        let running = matches!(state, SshTunnelState::Live);
        status_from_profile(
            &self.profile,
            state,
            Some(self.started_at_ms),
            running,
            self.command.clone(),
        )
    }

    /// Record a failed reconnect: bump the counter, arm the next backoff window,
    /// and give up once the attempt ceiling is reached.
    fn record_failure(&mut self, now: u64) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        if self.consecutive_failures >= MAX_RECONNECT_ATTEMPTS {
            self.gave_up = true;
        }
        self.next_retry_at_ms = now.saturating_add(backoff_ms(self.consecutive_failures));
    }

    /// One supervisor step. If this tunnel should be up but isn't, attempt a
    /// single reconnect subject to backoff. Returns `true` when the *reported*
    /// state changed, so the caller knows to re-emit/mirror. MUST run off the
    /// async runtime — `spawn_tunnel` + readiness wait can block for seconds.
    fn supervise(&mut self, now: u64) -> bool {
        let before = self.state_label();

        if self.process_running() {
            // Healthy: clear failure/backoff so a fresh drop restarts at BASE
            // rather than inheriting a stale long delay.
            self.consecutive_failures = 0;
            self.next_retry_at_ms = 0;
            self.gave_up = false;
            return self.state_label() != before;
        }

        if !self.profile.auto_reconnect || self.gave_up || now < self.next_retry_at_ms {
            return self.state_label() != before;
        }

        match spawn_tunnel(
            &self.profile,
            secret_str(&self.password),
            self.interactor.clone(),
        ) {
            Ok(mut process) => {
                // For port-bound system-ssh forwards, confirm the local port is
                // actually accepting before calling it live; otherwise we'd flap
                // Live→Down on a half-open child.
                let ready = match (self.profile.forward_kind, &mut process) {
                    (
                        SshForwardKind::Local | SshForwardKind::Socks,
                        SshProcess::System { child, .. },
                    ) => wait_for_ready(child, self.profile.local_port).is_ok(),
                    _ => true,
                };
                if ready {
                    self.process = Some(process);
                    self.started_at_ms = now_ms();
                    self.consecutive_failures = 0;
                    self.next_retry_at_ms = 0;
                } else {
                    // `process` drops here → its child is killed.
                    self.record_failure(now);
                }
            }
            Err(e) => {
                tracing::warn!(
                    tunnel_id = %self.profile.id,
                    error = %e,
                    attempt = self.consecutive_failures + 1,
                    "SSH tunnel auto-reconnect attempt failed"
                );
                self.record_failure(now);
            }
        }

        self.state_label() != before
    }
}

#[derive(Default)]
pub struct SshManager {
    tunnels: HashMap<String, RunningSshTunnel>,
}

impl SshManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(
        &mut self,
        profile: EffectiveSshTunnel,
        password: Option<SecretString>,
        interactor: Option<Arc<dyn SshInteractor>>,
    ) -> Result<SshTunnelRuntimeStatus> {
        let id = profile.id.as_str().to_string();
        if self.is_running(&id) {
            return Err(SshError::AlreadyRunning(id));
        }

        let command = equivalent_ssh_command(&profile);
        let mut process = spawn_tunnel(&profile, secret_str(&password), interactor.clone())?;
        if matches!(
            profile.forward_kind,
            SshForwardKind::Local | SshForwardKind::Socks
        ) {
            if let SshProcess::System { child, .. } = &mut process {
                wait_for_ready(child, profile.local_port)?;
            }
        }

        let started_at_ms = now_ms();
        let mut running = RunningSshTunnel {
            profile,
            command,
            process: Some(process),
            password,
            interactor,
            started_at_ms,
            consecutive_failures: 0,
            next_retry_at_ms: 0,
            gave_up: false,
        };
        let status = running.status();
        self.tunnels.insert(id, running);
        Ok(status)
    }

    /// Background-supervisor entry point. Attempts a backed-off reconnect of every
    /// dropped, auto-reconnect-enabled tunnel and returns `true` if any tunnel's
    /// reported state changed (so the caller can re-mirror + emit). Call from a
    /// blocking context — it can spawn `ssh` and wait on readiness.
    pub fn reconnect_due(&mut self) -> bool {
        let now = now_ms();
        let mut changed = false;
        for tunnel in self.tunnels.values_mut() {
            if tunnel.supervise(now) {
                changed = true;
            }
        }
        changed
    }

    pub fn stop(&mut self, id: &str) -> Result<()> {
        let tunnel = self
            .tunnels
            .remove(id)
            .ok_or_else(|| SshError::NotRunning(id.to_string()))?;
        drop(tunnel);
        Ok(())
    }

    pub fn stop_all(&mut self) -> usize {
        let count = self.tunnels.len();
        self.tunnels.clear();
        count
    }

    pub fn is_running(&mut self, id: &str) -> bool {
        self.tunnels
            .get_mut(id)
            .map(|t| t.status().running)
            .unwrap_or(false)
    }

    pub fn list(&mut self, profiles: &[EffectiveSshTunnel]) -> Vec<SshTunnelRuntimeStatus> {
        let mut out: Vec<SshTunnelRuntimeStatus> = profiles
            .iter()
            .map(|profile| {
                self.tunnels
                    .get_mut(profile.id.as_str())
                    .map(|t| t.status())
                    .unwrap_or_else(|| {
                        status_from_profile(
                            profile,
                            SshTunnelState::Down,
                            None,
                            false,
                            equivalent_ssh_command(profile),
                        )
                    })
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }
}

fn status_from_profile(
    profile: &EffectiveSshTunnel,
    state: SshTunnelState,
    started_at_ms: Option<u64>,
    running: bool,
    command: String,
) -> SshTunnelRuntimeStatus {
    SshTunnelRuntimeStatus {
        id: profile.id.as_str().to_string(),
        connection_id: profile.connection_id.as_str().to_string(),
        name: profile.name.clone(),
        ssh_host: profile.ssh_host.clone(),
        ssh_port: profile.ssh_port,
        ssh_user: profile.ssh_user.clone(),
        auth_kind: profile.auth_kind,
        key_path: profile.key_path.clone(),
        local_host: profile.local_host.clone(),
        local_port: profile.local_port,
        remote_host: profile.remote_host.clone(),
        remote_port: profile.remote_port,
        forward_kind: profile.forward_kind,
        proxy_jump: profile.proxy_jump.clone(),
        keep_alive: profile.keep_alive,
        auto_reconnect: profile.auto_reconnect,
        state,
        running,
        started_at_ms,
        command,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn state_file_path(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join(STATE_FILE)
}

pub fn write_state(
    data_dir: &std::path::Path,
    tunnels: &[SshTunnelRuntimeStatus],
) -> std::io::Result<()> {
    let path = state_file_path(data_dir);
    let json = serde_json::to_vec_pretty(tunnels)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    // Owner-only: the mirror carries key paths, hosts/users, and the full
    // equivalent ssh command line — not for other local users.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, &path)
}

pub fn read_state(data_dir: &std::path::Path) -> Vec<SshTunnelRuntimeStatus> {
    let Ok(bytes) = std::fs::read(state_file_path(data_dir)) else {
        return Vec::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{SshAuthKind, SshConnectionId, SshForwardKind, SshTunnelId};

    fn profile(id: &str, name: &str) -> EffectiveSshTunnel {
        EffectiveSshTunnel {
            id: SshTunnelId::new(id),
            connection_id: SshConnectionId::new("host"),
            name: name.into(),
            ssh_host: "host".into(),
            ssh_port: 22,
            ssh_user: "me".into(),
            auth_kind: SshAuthKind::Key,
            key_path: None,
            local_host: "127.0.0.1".into(),
            local_port: 15432,
            remote_host: "localhost".into(),
            remote_port: 5432,
            forward_kind: SshForwardKind::Local,
            proxy_jump: None,
            keep_alive: false,
            auto_reconnect: false,
        }
    }

    #[test]
    fn list_includes_saved_profiles_that_are_down() {
        let mut manager = SshManager::new();
        let statuses = manager.list(&[profile("b", "Beta"), profile("a", "Alpha")]);
        assert_eq!(statuses.len(), 2);
        assert_eq!(statuses[0].name, "Alpha");
        assert_eq!(statuses[0].state, SshTunnelState::Down);
        assert!(statuses[0].command.starts_with("ssh -N"));
    }

    /// The event-channel projection must never carry the private-key path or
    /// the equivalent command line — pins the P1-1 fix from the 2026-06-10
    /// SSH security assessment.
    #[test]
    fn event_status_carries_no_key_path_or_command() {
        let mut manager = SshManager::new();
        let with_key = EffectiveSshTunnel {
            key_path: Some("/Users/me/.ssh/id_ed25519".into()),
            ..profile("a", "Alpha")
        };
        let statuses = manager.list(&[with_key]);
        let full = serde_json::to_value(&statuses[0]).unwrap();
        assert!(
            full.get("keyPath").is_some(),
            "command return keeps keyPath"
        );

        let event: SshTunnelEventStatus = (&statuses[0]).into();
        let json = serde_json::to_value(&event).unwrap();
        assert!(json.get("keyPath").is_none());
        assert!(json.get("command").is_none());
        assert_eq!(json.get("hasKey"), Some(&serde_json::Value::Bool(true)));
        let raw = serde_json::to_string(&event).unwrap();
        assert!(!raw.contains("id_ed25519"));
        assert!(!raw.contains("ssh -N"));
        // The fields the UI renders live state from survive the projection.
        assert_eq!(json.get("id").and_then(|v| v.as_str()), Some("a"));
        assert_eq!(json.get("localPort").and_then(|v| v.as_u64()), Some(15432));
    }

    #[test]
    fn state_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let mut manager = SshManager::new();
        let statuses = manager.list(&[profile("a", "Alpha")]);
        write_state(tmp.path(), &statuses).unwrap();
        let read = read_state(tmp.path());
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].id, "a");
        // The mirror carries the full equivalent ssh command line — owner-only
        // (P2-2, 2026-06-10 assessment).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(state_file_path(tmp.path()))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    fn profile_auto(id: &str, name: &str) -> EffectiveSshTunnel {
        EffectiveSshTunnel {
            auto_reconnect: true,
            ..profile(id, name)
        }
    }

    /// A `RunningSshTunnel` with no live process — i.e. a dropped tunnel, the
    /// supervisor's input. Built directly (no network) so we can exercise the
    /// pure state machine deterministically.
    fn dropped(profile: EffectiveSshTunnel) -> RunningSshTunnel {
        RunningSshTunnel {
            command: equivalent_ssh_command(&profile),
            profile,
            process: None,
            password: None,
            interactor: None,
            started_at_ms: 0,
            consecutive_failures: 0,
            next_retry_at_ms: 0,
            gave_up: false,
        }
    }

    #[test]
    fn backoff_grows_then_caps() {
        assert_eq!(backoff_ms(0), 0, "no failures retries immediately");
        assert_eq!(backoff_ms(1), 1_000);
        assert_eq!(backoff_ms(2), 2_000);
        assert_eq!(backoff_ms(3), 4_000);
        assert_eq!(backoff_ms(7), 60_000, "2^6*1s = 64s clamps to the 60s cap");
        assert_eq!(backoff_ms(50), 60_000, "stays capped, never overflows");
    }

    #[test]
    fn record_failure_backs_off_then_gives_up() {
        let mut t = dropped(profile_auto("a", "A"));
        for attempt in 1..MAX_RECONNECT_ATTEMPTS {
            t.record_failure(0);
            assert!(!t.gave_up, "still retrying at attempt {attempt}");
        }
        t.record_failure(0); // the MAX_RECONNECT_ATTEMPTS-th failure
        assert!(t.gave_up, "supervisor gives up at the attempt ceiling");
        assert_eq!(t.consecutive_failures, MAX_RECONNECT_ATTEMPTS);
        assert_eq!(t.next_retry_at_ms, backoff_ms(MAX_RECONNECT_ATTEMPTS));
    }

    #[test]
    fn dropped_auto_reconnect_reads_reconnecting_until_giveup() {
        let mut t = dropped(profile_auto("a", "A"));
        assert_eq!(t.state_label(), SshTunnelState::Reconnecting);
        t.gave_up = true;
        assert_eq!(
            t.state_label(),
            SshTunnelState::Down,
            "after giving up the UI sees Down, not a perpetual Reconnecting"
        );
    }

    #[test]
    fn dropped_without_auto_reconnect_reads_down() {
        let mut t = dropped(profile("a", "A")); // auto_reconnect = false
        assert_eq!(t.state_label(), SshTunnelState::Down);
    }

    #[test]
    fn supervise_respects_backoff_window_and_does_not_spawn_early() {
        // Arm a far-future retry window; supervise must be a no-op (no spawn
        // attempt, no state change) until the window opens.
        let mut t = dropped(profile_auto("a", "A"));
        t.consecutive_failures = 3;
        t.next_retry_at_ms = u64::MAX;
        let changed = t.supervise(1); // now=1 << next_retry_at_ms
        assert!(
            !changed,
            "still Reconnecting before and after — no transition"
        );
        assert_eq!(t.consecutive_failures, 3, "no attempt was made");
    }
}
