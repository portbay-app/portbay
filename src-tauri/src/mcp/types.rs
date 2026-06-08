//! Agent-facing input and output types for the MCP tool surface.
//!
//! Every `*Args` struct derives [`schemars::JsonSchema`] so its doc-comments
//! become the `inputSchema` field descriptions an agent reads when deciding
//! which tool to call and how to fill it in. Spend the words here — good
//! descriptions are the difference between an agent picking the right tool
//! and flailing. Output structs derive `JsonSchema` too so tools can publish
//! an `outputSchema` and return `structuredContent`.
//!
//! These are deliberately *separate* from the Tauri IPC DTOs in
//! `commands::dto`: those are tuned for the Svelte frontend, these are tuned
//! for an LLM. Keeping them apart means schemars never leaks into the default
//! (non-`mcp`) build.

use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::registry::ProjectType;

// =============================================================================
// Enums (mirrors of core enums, so schemars stays out of the default build)
// =============================================================================

/// The framework / project type. Omit on `add`/`setup` to auto-detect from
/// the folder contents.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum McpProjectKind {
    Next,
    Vite,
    Php,
    Python,
    Static,
    Node,
    Flutter,
    Xcode,
    Android,
    Expo,
    Custom,
}

impl From<McpProjectKind> for ProjectType {
    fn from(k: McpProjectKind) -> Self {
        match k {
            McpProjectKind::Next => ProjectType::Next,
            McpProjectKind::Vite => ProjectType::Vite,
            McpProjectKind::Php => ProjectType::Php,
            McpProjectKind::Python => ProjectType::Python,
            McpProjectKind::Static => ProjectType::Static,
            McpProjectKind::Node => ProjectType::Node,
            McpProjectKind::Flutter => ProjectType::Flutter,
            McpProjectKind::Xcode => ProjectType::Xcode,
            McpProjectKind::Android => ProjectType::Android,
            McpProjectKind::Expo => ProjectType::Expo,
            McpProjectKind::Custom => ProjectType::Custom,
        }
    }
}

impl From<ProjectType> for McpProjectKind {
    fn from(t: ProjectType) -> Self {
        match t {
            ProjectType::Next => McpProjectKind::Next,
            ProjectType::Vite => McpProjectKind::Vite,
            ProjectType::Php => McpProjectKind::Php,
            ProjectType::Python => McpProjectKind::Python,
            ProjectType::Static => McpProjectKind::Static,
            ProjectType::Node => McpProjectKind::Node,
            ProjectType::Flutter => McpProjectKind::Flutter,
            ProjectType::Xcode => McpProjectKind::Xcode,
            ProjectType::Android => McpProjectKind::Android,
            ProjectType::Expo => McpProjectKind::Expo,
            ProjectType::Custom => McpProjectKind::Custom,
        }
    }
}

/// Starter template the agent can scaffold from scratch.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum McpTemplate {
    /// Next.js (TypeScript, App Router, Tailwind) via `pnpm create next-app`.
    Nextjs,
    /// Vite (vanilla TypeScript) via `pnpm create vite`.
    Vite,
    /// Astro (minimal) via `pnpm create astro`.
    Astro,
    /// Laravel via `composer create-project laravel/laravel`.
    Laravel,
    /// A bare PHP project (single `index.php`), served by Caddy.
    Php,
}

// =============================================================================
// Tool inputs
// =============================================================================

/// Register an existing local folder as a PortBay project.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct AddProjectArgs {
    /// Absolute path to the project folder (e.g. `/Users/me/code/blog`).
    /// The folder must already exist.
    pub path: String,
    /// Human-readable display name. Defaults to the folder name.
    #[serde(default)]
    pub name: Option<String>,
    /// Hostname without scheme (e.g. `blog.test`). Defaults to
    /// `<slug>.<domain-suffix>` (the suffix is usually `.test`).
    #[serde(default)]
    pub hostname: Option<String>,
    /// Framework. Omit to auto-detect from the folder (recommended — call
    /// `portbay_detect_project` first if you want to preview the guess).
    #[serde(default)]
    pub kind: Option<McpProjectKind>,
    /// Port the dev server binds to (e.g. `3000`). Omit for static / pure-PHP
    /// projects that Caddy serves directly.
    #[serde(default)]
    pub port: Option<u16>,
    /// Shell command that starts the dev server, run inside the folder
    /// (e.g. `pnpm dev`). Omit for static / Caddy-only projects.
    #[serde(default)]
    pub start_command: Option<String>,
    /// Enable local HTTPS via mkcert. Defaults to `true`.
    #[serde(default)]
    pub https: Option<bool>,
    /// Start the project on daemon boot. Defaults to `false`.
    #[serde(default)]
    pub auto_start: Option<bool>,
    /// PHP version label (e.g. `8.3`). Only meaningful for PHP projects.
    #[serde(default)]
    pub php_version: Option<String>,
    /// Document root relative to the folder (e.g. `public`). PHP projects only.
    #[serde(default)]
    pub document_root: Option<String>,
}

