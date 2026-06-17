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
    /// Python project. Detected from `pyproject.toml`/`requirements.txt`/
    /// `manage.py`/etc. A web framework (Django/FastAPI/Flask) gets an
    /// inferred dev command + port; a bare script/research project gets
    /// neither and runs as a board/process-only project.
    Python,
    Static,
    Node,
    /// JS meta-frameworks that sit on top of the Node runtime. They all launch
    /// like a plain Node app (run the dev script behind Caddy) but get their own
    /// detection, default dev port, brand logo, and artifact-clean dirs so they
    /// stop masquerading as generic `Node`. Wire values follow snake_case:
    /// `astro`, `svelte_kit`, `nuxt`, `remix`, `gatsby`, `angular`,
    /// `solid_start`, `qwik`, `vue_cli`, `preact`.
    Astro,
    SvelteKit,
    Nuxt,
    Remix,
    Gatsby,
    Angular,
    SolidStart,
    Qwik,
    VueCli,
    Preact,
    /// Non-JS language runtimes. Each launches generically — run the project's
    /// `start_command` and let Caddy reverse-proxy its `port` — so they share
    /// the wildcard launch path. The specific framework (Laravel, Rails, Gin,
    /// Phoenix, …) is carried separately in `Project::framework`, which supplies
    /// the brand logo and the detection-time smart defaults (document root,
    /// port, dev command). `Go`/`Ruby` map to managed runtimes; the rest run on
    /// the user's system toolchain.
    Go,
    Ruby,
    Rust,
    Deno,
    Elixir,
    DotNet,
    Java,
    Kotlin,
    Scala,
    Clojure,
    Crystal,
    Dart,
    Swift,
    Zig,
    Nim,
    Haskell,
    OCaml,
    Flutter,
    Xcode,
    Android,
    /// Expo / React Native managed app. Play runs the Metro dev server
    /// (`npx expo start`); the iOS/Android simulator opens from there.
    Expo,
    Custom,
}

impl ProjectType {
    /// JS meta-frameworks that run on the Node runtime (Astro, SvelteKit,
    /// Nuxt, …). They share Node's launch path, runtime inheritance, and
    /// sandboxed-PATH handling — only their detection, default port, logo, and
    /// artifact dirs differ. `Next`/`Vite`/`Node` are the original members.
    pub fn is_node_family(self) -> bool {
        matches!(
            self,
            ProjectType::Next
                | ProjectType::Vite
                | ProjectType::Node
                | ProjectType::Astro
                | ProjectType::SvelteKit
                | ProjectType::Nuxt
                | ProjectType::Remix
                | ProjectType::Gatsby
                | ProjectType::Angular
                | ProjectType::SolidStart
                | ProjectType::Qwik
                | ProjectType::VueCli
                | ProjectType::Preact
        )
    }
}

/// The detected *sub-stack* of a project — the specific framework/CMS sitting
/// on top of a language runtime (Laravel on PHP, Django on Python, Rails on
/// Ruby, Phoenix on Elixir, …).
///
/// This is the second detection axis, orthogonal to [`ProjectType`]: the kind
/// says *how to launch*, the framework says *what it is*. It drives the brand
/// logo, the display label, and the detection-time smart defaults (document
/// root, dev port, dev command). `None` means "no recognised framework" — a
/// plain PHP/Go/Rust project, which still renders its parent-language logo.
///
/// Adding a framework here is cheap: a new variant plus one row in the
/// detector. The launch pipeline never matches on it, so there is no
/// per-framework launch code to duplicate. Wire values are snake_case.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Framework {
    // PHP
    Laravel,
    Symfony,
    WordPress,
    Drupal,
    Statamic,
    CraftCms,
    CodeIgniter,
    CakePhp,
    Joomla,
    Yii,
    Magento,
    Slim,
    // Python
    Django,
    FastApi,
    Flask,
    Streamlit,
    Reflex,
    Gradio,
    // Ruby
    Rails,
    Sinatra,
    Jekyll,
    Hanami,
    // Go
    Hugo,
    Gin,
    Echo,
    Fiber,
    // Rust
    Actix,
    Axum,
    Rocket,
    Leptos,
    // Deno
    Fresh,
    // Elixir
    Phoenix,
    // .NET
    AspNet,
    // JVM
    Spring,
    Ktor,
    // JS UI libraries — surfaced when the kind is the generic Vite/Node (a
    // specific kind like Next/Astro already carries its own logo). Plus the
    // smaller JS meta-frameworks that don't warrant their own launch kind.
    React,
    Vue,
    Svelte,
    SolidJs,
    Preact,
    Lit,
    Alpine,
    Ember,
    ReactRouter,
    Eleventy,
    Redwood,
    Docusaurus,
    // Swift
    Vapor,
}

/// Web server used for PHP document-root projects.
///
/// Caddy remains PortBay's edge router for local hostnames and TLS. When a PHP
/// project chooses Apache or Nginx, PortBay launches that server on the
/// project's loopback `port` and Caddy reverse-proxies the public hostname to
/// it. This avoids multiple daemons fighting over :80/:443 while still giving
/// project-level Apache/Nginx/Caddy behavior.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebServer {
    Caddy,
    Nginx,
    Apache,
}

impl WebServer {
    pub fn id(&self) -> &'static str {
        match self {
            WebServer::Caddy => "caddy",
            WebServer::Nginx => "nginx",
            WebServer::Apache => "apache",
        }
    }
}

