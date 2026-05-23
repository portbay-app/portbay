//! Database engine catalogue, detection, provisioning, and supervision.
//!
//! PortBay owns the full lifecycle of database *instances*: it initializes
//! an isolated data directory, renders an engine config, and supervises the
//! daemon through Process Compose. This module is the engine-specific
//! knowledge layer — everything that differs between MySQL, MariaDB,
//! PostgreSQL, Redis, and MongoDB lives here so the command + reconciler
//! layers stay engine-agnostic.
//!
//! Binary resolution is Homebrew-prefix aware (handles custom prefixes like
//! `/Volumes/.../Homebrew`) and falls back to the login-shell PATH that
//! `runtimes::env::bootstrap_user_env` merges in at startup.
//!
//! Provisioning runs at instance *create* time, not in the reconcile tick:
//! `initdb` / `mysqld --initialize` are slow and one-shot, so we do them
//! once behind an idempotency guard and let the reconciler assume a ready
//! data directory.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::registry::{DatabaseEngine, DatabaseInstance};

/// Per-engine static metadata.
#[derive(Debug, Clone, Copy)]
struct EngineSpec {
    engine: DatabaseEngine,
    /// Homebrew formula opt-dir names, newest series first. Used to locate
    /// `<prefix>/opt/<formula>/bin/<binary>`.
    formulae: &'static [&'static str],
    /// Daemon binary candidates (the long-running server).
    daemons: &'static [&'static str],
    /// CLI client binary candidates.
    clients: &'static [&'static str],
    /// Init/bootstrap binary candidates (empty when the engine needs none).
    init_bins: &'static [&'static str],
}

const SPECS: &[EngineSpec] = &[
    EngineSpec {
        engine: DatabaseEngine::Mysql,
        formulae: &["mysql", "mysql@8.4", "mysql@8.0", "mysql@5.7"],
        daemons: &["mysqld"],
        clients: &["mysql"],
        init_bins: &[], // MySQL inits via `mysqld --initialize-insecure`
    },
    EngineSpec {
        engine: DatabaseEngine::Mariadb,
        formulae: &["mariadb"],
        daemons: &["mariadbd", "mysqld"],
        clients: &["mariadb", "mysql"],
        init_bins: &["mariadb-install-db", "mysql_install_db"],
    },
    EngineSpec {
        engine: DatabaseEngine::Postgres,
        formulae: &[
            "postgresql@17",
            "postgresql@16",
            "postgresql@15",
            "postgresql@14",
            "postgresql",
        ],
        daemons: &["postgres"],
        clients: &["psql"],
        init_bins: &["initdb"],
    },
    EngineSpec {
        engine: DatabaseEngine::Redis,
        formulae: &["redis"],
        daemons: &["redis-server"],
        clients: &["redis-cli"],
        init_bins: &[],
    },
    EngineSpec {
        engine: DatabaseEngine::Mongo,
        formulae: &["mongodb-community", "mongodb-community@7.0", "mongodb-community@6.0"],
        daemons: &["mongod"],
        clients: &["mongosh", "mongo"],
        init_bins: &[],
    },
];

fn spec(engine: DatabaseEngine) -> &'static EngineSpec {
    SPECS
        .iter()
        .find(|s| s.engine == engine)
        .expect("every DatabaseEngine variant has a spec")
}

// ===========================================================================
// Binary resolution
// ===========================================================================