/// Patch an existing project. Only the fields you set are changed; everything
/// else is left as-is.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct UpdateProjectArgs {
    /// The project id (slug) to update — as returned by `portbay_list_projects`.
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// New hostname without scheme. Changing this re-issues the cert and
    /// rewrites the `/etc/hosts` entry on the next reconcile.
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub start_command: Option<String>,
    #[serde(default)]
    pub https: Option<bool>,
    #[serde(default)]
    pub auto_start: Option<bool>,
    /// Replace the project's tags with this list.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

/// Reference a single project by id.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct IdArgs {
    /// The project id (slug), as returned by `portbay_list_projects`.
    pub id: String,
}

/// Start a project. Requires the PortBay daemon to be running.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct StartArgs {
    /// The project id (slug) to start.
    pub id: String,
    /// If the PortBay daemon isn't running, attempt to launch the PortBay app
    /// first, then start the project. Defaults to `false` — when false and the
    /// daemon is down, the tool returns a `SIDECAR_DOWN` error telling you to
    /// open the app. Set `true` only when the user is at their machine and
    /// expects the app to open.
    #[serde(default)]
    pub auto_launch: Option<bool>,
}

/// Inspect a project's live runtime state. Omit `id` for all projects.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct StatusArgs {
    /// Project id (slug). Omit to get the status of every project.
    #[serde(default)]
    pub id: Option<String>,
}

/// Detect the framework + suggested defaults for a folder.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct DetectArgs {
    /// Absolute path to the folder to inspect.
    pub path: String,
}

/// Read recent log output for a project. The best first step when a project
/// won't start or is crashing.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct LogsArgs {
    /// The project id (slug) whose logs you want.
    pub id: String,
    /// How many trailing lines to return. Defaults to 200.
    #[serde(default)]
    pub lines: Option<u32>,
    /// Offset into the log buffer (0 = newest). Defaults to 0.
    #[serde(default)]
    pub offset: Option<u64>,
}

/// Import a project from a committed `.portbay.json` file.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ImportConfigArgs {
    /// Absolute path to the project folder containing `.portbay.json`, OR the
    /// absolute path to the `.portbay.json` file itself.
    pub path: String,
    /// Values for any secrets the file declares (key → value). Secrets the
    /// file lists but you omit are registered as empty placeholders.
    #[serde(default)]
    pub secrets: Option<std::collections::BTreeMap<String, String>>,
}

/// Register an existing folder and immediately start it — the one-call
/// "set this up for me" flow. Auto-detects the framework unless `kind` is set.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SetupArgs {
    /// Absolute path to the project folder (must already exist).
    pub path: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub kind: Option<McpProjectKind>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub start_command: Option<String>,
    /// Enable local HTTPS via mkcert. Defaults to `true`.
    #[serde(default)]
    pub https: Option<bool>,
    /// Start the project right after registering it. Defaults to `true`.
    #[serde(default)]
    pub start_now: Option<bool>,
    /// If the daemon isn't running and `start_now` is true, launch the PortBay
    /// app first. Defaults to `false`.
    #[serde(default)]
    pub auto_launch: Option<bool>,
}

/// Apply a named stack recipe to an existing folder, registering it and
/// (optionally) starting it. List the available recipes with
/// `portbay_list_recipes` first.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SetupFromRecipeArgs {
    /// Recipe id, e.g. `laravel`, `next`, `vite`. See `portbay_list_recipes`.
    pub recipe: String,
    /// Absolute path to the existing project folder.
    pub path: String,
    /// Display name. Defaults to the folder name.
    #[serde(default)]
    pub name: Option<String>,
    /// Hostname without scheme. Defaults to `<slug>.<domain-suffix>`.
    #[serde(default)]
    pub hostname: Option<String>,
    /// Override the recipe's default language version (e.g. `8.2`).
    #[serde(default)]
    pub php_version: Option<String>,
    /// Override the recipe's HTTPS default.
    #[serde(default)]
    pub https: Option<bool>,
    /// Start the project after registering. Defaults to `true`.
    #[serde(default)]
    pub start_now: Option<bool>,
    /// If the daemon isn't running and `start_now` is true, launch the app
    /// first. Defaults to `false`.
    #[serde(default)]
    pub auto_launch: Option<bool>,
}