/// How PortBay decides a project is "actually serving" rather than just
/// "the process is alive."
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Readiness {
    /// "Serving HTTP on the project port" — the most common case for Next,
    /// Vite, PHP. NOTE: the supervisor probes this at the TCP layer (see
    /// `readiness_to_pc_probe`): the probe recurs every 2 s for the life of
    /// the process, and real GETs make framework dev servers re-render and
    /// spam every HMR client with rebuild cycles.
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

    /// The detected sub-stack (Laravel, Django, Rails, …), orthogonal to
    /// `kind`. Drives the brand logo + label only; the launch pipeline never
    /// reads it. `None` for an unrecognised or frameworkless project. Optional
    /// + skipped when empty, so older registries deserialize unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framework: Option<Framework>,

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
    /// mkcert-issued certificate. Defaults to `true`: PortBay issues a
    /// locally-trusted cert on add, so `https://<host>` works out of the box
    /// (the Herd/Valet convention). Every add path that omits the field —
    /// import, portfile, legacy registries — inherits HTTPS-on.
    #[serde(default = "default_true")]
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

    /// Shell commands run *before* the dev server on every start (install
    /// deps, migrate the DB, codegen). The reconciler emits each as a
    /// one-shot Process Compose process chained ahead of the main process
    /// via `depends_on: { condition: process_completed_successfully }`, so a
    /// non-zero exit blocks the dev server from starting at all. Additive —
    /// absent on registries written before hooks landed (deserialises to an
    /// empty vec), so it needs no schema-version bump.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_start: Vec<String>,

    /// Shell commands run *after* the dev server reports ready (smoke checks,
    /// cache warm-up, opening a watcher). Emitted as one-shot processes that
    /// depend on the main process reaching `process_healthy` — or
    /// `process_started` when the project has no readiness probe to gate on. A
    /// non-zero exit is surfaced as a warning but never tears the running
    /// project down. Additive, same migration story as `pre_start`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_start: Vec<String>,

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

    /// Web server selected for PHP document-root projects. Absent means Caddy.
    /// Ignored for non-PHP projects and for PHP projects that provide a custom
    /// `start_command` (those are reverse-proxied like any dev server).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_server: Option<WebServer>,

    // ----- Mobile run configuration (optional) -------------------------
    /// Project-local run settings for Flutter, Xcode, and Android projects.
    /// The Play command is still stored in `start_command` for Process Compose;
    /// this structured config lets the UI edit scheme/flavor/device settings
    /// without making users hand-author shell commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mobile_run: Option<MobileRunConfig>,

    // ----- Runtime selection (schema v2+) -------------------------------
    /// Pinned language runtime — which language toolchain and version
    /// PortBay launches this project with. Introduced in registry schema v2;
    /// migrated v1 registries derive it from the legacy `php_version` (see
    /// [`crate::registry::migrate`]). `None` means "fall back to the project
    /// type's default runtime resolution." Kept alongside `php_version`
    /// through the transition — existing consumers still read `php_version`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<Runtime>,

    // ----- Monorepo / workspace (optional) ------------------------------
    /// When set, this project runs a single app inside a monorepo via a
    /// workspace filter rather than as a standalone folder; `path` stays the
    /// monorepo root (so the root lockfile, `.env`, and task-runner config
    /// resolve). Additive field — absent on standalone projects and on
    /// registries written before it landed (deserialises to `None`), so it
    /// needs no schema-version bump, matching how `databases`/`runtimes` were
    /// added.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<Workspace>,

    /// Per-project CORS policy applied at the Caddy edge. **Pro-gated** (the
    /// `custom_port_cors` entitlement): the `add`/`update` paths reject
    /// introducing or changing a custom policy without Pro, but an existing
    /// policy keeps being served on downgrade — we never strip a configured
    /// value. `None`/empty = PortBay's default (no CORS headers), the free,
    /// always-available behaviour. Additive — absent on free projects and
    /// pre-existing registries (deserialises to `None`); no schema bump.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cors: Option<CorsConfig>,

    /// Pro sandbox runner configuration. `None` means normal unrestricted
    /// local execution. Additive field for registries written before sandbox
    /// mode existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<SandboxConfig>,

    /// Per-project domain / routing settings (Domains page). Additive — see
    /// [`DomainConfig`]. `None` means every setting takes its default, which
    /// reproduces PortBay's behaviour from before these knobs existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<DomainConfig>,

    /// Bring-your-own named Cloudflare tunnel for a stable public hostname
    /// (Pro). `None` = only the free zero-config Quick Share is offered. Stores
    /// the user's tunnel UUID, credentials-file path, and chosen hostname — no
    /// secrets (the credentials file stays in `~/.cloudflared`, owned by the
    /// user; PortBay only references it). Additive; absent registries → `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tunnel: Option<CustomTunnelConfig>,

    /// One-click deploy target: push this project's files to a saved SSH host
    /// and run an ordered list of remote build/release steps. `None` means the
    /// project has no deploy configured (the default). Additive — absent on
    /// projects and registries written before deploy landed (deserialises to
    /// `None`), so it needs no schema bump, matching `tunnel`/`cors`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deploy: Option<ProjectDeploy>,
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

    pub fn web_server_effective(&self) -> WebServer {
        self.web_server.unwrap_or(WebServer::Caddy)
    }

    /// Whether PortBay should issue / renew this project's mkcert certificate.
    /// Defaults `true` so HTTPS projects predating [`DomainConfig`] keep their
    /// managed cert.
    pub fn auto_manage_cert(&self) -> bool {
        self.domain
            .as_ref()
            .is_none_or(|d| d.auto_manage_cert && d.ssl_mode == SslMode::AutomaticLocal)
    }

    pub fn ssl_mode(&self) -> SslMode {
        self.domain
            .as_ref()
            .map(|d| {
                if d.auto_manage_cert {
                    d.ssl_mode
                } else {
                    SslMode::CustomCertificate
                }
            })
            .unwrap_or_default()
    }

    pub fn custom_cert_paths(&self) -> Option<(&str, &str)> {
        let domain = self.domain.as_ref()?;
        let cert = domain.custom_cert_path.as_deref()?.trim();
        let key = domain.custom_key_path.as_deref()?.trim();
        (!cert.is_empty() && !key.is_empty()).then_some((cert, key))
    }

    /// Effective resolver mode for this project's hostname.
    pub fn resolver_mode(&self) -> ResolverMode {
        self.domain
            .as_ref()
            .map(|d| d.resolver_mode)
            .unwrap_or_default()
    }

    /// Path prefix to strip before proxying upstream, if a meaningful one is
    /// set. Empty and `"/"` are treated as "serve from root" → `None`.
    pub fn path_prefix(&self) -> Option<&str> {
        self.domain
            .as_ref()
            .and_then(|d| d.path_prefix.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "/")
    }

    /// Whether to also route + certify `*.hostname`.
    pub fn include_wildcard_subdomains(&self) -> bool {
        self.domain
            .as_ref()
            .is_some_and(|d| d.include_wildcard_subdomains)
    }

    /// Whether this project's route should exist only while it's running.
    pub fn expose_when_running(&self) -> bool {
        self.domain.as_ref().is_some_and(|d| d.expose_when_running)
    }

    /// Process Compose id that represents this project at runtime, when one
    /// exists. Used by Start/Stop/status to find the process to act on.
    ///
    /// MUST stay in lockstep with `process_compose::config::project_to_pc_process`,
    /// which emits a PC entry keyed by the project id for any project with an
    /// explicit `start_command` **or** a monorepo `workspace` binding (it derives
    /// the dev command from the workspace, e.g. `pnpm --filter @app/web dev`). If
    /// the two disagree, Start/Stop silently no-op on the projects this misses:
    /// that's exactly how workspace-backed projects (no explicit command) used to
    /// have a running process the UI couldn't start or stop. PHP projects served
    /// by generated Nginx/Apache configs use a derived backend process id instead.
    /// Pure Caddy/PHP-FPM and static file-server projects have no PC process.
    pub fn process_compose_id(&self) -> Option<String> {
        if self.start_command.is_some() || self.workspace.is_some() {
            return Some(self.id.as_str().to_string());
        }
        if self.kind == ProjectType::Php {
            return match self.web_server_effective() {
                WebServer::Caddy => None,
                WebServer::Nginx => Some(format!("web-nginx-{}", self.id)),
                WebServer::Apache => Some(format!("web-apache-{}", self.id)),
            };
        }
        None
    }

    /// A static site served straight off disk by Caddy's `file_server`, with no
    /// supervised process to start or stop. Because there's no process, its
    /// "running" state can't come from Process Compose — it's an explicit serve
    /// toggle (Play/Stop) tracked in the session set: started ⇒ Caddy publishes
    /// its route and the UI shows Running; stopped ⇒ the route is suppressed and
    /// the site stops serving. See `reconciler::caddy::suppressed_routes`,
    /// `commands::lifecycle::{start,stop}_project`, and the status poller.
    pub fn is_static_served(&self) -> bool {
        self.kind == ProjectType::Static && self.process_compose_id().is_none()
    }
}

