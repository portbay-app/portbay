//! Stale / orphaned sidecar reclamation.
//!
//! PortBay spawns four long-lived helper processes: `caddy`, `dnsmasq`,
//! `mailpit` (spawned directly as children of the app), and `process-compose`
//! (which in turn supervises the user's dev servers). On a clean quit
//! [`crate::state::AppState::shutdown_all`] tears them all down. But a crash —
//! or a `tauri dev` rebuild that SIGKILLs the old app binary — runs no `Drop`
//! and no shutdown handler, so those children get **reparented to launchd
//! (PPID 1)** and keep squatting their ports (`:443`, the mailpit SMTP port,
//! the DNS port, …). The next launch then spawns a *second* stack whose Caddy
//! can't bind `:443`, silently falls back to an alternate port, and serves no
//! TLS for the canonical host — exactly the `ERR_SSL_PROTOCOL_ERROR` incident
//! this module exists to prevent.
//!
//! ## Strategy: terminate-and-respawn, not adopt
//!
//! We never try to *adopt* an orphan (Tauri's `CommandChild` can't wrap a PID
//! we didn't spawn, and an orphan's on-disk config predates whatever the user
//! changed since it crashed — adopting it would perpetuate the stale-config
//! bug). Instead we **reap** PortBay-owned stale instances before booting a
//! fresh stack, then verify the canonical port was actually released before
//! the fresh daemon tries to bind it.
//!
//! ## Safety: identify by config-path signature, never by name alone
//!
//! This machine may also run ServBay (`/Applications/ServBay/bin/caddy`), a
//! Homebrew `dnsmasq`, or the user's own `mailpit`. We must NEVER kill those.
//! Every PortBay sidecar carries a `<data-dir>/PortBay/…` config path in its
//! argv (`--config …/caddy.bootstrap.json`, `-C …/dnsmasq.conf`,
//! `--db-file …/mailpit.db`, `-f …/process-compose.yaml`) — a path unique to
//! this install. A process matches only when its command line contains **both**
//! the binary-name marker **and** that config-path marker, so a foreign caddy
//! (whose config lives under `/Applications/ServBay/…`) can never match. This
//! is the same precise signature [`crate::process_compose::lifecycle`] already
//! uses for process-compose; this module generalises it to all four.

use std::path::PathBuf;
use std::time::{Duration, Instant};

pub use crate::process_compose::lifecycle::SweepMode;

/// How long to let a reaped instance forward SIGTERM to anything it supervises
/// before we SIGKILL it. Matches `process_compose::lifecycle`'s sweep grace.
const SWEEP_GRACE: Duration = Duration::from_millis(1500);

/// How long to wait for a reclaimed port's listener to actually disappear
/// before giving up. SIGKILL is near-instant, but the kernel can hold the
/// socket in `TIME_WAIT`/teardown for a beat; binding too eagerly would race.
const PORT_RELEASE_TIMEOUT: Duration = Duration::from_secs(3);

/// Poll interval while waiting for a port to free up.
const PORT_RELEASE_POLL: Duration = Duration::from_millis(100);

/// The helper processes PortBay owns. Caddy/dnsmasq/mailpit/process-compose are
/// spawned directly as app children; `php-fpm` is one level deeper (a
/// process-compose child), but it leaks the same way — when PC is SIGKILLed at
/// boot before it can drain its children, the FPM master orphans to launchd and
/// keeps its unix socket bound, so the fresh build's `php-fpm` can't start
/// ("Another FPM instance seems to already listen…"). Same signature-based
/// reclaim, so it lives here too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarKind {
    Caddy,
    Dnsmasq,
    Mailpit,
    PhpFpm,
    ProcessCompose,
}

impl SidecarKind {
    /// Every kind, in boot order (PC last because it's the supervisor; php-fpm
    /// just before it, since it's a thing PC supervises).
    pub const ALL: [SidecarKind; 5] = [
        SidecarKind::Caddy,
        SidecarKind::Dnsmasq,
        SidecarKind::Mailpit,
        SidecarKind::PhpFpm,
        SidecarKind::ProcessCompose,
    ];