/// Scaffold a brand-new project from a starter template, register it, and
/// (optionally) start it.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SetupFromTemplateArgs {
    /// Which starter template to scaffold.
    pub template: McpTemplate,
    /// Absolute path to the parent directory the new project folder is created
    /// inside (e.g. `/Users/me/code`).
    pub parent_path: String,
    /// Name of the new project folder to create under `parent_path`.
    pub name: String,
    /// Start the project after scaffolding + registering. Defaults to `false`
    /// (scaffolding can take a while; the agent usually reports the URL and
    /// lets the user start it). Requires the daemon to be running.
    #[serde(default)]
    pub start_now: Option<bool>,
}

// =============================================================================
// Tool outputs
// =============================================================================

/// One project plus its live runtime state (when the daemon is reachable).
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ProjectSummary {
    /// Stable slug id used to reference this project in other tools.
    pub id: String,
    pub name: String,
    /// Framework / project type (`next`, `vite`, `php`, …).
    pub kind: String,
    pub hostname: String,
    /// Full URL to open in a browser (scheme + hostname).
    pub url: String,
    pub https: bool,
    pub port: Option<u16>,
    /// Lifecycle status: `running`, `starting`, `stopped`, `crashed`,
    /// `unhealthy`, `port_conflict`, or `unknown` when the daemon is down.
    pub status: String,
    /// Process id when running.
    pub pid: Option<u32>,
    /// Number of times the process has restarted.
    pub restarts: Option<u32>,
    /// Last readiness-probe result (e.g. `Ready`), when known.
    pub ready: Option<String>,
}

/// Result of `portbay_list_projects`.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ListProjectsResult {
    /// Whether the PortBay daemon answered. When false, `status`/`pid` fields
    /// reflect the registry only and live state is unknown.
    pub daemon_reachable: bool,
    pub projects: Vec<ProjectSummary>,
}

/// A mutation acknowledgement (add / update / remove / lifecycle).
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct OpResult {
    pub ok: bool,
    /// The project the operation acted on, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectSummary>,
    /// Human-readable summary of what happened.
    pub detail: String,
    /// Non-fatal warnings (e.g. `/etc/hosts` couldn't be updated without sudo).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Result of `portbay_detect_project` — a non-committal preview the agent can
/// confirm with the user before calling `portbay_add_project`.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct DetectResult {
    pub kind: String,
    pub suggested_id: String,
    pub suggested_name: String,
    pub suggested_hostname: String,
    pub suggested_port: Option<u16>,
    pub suggested_start_command: Option<String>,
    pub suggested_document_root: Option<String>,
    pub suggested_php_version: Option<String>,
}

// `portbay_doctor` returns `crate::doctor::DoctorReport` directly (the shared
// core behind the CLI `portbay doctor`), so there's no MCP-local doctor type.

/// One sidecar's state.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SidecarReport {
    pub name: String,
    /// `running`, `stopped`, `not_installed`, or `unknown`.
    pub state: String,
    pub detail: String,
}

/// Result of `portbay_sidecar_status`.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SidecarStatusResult {
    pub daemon_reachable: bool,
    pub sidecars: Vec<SidecarReport>,
}

/// Result of `portbay_logs`.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct LogsResult {
    pub id: String,
    pub lines: Vec<String>,
}

/// One stack recipe in the catalog.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct RecipeSummary {
    pub id: String,
    pub title: String,
    pub description: String,
    /// Project type the recipe registers (`next`, `php`, …).
    pub project_type: String,
    pub php_version: Option<String>,
    pub document_root: Option<String>,
    pub https: bool,
    /// Recommended database engine (`engine:version`) when the stack expects
    /// one. Automatic provisioning is a follow-on — see `composes_fully`.
    pub database: Option<String>,
    /// Whether the stack benefits from a local mail catcher.
    pub mail: bool,
    /// True when the recipe composes entirely today; false when it recommends
    /// a database or mail service that PortBay can't yet provision
    /// automatically (the project still registers, with a note).
    pub composes_fully: bool,
}

