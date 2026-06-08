use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::preferences::AiPrefs;

#[derive(Default)]
pub struct OllamaManager {
    child: Option<Child>,
}

impl OllamaManager {
    pub fn new() -> Self {
        Self { child: None }
    }

    pub fn pid(&mut self) -> Option<u32> {
        self.reap_if_exited();
        self.child.as_ref().map(|child| child.id())
    }

    pub fn is_managed_running(&mut self) -> bool {
        self.pid().is_some()
    }

    pub fn set_child(&mut self, child: Child) {
        self.child = Some(child);
    }

    pub fn take_child(&mut self) -> Option<Child> {
        self.reap_if_exited();
        self.child.take()
    }

    fn reap_if_exited(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        if matches!(child.try_wait(), Ok(Some(_))) {
            self.child = None;
        }
    }
}

// --- Managed-ownership record --------------------------------------------
//
// `<data-dir>/PortBay/ollama-managed.json` — written when PortBay spawns
// `ollama serve`, so a server that outlives an app restart (or crash) is
// still recognised as OURS and stays stoppable. The pid is re-verified
// against a live `ollama` executable on every read; a recycled pid never
// matches.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedOllamaRecord {
    pub pid: u32,
    pub endpoint: String,
}

fn managed_record_path(logs_dir: &Path) -> PathBuf {
    logs_dir
        .parent()
        .unwrap_or(logs_dir)
        .join("ollama-managed.json")
}