    /// Human-facing name used in logs and `doctor` rows.
    pub fn display_name(&self) -> &'static str {
        match self {
            SidecarKind::Caddy => "caddy",
            SidecarKind::Dnsmasq => "dnsmasq",
            SidecarKind::Mailpit => "mailpit",
            SidecarKind::PhpFpm => "php-fpm",
            SidecarKind::ProcessCompose => "process-compose",
        }
    }

    /// Substring that must appear in the process's command line to be a
    /// candidate. The bundled binary is named e.g. `caddy-aarch64-apple-darwin`,
    /// so the bare binary name is a substring of that. Secondary signal only —
    /// [`Self::config_marker`] carries the real specificity.
    pub fn name_marker(&self) -> &'static str {
        // process-compose's binary keeps the hyphen; the others are single words.
        self.display_name()
    }

    /// The argv flag each sidecar passes its config path with — `caddy
    /// --config`, `dnsmasq -C`, `mailpit --db-file`, `process-compose -f`. Baked
    /// into the marker so the match keys on the *launch shape*, not merely on
    /// the path appearing anywhere in a command line.
    fn config_flag(&self) -> &'static str {
        match self {
            SidecarKind::Caddy => "--config",
            SidecarKind::Dnsmasq => "-C",
            SidecarKind::Mailpit => "--db-file",
            // `php-fpm -F -y <pool.conf>` (see process_compose::config).
            SidecarKind::PhpFpm => "-y",
            SidecarKind::ProcessCompose => "-f",
        }
    }

    fn config_file(&self) -> &'static str {
        match self {
            SidecarKind::Caddy => "caddy.bootstrap.json",
            SidecarKind::Dnsmasq => "dnsmasq.conf",
            SidecarKind::Mailpit => "mailpit.db",
            // php-fpm's pool config lives in a per-version subdir
            // (`…/PortBay/php/<ver>/php-fpm.conf`), so we deliberately key on the
            // version-agnostic `php` dir prefix rather than a single file: the
            // marker `-y …/PortBay/php` matches a stale pool of ANY version,
            // which is exactly what we want to reap. The data-dir prefix still
            // keeps a foreign (Herd/ServBay/Homebrew) php-fpm from matching.
            SidecarKind::PhpFpm => "php",
            SidecarKind::ProcessCompose => "process-compose.yaml",
        }
    }

    /// The argv marker that uniquely identifies a *PortBay-owned* instance:
    /// `<flag> <data-dir>/PortBay/<file>` (e.g.
    /// `--config /…/PortBay/caddy.bootstrap.json`). `None` only when the
    /// platform data dir can't be resolved (which would also have broken boot).
    ///
    /// Including the flag is what makes this safe: the bare config *path* also
    /// appears in the argv of a `tail`/`grep`/editor a user might run against
    /// the file, and — because the binary name is a substring of the config
    /// filename (`caddy` ⊂ `caddy.bootstrap.json`) — such a process would
    /// otherwise satisfy *both* halves of the signature and we'd kill it. A
    /// `grep` never carries `--config <path>`; only the daemon we launched does.
    pub fn config_marker(&self) -> Option<String> {
        let path = portbay_data_dir()?.join(self.config_file());
        Some(format!("{} {}", self.config_flag(), path.to_string_lossy()))
    }

    /// Canonical ports PortBay tries to bind for this sidecar. Used by `doctor`
    /// to detect squatting. These are the *defaults*; the live app may have
    /// fallen back to an alternate port, but the defaults are what a fresh boot
    /// reaches for and what the incident hinged on (`:443`).
    pub fn canonical_ports(&self) -> &'static [u16] {
        match self {
            // 443 = HTTPS, 80 = HTTP, 2019 = admin API.
            SidecarKind::Caddy => &[443, 80, 2019],
            // PortBay's dnsmasq runs on a high non-privileged port by default.
            SidecarKind::Dnsmasq => &[53053],
            // 1025 = SMTP, 8025 = web UI.
            SidecarKind::Mailpit => &[1025, 8025],
            // php-fpm listens on a unix socket, not a TCP port — nothing for
            // `doctor` to flag as squatted, and nothing to wait on after a
            // reclaim (killing the master frees the socket).
            SidecarKind::PhpFpm => &[],
            SidecarKind::ProcessCompose => &[9999],
        }
    }

    /// The one port whose release gates a clean rebind after a reclaim. Only
    /// Caddy's `:443` matters: dnsmasq and mailpit pick a fresh free port via
    /// their own scan, so there's nothing to wait for. Returning `None` skips
    /// the post-reclaim port wait entirely for those kinds.
    pub fn rebind_gate_port(&self) -> Option<u16> {
        match self {
            SidecarKind::Caddy => Some(443),
            _ => None,
        }
    }

    /// Build the full match signature for this kind, or `None` if the data dir
    /// is unresolvable.
    pub fn signature(&self) -> Option<Signature> {
        Some(Signature {
            name_marker: self.name_marker().to_string(),
            config_marker: self.config_marker()?,
        })
    }
}

