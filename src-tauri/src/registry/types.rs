use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A short, stable, URL-friendly identifier for a project.
///
/// IDs are also used as `@id` values on Caddy routes and as process names
/// inside Process Compose's YAML, so they must round-trip through HTTP
/// paths and YAML keys cleanly. We don't enforce a regex at this layer —
/// the CLI normalises user input before constructing one.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectId(String);

impl ProjectId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ProjectId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for ProjectId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// The kinds of projects PortBay knows how to launch.
///
/// Unknown / user-supplied launch commands go under `Custom`. We deliberately
/// keep this small in v1; new variants are cheap to add later.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectType {
    Next,
    Vite,
    Php,
    Static,
    Node,
    Custom,
}

/// How PortBay decides a project is "actually serving" rather than just
/// "the process is alive."
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Readiness {
    /// HTTP GET against a path. The most common case for Next, Vite, PHP.
    Http {
        path: String,
        #[serde(default = "default_readiness_timeout")]
        timeout_seconds: u32,
    },
    /// Plain TCP connect — for projects without an HTTP layer.
    Tcp {
        #[serde(default = "default_readiness_timeout")]
        timeout_seconds: u32,
    },
    /// Trust the process — readiness == is_running. Honest about its limits.
    Process,
}

fn default_readiness_timeout() -> u32 {
    75
}

/// A project that PortBay manages.
///
/// JSON field naming intentionally matches the example in
/// `ASSESSMENT_AND_PLAN.md` §7.1 so the doc and the code don't drift.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub path: PathBuf,

    #[serde(rename = "type")]
    pub kind: ProjectType,

    /// Shell command launched by Process Compose for this project's main
    /// dev server. `None` means "service-only" — e.g. a static-file PHP
    /// project that's served entirely by Caddy + PHP-FPM, no separate
    /// dev-server process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_command: Option<String>,

    /// The primary HTTP port the dev server binds to. `None` for projects
    /// served only via Caddy (php_fpm, file_server).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,

    /// Additional ports owned by this project (Vite + API split, multi-port
    /// apps, etc.). PortBay reserves these in the conflict checker.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_ports: Vec<u16>,

    /// The local hostname Caddy routes to this project. Already includes
    /// the domain suffix (e.g. `marketing-site.test`).
    pub hostname: String,

    /// Whether Caddy should terminate TLS for this hostname using a
    /// mkcert-issued certificate.
    #[serde(default)]
    pub https: bool,

    /// Shared services the project depends on (e.g. `["caddy", "php-fpm", "mysql"]`).
    /// Resolved against the built-in service catalogue at launch time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<String>,

    /// Environment variables passed to the dev server.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,

    /// How PortBay decides this project is ready to receive traffic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness: Option<Readiness>,

    /// If true, PortBay starts this project automatically when the daemon
    /// comes up. If false, the user must press Play.
    #[serde(default)]
    pub auto_start: bool,

    /// User-supplied tags for filtering / grouping in the UI.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    // ----- PHP-specific (optional) --------------------------------------
    /// For `type: "php"` projects, the document root relative to `path`
    /// (commonly `"public"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_root: Option<String>,

    /// PHP version label to bind to (e.g. `"8.3"`). PHP-FPM service
    /// resolution uses this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub php_version: Option<String>,

    // ----- Runtime selection (schema v2+) -------------------------------
    /// Pinned language runtime — which language toolchain and version
    /// PortBay launches this project with. Introduced in registry schema v2;
    /// migrated v1 registries derive it from the legacy `php_version` (see
    /// [`crate::registry::migrate`]). `None` means "fall back to the project
    /// type's default runtime resolution." Kept alongside `php_version`
    /// through the transition — existing consumers still read `php_version`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<Runtime>,
}

impl Project {
    /// The PHP version this project should be served with, or `None` when it
    /// isn't a PHP project. Prefers the structured [`Project::runtime`] pin
    /// (the v2+ source of truth) and falls back to the legacy
    /// [`Project::php_version`] field for projects that predate it (imported
    /// sites, un-migrated registries).
    ///
    /// Both the Caddy FastCGI route and the FPM-pool reconciler resolve the
    /// version through this one method, so they can never dial a socket the
    /// other side didn't spawn. A project carrying a non-PHP `runtime` pin
    /// returns `None` — it explicitly targets another toolchain.
    pub fn php_version_effective(&self) -> Option<&str> {
        match &self.runtime {
            Some(rt) if rt.lang == "php" => Some(rt.version.as_str()),
            Some(_) => None,
            None => self.php_version.as_deref(),
        }
    }
}

