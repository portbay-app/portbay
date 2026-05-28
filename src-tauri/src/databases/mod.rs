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
use std::sync::OnceLock;
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
        engine: DatabaseEngine::Memcached,
        formulae: &["memcached"],
        daemons: &["memcached"],
        clients: &[],
        init_bins: &[],
    },
    EngineSpec {
        engine: DatabaseEngine::Mongo,
        formulae: &[
            "mongodb-community",
            "mongodb-community@7.0",
            "mongodb-community@6.0",
        ],
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
/// two standard locations. Memoised for the process lifetime — `brew --prefix`
/// forks a subprocess, and `list_database_instances` would otherwise re-run it
/// several times per status tick.
pub fn brew_prefix() -> Option<PathBuf> {
    static CACHE: OnceLock<Option<PathBuf>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
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
        })
        .clone()
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

// ===========================================================================
// Managed-engine resolution (PortBay-managed install wins over Homebrew)
// ===========================================================================
//
// A PortBay-managed engine is installed under
// `<app-data>/database-engines/<engine>/<version>/`, with every binary
// (daemon, client, init helper) under `<dir>/bin/`. When such an install
// exists, its `bin` dir is searched before the Homebrew opt dirs / PATH, so
// an engine installed through PortBay is used in preference to any system copy
// — the engine lives inside the PortBay environment without being bundled.