pub fn write_managed_record(logs_dir: &Path, pid: u32, endpoint: &str) -> std::io::Result<()> {
    let path = managed_record_path(logs_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let record = ManagedOllamaRecord {
        pid,
        endpoint: endpoint.to_string(),
    };
    let bytes = serde_json::to_vec_pretty(&record).map_err(std::io::Error::other)?;
    std::fs::write(path, bytes)
}

pub fn remove_managed_record(logs_dir: &Path) -> std::io::Result<()> {
    let path = managed_record_path(logs_dir);
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// The recorded managed pid, when it still points at a live `ollama` process
/// serving `endpoint`. Endpoint mismatches don't match — a server we spawned
/// for an old endpoint config is not the one the page is looking at.
pub fn managed_record_pid(logs_dir: &Path, endpoint: &str) -> Option<u32> {
    let record = read_managed_record(logs_dir)?;
    (record.endpoint == endpoint && is_ollama_pid(record.pid)).then_some(record.pid)
}

/// The recorded managed pid regardless of endpoint — shutdown kills anything
/// PortBay spawned, even if the endpoint preference changed since.
pub fn managed_record_any_pid(logs_dir: &Path) -> Option<u32> {
    let record = read_managed_record(logs_dir)?;
    is_ollama_pid(record.pid).then_some(record.pid)
}

fn read_managed_record(logs_dir: &Path) -> Option<ManagedOllamaRecord> {
    let bytes = std::fs::read(managed_record_path(logs_dir)).ok()?;
    serde_json::from_slice::<ManagedOllamaRecord>(&bytes).ok()
}

/// Recent `is_ollama_pid` results, keyed by pid, with the time each was taken.
/// The underlying check refreshes the full `sysinfo` process table (20–80 ms),
/// which is far too heavy to run inline on every `ollama_overview` (it checks
/// twice) and the 5 s `ollama_running` poll — repeated on the shared async
/// worker pool, it's the starvation pattern that stalls other commands. A short
/// TTL collapses those repeated identical checks into a single scan.
static PID_ALIVE_CACHE: Lazy<Mutex<HashMap<u32, (bool, Instant)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// 1.5 s — under the 5 s poll, so liveness still tracks the process within one
/// cycle, while the twice-per-overview checks collapse to one scan.
const PID_ALIVE_TTL: Duration = Duration::from_millis(1_500);

/// Whether `pid` is a live `ollama` process. Cached for [`PID_ALIVE_TTL`] to
/// keep the expensive process-table scan off the hot async-poll path.
pub fn is_ollama_pid(pid: u32) -> bool {
    if let Ok(cache) = PID_ALIVE_CACHE.lock() {
        if let Some((alive, at)) = cache.get(&pid) {
            if at.elapsed() < PID_ALIVE_TTL {
                return *alive;
            }
        }
    }
    let alive = scan_is_ollama_pid(pid);
    if let Ok(mut cache) = PID_ALIVE_CACHE.lock() {
        // Drop expired entries so a churn of short-lived pids can't grow the
        // map without bound.
        cache.retain(|_, (_, at)| at.elapsed() < PID_ALIVE_TTL);
        cache.insert(pid, (alive, Instant::now()));
    }
    alive
}

/// The uncached process-table scan behind [`is_ollama_pid`].
fn scan_is_ollama_pid(pid: u32) -> bool {
    let mut system = sysinfo::System::new();
    system.refresh_processes();
    let Some(process) = system.process(sysinfo::Pid::from_u32(pid)) else {
        return false;
    };
    process
        .exe()
        .and_then(|exe| exe.file_name())
        .and_then(|name| name.to_str())
        .map(|name| name == "ollama")
        .unwrap_or_else(|| process.name() == "ollama")
}

#[cfg(unix)]
pub fn kill_pid(pid: u32) -> std::io::Result<()> {
    let r = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    if r == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
pub fn kill_pid(pid: u32) -> std::io::Result<()> {
    std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T"])
        .status()
        .map(|_| ())
}

/// Force-kill a pid (SIGKILL / `taskkill /F`). The escalation when a graceful
/// SIGTERM hasn't taken effect within the stop grace window — a server stuck
/// releasing GPU memory would otherwise be left running as an untracked orphan.
#[cfg(unix)]
pub fn kill_pid_force(pid: u32) -> std::io::Result<()> {
    let r = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    if r == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
pub fn kill_pid_force(pid: u32) -> std::io::Result<()> {
    std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()
        .map(|_| ())
}

#[cfg(unix)]
pub fn pid_alive(pid: u32) -> bool {
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
pub fn pid_alive(pid: u32) -> bool {
    is_ollama_pid(pid)
}

/// Synchronously stop everything PortBay spawned — the in-memory child and/or
/// the recorded orphan from a previous run — and drop the ownership record.
/// External servers are never touched here: this is the app-quit path, and
/// quitting PortBay must not take down a server some other app owns.
pub fn shutdown_managed(manager: &mut OllamaManager, logs_dir: &Path) {
    if let Some(mut child) = manager.take_child() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Some(pid) = managed_record_any_pid(logs_dir) {
        let _ = kill_pid(pid);
    }
    let _ = remove_managed_record(logs_dir);
}

// --- Binary resolution -------------------------------------------------------
//
// THE canonical "where is ollama?" answer, shared by the manager (Settings →
// AI, `ollama serve` spawn) and — in `tasks` builds — board dispatch
// (`context::launchers::resolve_agent`), so the two can never disagree about
// which Ollama is installed. Lives here rather than `context::launchers`
// because that module is `tasks`-gated and the manager ships in the OSS build.

/// A resolved Ollama binary; `from_pref` is true when the user's explicit
/// path setting won (surfaces as "custom path" in the integrations review).
pub struct ResolvedBinary {
    pub path: PathBuf,
    pub from_pref: bool,
}

/// Resolve the Ollama binary, most explicit channel first:
///   1. the user-set path (Settings → AI), tilde-expanded,
///   2. the PortBay-managed install (downloaded in-app from the signed
///      runtimes manifest, like PHP) — preferred over any system copy,
///   3. `$PATH` (a GUI-launched app's PATH is truncated, hence the rest),
///   4. the conventional install prefixes,
///   5. the official `Ollama.app` bundle's embedded CLI,
///   6. the running `ollama serve` process's own executable — Ollama is a
///      single multi-purpose binary (`serve`/`run`/`list`), so a live server
///      IS the CLI. This is what finds an install outside every conventional
///      prefix (e.g. an external drive) with zero configuration: nothing puts
///      it on `$PATH`, but the user has its server running.
pub fn resolve_binary(prefs: &AiPrefs) -> Option<ResolvedBinary> {
    let custom = prefs.binary_path.trim();
    if !custom.is_empty() {
        let path = PathBuf::from(expand_tilde(custom));
        if is_executable(&path) {
            return Some(ResolvedBinary {
                path,
                from_pref: true,
            });
        }
    }
    let resolved = detect_binary()?;
    // Remember every successful detection: a binary found ONLY through the
    // running serve process (external drive, never on PATH) would otherwise
    // become undetectable the moment that process stops — which bricked the
    // Start button right after a takeover Stop.
    remember_binary(&resolved);
    Some(ResolvedBinary {
        path: resolved,
        from_pref: false,
    })
}

fn detect_binary() -> Option<PathBuf> {
    if let Some(p) = managed_install_binary() {
        return Some(p);
    }
    if let Ok(p) = which::which("ollama") {
        return Some(p);
    }
    for p in ["/opt/homebrew/bin/ollama", "/usr/local/bin/ollama"] {
        let path = PathBuf::from(p);
        if is_executable(&path) {
            return Some(path);
        }
    }
    app_bundle_cli()
        .or_else(cached_binary)
        .or_else(running_serve_exe)
}

// --- Last-known-binary cache ----------------------------------------------
//
// `<data-dir>/PortBay/ollama-binary` — one line, the last path
// `resolve_binary` settled on. Lets a binary that is discoverable only
// while its server runs (resolution channel 6) survive that server
// stopping. Validated as executable on every read; a deleted or moved
// binary simply falls out of the chain.

fn binary_cache_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("PortBay/ollama-binary"))
}

fn remember_binary(path: &Path) {
    static LAST: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if last.as_deref() == Some(path) {
        return; // overview polls every 3s — don't rewrite an unchanged path
    }
    let Some(cache) = binary_cache_path() else {
        return;
    };
    if let Some(parent) = cache.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::write(&cache, path.to_string_lossy().as_bytes()).is_ok() {
        *last = Some(path.to_path_buf());
    }
}

fn cached_binary() -> Option<PathBuf> {
    let cache = binary_cache_path()?;
    let path = PathBuf::from(std::fs::read_to_string(cache).ok()?.trim());
    is_executable(&path).then_some(path)
}

/// The newest PortBay-managed Ollama install, if the user downloaded one
/// in-app. Same layout as every managed runtime:
/// `<data-dir>/PortBay/runtimes/ollama/<version>/bin/ollama`.
pub fn managed_install_binary() -> Option<PathBuf> {
    let root = dirs::data_dir()?.join("PortBay/runtimes/ollama");
    let mut versions: Vec<(String, PathBuf)> = std::fs::read_dir(&root)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            // Skip the installer's `.staging-*` work dirs.
            if name.starts_with('.') {
                return None;
            }
            let bin = entry.path().join("bin/ollama");
            is_executable(&bin).then_some((name, bin))
        })
        .collect();
    // Lexicographic is fine for x.y.z of same width; semver-aware ordering
    // matters little here because installs replace each other in practice.
    versions.sort_by(|a, b| b.0.cmp(&a.0));
    versions.into_iter().next().map(|(_, bin)| bin)
}

/// The CLI the official `Ollama.app` embeds — installs that never symlinked
/// it into `/usr/local/bin` resolve through here.
fn app_bundle_cli() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join("Applications"));
        }
        for root in roots {
            let cand = root.join("Ollama.app/Contents/Resources/ollama");
            if is_executable(&cand) {
                return Some(cand);
            }
        }
    }
    None
}

/// The executable of an already-running `ollama` process, if one is up.
fn running_serve_exe() -> Option<PathBuf> {
    let mut system = sysinfo::System::new();
    system.refresh_processes();
    system.processes().values().find_map(|p| {
        let exe = p.exe()?;
        (exe.file_name()?.to_str()? == "ollama" && is_executable(exe)).then(|| exe.to_path_buf())
    })
}

pub(crate) fn expand_tilde(value: &str) -> String {
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    value.to_string()
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}
#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}