/// A pinned language runtime for a project: which language toolchain and
/// which version to launch it with. See [`Project::runtime`].
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Runtime {
    /// Stable language id, matching a
    /// [`LanguageRuntime::id`](crate::runtimes::LanguageRuntime::id)
    /// (`"php"`, `"node"`, `"python"`, …).
    pub lang: String,
    /// Version label, e.g. `"8.3"` or `"20.11.0"`.
    pub version: String,
}

/// A named cluster of projects (e.g. "Marketing Stack") for batch operations.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub projects: Vec<ProjectId>,
}

/// Stable, URL/YAML-safe identifier for a database instance. Mirrors
/// [`ProjectId`] — it becomes a Process Compose process name (prefixed
/// `db-`) so it must round-trip cleanly through YAML keys.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DatabaseInstanceId(String);

impl DatabaseInstanceId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DatabaseInstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for DatabaseInstanceId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for DatabaseInstanceId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// The database engines PortBay can provision and supervise.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseEngine {
    Mysql,
    Mariadb,
    Postgres,
    Redis,
    Mongo,
}

impl DatabaseEngine {
    /// Stable string id used in slugs, the engine catalogue, and the wire
    /// protocol. Matches the `serde(rename_all = "snake_case")` output.
    pub fn id(&self) -> &'static str {
        match self {
            DatabaseEngine::Mysql => "mysql",
            DatabaseEngine::Mariadb => "mariadb",
            DatabaseEngine::Postgres => "postgres",
            DatabaseEngine::Redis => "redis",
            DatabaseEngine::Mongo => "mongo",
        }
    }

    /// Human-facing engine name (no version).
    pub fn label(&self) -> &'static str {
        match self {
            DatabaseEngine::Mysql => "MySQL",
            DatabaseEngine::Mariadb => "MariaDB",
            DatabaseEngine::Postgres => "PostgreSQL",
            DatabaseEngine::Redis => "Redis",
            DatabaseEngine::Mongo => "MongoDB",
        }
    }

    /// Canonical default listening port for the engine.
    pub fn default_port(&self) -> u16 {
        match self {
            DatabaseEngine::Mysql | DatabaseEngine::Mariadb => 3306,
            DatabaseEngine::Postgres => 5432,
            DatabaseEngine::Redis => 6379,
            DatabaseEngine::Mongo => 27017,
        }
    }

    /// Parse from the stable string id. Returns `None` for unknown ids.
    pub fn from_id(s: &str) -> Option<Self> {
        match s {
            "mysql" => Some(DatabaseEngine::Mysql),
            "mariadb" => Some(DatabaseEngine::Mariadb),
            "postgres" => Some(DatabaseEngine::Postgres),
            "redis" => Some(DatabaseEngine::Redis),
            "mongo" => Some(DatabaseEngine::Mongo),
            _ => None,
        }
    }
}

/// A database server instance PortBay provisions and supervises.
///
/// Each instance owns an isolated data directory under the app-data dir,
/// runs on its own port, and is launched by Process Compose. Instances
/// can be linked to projects, which injects connection env vars into the
/// linked project's process (see [`DatabaseInstance::connection_env`]).
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct DatabaseInstance {
    pub id: DatabaseInstanceId,
    pub name: String,
    pub engine: DatabaseEngine,

    /// Engine version detected at create time (display only, e.g. "8.4.0").
    #[serde(default)]
    pub version: String,

    /// Listening port. Allocated free at create time.
    pub port: u16,

    /// PortBay-owned data directory (absolute).
    pub data_dir: PathBuf,

    /// Engine config file the daemon reads (absolute). `None` for engines
    /// launched purely with CLI flags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_path: Option<PathBuf>,

    /// Unix socket path the daemon binds (absolute), when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket_path: Option<PathBuf>,

    /// Whether the daemon auto-starts when PortBay boots.
    #[serde(default)]
    pub auto_start: bool,

    /// Projects this instance is linked to. Linking injects connection
    /// env vars into each project's process.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linked_projects: Vec<ProjectId>,
}