/// Per-project CORS policy applied at the Caddy edge. The basic listen port
/// is **not** gated (every project needs one); only this custom cross-origin
/// policy is a Pro feature. `allowed_origins` empty means the feature is off
/// and PortBay adds no CORS headers — identical to today's behaviour.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct CorsConfig {
    /// Exact origins allowed. When a request's `Origin` matches one of these,
    /// Caddy echoes it into `Access-Control-Allow-Origin` and answers
    /// preflight `OPTIONS` with the standard allow headers. Empty = off.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_origins: Vec<String>,

    /// Send `Access-Control-Allow-Credentials: true` for matched origins.
    #[serde(default)]
    pub allow_credentials: bool,
}

/// Per-project sandbox runner configuration. This is intentionally explicit
/// instead of a tag so UI, import, and lifecycle code share one auditable
/// security contract.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxConfig {
    /// Whether the next supervised run is wrapped by the generated sandbox
    /// profile. Keeping the object while disabled lets the UI preserve policy
    /// choices between runs.
    #[serde(default)]
    pub enabled: bool,
    /// Network access granted inside the profile.
    #[serde(default)]
    pub network: SandboxNetworkPolicy,
    /// Remove mutable sandbox cache/temp dirs before each sandboxed start.
    #[serde(default = "default_true")]
    pub ephemeral: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            network: SandboxNetworkPolicy::LoopbackOnly,
            ephemeral: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxNetworkPolicy {
    /// Allow loopback bind/connect only. This is the safest useful default for
    /// local dev servers once dependencies are already installed.
    #[default]
    LoopbackOnly,
    /// Allow outbound package-manager access plus loopback dev-server bind.
    Outbound,
    /// Allow all networking. Useful for projects that legitimately need LAN
    /// devices or multicast during inspection.
    Full,
    /// Block networking. The process can still run local scripts.
    Blocked,
}

impl SandboxConfig {
    pub fn enabled(network: SandboxNetworkPolicy, ephemeral: bool) -> Self {
        Self {
            enabled: true,
            network,
            ephemeral,
        }
    }
}

/// How a single project's hostname is published to the local resolver.
///
/// `Auto` keeps PortBay's global behaviour (an `/etc/hosts` row unless the
/// dnsmasq wildcard resolver is installed for the suffix). The explicit modes
/// override that per project: `Hosts` always writes the `/etc/hosts` row even
/// when the wildcard would cover it; `Dnsmasq` never writes one, trusting the
/// wildcard resolver (so the host won't resolve until that resolver exists).
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResolverMode {
    #[default]
    Auto,
    Hosts,
    Dnsmasq,
}

/// Certificate source for a project's HTTPS edge.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SslMode {
    /// PortBay issues and renews a locally trusted mkcert certificate.
    #[default]
    AutomaticLocal,
    /// User-provided certificate and private key paths.
    CustomCertificate,
    /// Explicit fallback for local testing where browser warnings are expected.
    SelfSigned,
    /// Placeholder for future public-domain automation. Not enabled for `.test`.
    PublicAcme,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AcmeIssuer {
    #[default]
    LetsEncrypt,
    ZeroSsl,
    GoogleTrustServices,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AcmeEnvironment {
    #[default]
    Production,
    Staging,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AcmeDnsProvider {
    #[default]
    None,
    Cloudflare,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AcmeKeyType {
    Rsa2048,
    Rsa4096,
    P256,
    #[default]
    P384,
}

impl AcmeKeyType {
    pub fn caddy_key_type(self) -> &'static str {
        match self {
            Self::Rsa2048 => "rsa2048",
            Self::Rsa4096 => "rsa4096",
            Self::P256 => "p256",
            Self::P384 => "p384",
        }
    }
}

/// Public ACME certificate request settings.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcmeConfig {
    #[serde(default)]
    pub issuer: AcmeIssuer,
    #[serde(default)]
    pub environment: AcmeEnvironment,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default)]
    pub key_type: AcmeKeyType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eab_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eab_hmac_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zerossl_api_key: Option<String>,
    #[serde(default)]
    pub dns_provider: AcmeDnsProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dns_api_token: Option<String>,
    #[serde(default)]
    pub force_request: bool,
    #[serde(default)]
    pub debug: bool,
}

impl Default for AcmeConfig {
    fn default() -> Self {
        Self {
            issuer: AcmeIssuer::LetsEncrypt,
            environment: AcmeEnvironment::Production,
            email: None,
            key_type: AcmeKeyType::P384,
            eab_key_id: None,
            eab_hmac_key: None,
            zerossl_api_key: None,
            dns_provider: AcmeDnsProvider::None,
            dns_api_token: None,
            force_request: false,
            debug: false,
        }
    }
}

impl AcmeConfig {
    pub fn directory_url(&self) -> &'static str {
        match (self.issuer, self.environment) {
            (AcmeIssuer::LetsEncrypt, AcmeEnvironment::Production) => {
                "https://acme-v02.api.letsencrypt.org/directory"
            }
            (AcmeIssuer::LetsEncrypt, AcmeEnvironment::Staging) => {
                "https://acme-staging-v02.api.letsencrypt.org/directory"
            }
            (AcmeIssuer::ZeroSsl, _) => "https://acme.zerossl.com/v2/DV90",
            (AcmeIssuer::GoogleTrustServices, AcmeEnvironment::Production) => {
                "https://dv.acme-v02.api.pki.goog/directory"
            }
            (AcmeIssuer::GoogleTrustServices, AcmeEnvironment::Staging) => {
                "https://dv.acme-v02.test-api.pki.goog/directory"
            }
        }
    }
}