/// Result of `portbay_list_recipes`.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ListRecipesResult {
    pub recipes: Vec<RecipeSummary>,
}

/// Result of `portbay_export_config`.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ExportResult {
    /// Absolute path of the written `.portbay.json`.
    pub wrote: String,
    pub env_count: usize,
    /// Names of secret env vars the file references (values are never written).
    pub secret_names: Vec<String>,
}

// =============================================================================
// Tunnel tool inputs
// =============================================================================

/// Look up one active tunnel by project id.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct TunnelStatusArgs {
    /// The project id (slug) whose tunnel you want. Must match the id used
    /// when the share was started in the PortBay app.
    pub id: String,
}

/// Look up one SSH tunnel by its id.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SshTunnelStatusArgs {
    /// The SSH tunnel id (slug), as returned by `portbay_list_ssh_tunnels`.
    pub id: String,
}

/// Run one command on a saved SSH connection's remote host. Only reachable when
/// the operator has explicitly enabled the `ssh-exec` toolset (see
/// `portbay_ssh_execute`).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SshExecuteArgs {
    /// The SSH connection id (slug), as returned by `portbay_list_ssh_connections`.
    pub connection_id: String,
    /// The shell command to run on the remote host.
    pub command: String,
    /// Optional working directory to run from (`cd <cwd> && <command>`).
    #[serde(default)]
    pub cwd: Option<String>,
}

// =============================================================================
// Group tool inputs
// =============================================================================

/// Reference a group by its id.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct GroupIdArgs {
    /// The group id (slug), as returned by `portbay_list_groups`.
    pub id: String,
}

/// Create a new project group.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct CreateGroupArgs {
    /// Human-readable display name for the group (e.g. `"Backend services"`).
    pub name: String,
    /// Explicit group id (url-safe slug). When omitted, derived from `name`.
    #[serde(default)]
    pub id: Option<String>,
    /// Project ids (slugs) to include in the group. May be empty — members
    /// can be added later via `portbay_update_group`.
    #[serde(default)]
    pub project_ids: Vec<String>,
}

/// Patch an existing group. Only the fields you set are changed.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct UpdateGroupArgs {
    /// The group id (slug) to update — as returned by `portbay_list_groups`.
    pub id: String,
    /// New display name. Leave unset to keep the current name.
    #[serde(default)]
    pub name: Option<String>,
    /// Full replacement member list. Leave unset to keep the current members.
    /// When set, this replaces the entire list — it is not a merge.
    #[serde(default)]
    pub project_ids: Option<Vec<String>>,
}

/// List the runnable apps inside a JS monorepo so the agent can register just
/// one instead of the whole turbo fan-out. Returns `null` for a plain
/// (non-monorepo) folder — use `portbay_detect_project` instead.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct DetectWorkspaceAppsArgs {
    /// Absolute path to the folder to inspect (typically the monorepo root).
    pub path: String,
}

// =============================================================================
// Workspace detection outputs
// =============================================================================

/// One runnable app found inside a JS monorepo.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct WorkspaceAppSummary {
    /// The package's `name` from its `package.json` (may include a scope prefix
    /// such as `@acme/web`).
    pub package: String,
    /// Directory path relative to the monorepo root (e.g. `apps/web`).
    pub rel_dir: String,
    /// Absolute path to the package directory.
    pub path: String,
    /// Detected framework (`next`, `vite`, `node`, …).
    pub kind: String,
    /// Suggested PortBay project id (url-safe slug derived from the leaf dir).
    pub suggested_id: String,
    /// Suggested hostname (e.g. `web.portbay.test`).
    pub suggested_hostname: String,
    /// Dev-server port detected from the framework, when applicable.
    pub suggested_port: Option<u16>,
    /// Shell command that starts this app in isolation (e.g. `pnpm dev`).
    pub suggested_start_command: Option<String>,
}

/// Result of `portbay_detect_workspace_apps`. When the path is not a
/// recognised JS monorepo, this is `null` — the caller should fall back to
/// `portbay_detect_project` for single-project detection.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct WorkspaceScanResult {
    /// Absolute path of the detected monorepo root.
    pub root: String,
    /// Package manager / build tool detected from the lockfile
    /// (`pnpm`, `npm`, `yarn`, `bun`).
    pub tool: String,
    /// Runnable apps found in the monorepo (those declaring a `dev` script).
    pub apps: Vec<WorkspaceAppSummary>,
}