impl DatabaseInstance {
    /// The Process Compose process name for this instance. Prefixed `db-`
    /// so it can't collide with a project id.
    pub fn process_id(&self) -> String {
        format!("db-{}", self.id)
    }

    /// Default super-user account name for the engine.
    pub fn default_account(&self) -> &'static str {
        match self.engine {
            DatabaseEngine::Postgres => "postgres",
            DatabaseEngine::Mysql | DatabaseEngine::Mariadb => "root",
            // Redis/Mongo have no user by default in a fresh local instance.
            DatabaseEngine::Redis | DatabaseEngine::Mongo => "",
        }
    }

    /// A connection URL a framework can consume.
    pub fn connection_url(&self) -> String {
        let port = self.port;
        match self.engine {
            DatabaseEngine::Mysql | DatabaseEngine::Mariadb => {
                format!("mysql://root@127.0.0.1:{port}/")
            }
            DatabaseEngine::Postgres => {
                format!("postgresql://postgres@127.0.0.1:{port}/postgres")
            }
            DatabaseEngine::Redis => format!("redis://127.0.0.1:{port}"),
            DatabaseEngine::Mongo => format!("mongodb://127.0.0.1:{port}"),
        }
    }

    /// Connection env vars injected into linked projects. Discrete `DB_*`
    /// vars plus a single `DATABASE_URL`. These are namespaced enough that
    /// they rarely clash with framework-specific vars, and the per-project
    /// `env` (set by the user) always overrides them downstream.
    pub fn connection_env(&self) -> std::collections::BTreeMap<String, String> {
        let mut env = std::collections::BTreeMap::new();
        env.insert("DATABASE_URL".into(), self.connection_url());
        env.insert("DB_CONNECTION".into(), self.engine.id().into());
        env.insert("DB_HOST".into(), "127.0.0.1".into());
        env.insert("DB_PORT".into(), self.port.to_string());
        let account = self.default_account();
        if !account.is_empty() {
            env.insert("DB_USERNAME".into(), account.into());
            env.insert("DB_PASSWORD".into(), String::new());
        }
        env
    }
}

/// Largest `cache-size` we'll write. dnsmasq itself warns past ~10k, and a
/// local dev resolver never needs more.
pub const MAX_DNS_CACHE_SIZE: u16 = 10_000;

/// Largest `local-ttl` we'll write (one day in seconds). Guards against a
/// runaway value pinning a stale answer for weeks.
pub const MAX_DNS_LOCAL_TTL: u32 = 86_400;

fn default_dns_cache_size() -> u16 {
    150
}

/// User-tunable dnsmasq daemon settings, editable from the DNS page.
///
/// PortBay's dnsmasq runs loopback-only and answers only for the wildcard
/// suffix (`listen-address=127.0.0.1`, `bind-interfaces`, `no-resolv`,
/// `no-hosts`). Those directives are fixed for safety and aren't represented
/// here. The fields below are the directives that are both safe and
/// meaningful on such a resolver — cache sizing and TTL behaviour. Changing
/// any of them regenerates `dnsmasq.conf` and restarts the daemon.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsmasqSettings {
    /// `cache-size=N` — number of names dnsmasq caches. dnsmasq's own
    /// default is 150; 0 disables caching entirely.
    #[serde(default = "default_dns_cache_size")]
    pub cache_size: u16,

    /// `local-ttl=N` — TTL (seconds) dnsmasq reports for names it answers
    /// authoritatively (our wildcard). 0 is dnsmasq's default and the safest
    /// for local dev, where the loopback target never changes.
    #[serde(default)]
    pub local_ttl: u32,

    /// When true, emit `no-negcache` so dnsmasq doesn't cache negative
    /// (NXDOMAIN) answers — handy while a hostname is still being wired up
    /// and a cached miss would otherwise linger.
    #[serde(default)]
    pub disable_negative_cache: bool,
}

impl Default for DnsmasqSettings {
    fn default() -> Self {
        Self {
            cache_size: default_dns_cache_size(),
            local_ttl: 0,
            disable_negative_cache: false,
        }
    }
}