/// The two-part identity test for a PortBay-owned sidecar process.
#[derive(Debug, Clone)]
pub struct Signature {
    /// Binary-name substring (weak signal — a foreign caddy matches this too).
    pub name_marker: String,
    /// `<flag> <data-dir>/PortBay/…` launch-shape substring (strong signal):
    /// the config path *as our daemon passes it*, flag included. Unique to this
    /// install AND to the way we spawn it, so neither a foreign install nor a
    /// `tail`/`grep` on the same file can match.
    pub config_marker: String,
}

impl Signature {
    /// A command line belongs to a PortBay-owned instance iff it carries BOTH
    /// the binary-name marker AND our config-path marker.
    pub fn matches(&self, command_line: &str) -> bool {
        command_line.contains(&self.name_marker) && command_line.contains(&self.config_marker)
    }
}

/// `<data-dir>/PortBay` — the directory all PortBay sidecar configs live in.
pub fn portbay_data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("PortBay"))
}

/// Reap PortBay-owned stale instances of `kind`, then (in `All` mode, used at
/// boot) wait for the rebind-gate port to be released so the fresh daemon can
/// bind it. Returns how many processes were reaped.
///
/// `mode` semantics mirror the process-compose sweep:
/// - [`SweepMode::All`] — reap every match regardless of parentage. Safe ONLY
///   at boot, before our own stack is up, where any match is by definition
///   stale. The rebind-gate port wait runs in this mode.
/// - [`SweepMode::OrphansOnly`] — reap only PPID-1 (launchd-reparented)
///   instances. Safe to run while a live PortBay is up — it can't touch a
///   sidecar still parented to the live app. No port wait (not a boot path).
pub fn reclaim_stale(kind: SidecarKind, mode: SweepMode) -> usize {
    let Some(sig) = kind.signature() else {
        return 0;
    };
    let Some(ps) = ps_snapshot() else {
        return 0;
    };
    let pids = matching_pids(&ps, &sig, &[], mode);
    let mut reaped = 0usize;
    for pid in pids {
        // PID-reuse guard: the `ps` snapshot is a moment old. Re-read this
        // pid's command line right before we kill it and confirm it STILL
        // matches our signature. If the pid was recycled into an unrelated
        // process between snapshot and now, this skips it — we never signal a
        // process we haven't just re-verified as ours.
        if !still_matches(pid, &sig) {
            tracing::debug!(
                pid,
                kind = kind.display_name(),
                "skipping reclaim — pid no longer matches signature (reused?)"
            );
            continue;
        }
        let _ = crate::port_holder::kill_gracefully(pid, SWEEP_GRACE);
        reaped += 1;
    }

    if reaped > 0 && mode == SweepMode::All {
        if let Some(port) = kind.rebind_gate_port() {
            if !wait_port_released(port, PORT_RELEASE_TIMEOUT) {
                tracing::warn!(
                    port,
                    kind = kind.display_name(),
                    "reclaimed stale instance but port still held after timeout — \
                     a foreign process may hold it; boot will fall back to an alternate port"
                );
            }
        }
    }

    if reaped > 0 {
        tracing::info!(
            count = reaped,
            kind = kind.display_name(),
            "reclaimed stale sidecar(s)"
        );
    }
    reaped
}

/// Poll until nothing is listening on `port` (verified by an actual `lsof`
/// re-check, not by assuming the kill worked), or until `timeout` elapses.
/// Returns `true` if the port is free.
pub fn wait_port_released(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if crate::port_holder::find(port).is_none() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(PORT_RELEASE_POLL);
    }
}

/// What [`detect_all`] found for one sidecar kind — the raw material for the
/// `doctor` integrity report.
#[derive(Debug, Clone)]
pub struct StackReport {
    pub kind: SidecarKind,
    /// PIDs of every PortBay-owned instance currently running (any parentage).
    pub owned_pids: Vec<u32>,
    /// Subset of `owned_pids` that are orphaned to launchd (PPID 1) — leaked by
    /// a crashed prior run and reclaimable via `OrphansOnly`.
    pub orphan_pids: Vec<u32>,
}