// =============================================================================
// Runtime tool inputs
// =============================================================================

/// Set or clear the default version for a language. Omit `version` (or pass
/// `null`) to clear the default; provide a version label to set it.
/// Rejected if the version is not currently surfaced by `portbay_list_runtimes`.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SetDefaultRuntimeArgs {
    /// Language id, e.g. `php`, `node`, `python`, `bun`, `go`, `ruby`,
    /// `flutter`. See `portbay_list_runtimes` for valid ids.
    pub lang: String,
    /// Version label to set as the default (e.g. `"8.3"`, `"20"`). Omit or
    /// pass `null` to clear the current default.
    #[serde(default)]
    pub version: Option<String>,
}

/// Register an existing binary as a manual runtime install. The binary is
/// probed for its version; it must report a recognisable version string.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct AddRuntimePathArgs {
    /// Language id the binary belongs to (e.g. `php`, `node`).
    pub lang: String,
    /// Absolute path to the runtime binary (e.g. `/usr/local/bin/php`).
    pub path: String,
}

/// Remove a manually-added runtime install by language + version label.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RemoveRuntimePathArgs {
    /// Language id (e.g. `php`, `node`).
    pub lang: String,
    /// Version label as returned by `portbay_list_runtimes` (e.g. `"8.3"`).
    pub version: String,
}

// =============================================================================
// Runtime tool outputs
// =============================================================================

/// One version of a runtime, as returned by `portbay_list_runtimes`.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct RuntimeVersionSummary {
    /// Version label (e.g. `"8.3"`, `"22.11.0"`).
    pub version: String,
    /// Where the install came from (`homebrew`, `asdf`, `mise`, `nvm`,
    /// `pyenv`, `system`, `manual`, …).
    pub source: String,
    /// Absolute path to the primary binary.
    pub binary: String,
    /// True when this version is the language's configured default.
    pub is_default: bool,
}

/// One language and its detected installs, as returned by
/// `portbay_list_runtimes`.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct RuntimeLanguageSummary {
    /// Stable language id (e.g. `php`, `node`, `python`).
    pub id: String,
    /// Human-readable label (e.g. `"PHP"`, `"Node.js"`).
    pub display_name: String,
    /// The version label currently configured as this language's default,
    /// or `null` when none is set.
    pub default_version: Option<String>,
    /// All detected + manually-added versions on this machine.
    pub versions: Vec<RuntimeVersionSummary>,
    /// Hint for installing via the system package manager when no versions
    /// are detected (e.g. `"brew install php"`).
    pub install_hint: String,
}

// =============================================================================
// Database tool inputs
// =============================================================================

/// Reference a single database instance by id.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct DatabaseIdArgs {
    /// The database instance id (slug), as returned by `portbay_list_databases`.
    pub id: String,
}

/// Provision and register a new database instance. The engine binary must
/// already be installed (check with `portbay_list_database_engines`; installing
/// an engine via Homebrew is done from the PortBay app).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct CreateDatabaseArgs {
    /// Engine id: `mysql`, `postgres`, `mariadb`, `redis`, `mongo`, or `memcached`.
    pub engine: String,
    /// Human-readable name. The instance id is slugified from this.
    pub name: String,
    /// Port to bind. Omit to auto-allocate from the engine's default upward.
    #[serde(default)]
    pub port: Option<u16>,
    /// Start the instance on daemon boot. Defaults to `false`.
    #[serde(default)]
    pub auto_start: Option<bool>,
}

/// Remove a database instance.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RemoveDatabaseArgs {
    /// The database instance id (slug) to remove.
    pub id: String,
    /// Also delete the instance's on-disk data directory. Defaults to `false`
    /// (the registration is removed but the data is kept).
    #[serde(default)]
    pub delete_data: Option<bool>,
}

/// Link or unlink a database instance and a project.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct LinkDatabaseArgs {
    /// The database instance id (slug).
    pub id: String,
    /// The project id (slug) to link/unlink. PortBay injects the instance's
    /// connection env vars into a linked project's process.
    pub project_id: String,
}

/// Toggle a database instance's auto-start.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SetDatabaseAutoStartArgs {
    /// The database instance id (slug).
    pub id: String,
    /// Whether the instance should start when the PortBay daemon boots.
    pub auto_start: bool,
}