/// Resolve the Homebrew prefix. Probes `brew --prefix`; falls back to the
/// two standard locations. Cached per-process would be nice but detection
/// is infrequent, so we keep it simple.
pub fn brew_prefix() -> Option<PathBuf> {
    if let Ok(brew) = which::which("brew") {
        if let Ok(out) = run_capture(&brew, &["--prefix"], Duration::from_secs(8)) {
            let p = PathBuf::from(out.trim());
            if p.exists() {
                return Some(p);
            }
        }
    }
    for guess in ["/opt/homebrew", "/usr/local"] {
        let p = PathBuf::from(guess);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn opt_bin_dirs(engine: DatabaseEngine, prefix: Option<&Path>) -> Vec<PathBuf> {
    let Some(prefix) = prefix else {
        return Vec::new();
    };
    spec(engine)
        .formulae
        .iter()
        .map(|f| prefix.join("opt").join(f).join("bin"))
        .collect()
}

/// Resolve a binary for an engine: search the engine's Homebrew opt `bin`
/// dirs first (handles keg-only formulae not on PATH), then the inherited
/// PATH. Returns the first existing match.
fn resolve_in(engine: DatabaseEngine, names: &[&str], prefix: Option<&Path>) -> Option<PathBuf> {
    let dirs = opt_bin_dirs(engine, prefix);
    for name in names {
        for dir in &dirs {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        if let Ok(p) = which::which(name) {
            return Some(p);
        }
    }
    None
}

/// Absolute path to the engine's daemon binary, or `None` if not installed.
pub fn daemon_binary(engine: DatabaseEngine) -> Option<PathBuf> {
    let prefix = brew_prefix();
    resolve_in(engine, spec(engine).daemons, prefix.as_deref())
}

/// Absolute path to the engine's CLI client, or `None`.
pub fn client_binary(engine: DatabaseEngine) -> Option<PathBuf> {
    let prefix = brew_prefix();
    resolve_in(engine, spec(engine).clients, prefix.as_deref())
}

fn init_binary(engine: DatabaseEngine, prefix: Option<&Path>) -> Option<PathBuf> {
    let spec = spec(engine);
    if spec.init_bins.is_empty() {
        return None;
    }
    resolve_in(engine, spec.init_bins, prefix)
}

// ===========================================================================
// Detection
// ===========================================================================

/// Result of probing the host for an engine.
#[derive(Debug, Clone)]
pub struct EngineDetection {
    pub installed: bool,
    pub version: String,
    pub daemon: Option<PathBuf>,
    pub client: Option<PathBuf>,
}

/// Probe an engine: is its daemon binary present, and what version?
pub fn detect(engine: DatabaseEngine) -> EngineDetection {
    let daemon = daemon_binary(engine);
    let client = client_binary(engine);
    let version = daemon
        .as_ref()
        .map(|b| probe_version(b))
        .unwrap_or_default();
    EngineDetection {
        installed: daemon.is_some(),
        version,
        daemon,
        client,
    }
}

fn probe_version(binary: &Path) -> String {
    let Ok(out) = run_capture(&binary.to_path_buf(), &["--version"], Duration::from_secs(3)) else {
        return String::new();
    };
    extract_version(&out)
}

/// Pull the first semver-ish token out of a `--version` line. Handles
/// `v7.0.8`, `v=7.2.4`, `(PostgreSQL) 16.2`, `Ver 8.4.0 …-MariaDB`.
pub fn extract_version(s: &str) -> String {
    for token in s.split(|c: char| c.is_whitespace() || c == ',') {
        let trimmed = token
            .strip_prefix("v=")
            .or_else(|| token.strip_prefix('v'))
            .unwrap_or(token)
            .trim_matches(|c: char| c == ')' || c == '(' || c == ';');
        if trimmed.contains('.')
            && trimmed.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
        {
            let clean: String = trimmed
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if clean.contains('.') {
                return clean;
            }
        }
    }
    String::new()
}

// ===========================================================================
// Instance paths
// ===========================================================================

/// Root directory PortBay owns for all database instances.
pub fn instances_root(app_data: &Path) -> PathBuf {
    app_data.join("databases")
}

/// The instance's own directory: `<app-data>/databases/<id>/`.
pub fn instance_dir(app_data: &Path, id: &str) -> PathBuf {
    instances_root(app_data).join(id)
}

/// The instance's data directory: `<instance-dir>/data`.
pub fn data_dir(app_data: &Path, id: &str) -> PathBuf {
    instance_dir(app_data, id).join("data")
}

/// Default config-file path for an engine instance.
pub fn config_path(engine: DatabaseEngine, app_data: &Path, id: &str) -> Option<PathBuf> {
    let dir = instance_dir(app_data, id);
    match engine {
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => Some(dir.join("my.cnf")),
        DatabaseEngine::Redis => Some(dir.join("redis.conf")),
        DatabaseEngine::Mongo => Some(dir.join("mongod.conf")),
        // PostgreSQL keeps postgresql.conf inside its data dir (initdb writes it).
        DatabaseEngine::Postgres => Some(data_dir(app_data, id).join("postgresql.conf")),
    }
}

/// Default unix socket path for an engine instance.
pub fn socket_path(engine: DatabaseEngine, app_data: &Path, id: &str) -> Option<PathBuf> {
    let dir = instance_dir(app_data, id);
    match engine {
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => Some(dir.join("mysql.sock")),
        DatabaseEngine::Redis => Some(dir.join("redis.sock")),
        DatabaseEngine::Mongo => Some(dir.join("mongod.sock")),
        // Postgres sockets live in a directory (-k), not a single file path.
        DatabaseEngine::Postgres => None,
    }
}

/// True when the data directory has already been initialized for this
/// engine. Cheap filesystem check — guards re-init.
pub fn is_initialized(engine: DatabaseEngine, data: &Path) -> bool {
    match engine {
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => data.join("mysql").is_dir(),
        DatabaseEngine::Postgres => data.join("PG_VERSION").is_file(),
        // Redis/Mongo need no schema init — an existing dir is enough.
        DatabaseEngine::Redis | DatabaseEngine::Mongo => data.is_dir(),
    }
}

// ===========================================================================
// Provisioning (create-time)
// ===========================================================================

/// Initialize an instance's data directory and write its config file.
///
/// Idempotent: skips init when the data dir is already initialized. The
/// `data` directory is created if missing. Returns the config path written
/// (when the engine uses one).
pub fn provision(
    engine: DatabaseEngine,
    daemon: &Path,
    app_data: &Path,
    id: &str,
    port: u16,
) -> Result<(), String> {
    let data = data_dir(app_data, id);
    std::fs::create_dir_all(&data)
        .map_err(|e| format!("create data dir {}: {e}", data.display()))?;

    let prefix = brew_prefix();

    if !is_initialized(engine, &data) {
        match engine {
            DatabaseEngine::Mysql => init_mysql(daemon, &data)?,
            DatabaseEngine::Mariadb => init_mariadb(engine, &data, prefix.as_deref())?,
            DatabaseEngine::Postgres => init_postgres(engine, &data, prefix.as_deref())?,
            DatabaseEngine::Redis | DatabaseEngine::Mongo => { /* dir is enough */ }
        }
    }

    write_config(engine, app_data, id, port)?;
    Ok(())
}

fn init_mysql(daemon: &Path, data: &Path) -> Result<(), String> {
    // basedir = <prefix>/opt/mysql (parent of the bin/ dir holding mysqld).
    // MySQL needs it to find share/ (english error messages, system schema).
    let basedir = daemon
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf());
    let datadir_arg = format!("--datadir={}", data.display());
    let mut args: Vec<String> = vec![
        "--no-defaults".into(),
        "--initialize-insecure".into(),
        datadir_arg,
    ];
    if let Some(base) = &basedir {
        args.push(format!("--basedir={}", base.display()));
    }
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_capture(&daemon.to_path_buf(), &arg_refs, Duration::from_secs(120))
        .map(|_| ())
        .map_err(|e| format!("mysqld --initialize-insecure failed: {}", truncate(&e, 800)))
}

fn init_mariadb(
    engine: DatabaseEngine,
    data: &Path,
    prefix: Option<&Path>,
) -> Result<(), String> {
    let init = init_binary(engine, prefix)
        .ok_or_else(|| "mariadb-install-db not found — reinstall MariaDB via Homebrew.".to_string())?;
    let datadir_arg = format!("--datadir={}", data.display());
    let args = vec![
        datadir_arg.as_str(),
        "--auth-root-authentication-method=normal",
        "--skip-test-db",
    ];
    run_capture(&init, &args, Duration::from_secs(120))
        .map(|_| ())
        .map_err(|e| format!("mariadb-install-db failed: {}", truncate(&e, 800)))
}

fn init_postgres(
    engine: DatabaseEngine,
    data: &Path,
    prefix: Option<&Path>,
) -> Result<(), String> {
    let initdb = init_binary(engine, prefix)
        .ok_or_else(|| "initdb not found — reinstall PostgreSQL via Homebrew.".to_string())?;
    let pgdata = format!("--pgdata={}", data.display());
    let args = vec![
        pgdata.as_str(),
        "--username=postgres",
        "--auth=trust",
        "--encoding=UTF8",
        "--no-locale",
    ];
    run_capture(&initdb, &args, Duration::from_secs(120))
        .map(|_| ())
        .map_err(|e| format!("initdb failed: {}", truncate(&e, 800)))
}

/// Render and write the engine config file for an instance.
fn write_config(
    engine: DatabaseEngine,
    app_data: &Path,
    id: &str,
    port: u16,
) -> Result<(), String> {
    let Some(cfg_path) = config_path(engine, app_data, id) else {
        return Ok(());
    };
    let data = data_dir(app_data, id);
    let sock = socket_path(engine, app_data, id);

    let body = match engine {
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => {
            let sock = sock.map(|s| s.display().to_string()).unwrap_or_default();
            format!(
                "[mysqld]\n\
                 datadir = {data}\n\
                 port = {port}\n\
                 socket = {sock}\n\
                 bind-address = 127.0.0.1\n\
                 mysqlx = OFF\n",
                data = data.display(),
            )
        }
        DatabaseEngine::Redis => {
            let sock = sock.map(|s| s.display().to_string()).unwrap_or_default();
            format!(
                "port {port}\n\
                 bind 127.0.0.1\n\
                 dir {data}\n\
                 unixsocket {sock}\n\
                 daemonize no\n",
                data = data.display(),
            )
        }
        DatabaseEngine::Mongo => {
            // mongod.conf is YAML.
            format!(
                "net:\n  bindIp: 127.0.0.1\n  port: {port}\nstorage:\n  dbPath: {data}\n",
                data = data.display(),
            )
        }
        DatabaseEngine::Postgres => {
            // postgresql.conf is generated by initdb; we don't overwrite it.
            // Port + socket dir are passed as run-time flags instead.
            return Ok(());
        }
    };

    if let Some(parent) = cfg_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create config dir {}: {e}", parent.display()))?;
    }
    std::fs::write(&cfg_path, body)
        .map_err(|e| format!("write config {}: {e}", cfg_path.display()))
}

// ===========================================================================
// Run command (supervision)
// ===========================================================================

/// Build the daemon launch command for an instance. The reconciler hands
/// this to Process Compose as the process `command`.
///
/// `daemon` is the resolved daemon binary; `app_data` lets us recompute
/// the config/socket/data paths deterministically (we don't trust the
/// stored ones blindly — the prefix may have moved).
pub fn run_command(instance: &DatabaseInstance, daemon: &Path, app_data: &Path) -> String {
    let id = instance.id.as_str();
    let bin = shell_quote(&daemon.to_string_lossy());
    let data = data_dir(app_data, id);
    match instance.engine {
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => {
            let cfg = config_path(instance.engine, app_data, id)
                .map(|p| shell_quote(&p.to_string_lossy()))
                .unwrap_or_default();
            format!("{bin} --defaults-file={cfg}")
        }
        DatabaseEngine::Postgres => {
            // Socket dir = instance dir; -c flags set the listen address.
            let sockdir = shell_quote(&instance_dir(app_data, id).to_string_lossy());
            let data_q = shell_quote(&data.to_string_lossy());
            format!(
                "{bin} -D {data_q} -p {port} -k {sockdir} -c listen_addresses=127.0.0.1",
                port = instance.port,
            )
        }
        DatabaseEngine::Redis => {
            let cfg = config_path(instance.engine, app_data, id)
                .map(|p| shell_quote(&p.to_string_lossy()))
                .unwrap_or_default();
            format!("{bin} {cfg}")
        }
        DatabaseEngine::Mongo => {
            let cfg = config_path(instance.engine, app_data, id)
                .map(|p| shell_quote(&p.to_string_lossy()))
                .unwrap_or_default();
            format!("{bin} --config {cfg}")
        }
    }
}

/// Build the CLI client invocation that the "Client" button runs in a
/// terminal, pointed at this instance's port.
pub fn client_invocation(instance: &DatabaseInstance, client: &Path) -> String {
    let c = client.to_string_lossy();
    let port = instance.port;
    match instance.engine {
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => {
            format!("{c} -u root -h 127.0.0.1 -P {port}")
        }
        DatabaseEngine::Postgres => format!("{c} -U postgres -h 127.0.0.1 -p {port} postgres"),
        DatabaseEngine::Mongo => format!("{c} mongodb://127.0.0.1:{port}"),
        DatabaseEngine::Redis => format!("{c} -h 127.0.0.1 -p {port}"),
    }
}

// ===========================================================================
// Subprocess helper
// ===========================================================================

/// Run a command synchronously with a hard timeout. Returns stdout on
/// success; on failure returns a combined stderr+stdout message (or
/// "timeout" / "spawn failed: …").
///
/// Blocking by design (it polls + sleeps). Async callers must invoke it
/// inside `tokio::task::spawn_blocking` so they don't stall the runtime.
pub(crate) fn run_capture(
    bin: &PathBuf,
    args: &[&str],
    timeout: Duration,
) -> Result<String, String> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn failed: {e}"))?;

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if started.elapsed() > timeout {
                    let _ = child.kill();
                    return Err("timeout".into());
                }
                std::thread::sleep(Duration::from_millis(80));
            }
            Err(e) => return Err(format!("wait failed: {e}")),
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("wait_with_output failed: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{}{}", stderr.trim(), stdout.trim());
        Err(if combined.is_empty() {
            format!("exit {}", output.status)
        } else {
            combined
        })
    }
}