/// Per-project routing / domain settings surfaced on the Domains page.
///
/// Additive — absent on projects and registries written before it landed
/// (deserialises to `None` on [`Project::domain`]); no schema-version bump,
/// matching how `workspace`/`cors`/`sandbox` were introduced. Every field
/// defaults to today's behaviour, so an absent config and an all-default
/// config are indistinguishable at reconcile time.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainConfig {
    /// Free-text note shown on the Domains page. No runtime effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,

    /// URL path prefix stripped before proxying upstream (Caddy `handle_path`).
    /// `None` / empty / `"/"` = serve from the root (today's behaviour).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,

    /// How this hostname is published to the local resolver.
    #[serde(default)]
    pub resolver_mode: ResolverMode,

    /// Whether PortBay issues / renews this hostname's mkcert certificate.
    /// Defaults `true` — every HTTPS project got a managed cert before this
    /// field existed.
    #[serde(default = "default_true")]
    pub auto_manage_cert: bool,

    /// Which certificate source this hostname uses.
    #[serde(default)]
    pub ssl_mode: SslMode,

    /// Certificate path for [`SslMode::CustomCertificate`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_cert_path: Option<String>,

    /// Private key path for [`SslMode::CustomCertificate`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_key_path: Option<String>,

    /// Public ACME issuer and challenge settings for [`SslMode::PublicAcme`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acme: Option<AcmeConfig>,

    /// Also route and certify `*.hostname`. The subdomains only resolve under
    /// the dnsmasq wildcard resolver — an `/etc/hosts` row can't express a
    /// wildcard.
    #[serde(default)]
    pub include_wildcard_subdomains: bool,

    /// Only publish the Caddy route while the project's process is running.
    #[serde(default)]
    pub expose_when_running: bool,
}

impl Default for DomainConfig {
    fn default() -> Self {
        Self {
            notes: None,
            path_prefix: None,
            resolver_mode: ResolverMode::Auto,
            auto_manage_cert: true,
            ssl_mode: SslMode::AutomaticLocal,
            custom_cert_path: None,
            custom_key_path: None,
            acme: None,
            include_wildcard_subdomains: false,
            expose_when_running: false,
        }
    }
}

impl CorsConfig {
    /// Whether this policy actually does anything (has ≥1 allowed origin).
    pub fn is_active(&self) -> bool {
        !self.allowed_origins.is_empty()
    }
}

/// A bring-your-own named Cloudflare tunnel attached to a project (Pro). PortBay
/// generates its **own** ingress config from these fields and runs the user's
/// tunnel; it never reads or edits `~/.cloudflared/config.yml`. Holds no secret
/// — `credentials_file` is a path into the user-owned `~/.cloudflared`.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomTunnelConfig {
    /// Cloudflare tunnel UUID (matches a `~/.cloudflared/<uuid>.json` creds file).
    pub tunnel_id: String,
    /// Absolute path to the tunnel's credentials JSON in `~/.cloudflared`.
    pub credentials_file: String,
    /// Stable public hostname the user has already `route dns`-ed to this tunnel.
    pub hostname: String,
}

impl CustomTunnelConfig {
    /// Whether this config is complete enough to run a tunnel.
    pub fn is_active(&self) -> bool {
        !self.tunnel_id.is_empty() && !self.credentials_file.is_empty() && !self.hostname.is_empty()
    }
}

/// One-click deploy configuration attached to a project: sync local files to a
/// saved SSH host over SFTP, then run an ordered list of remote commands. The
/// connection's credentials live with the [`SshConnection`]; this stores only
/// the target host id, paths, build steps, and upload excludes — no secrets.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDeploy {
    /// Saved SSH connection to deploy to.
    pub connection_id: SshConnectionId,
    /// Absolute remote directory the files are synced into (e.g. `/var/www/app`).
    pub remote_path: String,
    /// Sub-directory of the project to upload (e.g. `dist` / `build`). `None` or
    /// blank uploads the whole project folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_subdir: Option<String>,
    /// Ordered remote commands run (from `remote_path`) after the sync; stops at
    /// the first non-zero exit, like an ad-hoc deploy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<String>,
    /// Path components skipped during the upload walk (matched against any
    /// segment of a file's relative path). Defaults to `node_modules` + `.git`
    /// on a fresh config; an explicit empty list round-trips as "exclude
    /// nothing" rather than re-defaulting.
    #[serde(default = "default_deploy_exclude")]
    pub exclude: Vec<String>,
}

fn default_deploy_exclude() -> Vec<String> {
    vec!["node_modules".into(), ".git".into()]
}

impl ProjectDeploy {
    /// Whether this config names a host + remote path to actually deploy to.
    pub fn is_active(&self) -> bool {
        !self.connection_id.as_str().is_empty() && !self.remote_path.trim().is_empty()
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

/// Package-manager / task-runner used to scope a single-app run inside a
/// monorepo. Determines the shape of the filtered dev command.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTool {
    Pnpm,
    Npm,
    Yarn,
    Bun,
    Turbo,
}

/// Set on a project that runs ONE app of a monorepo via a workspace filter.
/// The project's `path` is the monorepo root; the dev server is scoped to a
/// single package so a `turbo run dev --parallel`-style fan-out doesn't start
/// every app in the repo.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    /// Filter token the tool understands — typically the package name
    /// (`@bookslash/web`) passed to `--filter` / `--workspace` / `workspace`.
    pub package: String,
    /// The app's directory RELATIVE to the monorepo root (e.g. `apps/web`).
    /// Used to attribute the spawned dev server's port to *this* project when
    /// several apps share one monorepo root.
    pub rel_dir: String,
    /// Which tool scopes the run.
    pub tool: WorkspaceTool,
}

impl Workspace {
    /// The dev command that runs only this app, scoped by `tool`. Used by the
    /// Process Compose config builder to fill in a `start_command` the user
    /// didn't set explicitly. Run from the monorepo root (the project `path`).
    pub fn derive_dev_command(&self) -> String {
        match self.tool {
            WorkspaceTool::Pnpm => format!("pnpm --filter {} dev", self.package),
            WorkspaceTool::Npm => format!("npm run dev --workspace {}", self.package),
            WorkspaceTool::Yarn => format!("yarn workspace {} dev", self.package),
            WorkspaceTool::Bun => format!("bun --filter {} dev", self.package),
            WorkspaceTool::Turbo => format!("turbo run dev --filter={}", self.package),
        }
    }