/// Search `managed_bin` (when set) for the first matching name, then fall back
/// to the Homebrew/system resolution.
fn resolve_preferring_managed(
    engine: DatabaseEngine,
    names: &[&str],
    managed_bin: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(dir) = managed_bin {
        for name in names {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    resolve_in(engine, names, brew_prefix().as_deref())
}

/// Daemon binary, preferring a PortBay-managed install at `managed_bin`.
pub fn daemon_binary_resolved(
    engine: DatabaseEngine,
    managed_bin: Option<&Path>,
) -> Option<PathBuf> {
    resolve_preferring_managed(engine, spec(engine).daemons, managed_bin)
}

/// CLI client binary, preferring a PortBay-managed install at `managed_bin`.
pub fn client_binary_resolved(
    engine: DatabaseEngine,
    managed_bin: Option<&Path>,
) -> Option<PathBuf> {
    resolve_preferring_managed(engine, spec(engine).clients, managed_bin)
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
/// Considers Homebrew/system installs only.
pub fn detect(engine: DatabaseEngine) -> EngineDetection {
    detect_resolved(engine, None)
}

/// Like [`detect`], but searches a PortBay-managed install at `managed_bin`
/// first. A managed engine reports as installed even when no system copy exists.
pub fn detect_resolved(engine: DatabaseEngine, managed_bin: Option<&Path>) -> EngineDetection {
    let daemon = daemon_binary_resolved(engine, managed_bin);
    let client = client_binary_resolved(engine, managed_bin);

    // Probe the daemon's raw `--version` once — needed both to extract the
    // numeric version and to disambiguate the MySQL/MariaDB pair, which share
    // the `mysqld` binary name. Without this, when only one of the two is
    // installed, `resolve_in`'s global-PATH fallback lets the *other* engine
    // claim that `mysqld` (e.g. MariaDB reported as installed at MySQL's version).
    let raw = daemon
        .as_ref()
        .map(|b| probe_version_raw(b))
        .unwrap_or_default();

    if daemon_identity_matches(engine, &raw) {
        EngineDetection {
            installed: daemon.is_some(),
            version: extract_version(&raw),
            daemon,
            client,
        }
    } else {
        // The resolved daemon belongs to the other engine — report this one as
        // not installed (its client resolution is cross-contaminated too).
        EngineDetection {
            installed: false,
            version: String::new(),
            daemon: None,
            client: None,
        }
    }
}

/// Raw `--version` output for a daemon binary; empty string on failure.
fn probe_version_raw(binary: &Path) -> String {
    run_capture(
        &binary.to_path_buf(),
        &["--version"],
        Duration::from_secs(3),
    )
    .unwrap_or_default()
}

/// Disambiguate the MySQL/MariaDB pair from a daemon's raw `--version` output.
/// They share the `mysqld` binary name; MariaDB's version string contains
/// "MariaDB", MySQL's does not. Returns true when `raw` is consistent with
/// `engine` — and, conservatively, when `raw` is empty (keep an unverifiable
/// install rather than hide a real one). Engines with unique daemon binaries
/// always match.
fn daemon_identity_matches(engine: DatabaseEngine, raw: &str) -> bool {
    let looks_like_mariadb = raw.to_lowercase().contains("mariadb");
    match engine {
        DatabaseEngine::Mariadb => raw.is_empty() || looks_like_mariadb,
        DatabaseEngine::Mysql => raw.is_empty() || !looks_like_mariadb,
        _ => true,
    }
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
            && trimmed
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
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

/// Root directory PortBay owns for managed engine *binaries* (distinct from
/// instance data): `<app-data>/database-engines/`.
pub fn engines_root(app_data: &Path) -> PathBuf {
    app_data.join("database-engines")
}

/// Install dir for a managed engine build:
/// `<app-data>/database-engines/<engine>/<version>/`.
pub fn managed_engine_dir(app_data: &Path, engine: DatabaseEngine, version: &str) -> PathBuf {
    engines_root(app_data).join(engine.id()).join(version)
}

/// The `bin` dir inside a managed engine install root.
pub fn managed_bin_dir(install_dir: &Path) -> PathBuf {
    install_dir.join("bin")
}

/// Relative path of the daemon binary inside a managed engine archive — the
/// layout the portbay-runtimes build must produce (every binary under `bin/`).
/// Used as the `expected_binary_rel` the installer validates after extraction.
pub fn expected_daemon_rel(engine: DatabaseEngine) -> PathBuf {
    PathBuf::from("bin").join(spec(engine).daemons[0])
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
        DatabaseEngine::Memcached => None,
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
        DatabaseEngine::Memcached => None,
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
        // Redis/Mongo/Memcached need no schema init - an existing dir is enough.
        DatabaseEngine::Redis | DatabaseEngine::Mongo | DatabaseEngine::Memcached => data.is_dir(),
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
    managed_bin: Option<&Path>,
) -> Result<(), String> {
    let data = data_dir(app_data, id);
    std::fs::create_dir_all(&data)
        .map_err(|e| format!("create data dir {}: {e}", data.display()))?;

    if !is_initialized(engine, &data) {
        match engine {
            DatabaseEngine::Mysql => init_mysql(daemon, &data)?,
            DatabaseEngine::Mariadb => init_mariadb(engine, &data, managed_bin)?,
            DatabaseEngine::Postgres => init_postgres(engine, &data, managed_bin)?,
            DatabaseEngine::Redis | DatabaseEngine::Mongo | DatabaseEngine::Memcached => {
                /* dir is enough */
            }
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
    managed_bin: Option<&Path>,
) -> Result<(), String> {
    let init = resolve_preferring_managed(engine, spec(engine).init_bins, managed_bin)
        .ok_or_else(|| "mariadb-install-db not found — install MariaDB first.".to_string())?;
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
    managed_bin: Option<&Path>,
) -> Result<(), String> {
    let initdb = resolve_preferring_managed(engine, spec(engine).init_bins, managed_bin)
        .ok_or_else(|| "initdb not found — install PostgreSQL first.".to_string())?;
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
            // `dir`/`unixsocket` values MUST be double-quoted: Redis splits config
            // values on whitespace, and the data dir lives under
            // `~/Library/Application Support/PortBay/…` (a space), which would
            // otherwise truncate the path. Redis supports quoted string values.
            format!(
                "port {port}\n\
                 bind 127.0.0.1\n\
                 dir \"{data}\"\n\
                 unixsocket \"{sock}\"\n\
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
        DatabaseEngine::Memcached => return Ok(()),
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
    std::fs::write(&cfg_path, body).map_err(|e| format!("write config {}: {e}", cfg_path.display()))
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
        DatabaseEngine::Memcached => {
            format!(
                "{bin} -l 127.0.0.1 -p {port} -U 0 -vv",
                port = instance.port
            )
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
        DatabaseEngine::Memcached => format!("nc 127.0.0.1 {port}"),
    }
}

// ===========================================================================
// Per-database (schema) management
// ===========================================================================
//
// Lists and creates/drops the *databases* (schemas) inside a running instance,
// by running one-shot queries through the engine's CLI client. Only the SQL
// engines expose a meaningful schema namespace — Redis (numbered DBs),
// Memcached (no schemas), and Mongo (created on first write) are excluded.

/// Whether per-database create/drop/list applies to this engine.
pub fn supports_schema_management(engine: DatabaseEngine) -> bool {
    matches!(
        engine,
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb | DatabaseEngine::Postgres
    )
}

/// A safe SQL identifier: a name we can interpolate into `CREATE DATABASE` /
/// `DROP DATABASE` without injection risk. Conservative on purpose — letters,
/// digits, underscores; must start with a letter or underscore; ≤ 64 chars.
fn validate_identifier(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic() || c == '_')
            .unwrap_or(false)
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "invalid database name `{name}` — use letters, digits, and underscores (starting with a letter or underscore)."
        ))
    }
}

/// System schemas hidden from the per-instance database list.
fn is_system_schema(engine: DatabaseEngine, name: &str) -> bool {
    match engine {
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => matches!(
            name,
            "information_schema" | "mysql" | "performance_schema" | "sys"
        ),
        DatabaseEngine::Postgres => matches!(name, "template0" | "template1"),
        _ => false,
    }
}

/// List the user databases/schemas in a running instance.
pub fn list_schemas(instance: &DatabaseInstance, client: &Path) -> Result<Vec<String>, String> {
    let port = instance.port.to_string();
    let args: Vec<&str> = match instance.engine {
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => vec![
            "-N",
            "-B",
            "-h",
            "127.0.0.1",
            "-P",
            &port,
            "-u",
            "root",
            "-e",
            "SHOW DATABASES",
        ],
        DatabaseEngine::Postgres => vec![
            "-h",
            "127.0.0.1",
            "-p",
            &port,
            "-U",
            "postgres",
            "-tAc",
            "SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname",
        ],
        _ => {
            return Err(format!(
                "listing databases isn't supported for {}.",
                instance.engine.label()
            ))
        }
    };
    let out = run_capture(&client.to_path_buf(), &args, Duration::from_secs(10))?;
    Ok(out
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !is_system_schema(instance.engine, l))
        .collect())
}

/// Create a database/schema in a running instance.
pub fn create_schema(instance: &DatabaseInstance, client: &Path, name: &str) -> Result<(), String> {
    validate_identifier(name)?;
    let port = instance.port.to_string();
    let sql;
    let args: Vec<&str> = match instance.engine {
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => {
            sql = format!("CREATE DATABASE `{name}`");
            vec!["-h", "127.0.0.1", "-P", &port, "-u", "root", "-e", &sql]
        }
        DatabaseEngine::Postgres => {
            sql = format!("CREATE DATABASE \"{name}\"");
            vec!["-h", "127.0.0.1", "-p", &port, "-U", "postgres", "-c", &sql]
        }
        _ => {
            return Err(format!(
                "creating databases isn't supported for {}.",
                instance.engine.label()
            ))
        }
    };
    run_capture(&client.to_path_buf(), &args, Duration::from_secs(15)).map(|_| ())
}

/// Drop a database/schema from a running instance.
pub fn drop_schema(instance: &DatabaseInstance, client: &Path, name: &str) -> Result<(), String> {
    validate_identifier(name)?;
    let port = instance.port.to_string();
    let sql;
    let args: Vec<&str> = match instance.engine {
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => {
            sql = format!("DROP DATABASE `{name}`");
            vec!["-h", "127.0.0.1", "-P", &port, "-u", "root", "-e", &sql]
        }
        DatabaseEngine::Postgres => {
            sql = format!("DROP DATABASE \"{name}\"");
            vec!["-h", "127.0.0.1", "-p", &port, "-U", "postgres", "-c", &sql]
        }
        _ => {
            return Err(format!(
                "dropping databases isn't supported for {}.",
                instance.engine.label()
            ))
        }
    };
    run_capture(&client.to_path_buf(), &args, Duration::from_secs(15)).map(|_| ())
}

// ===========================================================================
// Per-project provisioning (dedicated database + user)
// ===========================================================================
//
// Creates a project-owned database and a login user with a caller-supplied
// (random, generated client-side with Web Crypto) password, then the command
// layer injects DB_* into the project's env. SQL engines only.

/// Turn an arbitrary project id/slug into a safe SQL identifier base: lowercase,
/// non-alphanumerics collapsed to `_`, guaranteed to start with a letter, capped.
pub fn sanitize_identifier(raw: &str) -> String {
    let mut s: String = raw
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    s.truncate(50);
    let needs_prefix = s
        .chars()
        .next()
        .map(|c| !(c.is_ascii_alphabetic() || c == '_'))
        .unwrap_or(true);
    if needs_prefix {
        s = format!("db_{s}");
    }
    s
}

/// A safe provisioning password: alphanumeric, 8–128 chars. The frontend
/// generates a strong random one; the alphanumeric constraint also means it
/// needs no quoting/escaping inside SQL string literals or connection URLs.
fn validate_password(pw: &str) -> Result<(), String> {
    if (8..=128).contains(&pw.len()) && pw.chars().all(|c| c.is_ascii_alphanumeric()) {
        Ok(())
    } else {
        Err("database password must be 8–128 alphanumeric characters.".to_string())
    }
}

/// Provision (idempotently) a dedicated database + login user on a running
/// instance. MySQL/MariaDB/PostgreSQL only.
pub fn provision_app_database(
    instance: &DatabaseInstance,
    client: &Path,
    database: &str,
    username: &str,
    password: &str,
) -> Result<(), String> {
    validate_identifier(database)?;
    validate_identifier(username)?;
    validate_password(password)?;
    let port = instance.port.to_string();
    match instance.engine {
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => {
            // One round-trip; identifiers are validated, password is alphanumeric.
            let sql = [
                format!("CREATE DATABASE IF NOT EXISTS `{database}`"),
                format!("CREATE USER IF NOT EXISTS '{username}'@'%' IDENTIFIED BY '{password}'"),
                format!("ALTER USER '{username}'@'%' IDENTIFIED BY '{password}'"),
                format!("GRANT ALL PRIVILEGES ON `{database}`.* TO '{username}'@'%'"),
                "FLUSH PRIVILEGES".to_string(),
            ]
            .join("; ");
            let args = vec!["-h", "127.0.0.1", "-P", &port, "-u", "root", "-e", &sql];
            run_capture(&client.to_path_buf(), &args, Duration::from_secs(20)).map(|_| ())
        }
        DatabaseEngine::Postgres => {
            // Role: create-or-update its password idempotently.
            let role = format!(
                "DO $$ BEGIN \
                   IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = '{username}') THEN \
                     CREATE ROLE \"{username}\" LOGIN PASSWORD '{password}'; \
                   ELSE \
                     ALTER ROLE \"{username}\" LOGIN PASSWORD '{password}'; \
                   END IF; \
                 END $$;"
            );
            run_pg(client, &port, &role)?;
            // CREATE DATABASE can't run in a DO block; tolerate "already exists".
            let _ = run_pg(
                client,
                &port,
                &format!("CREATE DATABASE \"{database}\" OWNER \"{username}\""),
            );
            run_pg(
                client,
                &port,
                &format!("GRANT ALL PRIVILEGES ON DATABASE \"{database}\" TO \"{username}\""),
            )
            .map(|_| ())
        }
        _ => Err(format!(
            "per-project provisioning isn't supported for {}.",
            instance.engine.label()
        )),
    }
}

fn run_pg(client: &Path, port: &str, sql: &str) -> Result<String, String> {
    let args = vec![
        "-h",
        "127.0.0.1",
        "-p",
        port,
        "-U",
        "postgres",
        "-v",
        "ON_ERROR_STOP=1",
        "-c",
        sql,
    ];
    run_capture(&client.to_path_buf(), &args, Duration::from_secs(20))
}

/// A `DATABASE_URL` for a provisioned project database (credentials inline).
pub fn app_connection_url(
    instance: &DatabaseInstance,
    username: &str,
    password: &str,
    database: &str,
) -> String {
    let port = instance.port;
    match instance.engine {
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => {
            format!("mysql://{username}:{password}@127.0.0.1:{port}/{database}")
        }
        DatabaseEngine::Postgres => {
            format!("postgresql://{username}:{password}@127.0.0.1:{port}/{database}")
        }
        _ => String::new(),
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
    if s.chars()
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

    #[test]
    fn mariadb_and_mysql_daemon_identity_is_disambiguated() {
        // Real-world `--version` lines from each engine's daemon.
        let mysql = "/opt/homebrew/opt/mysql/bin/mysqld  Ver 8.4.9 for macos15 on arm64 (Homebrew)";
        let mariadb =
            "/opt/homebrew/opt/mariadb/bin/mariadbd  Ver 11.4.3-MariaDB Source distribution";
        // Older MariaDB ships its daemon as `mysqld` but still reports MariaDB.
        let mariadb_as_mysqld = "mysqld  Ver 10.11.6-MariaDB on macos";

        // MariaDB must accept only MariaDB version strings — never MySQL's mysqld.
        assert!(daemon_identity_matches(DatabaseEngine::Mariadb, mariadb));
        assert!(daemon_identity_matches(
            DatabaseEngine::Mariadb,
            mariadb_as_mysqld
        ));
        assert!(
            !daemon_identity_matches(DatabaseEngine::Mariadb, mysql),
            "MariaDB must not claim MySQL's mysqld"
        );

        // MySQL must reject MariaDB's daemon (the reverse contamination).
        assert!(daemon_identity_matches(DatabaseEngine::Mysql, mysql));
        assert!(
            !daemon_identity_matches(DatabaseEngine::Mysql, mariadb),
            "MySQL must not claim MariaDB's daemon"
        );

        // Empty probe output (e.g. --version failed) is kept conservatively.
        assert!(daemon_identity_matches(DatabaseEngine::Mariadb, ""));
        assert!(daemon_identity_matches(DatabaseEngine::Mysql, ""));

        // Engines with unique daemon binaries always match.
        assert!(daemon_identity_matches(
            DatabaseEngine::Postgres,
            "postgres (PostgreSQL) 16.2"
        ));
        assert!(daemon_identity_matches(
            DatabaseEngine::Redis,
            "Redis server v=7.2.6"
        ));
    }

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
        assert_eq!(
            data_dir(app, "x"),
            PathBuf::from("/tmp/pb/databases/x/data")
        );
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
        let cmd = run_command(
            &inst,
            Path::new("/opt/homebrew/opt/mysql/bin/mysqld"),
            Path::new("/tmp/pb"),
        );
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
        let cmd = run_command(
            &inst,
            Path::new("/usr/bin/redis-server"),
            Path::new("/tmp/pb"),
        );
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
    fn memcached_run_command_targets_instance_port() {
        let inst = instance(DatabaseEngine::Memcached, 11212);
        let cmd = run_command(&inst, Path::new("/usr/bin/memcached"), Path::new("/tmp/pb"));
        assert!(cmd.contains("memcached"));
        assert!(cmd.contains("-l 127.0.0.1"));
        assert!(cmd.contains("-p 11212"));
    }

    #[test]
    fn client_invocation_targets_instance_port() {
        let inst = instance(DatabaseEngine::Mysql, 3307);
        assert!(client_invocation(&inst, Path::new("/usr/bin/mysql")).contains("-P 3307"));
        let pg = instance(DatabaseEngine::Postgres, 5433);
        assert!(client_invocation(&pg, Path::new("/usr/bin/psql")).contains("-p 5433"));
        let memcached = instance(DatabaseEngine::Memcached, 11212);
        assert_eq!(
            client_invocation(&memcached, Path::new("/usr/bin/nc")),
            "nc 127.0.0.1 11212"
        );
    }

    #[test]
    fn shell_quote_wraps_paths_with_spaces() {
        assert_eq!(shell_quote("/tmp/pb/data"), "/tmp/pb/data");
        assert_eq!(shell_quote("/My Drive/db"), "'/My Drive/db'");
    }

    #[test]
    fn expected_daemon_rel_lives_under_bin() {
        assert_eq!(
            expected_daemon_rel(DatabaseEngine::Mysql),
            PathBuf::from("bin/mysqld")
        );
        assert_eq!(
            expected_daemon_rel(DatabaseEngine::Postgres),
            PathBuf::from("bin/postgres")
        );
        assert_eq!(
            expected_daemon_rel(DatabaseEngine::Redis),
            PathBuf::from("bin/redis-server")
        );
    }

    #[test]
    fn managed_engine_dir_is_namespaced_by_engine_and_version() {
        let app = Path::new("/tmp/pb");
        assert_eq!(
            managed_engine_dir(app, DatabaseEngine::Postgres, "16.2"),
            PathBuf::from("/tmp/pb/database-engines/postgres/16.2")
        );
        assert_eq!(
            managed_bin_dir(&managed_engine_dir(app, DatabaseEngine::Redis, "7.4.0")),
            PathBuf::from("/tmp/pb/database-engines/redis/7.4.0/bin")
        );
    }

    #[test]
    fn resolve_prefers_managed_install_over_system() {
        // A binary in the managed bin dir wins, without consulting Homebrew/PATH.
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let mysqld = bin.join("mysqld");
        std::fs::write(&mysqld, b"#!/bin/sh\n").unwrap();
        let resolved = daemon_binary_resolved(DatabaseEngine::Mysql, Some(&bin));
        assert_eq!(resolved.as_deref(), Some(mysqld.as_path()));
    }

    #[test]
    fn validate_identifier_rejects_injection_and_bad_names() {
        // Valid
        assert!(validate_identifier("myapp").is_ok());
        assert!(validate_identifier("my_app_dev").is_ok());
        assert!(validate_identifier("_internal").is_ok());
        // Invalid — injection / shell / SQL metacharacters and edge cases
        assert!(validate_identifier("").is_err());
        assert!(validate_identifier("1abc").is_err()); // leading digit
        assert!(validate_identifier("a; DROP DATABASE x").is_err());
        assert!(validate_identifier("a`b").is_err());
        assert!(validate_identifier("a\"b").is_err());
        assert!(validate_identifier("a-b").is_err());
        assert!(validate_identifier("a b").is_err());
        assert!(validate_identifier(&"x".repeat(65)).is_err()); // too long
    }

    #[test]
    fn schema_management_only_for_sql_engines() {
        assert!(supports_schema_management(DatabaseEngine::Mysql));
        assert!(supports_schema_management(DatabaseEngine::Mariadb));
        assert!(supports_schema_management(DatabaseEngine::Postgres));
        assert!(!supports_schema_management(DatabaseEngine::Redis));
        assert!(!supports_schema_management(DatabaseEngine::Mongo));
        assert!(!supports_schema_management(DatabaseEngine::Memcached));
    }

    #[test]
    fn sanitize_identifier_produces_safe_sql_names() {
        assert_eq!(sanitize_identifier("my-app"), "my_app");
        assert_eq!(sanitize_identifier("My App 2"), "my_app_2");
        assert_eq!(sanitize_identifier("123start"), "db_123start"); // leading digit
        assert_eq!(sanitize_identifier(""), "db_"); // empty → prefixed
                                                    // Every result is a valid identifier.
        for raw in ["my-app", "My App 2", "123start", "weird!!name", ""] {
            assert!(validate_identifier(&sanitize_identifier(raw)).is_ok());
        }
    }

    #[test]
    fn validate_password_requires_alnum_and_length() {
        assert!(validate_password("abc12345").is_ok());
        assert!(validate_password("short7").is_err()); // < 8
        assert!(validate_password("has space12").is_err());
        assert!(validate_password("has'quote12").is_err());
        assert!(validate_password(&"a".repeat(129)).is_err());
    }

    #[test]
    fn app_connection_url_embeds_credentials_per_engine() {
        let mysql = instance(DatabaseEngine::Mysql, 3307);
        assert_eq!(
            app_connection_url(&mysql, "u", "pw", "u_dev"),
            "mysql://u:pw@127.0.0.1:3307/u_dev"
        );
        let pg = instance(DatabaseEngine::Postgres, 5433);
        assert_eq!(
            app_connection_url(&pg, "u", "pw", "u_dev"),
            "postgresql://u:pw@127.0.0.1:5433/u_dev"
        );
    }

    #[test]
    fn init_marker_paths_match_engines() {
        let data = Path::new("/x/data");
        // Just exercise the match arms compile + return false on a fake dir.
        assert!(!is_initialized(DatabaseEngine::Mysql, data));
        assert!(!is_initialized(DatabaseEngine::Postgres, data));
    }
}