/// Inspect the schema (tables, columns, foreign keys) of a database instance.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct DatabaseSchemaArgs {
    /// The database instance id (slug), from `portbay_list_databases`.
    pub id: String,
}

/// Run a read-only query against a database instance.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct DatabaseQueryArgs {
    /// The database instance id (slug), from `portbay_list_databases`.
    pub id: String,
    /// A single read-only statement (`SELECT`/`WITH`/`SHOW`/`EXPLAIN`/…).
    /// Writes, DDL, multiple statements, and CTE-wrapped writes are rejected.
    pub sql: String,
    /// For server engines, the schema/database to run against (e.g. `app_dev`).
    /// Omit for SQLite or to use the instance default.
    #[serde(default)]
    pub schema: Option<String>,
    /// Max rows to return (clamped to 1..=500; defaults to 100). Unbounded
    /// `SELECT`s are capped server-side regardless.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Get the query plan (`EXPLAIN`) for a read-only statement.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct DatabaseExplainArgs {
    /// The database instance id (slug), from `portbay_list_databases`.
    pub id: String,
    /// A single read-only statement to explain.
    pub sql: String,
    /// For server engines, the schema/database to plan against. Omit for SQLite.
    #[serde(default)]
    pub schema: Option<String>,
    /// When true, run `EXPLAIN ANALYZE` (PostgreSQL: ANALYZE + BUFFERS) to collect
    /// actual execution timing and buffer hit data. The query is actually executed.
    /// Ignored for SQLite. Defaults to false.
    #[serde(default)]
    pub analyze: bool,
}

/// Run a WRITE/DDL statement, gated on the user approving it in the PortBay app.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct DatabaseExecuteArgs {
    /// The database instance id (slug), from `portbay_list_databases`.
    pub id: String,
    /// A single write/DDL statement (INSERT/UPDATE/DELETE/CREATE/ALTER/…). It
    /// will NOT run until the user approves it in PortBay. Read-only statements
    /// are rejected — use `portbay_db_query` for those.
    pub sql: String,
    /// For server engines, the schema/database to run against. Omit for SQLite.
    #[serde(default)]
    pub schema: Option<String>,
}

// =============================================================================
// DNS / domain tool inputs
// =============================================================================

/// Change the local domain suffix.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SetDomainSuffixArgs {
    /// The new suffix (e.g. `test`, `localhost`, `portbay.test`). Reserved
    /// public TLDs like `.com` are rejected. Every project hostname is rewritten
    /// to this suffix and their HTTPS certs are dropped (the app reissues them).
    pub suffix: String,
}

// =============================================================================
// Sandbox tool inputs
// =============================================================================

/// Enable Sandboxed Run on a project (macOS only). The project keeps running
/// under Process Compose, but its launch command is wrapped by a generated
/// Seatbelt profile that denies access to credential stores, browser data, and
/// every `.env` outside the project.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct EnableSandboxArgs {
    /// The project id (slug) to sandbox.
    pub id: String,
    /// Network access granted inside the sandbox: `loopback_only` (default —
    /// local dev server only), `outbound` (also allow package-manager fetches),
    /// `full` (all networking), or `blocked` (no networking).
    #[serde(default)]
    pub network: Option<String>,
    /// Wipe the per-run cache/temp scratch dir before each sandboxed start and
    /// steer `TMPDIR` + package-manager caches into it. Defaults to `true`.
    #[serde(default)]
    pub ephemeral: Option<bool>,
}

/// Report sandbox state for one project (set `id`) or all projects (omit `id`).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SandboxStatusArgs {
    /// Project id to report on. Omit to list every project's sandbox state.
    #[serde(default)]
    pub id: Option<String>,
}

/// Read recent sandbox-denial lines from a project's logs.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SandboxViolationsArgs {
    /// The project id (slug) whose logs to scan.
    pub id: String,
    /// How many recent log lines to scan for `deny(...)` / "operation not
    /// permitted" entries. Defaults to 250.
    #[serde(default)]
    pub limit: Option<u32>,
}

// =============================================================================
// Certificate tool inputs
// =============================================================================

/// Report local-HTTPS cert metadata for one project (set `id`) or all projects
/// that have a cert (omit `id`).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct CertInfoArgs {
    /// Project id to report on. Omit to list every project's cert.
    #[serde(default)]
    pub id: Option<String>,
}

// =============================================================================
// HTTP request inspector tool inputs
// =============================================================================