    /// Absolute path to the app's directory, given the monorepo root (the
    /// project `path`). The dev server's working directory in practice — what
    /// port attribution should match against.
    pub fn app_dir(&self, root: &std::path::Path) -> PathBuf {
        root.join(&self.rel_dir)
    }
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

/// Stable identifier for a saved SSH tunnel profile.
///
/// Kept separate from Cloudflare tunnel ids because this models the inverse
/// flow: a remote service forwarded onto localhost.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SshTunnelId(String);

impl SshTunnelId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SshTunnelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for SshTunnelId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for SshTunnelId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Stable identifier for a saved SSH connection (a host + its credentials).
///
/// A connection is the anchor every SSH capability hangs on — port-forwards
/// today, file transfer / deploy / shell as they land. Tunnels reference a
/// connection by this id rather than re-stating host + auth.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SshConnectionId(String);

impl SshConnectionId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SshConnectionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for SshConnectionId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for SshConnectionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SshAuthKind {
    #[default]
    Key,
    Password,
    /// Authenticate via the running SSH agent (`SSH_AUTH_SOCK`). Lets a user
    /// pick "use my agent" explicitly; the connect pipeline also falls back to
    /// the agent automatically, so agent-only hosts work without selecting this.
    Agent,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SshForwardKind {
    #[default]
    Local,
    Reverse,
    Socks,
}

/// Which forward-proxy protocol fronts a connection's first transport hop.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SshProxyKind {
    /// SOCKS5 (RFC 1928), optionally with RFC 1929 username/password auth.
    Socks5,
    /// HTTP CONNECT, optionally with a Basic `Proxy-Authorization` header.
    Http,
}

/// A forward proxy the in-process russh path dials before reaching the SSH
/// target (or the first jump host). Registry-safe: it carries the proxy
/// address and an optional username, but **never** the proxy password — that
/// lives in the OS keychain keyed `proxy:<connection-id>`, mirroring how the
/// host password is stored. An open proxy has no `username` and no keychain
/// entry. Only the first transport hop is proxied; jump hosts beyond it are
/// reached by tunnelling through the SSH chain.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshProxyConfig {
    pub kind: SshProxyKind,
    pub host: String,
    pub port: u16,
    /// Proxy auth username. `None` = open proxy (no auth). When set, the
    /// password is loaded from the keychain at connect time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

/// A saved SSH connection: where to connect and how to authenticate.
/// Registry-safe by design — hostnames, ports, usernames, and optional key
/// paths only; a password, passphrase, or private-key material is never stored
/// here (passwords live in the OS keychain, keyed by this connection id).
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConnection {
    pub id: SshConnectionId,
    pub name: String,
    pub ssh_host: String,
    #[serde(default = "default_ssh_port")]
    pub ssh_port: u16,
    pub ssh_user: String,
    #[serde(default)]
    pub auth_kind: SshAuthKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_jump: Option<String>,
    /// Optional reusable [`SshIdentity`] this connection borrows its user / key /
    /// auth from. When set and present, the identity supplies those fields
    /// (the connection's own non-empty user / key_path still override). Absent
    /// on every pre-identities registry, so old files load unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_id: Option<SshIdentityId>,
    /// Optional forward proxy (SOCKS5 / HTTP CONNECT) dialled before the first
    /// transport hop. `None` = connect directly. Additive — absent on every
    /// pre-proxy registry, so old files load unchanged (no schema bump).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<SshProxyConfig>,
    /// Display/UX-only metadata (tags, colour, notes, detected OS, last-used).
    /// `#[serde(flatten)]` keeps these at the JSON top level for the frontend,
    /// while grouping them in Rust so struct literals don't restate five
    /// defaults. All inner fields default, so old registries load unchanged.
    #[serde(flatten)]
    pub metadata: SshConnectionMeta,
}

/// Stable identifier for a saved [`SshIdentity`].
#[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SshIdentityId(String);

impl SshIdentityId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SshIdentityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A reusable credential — a username + key/agent/password method — shareable
/// across many connections so the same login isn't restated per host. Like
/// [`SshConnection`] it is secret-free; a password lives in the OS keychain
/// (keyed by the borrowing connection, unchanged from the per-connection path).
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshIdentity {
    pub id: SshIdentityId,
    pub name: String,
    #[serde(default)]
    pub ssh_user: String,
    #[serde(default)]
    pub auth_kind: SshAuthKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
}

/// Optional, presentation-only metadata for an [`SshConnection`]. Never holds
/// secrets — only labels and a cached `detected_os` / `last_used` stamp.
#[derive(Debug, Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConnectionMeta {
    /// Free-form labels for grouping/filtering on the dashboard.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// A CSS colour (hex or token) for the host's dot, when the user picks one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// User notes shown on the host detail view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Cached `uname`/os-release result, refreshed on demand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detected_os: Option<String>,
    /// Environment id driving the host's brand mark (e.g. `cpanel`, `ubuntu`,
    /// `aws`, `generic`). Set automatically by detection and overridable from
    /// the host form. Presentation-only; absent on pre-environment registries.
    ///
    /// Note: this is the **provider/OS** mark, distinct from [`Self::stage`]
    /// (the deployment tier shown in the dashboard's "Environment" column).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// Deployment tier shown in the dashboard's "Environment" column
    /// (`production` / `staging` / `research` / `sandbox`). Free-form; the UI
    /// offers a known set. Absent on pre-stage registries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    /// Provider region label (`us-east-1`, `nyc3`, …), shown next to the
    /// provider mark. Auto-detected from cloud metadata (or set in the form);
    /// presentation-only; absent on pre-region registries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// Detected cloud **provider** (`aws`, `digitalocean`, `gcp`, `azure`,
    /// `hetzner`, …), distinct from [`Self::environment`] (which may be a control
    /// panel or distro). Captured from DMI vendor during OS detection so a
    /// cPanel box on, say, AWS still shows its real host provider + region.
    /// Presentation-only; absent on pre-provider registries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Epoch seconds when the host was first saved, for the detail "Created"
    /// row. Stamped once on first save; absent on pre-created_at registries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    /// Epoch seconds of the last successful use, for dashboard ordering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<u64>,
}

/// A saved SSH port-forward, layered on an [`SshConnection`]. Holds only the
/// forward coordinates; the host + auth come from the referenced connection
/// (resolved into an `EffectiveSshTunnel` before use).
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTunnelConnection {
    pub id: SshTunnelId,
    pub name: String,
    pub connection_id: SshConnectionId,
    pub local_host: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    #[serde(default)]
    pub forward_kind: SshForwardKind,
    #[serde(default)]
    pub keep_alive: bool,
    #[serde(default)]
    pub auto_reconnect: bool,
}