impl StackReport {
    /// More than one owned instance = a duplicate stack (the bug we self-heal).
    pub fn is_duplicate(&self) -> bool {
        self.owned_pids.len() > 1
    }
    pub fn has_orphans(&self) -> bool {
        !self.orphan_pids.is_empty()
    }
}

/// Scan the process table once per kind and classify PortBay-owned instances
/// into all-owned vs orphaned-to-launchd. Read-only — kills nothing. Used by
/// `portbay doctor` to flag duplicate/orphaned stacks.
pub fn detect_all() -> Vec<StackReport> {
    let Some(ps) = ps_snapshot() else {
        return Vec::new();
    };
    SidecarKind::ALL
        .iter()
        .filter_map(|&kind| {
            let sig = kind.signature()?;
            Some(StackReport {
                kind,
                owned_pids: matching_pids(&ps, &sig, &[], SweepMode::All),
                orphan_pids: matching_pids(&ps, &sig, &[], SweepMode::OrphansOnly),
            })
        })
        .collect()
}

/// Who, if anyone, holds a canonical port — and is it one of ours or a foreign
/// process we must leave alone? Used by `doctor` to explain a `:443` squat.
#[derive(Debug, Clone)]
pub struct PortSquat {
    pub port: u16,
    pub holder: String,
    pub pid: u32,
    /// True when the holder's command line carries this kind's PortBay config
    /// marker — i.e. it's our own (possibly orphaned) sidecar, safe to reclaim.
    /// False = a foreign process (ServBay, the user's own daemon) — never kill.
    pub portbay_owned: bool,
    pub orphaned: bool,
}