/// Read recent HTTP requests Caddy handled.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RecentRequestsArgs {
    /// How many recent requests to return (oldest→newest). Defaults to 200,
    /// capped at 2000.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Only return requests routed to this project id (slug). Omit for all
    /// projects' traffic.
    #[serde(default)]
    pub project: Option<String>,
}

// =============================================================================
// Migration-import tool inputs (Herd / ServBay / MAMP → PortBay)
// =============================================================================

/// Preview the sites a migration source exposes before importing.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct PreviewImportArgs {
    /// The source tool to scan: `herd`, `servbay`, or `mamp`.
    pub source: String,
}

/// Import sites from a migration source into the registry.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ImportProjectsArgs {
    /// The source tool to import from: `herd`, `servbay`, or `mamp`.
    pub source: String,
    /// Suggested ids (from `portbay_preview_import`) to import. Omit or leave
    /// empty together with `all: true` to import every site the source exposes.
    #[serde(default)]
    pub ids: Option<Vec<String>>,
    /// Import every site the source exposes, ignoring `ids`. Defaults to false.
    #[serde(default)]
    pub all: Option<bool>,
}

// ---------------------------------------------------------------------------
// Per-project task board ("Project Context & Task Authority")
// ---------------------------------------------------------------------------

/// List a project's task cards.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct TasksListArgs {
    /// Project id.
    pub project: String,
    /// Optional column filter: `Backlog`, `Todo`, `InProgress`, `Blocked`,
    /// `Review`, `Done`, or `Rejected`.
    #[serde(default)]
    pub status: Option<String>,
    /// Include each card's full body, checklist items, attachments and links.
    /// Default `false`: cards come back as compact summaries (`bodyChars`,
    /// `checklist: {done,total}`, `attachments`/`links` counts) — fetch one
    /// card's detail with `portbay_task_get` instead of pulling every body on
    /// the board into context.
    #[serde(default)]
    pub full: Option<bool>,
}

/// Get the next actionable card (the top `Todo`).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct TaskNextArgs {
    /// Project id.
    pub project: String,
}

/// Read one card in full (all fields + the markdown body).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct TaskGetArgs {
    /// Project id.
    pub project: String,
    /// Card id.
    pub id: String,
}

/// Create a new card. Lands in `Backlog` unless `status` is given.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct TaskCreateArgs {
    /// Project id.
    pub project: String,
    /// Card title.
    pub title: String,
    /// Markdown description / body.
    #[serde(default)]
    pub body: Option<String>,
    /// Initial column. Defaults to `Backlog`.
    #[serde(default)]
    pub status: Option<String>,
    /// `critical`, `high`, `medium`, or `low`.
    #[serde(default)]
    pub priority: Option<String>,
    /// Acceptance criteria.
    #[serde(default)]
    pub acceptance: Option<String>,
    /// Files or modules this task is expected to touch — a starting set for the
    /// agent (it may touch more). Naming them lifts first-run success.
    #[serde(default)]
    pub touchpoints: Option<Vec<String>>,
    /// Labels to tag the card with.
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    /// Numeric effort estimate.
    #[serde(default)]
    pub estimate: Option<f64>,
    /// Seed from a built-in template by name ("Implement feature", "Fix bug",
    /// "Write tests", "Refactor"). Explicit fields above win; the template fills
    /// the rest (body, checklist sub-steps, acceptance criteria, labels).
    #[serde(default)]
    pub template: Option<String>,
}

/// Tick or untick a card's checklist item.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct TaskCheckArgs {
    /// Project id.
    pub project: String,
    /// Card id.
    pub id: String,
    /// Checklist item index.
    pub idx: u32,
    /// Mark done (true) or reopen (false). Defaults to true.
    #[serde(default)]
    pub done: Option<bool>,
    /// Run id from the dispatch prompt (validated against the card's claim).
    #[serde(default)]
    pub run_id: Option<String>,
}

/// Append items to a card's checklist — the agent's own sub-step tracker.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct TaskChecklistAddArgs {
    /// Project id.
    pub project: String,
    /// Card id.
    pub id: String,
    /// Sub-task descriptions to append (e.g. `["P0: wire form", "P1: tests"]`).
    pub items: Vec<String>,
    /// Optional checklist heading (set on first use).
    #[serde(default)]
    pub label: Option<String>,
    /// Run id from the dispatch prompt.
    #[serde(default)]
    pub run_id: Option<String>,
}