fn default_ssh_port() -> u16 {
    22
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
    Memcached,
    /// SQLite — a *file-based* engine. Unlike the others it has no daemon,
    /// no listening port, and no socket: a "database" is a single `.sqlite`
    /// file on disk. It is therefore never supervised by Process Compose;
    /// see [`DatabaseEngine::is_file_based`].
    Sqlite,
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
            DatabaseEngine::Memcached => "memcached",
            DatabaseEngine::Sqlite => "sqlite",
        }
    }

    /// The driver key a Laravel `.env` expects in `DB_CONNECTION` — which is
    /// *not* always [`id`]. Laravel uses `pgsql` for PostgreSQL (not
    /// `postgres`), and MariaDB connects through the `mysql` driver on every
    /// Laravel version (the dedicated `mariadb` driver only exists on Laravel
    /// 11+), so we map it to `mysql` for maximum compatibility. Non-SQL engines
    /// have no Laravel driver; they fall back to [`id`].
    pub fn laravel_driver_id(&self) -> &'static str {
        match self {
            DatabaseEngine::Mysql | DatabaseEngine::Mariadb => "mysql",
            DatabaseEngine::Postgres => "pgsql",
            DatabaseEngine::Sqlite => "sqlite",
            _ => self.id(),
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
            DatabaseEngine::Memcached => "Memcached",
            DatabaseEngine::Sqlite => "SQLite",
        }
    }

    /// Canonical default listening port for the engine. File-based engines
    /// ([`Self::is_file_based`]) have no port and return 0.
    pub fn default_port(&self) -> u16 {
        match self {
            DatabaseEngine::Mysql | DatabaseEngine::Mariadb => 3306,
            DatabaseEngine::Postgres => 5432,
            DatabaseEngine::Redis => 6379,
            DatabaseEngine::Mongo => 27017,
            DatabaseEngine::Memcached => 11211,
            DatabaseEngine::Sqlite => 0,
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
            "memcached" => Some(DatabaseEngine::Memcached),
            "sqlite" => Some(DatabaseEngine::Sqlite),
            _ => None,
        }
    }

    /// File-based engines store each database as a single file on disk and run
    /// no daemon. PortBay never allocates a port, renders a config, or
    /// supervises a Process Compose process for them; their lifecycle is a
    /// no-op (always "available"). Currently only SQLite.
    pub fn is_file_based(&self) -> bool {
        matches!(self, DatabaseEngine::Sqlite)
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

    /// Absolute path to the database file, for file-based engines (SQLite).
    /// `None` for daemon engines, which locate their storage via `data_dir`.
    /// This is the file injected as `DB_DATABASE` into linked projects, so it
    /// can point either at a PortBay-managed file under `data_dir` or — when an
    /// existing project `.sqlite` is *adopted* — at the file in place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<PathBuf>,

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
            // Redis/Mongo/Memcached have no user by default in a fresh local
            // instance; SQLite is a bare file with no auth at all.
            DatabaseEngine::Redis
            | DatabaseEngine::Mongo
            | DatabaseEngine::Memcached
            | DatabaseEngine::Sqlite => "",
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
            DatabaseEngine::Memcached => format!("memcached://127.0.0.1:{port}"),
            // SQLite has no host/port — the "URL" is the file path. The triple
            // slash is the standard `sqlite:///absolute/path` form.
            DatabaseEngine::Sqlite => {
                let p = self.file_path.as_ref().map(|p| p.display().to_string());
                match p {
                    Some(path) => format!("sqlite://{path}"),
                    None => "sqlite://".to_string(),
                }
            }
        }
    }

    /// Connection env vars injected into linked projects. Discrete `DB_*`
    /// vars plus a single `DATABASE_URL`. These are namespaced enough that
    /// they rarely clash with framework-specific vars, and the per-project
    /// `env` (set by the user) always overrides them downstream.
    pub fn connection_env(&self) -> std::collections::BTreeMap<String, String> {
        let mut env = std::collections::BTreeMap::new();
        env.insert("DATABASE_URL".into(), self.connection_url());
        // `DB_CONNECTION` is a Laravel-ism; emit the Laravel driver key so a
        // Laravel project reads the right driver (`pgsql`, not `postgres`).
        // Non-Laravel stacks ignore this var and read `DATABASE_URL` instead.
        env.insert(
            "DB_CONNECTION".into(),
            self.engine.laravel_driver_id().into(),
        );

        // File-based engines (SQLite) carry no host/port/account — the only
        // connection coordinate is the file path, surfaced as DB_DATABASE.
        if self.engine.is_file_based() {
            if let Some(path) = &self.file_path {
                env.insert("DB_DATABASE".into(), path.display().to_string());
            }
            return env;
        }

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

/// A PortBay-managed database engine — our own signed build of an engine
/// (MySQL/PostgreSQL/Redis/…) fetched on demand into
/// `Application Support/PortBay/database-engines/<engine>/<version>/`, mirroring
/// the managed-runtime ([`ManagedRuntime`]) model. When present, it is preferred
/// over any Homebrew/system install when resolving the engine's binaries, so an
/// engine installed through PortBay lives entirely inside the PortBay environment
/// without bundling it into the app installer.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedDatabaseEngine {
    pub engine: DatabaseEngine,
    /// Full version installed (e.g. "8.4.0").
    pub version: String,
    /// The install root: `<app-data>/database-engines/<engine>/<version>/`.
    /// Binaries live under `<dir>/bin/`.
    pub dir: PathBuf,
    /// "aarch64" or "x86_64".
    pub arch: String,
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
    /// PortBay-managed runtimes — our own lean builds fetched on demand into
    /// `Application Support/PortBay/runtimes/<lang>/<version>/` (the Herd
    /// delivery model). These are preferred over any detected install when
    /// resolving a project's binary: they're ours, signed, and never a
    /// competitor's. Populated by the download manager (a follow-up slice).
    #[serde(default)]
    pub managed: Vec<ManagedRuntime>,
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

impl RuntimeSettings {
    /// The runtime a freshly-added project of `kind` inherits from the
    /// configured per-language defaults, or `None` when the type has no
    /// managed runtime (Static/Custom) or no default is set for its language.
    ///
    /// Single source of truth shared by the GUI `add_project` command and the
    /// CLI `portbay add`, so the two can't drift on inheritance behaviour.
    pub fn default_for(&self, kind: ProjectType) -> Option<Runtime> {
        let lang = match kind {
            // Every JS meta-framework (Next/Vite/Astro/Nuxt/…) inherits the
            // managed `node` runtime.
            ProjectType::Next
            | ProjectType::Vite
            | ProjectType::Node
            | ProjectType::Astro
            | ProjectType::SvelteKit
            | ProjectType::Nuxt
            | ProjectType::Remix
            | ProjectType::Gatsby
            | ProjectType::Angular
            | ProjectType::SolidStart
            | ProjectType::Qwik
            | ProjectType::VueCli
            | ProjectType::Preact => "node",
            ProjectType::Php => "php",
            ProjectType::Python => "python",
            ProjectType::Flutter => "flutter",
            // Managed runtimes exist for Go and Ruby; the rest run on the
            // user's system toolchain (no PortBay-managed version to inherit).
            ProjectType::Go => "go",
            ProjectType::Ruby => "ruby",
            ProjectType::Rust
            | ProjectType::Deno
            | ProjectType::Elixir
            | ProjectType::DotNet
            | ProjectType::Java
            | ProjectType::Kotlin
            | ProjectType::Scala
            | ProjectType::Clojure
            | ProjectType::Crystal
            | ProjectType::Dart
            | ProjectType::Swift
            | ProjectType::Zig
            | ProjectType::Nim
            | ProjectType::Haskell
            | ProjectType::OCaml
            | ProjectType::Static
            | ProjectType::Xcode
            | ProjectType::Android
            | ProjectType::Expo
            | ProjectType::Custom => return None,
        };
        self.defaults.get(lang).map(|version| Runtime {
            lang: lang.to_string(),
            version: version.clone(),
        })
    }
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
    /// FPM listen transport: `socket` (PortBay-owned unix socket) or `tcp`.
    #[serde(default = "default_fpm_listen")]
    pub listen: String,
    /// Loopback port used when `listen == "tcp"`.
    #[serde(default = "default_fpm_tcp_port")]
    pub tcp_port: u16,
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
    /// FPM `request_slowlog_timeout`; `0` disables slow logging.
    #[serde(default)]
    pub request_slowlog_timeout: String,
    /// Optional FPM `slowlog` path. Blank resolves to PortBay's per-version
    /// config dir at render time.
    #[serde(default)]
    pub slowlog: String,
    /// Forward worker stdout/stderr into the FPM error log.
    #[serde(default)]
    pub catch_workers_output: bool,
    /// Prefix forwarded worker output with FPM metadata.
    #[serde(default = "default_true")]
    pub decorate_workers_output: bool,
    /// Emit FPM access logs into the per-version config dir.
    #[serde(default)]
    pub access_log: bool,
    /// Free-form pool directives appended after PortBay-managed settings.
    /// Only a constrained allowlist is accepted by the runtime apply path.
    #[serde(default)]
    pub raw_params: String,
}

impl Default for FpmTuning {
    fn default() -> Self {
        Self {
            listen: default_fpm_listen(),
            tcp_port: default_fpm_tcp_port(),
            pm: "dynamic".into(),
            max_children: 8,
            start_servers: 2,
            min_spare_servers: 1,
            max_spare_servers: 3,
            max_requests: 500,
            request_slowlog_timeout: String::new(),
            slowlog: String::new(),
            catch_workers_output: false,
            decorate_workers_output: true,
            access_log: false,
            raw_params: String::new(),
        }
    }
}

fn default_fpm_listen() -> String {
    "socket".into()
}

fn default_fpm_tcp_port() -> u16 {
    9000
}

fn default_true() -> bool {
    true
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

/// One PortBay-managed runtime install: a lean build PortBay downloaded (or
/// bundled) and owns end-to-end. Unlike [`ManualRuntime`], the binary lives
/// inside PortBay's own `Application Support` tree, so its arch is recorded for
/// integrity checks and to ignore a stale entry left by an OS migration.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedRuntime {
    /// Language id this install belongs to ("php", "nginx", …).
    pub lang: String,
    /// Full version of the build, e.g. "8.3.14".
    pub version: String,
    /// Absolute path to the primary binary inside PortBay's runtimes tree.
    pub binary: PathBuf,
    /// Architecture this build targets ("aarch64" / "x86_64").
    pub arch: String,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MobileRunConfig {
    /// Flutter flavor or Android build variant, e.g. `staging` / `debug`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flavor: Option<String>,
    /// Xcode scheme or Android module, e.g. `App` / `app`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Flutter device id, Android serial, or xcodebuild destination string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
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
    fn project_deploy_round_trips_and_defaults_exclude() {
        let d = ProjectDeploy {
            connection_id: SshConnectionId::new("web"),
            remote_path: "/var/www/app".into(),
            local_subdir: Some("dist".into()),
            steps: vec!["npm ci".into(), "npm run build".into()],
            exclude: vec!["node_modules".into(), ".git".into()],
        };
        let json = serde_json::to_value(&d).unwrap();
        assert_eq!(json["connectionId"], "web");
        assert_eq!(json["remotePath"], "/var/www/app");
        assert_eq!(json["localSubdir"], "dist");
        let back: ProjectDeploy = serde_json::from_value(json).unwrap();
        assert_eq!(back, d);

        // A config with no `exclude` key falls back to the node_modules/.git
        // default; an explicit empty list round-trips as "exclude nothing".
        let defaulted: ProjectDeploy =
            serde_json::from_str(r#"{ "connectionId": "web", "remotePath": "/srv" }"#).unwrap();
        assert_eq!(defaulted.exclude, vec!["node_modules", ".git"]);
        assert!(defaulted.local_subdir.is_none());
        let emptied: ProjectDeploy = serde_json::from_str(
            r#"{ "connectionId": "web", "remotePath": "/srv", "exclude": [] }"#,
        )
        .unwrap();
        assert!(emptied.exclude.is_empty());
    }

    #[test]
    fn project_without_deploy_loads_and_omits_field() {
        // A project JSON written before deploy existed deserialises with
        // `deploy: None`, and a project with no deploy doesn't emit the key.
        let older = serde_json::json!({
            "id": "app",
            "name": "App",
            "path": "/tmp/app",
            "type": "next",
            "hostname": "app.test"
        });
        let loaded: Project = serde_json::from_value(older).unwrap();
        assert!(loaded.deploy.is_none());
        let json = serde_json::to_value(&loaded).unwrap();
        assert!(json.get("deploy").is_none());
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
            cors: None,
            sandbox: None,
            domain: None,
            tunnel: None,
            deploy: None,
            id: ProjectId::new("marketing-site"),
            name: "Marketing Site".into(),
            path: PathBuf::from("/Volumes/DEVSSD/Projects/Clients/Marketing Site"),
            kind: ProjectType::Next,
            framework: None,
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
            pre_start: vec![],
            post_start: vec![],
            tags: vec!["client".into(), "nextjs".into()],
            document_root: None,
            php_version: None,
            web_server: None,
            mobile_run: None,
            runtime: None,
            workspace: None,
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
            cors: None,
            sandbox: None,
            domain: None,
            tunnel: None,
            deploy: None,
            framework: None,
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
            pre_start: vec![],
            post_start: vec![],
            tags: vec![],
            document_root: None,
            php_version: None,
            web_server: None,
            mobile_run: None,
            runtime: None,
            workspace: None,
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
    fn is_static_served_only_for_processless_static_sites() {
        // A static site with no command: served straight off disk by Caddy,
        // its Running/Stopped is the session set — this is the play/pause case.
        let mut p = bare_php_project();
        p.kind = ProjectType::Static;
        p.start_command = None;
        p.web_server = None;
        assert!(p.is_static_served());

        // A static site that DOES run a process (e.g. a build watcher) has a PC
        // process, so its status comes from there — not session-toggled.
        let mut with_cmd = p.clone();
        with_cmd.start_command = Some("npm run watch".into());
        assert!(!with_cmd.is_static_served());

        // Non-static kinds are never static-served, command or not.
        let mut node = p.clone();
        node.kind = ProjectType::Node;
        assert!(!node.is_static_served());
    }

    #[test]
    fn process_compose_id_maps_generated_php_backends() {
        let mut p = bare_php_project();
        assert_eq!(p.process_compose_id(), None);

        p.web_server = Some(WebServer::Nginx);
        assert_eq!(
            p.process_compose_id().as_deref(),
            Some("web-nginx-legacy-php")
        );

        p.web_server = Some(WebServer::Apache);
        assert_eq!(
            p.process_compose_id().as_deref(),
            Some("web-apache-legacy-php")
        );

        p.start_command = Some("php artisan serve".into());
        assert_eq!(p.process_compose_id().as_deref(), Some("legacy-php"));
    }

    #[test]
    fn process_compose_id_covers_workspace_projects_without_a_start_command() {
        // A monorepo app pinned by workspace filter, no explicit start_command —
        // the real bookslash case. `project_to_pc_process` emits a PC entry keyed
        // by the project id for it, so `process_compose_id` MUST return that id;
        // otherwise Start/Stop silently no-op on the project (it has a running
        // process the UI can neither start nor stop). Regression guard for that.
        let mut p = bare_php_project();
        p.id = ProjectId::new("bookslash");
        p.kind = ProjectType::Node;
        p.start_command = None;
        p.web_server = None;
        p.workspace = Some(Workspace {
            package: "@bookslash/web".into(),
            rel_dir: "apps/web".into(),
            tool: WorkspaceTool::Turbo,
        });
        assert_eq!(
            p.process_compose_id().as_deref(),
            Some("bookslash"),
            "workspace projects must resolve to their id-named PC process"
        );
    }

    #[test]
    fn workspace_derives_tool_specific_dev_command() {
        let mk = |tool| Workspace {
            package: "@bookslash/web".into(),
            rel_dir: "apps/web".into(),
            tool,
        };
        assert_eq!(
            mk(WorkspaceTool::Pnpm).derive_dev_command(),
            "pnpm --filter @bookslash/web dev"
        );
        assert_eq!(
            mk(WorkspaceTool::Npm).derive_dev_command(),
            "npm run dev --workspace @bookslash/web"
        );
        assert_eq!(
            mk(WorkspaceTool::Yarn).derive_dev_command(),
            "yarn workspace @bookslash/web dev"
        );
        assert_eq!(
            mk(WorkspaceTool::Bun).derive_dev_command(),
            "bun --filter @bookslash/web dev"
        );
        assert_eq!(
            mk(WorkspaceTool::Turbo).derive_dev_command(),
            "turbo run dev --filter=@bookslash/web"
        );
    }

    #[test]
    fn workspace_app_dir_joins_rel_dir_onto_root() {
        let ws = Workspace {
            package: "@bookslash/web".into(),
            rel_dir: "apps/web".into(),
            tool: WorkspaceTool::Pnpm,
        };
        assert_eq!(
            ws.app_dir(std::path::Path::new("/repos/BookSlash")),
            PathBuf::from("/repos/BookSlash/apps/web")
        );
    }

    #[test]
    fn project_omits_workspace_when_absent_and_loads_older_json_as_none() {
        // Standalone project: workspace is skipped from the wire shape.
        let mut p = bare_php_project();
        assert!(p.workspace.is_none());
        let json = serde_json::to_value(&p).unwrap();
        assert!(
            json.get("workspace").is_none(),
            "absent workspace must be omitted, keeping the field additive"
        );

        // A pre-workspace registry blob (no `workspace` key) still loads,
        // defaulting the field to None — what makes the field need no bump.
        let older = serde_json::json!({
            "id": "legacy", "name": "Legacy", "path": "/tmp/legacy",
            "type": "static", "hostname": "legacy.test"
        });
        let loaded: Project = serde_json::from_value(older).unwrap();
        assert!(loaded.workspace.is_none());

        // And a project carrying a workspace round-trips through JSON.
        p.workspace = Some(Workspace {
            package: "@bookslash/web".into(),
            rel_dir: "apps/web".into(),
            tool: WorkspaceTool::Pnpm,
        });
        let round: Project = serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
        assert_eq!(round.workspace.as_ref().unwrap().package, "@bookslash/web");
        assert_eq!(round.workspace.as_ref().unwrap().tool, WorkspaceTool::Pnpm);
    }

    #[test]
    fn domain_config_is_additive_and_defaults_to_todays_behaviour() {
        // A registry blob written before `domain` existed still loads, with the
        // field defaulting to None — what makes it need no schema bump.
        let older = serde_json::json!({
            "id": "legacy", "name": "Legacy", "path": "/tmp/legacy",
            "type": "static", "hostname": "legacy.test", "https": true
        });
        let loaded: Project = serde_json::from_value(older).unwrap();
        assert!(loaded.domain.is_none());
        // Absent config reproduces the old behaviour through the accessors.
        assert!(loaded.auto_manage_cert(), "cert auto-manage defaults on");
        assert_eq!(loaded.resolver_mode(), ResolverMode::Auto);
        assert!(loaded.path_prefix().is_none());
        assert!(!loaded.include_wildcard_subdomains());
        assert!(!loaded.expose_when_running());

        // A partial config fills the rest from defaults (notably cert=true).
        let cfg: DomainConfig =
            serde_json::from_str(r#"{ "includeWildcardSubdomains": true }"#).unwrap();
        assert!(cfg.auto_manage_cert);
        assert!(cfg.include_wildcard_subdomains);
        assert_eq!(cfg.resolver_mode, ResolverMode::Auto);

        // "/" and blank path prefixes are treated as "serve from root".
        let mut p = bare_php_project();
        p.domain = Some(DomainConfig {
            path_prefix: Some("/".into()),
            ..DomainConfig::default()
        });
        assert!(p.path_prefix().is_none());
        p.domain = Some(DomainConfig {
            path_prefix: Some("/api".into()),
            ..DomainConfig::default()
        });
        assert_eq!(p.path_prefix(), Some("/api"));
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

    #[test]
    fn ssh_connection_loads_without_stage_region_created_at() {
        // A pre-redesign registry has none of the new metadata keys. It must
        // still deserialise, with the new fields falling back to None.
        let older = serde_json::json!({
            "id": "old-host",
            "name": "Legacy box",
            "sshHost": "1.2.3.4",
            "sshUser": "deploy",
            "tags": ["client"],
            "environment": "aws",
            "lastUsed": 1_700_000_000_u64,
        });
        let conn: SshConnection = serde_json::from_value(older).unwrap();
        assert_eq!(conn.ssh_port, 22, "port default applies");
        assert_eq!(conn.metadata.environment.as_deref(), Some("aws"));
        assert_eq!(conn.metadata.stage, None);
        assert_eq!(conn.metadata.region, None);
        assert_eq!(conn.metadata.created_at, None);
    }

    #[test]
    fn ssh_connection_meta_round_trips_new_fields() {
        let conn = SshConnection {
            id: SshConnectionId::new("h1"),
            name: "Staging API".into(),
            ssh_host: "api-staging.example.net".into(),
            ssh_port: 22,
            ssh_user: "ubuntu".into(),
            auth_kind: SshAuthKind::Key,
            key_path: None,
            proxy_jump: None,
            identity_id: None,
            proxy: None,
            metadata: SshConnectionMeta {
                stage: Some("staging".into()),
                region: Some("nyc3".into()),
                created_at: Some(1_712_000_000),
                environment: Some("digitalocean".into()),
                ..Default::default()
            },
        };
        let round: SshConnection =
            serde_json::from_value(serde_json::to_value(&conn).unwrap()).unwrap();
        assert_eq!(round.metadata.stage.as_deref(), Some("staging"));
        assert_eq!(round.metadata.region.as_deref(), Some("nyc3"));
        assert_eq!(round.metadata.created_at, Some(1_712_000_000));
        // camelCase at the JSON top level (the `#[serde(flatten)]` contract).
        let json = serde_json::to_value(&conn).unwrap();
        assert!(json.get("createdAt").is_some());
        assert!(json.get("stage").is_some());
        assert!(json.get("region").is_some());
    }
}