/// Check each of `kind`'s canonical ports for a listener and classify it.
/// Returns one entry per *held* port (free ports are omitted).
pub fn port_squatters(kind: SidecarKind) -> Vec<PortSquat> {
    let marker = kind.config_marker();
    kind.canonical_ports()
        .iter()
        .filter_map(|&port| {
            let holder = crate::port_holder::find(port)?;
            let portbay_owned = match (&marker, &holder.command_line) {
                (Some(m), Some(cmd)) => cmd.contains(m),
                _ => false,
            };
            Some(PortSquat {
                port,
                holder: holder.command.clone(),
                pid: holder.pid,
                portbay_owned,
                orphaned: holder.orphaned,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Single-instance pidfile
// ---------------------------------------------------------------------------

/// Path to the advisory pidfile, `<data-dir>/PortBay/portbay.pid`. The
/// `tauri-plugin-single-instance` plugin is the real guard against a second
/// app spawning a second stack; this file exists so the CLI / `doctor` can
/// cheaply tell whether the app is up (and which PID), without scanning.
pub fn pidfile_path() -> Option<PathBuf> {
    Some(portbay_data_dir()?.join("portbay.pid"))
}

/// Record the current process as the live app instance. Best-effort.
pub fn write_pidfile() {
    let Some(path) = pidfile_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, std::process::id().to_string()) {
        tracing::warn!(error = %e, "failed to write pidfile");
    }
}

/// Remove the pidfile on graceful shutdown. Best-effort (a crash leaves it,
/// which is fine — [`app_running`] re-checks liveness, never trusts the file).
pub fn remove_pidfile() {
    if let Some(path) = pidfile_path() {
        let _ = std::fs::remove_file(path);
    }
}

/// The live app's PID if one is running, per the pidfile — but only after
/// confirming that PID is actually alive (a stale pidfile from a crashed run
/// is ignored). `None` means no live app.
pub fn app_running() -> Option<u32> {
    let path = pidfile_path()?;
    let pid: u32 = std::fs::read_to_string(path).ok()?.trim().parse().ok()?;
    if pid_alive(pid) {
        Some(pid)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Internals (pure where possible, so the safety-critical matcher is unit-tested)
// ---------------------------------------------------------------------------

/// Snapshot the process table as `pid ppid command` lines. `None` if `ps`
/// is unavailable (then we reclaim nothing — fail safe).
fn ps_snapshot() -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid=,command="])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Parse `ps -axo pid=,ppid=,command=` output and return the pids of every line
/// matching `sig`, excluding the current process and anything in `exclude`, and
/// honouring `mode`. Pure string work so it's unit-testable without spawning.
fn matching_pids(ps_output: &str, sig: &Signature, exclude: &[u32], mode: SweepMode) -> Vec<u32> {
    let me = std::process::id();
    ps_output
        .lines()
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let pid = cols.next()?.parse::<u32>().ok()?;
            let ppid = cols.next()?.parse::<u32>().ok()?;
            if pid == me || exclude.contains(&pid) {
                return None;
            }
            if mode == SweepMode::OrphansOnly && ppid != 1 {
                return None;
            }
            if sig.matches(line) {
                Some(pid)
            } else {
                None
            }
        })
        .collect()
}

/// Re-read a single pid's command line and confirm it still matches `sig`.
/// The PID-reuse guard before any kill.
fn still_matches(pid: u32, sig: &Signature) -> bool {
    match process_command_line(pid) {
        Some(cmd) => sig.matches(&cmd),
        None => false,
    }
}

/// Current command line for one pid via `ps -p <pid> -o command=`.
fn process_command_line(pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// `kill -0`: tests signal-delivery feasibility without sending a signal.
fn pid_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A representative `ps -axo pid=,ppid=,command=` table mixing PortBay's
    /// own sidecars with ServBay's and foreign ones. PortBay's data dir on this
    /// fixture machine is `/Users/dev/Library/Application Support/PortBay`.
    const DATA: &str = "/Users/dev/Library/Application Support/PortBay";

    fn ps_fixture() -> String {
        format!(
            "\
  501     1 /Applications/PortBay.app/Contents/MacOS/caddy-aarch64-apple-darwin run --config {DATA}/caddy.bootstrap.json
  502     1 /Applications/ServBay/bin/caddy run --config /Applications/ServBay/etc/caddy/caddy.json
  503  4242 /opt/homebrew/bin/caddy run --config /Users/dev/caddy/Caddyfile
  504     1 /Applications/PortBay.app/Contents/MacOS/dnsmasq-aarch64-apple-darwin -C {DATA}/dnsmasq.conf
  505     1 /Applications/ServBay/bin/dnsmasq -C /Applications/ServBay/etc/dnsmasq/dnsmasq.conf
  506  4242 /Applications/PortBay.app/Contents/MacOS/mailpit --smtp 127.0.0.1:1025 --listen 127.0.0.1:8025 --db-file {DATA}/mailpit.db --max 1000
  507  4242 /opt/homebrew/bin/mailpit --smtp 127.0.0.1:1025
  508     1 /Applications/PortBay.app/Contents/MacOS/process-compose -f {DATA}/process-compose.yaml --port 9999 --tui=false up
  509  4242 /usr/bin/process-compose -f /Users/dev/other/process-compose.yaml up
  510     1 {DATA}/runtimes/php/8.4.21/sbin/php-fpm -F -y {DATA}/php/8.4.21/php-fpm.conf
  511     1 /Applications/ServBay/package/sbin/php-fpm -F -y /Applications/ServBay/etc/php/8.4/php-fpm.conf
",
        )
    }

    /// Build a signature against the fixture data dir (`DATA`) rather than the
    /// machine's real one, so the matcher can be tested deterministically. Uses
    /// the same `<flag> <path>` shape as the production `config_marker`.
    fn sig(kind: SidecarKind) -> Signature {
        Signature {
            name_marker: kind.name_marker().to_string(),
            config_marker: format!("{} {DATA}/{}", config_flag(kind), config_file(kind)),
        }
    }

    fn config_flag(kind: SidecarKind) -> &'static str {
        match kind {
            SidecarKind::Caddy => "--config",
            SidecarKind::Dnsmasq => "-C",
            SidecarKind::Mailpit => "--db-file",
            SidecarKind::PhpFpm => "-y",
            SidecarKind::ProcessCompose => "-f",
        }
    }

    fn config_file(kind: SidecarKind) -> &'static str {
        match kind {
            SidecarKind::Caddy => "caddy.bootstrap.json",
            SidecarKind::Dnsmasq => "dnsmasq.conf",
            SidecarKind::Mailpit => "mailpit.db",
            // Version-agnostic dir prefix — see the production `config_file`.
            SidecarKind::PhpFpm => "php",
            SidecarKind::ProcessCompose => "process-compose.yaml",
        }
    }

    #[test]
    fn matches_only_portbay_caddy_never_servbay_or_homebrew() {
        let pids = matching_pids(&ps_fixture(), &sig(SidecarKind::Caddy), &[], SweepMode::All);
        assert_eq!(pids, vec![501], "must match PortBay's caddy only");
    }

    #[test]
    fn matches_only_portbay_dnsmasq_never_servbay() {
        let pids = matching_pids(
            &ps_fixture(),
            &sig(SidecarKind::Dnsmasq),
            &[],
            SweepMode::All,
        );
        assert_eq!(pids, vec![504], "ServBay dnsmasq (505) must not match");
    }

    #[test]
    fn matches_only_portbay_mailpit_never_homebrew() {
        let pids = matching_pids(
            &ps_fixture(),
            &sig(SidecarKind::Mailpit),
            &[],
            SweepMode::All,
        );
        assert_eq!(pids, vec![506], "homebrew mailpit (507) must not match");
    }

    #[test]
    fn matches_only_portbay_process_compose_never_foreign() {
        let pids = matching_pids(
            &ps_fixture(),
            &sig(SidecarKind::ProcessCompose),
            &[],
            SweepMode::All,
        );
        assert_eq!(
            pids,
            vec![508],
            "foreign process-compose (509) must not match"
        );
    }

    #[test]
    fn matches_only_portbay_php_fpm_never_foreign() {
        let pids = matching_pids(&ps_fixture(), &sig(SidecarKind::PhpFpm), &[], SweepMode::All);
        assert_eq!(
            pids,
            vec![510],
            "PortBay's php-fpm (510) must match; ServBay's (511) must not — \
             its pool config is outside our data dir"
        );
    }

    #[test]
    fn php_fpm_marker_is_version_agnostic() {
        // The `-y …/PortBay/php` prefix must catch a pool of ANY version, so a
        // stale 8.3 master gets reaped by the same signature that finds 8.4.
        let s = sig(SidecarKind::PhpFpm);
        assert!(s.matches(&format!(
            "{DATA}/runtimes/php/8.3.14/sbin/php-fpm -F -y {DATA}/php/8.3.14/php-fpm.conf"
        )));
    }

    #[test]
    fn orphans_only_respects_ppid() {
        // 506 (mailpit) has PPID 4242 (a live parent), so OrphansOnly skips it
        // even though it's PortBay-owned — we never reach into a live app's tree.
        let all = matching_pids(
            &ps_fixture(),
            &sig(SidecarKind::Mailpit),
            &[],
            SweepMode::All,
        );
        let orphans = matching_pids(
            &ps_fixture(),
            &sig(SidecarKind::Mailpit),
            &[],
            SweepMode::OrphansOnly,
        );
        assert_eq!(all, vec![506]);
        assert!(orphans.is_empty(), "live-parented mailpit is not an orphan");

        // 501 (caddy) IS orphaned (PPID 1) → OrphansOnly catches it.
        let caddy_orphans = matching_pids(
            &ps_fixture(),
            &sig(SidecarKind::Caddy),
            &[],
            SweepMode::OrphansOnly,
        );
        assert_eq!(caddy_orphans, vec![501]);
    }

    #[test]
    fn exclude_list_is_honoured() {
        let pids = matching_pids(
            &ps_fixture(),
            &sig(SidecarKind::Caddy),
            &[501],
            SweepMode::All,
        );
        assert!(pids.is_empty(), "excluded pid must be skipped");
    }

    #[test]
    fn signature_requires_both_name_and_config_marker() {
        let s = sig(SidecarKind::Caddy);
        // config marker present but a different binary → no match
        assert!(!s.matches(&format!("/usr/bin/grep {DATA}/caddy.bootstrap.json")));
        // caddy present but foreign config → no match
        assert!(!s.matches("/Applications/ServBay/bin/caddy run --config /x/caddy.json"));
        // both present → match
        assert!(s.matches(&format!(
            "/x/caddy-aarch64-apple-darwin run --config {DATA}/caddy.bootstrap.json"
        )));
    }

    #[test]
    fn config_markers_are_distinct_per_kind() {
        // Defensive: a caddy signature must not match a mailpit command line and
        // vice versa, even though both live under the same data dir.
        let caddy = sig(SidecarKind::Caddy);
        let mailpit_cmd = format!("/x/mailpit --db-file {DATA}/mailpit.db --smtp 127.0.0.1:1025");
        assert!(!caddy.matches(&mailpit_cmd));
    }

    #[test]
    fn canonical_ports_cover_the_incident_ports() {
        assert!(SidecarKind::Caddy.canonical_ports().contains(&443));
        assert!(SidecarKind::Mailpit.canonical_ports().contains(&1025));
        assert_eq!(SidecarKind::Caddy.rebind_gate_port(), Some(443));
        assert_eq!(SidecarKind::Mailpit.rebind_gate_port(), None);
    }
}
