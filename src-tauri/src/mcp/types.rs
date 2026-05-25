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
    Static,
    Node,
    Flutter,
    Xcode,
    Android,
    Custom,
}

impl From<McpProjectKind> for ProjectType {
    fn from(k: McpProjectKind) -> Self {
        match k {
            McpProjectKind::Next => ProjectType::Next,
            McpProjectKind::Vite => ProjectType::Vite,
            McpProjectKind::Php => ProjectType::Php,
            McpProjectKind::Static => ProjectType::Static,
            McpProjectKind::Node => ProjectType::Node,
            McpProjectKind::Flutter => ProjectType::Flutter,
            McpProjectKind::Xcode => ProjectType::Xcode,
            McpProjectKind::Android => ProjectType::Android,
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
            ProjectType::Static => McpProjectKind::Static,
            ProjectType::Node => McpProjectKind::Node,
            ProjectType::Flutter => McpProjectKind::Flutter,
            ProjectType::Xcode => McpProjectKind::Xcode,
            ProjectType::Android => McpProjectKind::Android,
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

/// One environment-health check.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct DoctorFinding {
    pub check: String,
    /// `ok`, `warn`, or `fail`.
    pub verdict: String,
    pub detail: String,
}

/// Result of `portbay_doctor`.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct DoctorResult {
    /// True when no check returned `fail`.
    pub ok: bool,
    pub findings: Vec<DoctorFinding>,
}

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