/// Minimal POSIX single-quote shell-quoting for paths embedded in the PC
/// command string. PC runs the command through `/bin/sh -c`, so paths with
/// spaces must be quoted. We only ever quote our own app-data paths.
fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    if s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '@' | '='))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn truncate(s: &str, limit: usize) -> &str {
    if s.len() <= limit {
        s
    } else {
        &s[..limit]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{DatabaseInstance, DatabaseInstanceId};

    fn instance(engine: DatabaseEngine, port: u16) -> DatabaseInstance {
        DatabaseInstance {
            id: DatabaseInstanceId::new("myapp"),
            name: "myapp".into(),
            engine,
            version: "1.0".into(),
            port,
            data_dir: PathBuf::from("/tmp/pb/databases/myapp/data"),
            config_path: None,
            socket_path: None,
            auto_start: false,
            linked_projects: vec![],
        }
    }

    #[test]
    fn extract_version_handles_engine_formats() {
        assert_eq!(extract_version("mysqld  Ver 8.4.0 for macos"), "8.4.0");
        assert_eq!(extract_version("postgres (PostgreSQL) 16.2"), "16.2");
        assert_eq!(extract_version("db version v7.0.8"), "7.0.8");
        assert_eq!(extract_version("Redis server v=7.2.4 sha=0"), "7.2.4");
    }

    #[test]
    fn paths_are_namespaced_under_databases() {
        let app = Path::new("/tmp/pb");
        assert_eq!(data_dir(app, "x"), PathBuf::from("/tmp/pb/databases/x/data"));
        assert_eq!(
            config_path(DatabaseEngine::Mysql, app, "x").unwrap(),
            PathBuf::from("/tmp/pb/databases/x/my.cnf")
        );
        assert_eq!(
            config_path(DatabaseEngine::Postgres, app, "x").unwrap(),
            PathBuf::from("/tmp/pb/databases/x/data/postgresql.conf")
        );
    }

    #[test]
    fn mysql_run_command_uses_defaults_file() {
        let inst = instance(DatabaseEngine::Mysql, 3307);
        let cmd = run_command(&inst, Path::new("/opt/homebrew/opt/mysql/bin/mysqld"), Path::new("/tmp/pb"));
        assert!(cmd.contains("mysqld"));
        assert!(cmd.contains("--defaults-file="));
        assert!(cmd.contains("/tmp/pb/databases/myapp/my.cnf"));
    }

    #[test]
    fn postgres_run_command_sets_port_and_socket_dir() {
        let inst = instance(DatabaseEngine::Postgres, 5433);
        let cmd = run_command(&inst, Path::new("/usr/bin/postgres"), Path::new("/tmp/pb"));
        assert!(cmd.contains("-p 5433"));
        assert!(cmd.contains("-D /tmp/pb/databases/myapp/data"));
        assert!(cmd.contains("-k /tmp/pb/databases/myapp"));
        assert!(cmd.contains("listen_addresses=127.0.0.1"));
    }

    #[test]
    fn redis_run_command_points_at_conf() {
        let inst = instance(DatabaseEngine::Redis, 6380);
        let cmd = run_command(&inst, Path::new("/usr/bin/redis-server"), Path::new("/tmp/pb"));
        assert!(cmd.contains("redis-server"));
        assert!(cmd.contains("/tmp/pb/databases/myapp/redis.conf"));
    }

    #[test]
    fn mongo_run_command_uses_config_flag() {
        let inst = instance(DatabaseEngine::Mongo, 27018);
        let cmd = run_command(&inst, Path::new("/usr/bin/mongod"), Path::new("/tmp/pb"));
        assert!(cmd.contains("mongod --config"));
        assert!(cmd.contains("/tmp/pb/databases/myapp/mongod.conf"));
    }

    #[test]
    fn client_invocation_targets_instance_port() {
        let inst = instance(DatabaseEngine::Mysql, 3307);
        assert!(client_invocation(&inst, Path::new("/usr/bin/mysql")).contains("-P 3307"));
        let pg = instance(DatabaseEngine::Postgres, 5433);
        assert!(client_invocation(&pg, Path::new("/usr/bin/psql")).contains("-p 5433"));
    }

    #[test]
    fn shell_quote_wraps_paths_with_spaces() {
        assert_eq!(shell_quote("/tmp/pb/data"), "/tmp/pb/data");
        assert_eq!(shell_quote("/My Drive/db"), "'/My Drive/db'");
    }

    #[test]
    fn init_marker_paths_match_engines() {
        let data = Path::new("/x/data");
        // Just exercise the match arms compile + return false on a fake dir.
        assert!(!is_initialized(DatabaseEngine::Mysql, data));
        assert!(!is_initialized(DatabaseEngine::Postgres, data));
    }
}