/// Post a comment on a card (shows in its activity thread).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct TaskCommentArgs {
    /// Project id.
    pub project: String,
    /// Card id.
    pub id: String,
    /// Comment text (markdown).
    pub text: String,
    /// Run id from the dispatch prompt.
    #[serde(default)]
    pub run_id: Option<String>,
}

/// Acknowledge a dispatched card — proves the agent engaged.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct TaskAckArgs {
    /// Project id.
    pub project: String,
    /// Card id.
    pub id: String,
    /// The run id from the dispatch prompt.
    pub run_id: String,
}

/// Advance a card and/or post a progress note. Validates `run_id` against the
/// card's active claim and enforces the transition rules (an agent may never
/// set `Rejected`).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct TaskUpdateArgs {
    /// Project id.
    pub project: String,
    /// Card id.
    pub id: String,
    /// The run id from the dispatch prompt. Required to advance a claimed card.
    #[serde(default)]
    pub run_id: Option<String>,
    /// New column: `InProgress`, `Done`, `Blocked`, `Review`, or `Todo`.
    #[serde(default)]
    pub status: Option<String>,
    /// A short progress note (recorded in the audit log; also a heartbeat).
    #[serde(default)]
    pub note: Option<String>,
    /// Reason, when moving to `Blocked`.
    #[serde(default)]
    pub reason: Option<String>,
    /// Record the files / modules you actually touched (or found relevant).
    /// Replaces the card's touchpoints — a working artifact for the next run
    /// and the human's review. Other card content stays human-owned.
    #[serde(default)]
    pub touchpoints: Option<Vec<String>>,
}

/// Read the current continuation brief.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct HandoffGetArgs {
    /// Project id.
    pub project: String,
}

/// Append a new entry to the rolling hand-off log (newest on top).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct HandoffUpdateArgs {
    /// Project id.
    pub project: String,
    /// The "where we left off" note: current goal, what was just done, and the
    /// next concrete step. Keep it minimal — the log is size-capped and prunes
    /// oldest entries first.
    pub narrative: String,
    /// Who is writing this entry (e.g. your agent name, "claude", "codex"). Used
    /// to sign the entry. Optional; defaults to "agent".
    #[serde(default)]
    pub author: Option<String>,
}

/// Finish (or block) a dispatched card in one call: acknowledge, optionally
/// comment + update the hand-off, then set the terminal status. Idempotent — a
/// retry once the card already sits in the target status is a no-op (no
/// duplicate comment / hand-off entry).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct TaskCompleteArgs {
    /// Project id.
    pub project: String,
    /// Card id.
    pub id: String,
    /// The run id from the dispatch prompt.
    pub run_id: String,
    /// Terminal status to set: `Done` (default), `Blocked`, or `Review`. You may
    /// NOT set `Rejected` — that's human-only.
    #[serde(default)]
    pub status: Option<String>,
    /// Optional acceptance/confirmation comment for the card's thread. Skip for
    /// routine work — a comment per task is not required.
    #[serde(default)]
    pub comment: Option<String>,
    /// Optional minimal hand-off note (what changed, the next step, open items),
    /// appended as the newest continuation-brief entry.
    #[serde(default)]
    pub handoff: Option<String>,
    /// Sign the hand-off entry (your agent name). Optional; falls back to the
    /// dispatched agent, then "agent".
    #[serde(default)]
    pub author: Option<String>,
    /// Record the files / modules you actually touched (working artifact for the
    /// next run and the human's review).
    #[serde(default)]
    pub touchpoints: Option<Vec<String>>,
}

/// Read the project's learnings memory (what works here).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct LearningsGetArgs {
    /// Project id.
    pub project: String,
}

/// Record a project learning — a validated approach or a correction the next run
/// here should know. Appended newest-first; an identical rule is a no-op.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct LearningAddArgs {
    /// Project id.
    pub project: String,
    /// The rule itself — a concise, actionable lesson (e.g. "Run `composer test`,
    /// not `phpunit` directly"). One lesson per call; keep it tight.
    pub text: String,
    /// WHY the rule holds — the reasoning that makes it durable (e.g. "the project
    /// wraps phpunit with an env bootstrap"). The thing that lets the next agent
    /// trust it instead of re-discovering it. Strongly recommended.
    #[serde(default)]
    pub why: Option<String>,
    /// Optional concrete "How to apply" guidance for the next run.
    #[serde(default)]
    pub how: Option<String>,
}