impl DnsmasqSettings {
    /// Clamp every field into a range dnsmasq will accept, so a value typed
    /// in the UI can never produce a config the daemon rejects on restart.
    pub fn sanitised(&self) -> Self {
        Self {
            cache_size: self.cache_size.min(MAX_DNS_CACHE_SIZE),
            local_ttl: self.local_ttl.min(MAX_DNS_LOCAL_TTL),
            disable_negative_cache: self.disable_negative_cache,
        }
    }
}

/// PortBay-managed language-runtime settings persisted in the registry:
/// installs the user added by hand (that auto-detection didn't surface),
/// the default version per language, and per-version PHP tuning. All fields
/// default to empty, so pre-runtimes registry files keep loading cleanly
/// (this is additive — no version bump).
#[derive(Debug, Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSettings {
    /// Manually-added installs (a binary path the detector didn't find).
    #[serde(default)]
    pub manual: Vec<ManualRuntime>,
    /// Default version per language id (e.g. `{"php": "8.3"}`). New projects
    /// inherit this when their runtime can't be auto-detected.
    #[serde(default)]
    pub defaults: BTreeMap<String, String>,
    /// Per-version PHP config the `/languages` editable tabs write
    /// (FPM pool tuning + php.ini overrides), keyed by version label
    /// (e.g. `"8.3"`). The reconciler folds these into the generated,
    /// PortBay-owned FPM pool config — the system php.ini is never touched.
    #[serde(default)]
    pub php: BTreeMap<String, PhpVersionConfig>,
}

/// PortBay-owned PHP config for a single detected version. Edited from the
/// `/languages` FPM and PHP tabs; consumed by the reconciler when it renders
/// the per-version FPM pool config.
#[derive(Debug, Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhpVersionConfig {
    /// FPM process-manager pool tuning.
    #[serde(default)]
    pub fpm: FpmTuning,
    /// php.ini override key → value (e.g. `{"memory_limit": "256M"}`).
    /// Emitted as `php_admin_value[key] = value` in the pool's `[www]`
    /// section, so it applies per-pool without editing the system ini.
    #[serde(default)]
    pub ini: BTreeMap<String, String>,
}

/// FPM process-pool tuning. Defaults mirror the historical hardcoded pool
/// config in [`crate::php::lifecycle::render_pool_config`], so a version with
/// no saved tuning renders byte-for-byte the same pool it always did.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FpmTuning {
    /// Process-manager mode: `dynamic`, `static`, or `ondemand`.
    pub pm: String,
    /// Hard ceiling on child processes (`pm.max_children`).
    pub max_children: u32,
    /// Children spawned at start (`pm.start_servers`; `dynamic` only).
    pub start_servers: u32,
    /// Lower bound on idle children (`pm.min_spare_servers`; `dynamic` only).
    pub min_spare_servers: u32,
    /// Upper bound on idle children (`pm.max_spare_servers`; `dynamic` only).
    pub max_spare_servers: u32,
    /// Requests a child handles before respawning (`pm.max_requests`).
    pub max_requests: u32,
}

impl Default for FpmTuning {
    fn default() -> Self {
        Self {
            pm: "dynamic".into(),
            max_children: 8,
            start_servers: 2,
            min_spare_servers: 1,
            max_spare_servers: 3,
            max_requests: 500,
        }
    }
}

/// One manually-added runtime install. PortBay reuses the binary in place —
/// it never copies or re-installs it (the detect-first model).
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualRuntime {
    /// Language id this install belongs to ("php", "node", …).
    pub lang: String,
    /// Version label `<binary> --version` reported at add time (e.g. "8.4").
    pub version: String,
    /// Absolute path to the binary the user browsed to.
    pub binary: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_id_roundtrips_through_json_as_a_bare_string() {
        let id = ProjectId::new("marketing-site");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"marketing-site\"");
        let back: ProjectId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn project_type_serialises_snake_case() {
        let v = serde_json::to_string(&ProjectType::Php).unwrap();
        assert_eq!(v, "\"php\"");
    }

    #[test]
    fn readiness_http_uses_tagged_form() {
        let r = Readiness::Http {
            path: "/".into(),
            timeout_seconds: 30,
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["type"], "http");
        assert_eq!(json["path"], "/");
        assert_eq!(json["timeout_seconds"], 30);
    }

    #[test]
    fn readiness_defaults_timeout_when_missing() {
        let json = r#"{ "type": "http", "path": "/" }"#;
        let r: Readiness = serde_json::from_str(json).unwrap();
        match r {
            Readiness::Http {
                path,
                timeout_seconds,
            } => {
                assert_eq!(path, "/");
                assert_eq!(timeout_seconds, 75);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn project_serialises_in_assessment_doc_shape() {
        // Mirrors the Next.js example in ASSESSMENT_AND_PLAN.md §7.1.
        let p = Project {
            id: ProjectId::new("marketing-site"),
            name: "Marketing Site".into(),
            path: PathBuf::from("/Volumes/DEVSSD/Projects/Clients/Marketing Site"),
            kind: ProjectType::Next,
            start_command: Some("pnpm dev".into()),
            port: Some(3010),
            extra_ports: vec![],
            hostname: "marketing-site.test".into(),
            https: true,
            services: vec!["caddy".into()],
            env: BTreeMap::new(),
            readiness: Some(Readiness::Http {
                path: "/".into(),
                timeout_seconds: 75,
            }),
            auto_start: false,
            tags: vec!["client".into(), "nextjs".into()],
            document_root: None,
            php_version: None,
            runtime: None,
        };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["id"], "marketing-site");
        assert_eq!(json["type"], "next");
        assert_eq!(json["port"], 3010);
        assert!(
            json.get("document_root").is_none(),
            "optional PHP fields should be omitted when empty"
        );
    }

    fn bare_php_project() -> Project {
        Project {
            id: ProjectId::new("legacy-php"),
            name: "Legacy PHP".into(),
            path: PathBuf::from("/tmp/legacy-php"),
            kind: ProjectType::Php,
            start_command: None,
            port: None,
            extra_ports: vec![],
            hostname: "legacy-php.test".into(),
            https: true,
            services: vec!["caddy".into(), "php-fpm".into()],
            env: BTreeMap::new(),
            readiness: None,
            auto_start: false,
            tags: vec![],
            document_root: None,
            php_version: None,
            runtime: None,
        }
    }

    #[test]
    fn php_version_effective_prefers_runtime_then_falls_back() {
        // Runtime pin wins.
        let mut p = bare_php_project();
        p.runtime = Some(Runtime {
            lang: "php".into(),
            version: "8.3".into(),
        });
        p.php_version = Some("7.4".into()); // stale legacy field is ignored
        assert_eq!(p.php_version_effective(), Some("8.3"));

        // No runtime → legacy field is the fallback (imported / un-migrated).
        let mut legacy = bare_php_project();
        legacy.php_version = Some("8.1".into());
        assert_eq!(legacy.php_version_effective(), Some("8.1"));

        // A non-PHP runtime pin means "not a PHP project" regardless of any
        // stray legacy value.
        let mut node = bare_php_project();
        node.runtime = Some(Runtime {
            lang: "node".into(),
            version: "22".into(),
        });
        node.php_version = Some("8.3".into());
        assert_eq!(node.php_version_effective(), None);

        // Nothing set at all.
        assert_eq!(bare_php_project().php_version_effective(), None);
    }

    #[test]
    fn dnsmasq_settings_default_matches_dnsmasq_defaults() {
        let s = DnsmasqSettings::default();
        assert_eq!(s.cache_size, 150);
        assert_eq!(s.local_ttl, 0);
        assert!(!s.disable_negative_cache);
    }

    #[test]
    fn dnsmasq_settings_partial_json_fills_defaults() {
        // A blob with only one field set still deserialises, the rest
        // falling back to defaults — this is what keeps the registry
        // forward-compatible.
        let s: DnsmasqSettings = serde_json::from_str(r#"{ "cacheSize": 500 }"#).unwrap();
        assert_eq!(s.cache_size, 500);
        assert_eq!(s.local_ttl, 0);
        assert!(!s.disable_negative_cache);
    }

    #[test]
    fn dnsmasq_settings_sanitise_clamps_out_of_range() {
        let s = DnsmasqSettings {
            cache_size: u16::MAX,
            local_ttl: u32::MAX,
            disable_negative_cache: true,
        }
        .sanitised();
        assert_eq!(s.cache_size, MAX_DNS_CACHE_SIZE);
        assert_eq!(s.local_ttl, MAX_DNS_LOCAL_TTL);
        assert!(s.disable_negative_cache);
    }
}
