//! MCP protocol adapter — thin `rmcp` wrappers over [`super::ops::McpContext`].
//!
//! Each tool maps typed `Parameters<T>` → an operation → a [`CallToolResult`]
//! carrying `structuredContent` (plus a JSON text mirror for older clients).
//! Tool *execution* errors come back as `isError: true` results holding the
//! standard PortBay error envelope (`code` / `whatHappened` / `whyItMatters`
//! / `whoCausedIt` / `actions`) so the agent can read the failure and recover
//! — exactly the recovery affordance the spec recommends.
//!
//! Annotations are set honestly: read-only tools are marked `read_only_hint`,
//! and `remove`/`stop_all` carry `destructive_hint` so clients can gate them
//! behind confirmation.

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler,
};
use serde::Serialize;
use serde_json::json;

use super::ops::McpContext;
use super::types::*;
use crate::error::{AppError, AppResult};

/// Named tool groups, so an operator can scope the surface an agent sees
/// (`--toolsets projects,diagnostics`) the way the GitHub MCP server does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolGroup {
    /// Registry CRUD + setup: list, status, detect, add, update, remove,
    /// export, import, setup.
    Projects,
    /// start, stop, restart, stop_all.
    Lifecycle,
    /// logs, doctor, sidecar_status.
    Diagnostics,
    /// setup_from_template — runs upstream scaffolders (network access).
    Scaffold,
    /// Group CRUD + batch lifecycle: list, create, update, remove,
    /// start, stop, restart.
    Groups,
    /// Read-only tunnel visibility: list active public tunnels, look up one by id.
    Tunnels,
    /// Runtime management: list detected versions, set defaults, add/remove manual paths.
    Runtimes,
    /// Database engines + owned instances: list, create, lifecycle, link, auto-start.
    Databases,
    /// DNS / domains: resolver status, DNS records, domain-suffix change.
    Dns,
    /// Sandboxed Run (macOS Seatbelt): status, enable, disable, violations.
    Sandbox,
    /// HTTP request inspector: recent Caddy requests, clear the log.
    Inspector,
    /// Local-HTTPS certificates: per-project cert info, reissue.
    Certs,
}

impl ToolGroup {
    pub fn all() -> Vec<ToolGroup> {
        vec![
            ToolGroup::Projects,
            ToolGroup::Lifecycle,
            ToolGroup::Diagnostics,
            ToolGroup::Scaffold,
            ToolGroup::Groups,
            ToolGroup::Tunnels,
            ToolGroup::Runtimes,
            ToolGroup::Databases,
            ToolGroup::Dns,
            ToolGroup::Sandbox,
            ToolGroup::Inspector,
            ToolGroup::Certs,
        ]
    }

    /// Parse a toolset name (case-insensitive). `all` expands to every group.
    pub fn parse(s: &str) -> Result<Vec<ToolGroup>, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "all" => Ok(Self::all()),
            "projects" => Ok(vec![ToolGroup::Projects]),
            "lifecycle" => Ok(vec![ToolGroup::Lifecycle]),
            "diagnostics" => Ok(vec![ToolGroup::Diagnostics]),
            "scaffold" => Ok(vec![ToolGroup::Scaffold]),
            "groups" => Ok(vec![ToolGroup::Groups]),
            "tunnels" => Ok(vec![ToolGroup::Tunnels]),
            "runtimes" => Ok(vec![ToolGroup::Runtimes]),
            "databases" => Ok(vec![ToolGroup::Databases]),
            "dns" => Ok(vec![ToolGroup::Dns]),
            "sandbox" => Ok(vec![ToolGroup::Sandbox]),
            "inspector" => Ok(vec![ToolGroup::Inspector]),
            "certs" => Ok(vec![ToolGroup::Certs]),
            other => Err(format!(
                "unknown toolset `{other}` (valid: projects, lifecycle, diagnostics, scaffold, \
                 groups, tunnels, runtimes, databases, dns, sandbox, inspector, certs, all)"
            )),
        }
    }

    /// Parse a comma-separated list (e.g. `projects,diagnostics`).
    pub fn parse_list(s: &str) -> Result<Vec<ToolGroup>, String> {
        let mut out: Vec<ToolGroup> = Vec::new();
        for part in s.split(',').filter(|p| !p.trim().is_empty()) {
            for g in Self::parse(part)? {
                if !out.contains(&g) {
                    out.push(g);
                }
            }
        }
        if out.is_empty() {
            return Err("no toolsets specified".into());
        }
        Ok(out)
    }
}

/// How the tool surface is scoped for a given server run.
#[derive(Debug, Clone)]
pub struct McpConfig {
    /// When true, every mutating tool (add/update/remove/lifecycle/import/
    /// export/setup/scaffold) is removed — the agent can inspect but never
    /// change anything. The enterprise-safe default for "let the agent look".
    pub read_only: bool,
    /// Which tool groups are exposed. Defaults to all.
    pub toolsets: Vec<ToolGroup>,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            read_only: false,
            toolsets: ToolGroup::all(),
        }
    }
}

/// Tool name → (group, is_mutating). The single source of truth for both
/// toolset scoping and read-only filtering. Tool names MUST match the
/// `#[tool(name = …)]` attributes exactly.
const TOOL_REGISTRY: &[(&str, ToolGroup, bool)] = &[
    ("portbay_list_projects", ToolGroup::Projects, false),
    ("portbay_status", ToolGroup::Projects, false),
    ("portbay_detect_project", ToolGroup::Projects, false),
    ("portbay_detect_workspace_apps", ToolGroup::Projects, false),
    ("portbay_list_recipes", ToolGroup::Projects, false),
    ("portbay_setup_from_recipe", ToolGroup::Projects, true),
    ("portbay_add_project", ToolGroup::Projects, true),
    ("portbay_update_project", ToolGroup::Projects, true),
    ("portbay_remove_project", ToolGroup::Projects, true),
    ("portbay_export_config", ToolGroup::Projects, true),
    ("portbay_import_config", ToolGroup::Projects, true),
    ("portbay_setup", ToolGroup::Projects, true),
    ("portbay_start", ToolGroup::Lifecycle, true),
    ("portbay_stop", ToolGroup::Lifecycle, true),
    ("portbay_restart", ToolGroup::Lifecycle, true),
    ("portbay_stop_all", ToolGroup::Lifecycle, true),
    ("portbay_logs", ToolGroup::Diagnostics, false),
    ("portbay_doctor", ToolGroup::Diagnostics, false),
    ("portbay_sidecar_status", ToolGroup::Diagnostics, false),
    ("portbay_setup_from_template", ToolGroup::Scaffold, true),
    ("portbay_list_groups", ToolGroup::Groups, false),
    ("portbay_create_group", ToolGroup::Groups, true),
    ("portbay_update_group", ToolGroup::Groups, true),
    ("portbay_remove_group", ToolGroup::Groups, true),
    ("portbay_start_group", ToolGroup::Groups, true),
    ("portbay_stop_group", ToolGroup::Groups, true),
    ("portbay_restart_group", ToolGroup::Groups, true),
    ("portbay_list_tunnels", ToolGroup::Tunnels, false),
    ("portbay_tunnel_status", ToolGroup::Tunnels, false),
    ("portbay_list_runtimes", ToolGroup::Runtimes, false),
    ("portbay_set_default_runtime", ToolGroup::Runtimes, true),
    ("portbay_add_runtime_path", ToolGroup::Runtimes, true),
    ("portbay_remove_runtime_path", ToolGroup::Runtimes, true),
    ("portbay_list_database_engines", ToolGroup::Databases, false),
    ("portbay_list_databases", ToolGroup::Databases, false),
    ("portbay_database_connection", ToolGroup::Databases, false),
    ("portbay_create_database", ToolGroup::Databases, true),
    ("portbay_remove_database", ToolGroup::Databases, true),
    ("portbay_start_database", ToolGroup::Databases, true),
    ("portbay_stop_database", ToolGroup::Databases, true),
    ("portbay_restart_database", ToolGroup::Databases, true),
    ("portbay_link_database", ToolGroup::Databases, true),
    ("portbay_unlink_database", ToolGroup::Databases, true),
    ("portbay_set_database_auto_start", ToolGroup::Databases, true),
    ("portbay_dns_status", ToolGroup::Dns, false),
    ("portbay_list_dns_records", ToolGroup::Dns, false),
    ("portbay_set_domain_suffix", ToolGroup::Dns, true),
    ("portbay_sandbox_status", ToolGroup::Sandbox, false),
    ("portbay_sandbox_violations", ToolGroup::Sandbox, false),
    ("portbay_enable_sandbox", ToolGroup::Sandbox, true),
    ("portbay_disable_sandbox", ToolGroup::Sandbox, true),
    ("portbay_recent_requests", ToolGroup::Inspector, false),
    ("portbay_clear_requests", ToolGroup::Inspector, true),
    ("portbay_cert_info", ToolGroup::Certs, false),
    ("portbay_reissue_cert", ToolGroup::Certs, true),
];

/// The PortBay MCP server. Holds the operations context and the (possibly
/// filtered) tool router. Cheap to clone (the context is just paths + a port).
#[derive(Clone)]
pub struct PortbayMcp {
    ctx: McpContext,
    /// The (filtered) tool router this instance dispatches through. We point
    /// `#[tool_handler(router = self.tool_router)]` at this field so read-only
    /// / toolset filtering done in `with_config` actually takes effect — the
    /// macro's default (`Self::tool_router()`) would regenerate the full set.
    tool_router: ToolRouter<PortbayMcp>,
    read_only: bool,
}

/// Map an operation result into a tool result. Success becomes structured
/// content; failure becomes an `isError` result carrying the error envelope
/// so the agent can self-correct.
fn finish<T: Serialize>(r: AppResult<T>) -> Result<CallToolResult, McpError> {
    match r {
        Ok(v) => {
            let value = serde_json::to_value(&v)
                .unwrap_or_else(|e| json!({ "_serializeError": e.to_string() }));
            Ok(CallToolResult::structured(value))
        }
        Err(e) => {
            let value = serde_json::to_value(&e)
                .unwrap_or_else(|_| json!({ "code": "INTERNAL", "whatHappened": e.to_string() }));
            Ok(CallToolResult::structured_error(value))
        }
    }
}

#[tool_router]
impl PortbayMcp {
    /// Build the server with the full tool surface.
    pub fn new(ctx: McpContext) -> Self {
        Self::with_config(ctx, McpConfig::default())
    }

    /// Build the server, filtering the tool surface per `config`: tools whose
    /// group isn't enabled are removed, and (in read-only mode) so is every
    /// mutating tool. Filtering at the router means removed tools don't appear
    /// in `tools/list` and can't be called at all.
    pub fn with_config(ctx: McpContext, config: McpConfig) -> Self {
        let mut router = Self::tool_router();
        for &(name, group, is_mutating) in TOOL_REGISTRY {
            let group_enabled = config.toolsets.contains(&group);
            let blocked_by_read_only = config.read_only && is_mutating;
            if !group_enabled || blocked_by_read_only {
                router.remove_route(name);
            }
        }
        Self {
            ctx,
            tool_router: router,
            read_only: config.read_only,
        }
    }

    // ---- Read / inspect -----------------------------------------------------

    #[tool(
        name = "portbay_list_projects",
        description = "List every project registered with PortBay, each with its hostname, URL, \
                       and — when the PortBay daemon is running — live status (running/stopped/\
                       crashed), PID, and restart count. Start here to see what exists before \
                       acting. `daemon_reachable: false` means live fields are unknown and only \
                       registry data is shown.",
        annotations(
            title = "List projects",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_projects(&self) -> Result<CallToolResult, McpError> {
        finish(self.ctx.list_projects().await)
    }

    #[tool(
        name = "portbay_status",
        description = "Get live runtime detail for one project (set `id`) or all projects (omit \
                       `id`): status, PID, restart count, and last readiness-probe result. Use \
                       this when diagnosing whether something is running. Requires the daemon \
                       for live data; without it, status reads `unknown`.",
        annotations(
            title = "Project status",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn status(
        &self,
        Parameters(args): Parameters<StatusArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.status(args.id.as_deref()).await)
    }

    #[tool(
        name = "portbay_detect_project",
        description = "Inspect a folder and return the detected framework plus suggested defaults \
                       (id, hostname, port, start command). Non-committal — nothing is registered. \
                       Call this first to preview what `portbay_add_project` would do, then confirm \
                       the details with the user.",
        annotations(
            title = "Detect framework",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn detect_project(
        &self,
        Parameters(args): Parameters<DetectArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.detect_project(&args.path))
    }

    #[tool(
        name = "portbay_detect_workspace_apps",
        description = "List the runnable apps inside a JS monorepo so you can register just one \
                       as a standalone PortBay project (instead of a root `turbo run dev` fan-out). \
                       Returns `null` for a plain (non-monorepo) folder — use \
                       `portbay_detect_project` instead for those. Each app entry carries \
                       suggested id, hostname, port, and start command ready for \
                       `portbay_add_project`.",
        annotations(
            title = "Detect workspace apps",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn detect_workspace_apps(
        &self,
        Parameters(args): Parameters<DetectWorkspaceAppsArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.detect_workspace_apps(&args.path))
    }

    #[tool(
        name = "portbay_logs",
        description = "Return recent log output for a project. The first thing to reach for when a \
                       project won't start or is crash-looping — read the logs, then suggest a fix. \
                       Requires the daemon to be running.",
        annotations(title = "Read logs", read_only_hint = true, open_world_hint = false)
    )]
    async fn logs(
        &self,
        Parameters(args): Parameters<LogsArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            self.ctx
                .logs(
                    &args.id,
                    args.lines.unwrap_or(200),
                    args.offset.unwrap_or(0),
                )
                .await,
        )
    }

    #[tool(
        name = "portbay_doctor",
        description = "Run an environment health check: registry readability, whether the daemon \
                       is reachable, required tooling on PATH, and the current license tier. Use \
                       when something is broken and you don't yet know what.",
        annotations(title = "Doctor", read_only_hint = true, open_world_hint = false)
    )]
    async fn doctor(&self) -> Result<CallToolResult, McpError> {
        finish(self.ctx.doctor().await)
    }

    #[tool(
        name = "portbay_sidecar_status",
        description = "Report the state of PortBay's background services. Process Compose is probed \
                       directly; the others (Caddy, mkcert, dnsmasq, mailpit) are managed by the \
                       daemon and reported as install-presence only. Prefer `portbay_doctor` for a \
                       fuller picture.",
        annotations(
            title = "Sidecar status",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn sidecar_status(&self) -> Result<CallToolResult, McpError> {
        finish(self.ctx.sidecar_status().await)
    }

    // ---- Mutations ----------------------------------------------------------

    #[tool(
        name = "portbay_add_project",
        description = "Register an existing local folder as a PortBay project: it gets a local \
                       hostname, optional HTTPS via mkcert, and managed start/stop. Omit `kind` to \
                       auto-detect the framework. Does NOT start the project — call `portbay_start` \
                       after, or use `portbay_setup` to register and start in one step. Returns the \
                       registered project and its URL.",
        annotations(title = "Add project", read_only_hint = false, open_world_hint = false)
    )]
    async fn add_project(
        &self,
        Parameters(args): Parameters<AddProjectArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.add_project(args).await)
    }

    #[tool(
        name = "portbay_update_project",
        description = "Change fields on an existing project (name, hostname, port, start command, \
                       HTTPS, auto-start, tags). Only the fields you provide are modified. Changing \
                       the hostname re-issues the cert on the next reconcile.",
        annotations(
            title = "Update project",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_project(
        &self,
        Parameters(args): Parameters<UpdateProjectArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.update_project(args).await)
    }

    #[tool(
        name = "portbay_remove_project",
        description = "Unregister a project and clean up its cert and hosts entry. The project's \
                       source files on disk are NOT touched. This is irreversible from PortBay's \
                       side — confirm with the user first.",
        annotations(
            title = "Remove project",
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn remove_project(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.remove_project(&args.id).await)
    }

    #[tool(
        name = "portbay_export_config",
        description = "Write a `.portbay.json` into the project folder capturing its setup so it \
                       can be committed and reproduced by teammates (and re-imported with \
                       `portbay_import_config`). Secret values are never written — only their names.",
        annotations(
            title = "Export config",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn export_config(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.export_config(&args.id).await)
    }

    #[tool(
        name = "portbay_import_config",
        description = "Register a project from a committed `.portbay.json`. Pass the project folder \
                       (or the file path directly). Provide `secrets` for any secret env vars the \
                       file declares; omitted ones are registered as empty placeholders.",
        annotations(
            title = "Import config",
            read_only_hint = false,
            open_world_hint = false
        )
    )]
    async fn import_config(
        &self,
        Parameters(args): Parameters<ImportConfigArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.import_config(args).await)
    }

    // ---- Lifecycle ----------------------------------------------------------

    #[tool(
        name = "portbay_start",
        description = "Start a registered project. Requires the PortBay daemon to be running; if \
                       it isn't, the call returns a SIDECAR_DOWN error telling you to open the app \
                       — unless you pass `auto_launch: true`, which opens PortBay first (use only \
                       when the user is at their machine).",
        annotations(
            title = "Start project",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn start(
        &self,
        Parameters(args): Parameters<StartArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            self.ctx
                .start(&args.id, args.auto_launch.unwrap_or(false))
                .await,
        )
    }

    #[tool(
        name = "portbay_stop",
        description = "Stop a running project. Requires the daemon to be running.",
        annotations(
            title = "Stop project",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn stop(&self, Parameters(args): Parameters<IdArgs>) -> Result<CallToolResult, McpError> {
        finish(self.ctx.stop(&args.id).await)
    }

    #[tool(
        name = "portbay_restart",
        description = "Restart a project (stop then start). Requires the daemon to be running.",
        annotations(
            title = "Restart project",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn restart(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.restart(&args.id).await)
    }

    #[tool(
        name = "portbay_stop_all",
        description = "Stop every running PortBay process — the universal kill switch. Requires the \
                       daemon to be running.",
        annotations(
            title = "Stop all",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn stop_all(&self) -> Result<CallToolResult, McpError> {
        finish(self.ctx.stop_all().await)
    }

    // ---- Composite / high-level --------------------------------------------

    #[tool(
        name = "portbay_setup",
        description = "The one-call \"set this up for me\" flow: register an existing folder \
                       (auto-detecting the framework) and immediately start it, returning the live \
                       URL. Set `start_now: false` to register without starting. This is the \
                       fastest path from \"I just scaffolded an app\" to \"it's running at \
                       https://name.test\".",
        annotations(
            title = "Set up project",
            read_only_hint = false,
            open_world_hint = false
        )
    )]
    async fn setup(
        &self,
        Parameters(args): Parameters<SetupArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.setup(args).await)
    }

    #[tool(
        name = "portbay_list_recipes",
        description = "List the available stack recipes — named blueprints (e.g. laravel, next, \
                       vite) that compose a project's framework, language version, document root, \
                       and HTTPS in one step. Map the user's request to a recipe id, then call \
                       portbay_setup_from_recipe. `composes_fully: false` means the recipe also \
                       recommends a database/mail service that isn't auto-provisioned yet.",
        annotations(title = "List recipes", read_only_hint = true, open_world_hint = false)
    )]
    async fn list_recipes(&self) -> Result<CallToolResult, McpError> {
        finish(Ok(self.ctx.list_recipes()))
    }

    #[tool(
        name = "portbay_setup_from_recipe",
        description = "Apply a named stack recipe to an existing folder: register it with the \
                       recipe's framework, language version, document root, and HTTPS, then start \
                       it. The fastest path when the user names a stack (\"set up a Laravel app \
                       at ~/code/blog\"). Use portbay_list_recipes to discover ids. For a brand-new \
                       project from scratch, use portbay_setup_from_template instead.",
        annotations(
            title = "Set up from recipe",
            read_only_hint = false,
            open_world_hint = false
        )
    )]
    async fn setup_from_recipe(
        &self,
        Parameters(args): Parameters<SetupFromRecipeArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.setup_from_recipe(args).await)
    }

    #[tool(
        name = "portbay_setup_from_template",
        description = "Scaffold a brand-new project from a starter template (Next.js, Vite, Astro, \
                       Laravel, or bare PHP) into `parent_path/name`, then register it with PortBay. \
                       Runs the upstream scaffolder (pnpm/composer), which may take a while and \
                       requires network access. Set `start_now: true` to also start it.",
        annotations(
            title = "Scaffold + set up",
            read_only_hint = false,
            open_world_hint = true
        )
    )]
    async fn setup_from_template(
        &self,
        Parameters(args): Parameters<SetupFromTemplateArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.setup_from_template(args).await)
    }

    // ---- Groups -------------------------------------------------------------

    #[tool(
        name = "portbay_list_groups",
        description = "List every project group registered with PortBay. Each group carries \
                       its member project ids, a `known_ids` subset (members that still exist \
                       in the registry), and a `member_count`. Use this to discover group ids \
                       before calling start/stop/restart/update/remove group tools.",
        annotations(
            title = "List groups",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_groups(&self) -> Result<CallToolResult, McpError> {
        finish(self.ctx.list_groups())
    }

    #[tool(
        name = "portbay_create_group",
        description = "Create a named group of projects. Groups let you start, stop, or restart \
                       multiple projects in one call. Provide a `name` (the id is derived from it \
                       automatically, or pass an explicit `id`). Pass `project_ids` as an array of \
                       slugs (e.g. `[\"blog\", \"api\"]`). The projects don't need to exist yet — \
                       unknown ids are tracked and surfaced via `known_ids` on list.",
        annotations(
            title = "Create group",
            read_only_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_group(
        &self,
        Parameters(args): Parameters<CreateGroupArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            self.ctx
                .create_group(args.id, args.name, args.project_ids),
        )
    }

    #[tool(
        name = "portbay_update_group",
        description = "Rename a group or replace its member list. Only the fields you set are \
                       changed. Pass `project_ids` to fully replace the member list (not a merge — \
                       any id not in the new list is removed). Pass `name` to rename without \
                       touching membership.",
        annotations(
            title = "Update group",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_group(
        &self,
        Parameters(args): Parameters<UpdateGroupArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            self.ctx
                .update_group(args.id, args.name, args.project_ids),
        )
    }

    #[tool(
        name = "portbay_remove_group",
        description = "Delete a group. The member projects are NOT affected — only the group \
                       record itself is removed. This is irreversible from PortBay's side — \
                       confirm with the user first.",
        annotations(
            title = "Remove group",
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn remove_group(
        &self,
        Parameters(args): Parameters<GroupIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.remove_group(args.id))
    }

    #[tool(
        name = "portbay_start_group",
        description = "Start every project in a group. Members that have no managed process \
                       (e.g. mobile/Xcode projects) are counted as succeeded and skipped. \
                       Stale members (removed from the registry but still listed in the group) \
                       are counted as failed. Requires the PortBay daemon to be running.",
        annotations(
            title = "Start group",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn start_group(
        &self,
        Parameters(args): Parameters<GroupIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.start_group(args.id).await)
    }

    #[tool(
        name = "portbay_stop_group",
        description = "Stop every project in a group. Members that have no managed process are \
                       counted as succeeded and skipped. Stale members are counted as failed. \
                       Requires the PortBay daemon to be running.",
        annotations(
            title = "Stop group",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn stop_group(
        &self,
        Parameters(args): Parameters<GroupIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.stop_group(args.id).await)
    }

    #[tool(
        name = "portbay_restart_group",
        description = "Restart every project in a group (stop then start). Members that have no \
                       managed process are counted as succeeded and skipped. Stale members are \
                       counted as failed. Requires the PortBay daemon to be running.",
        annotations(
            title = "Restart group",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn restart_group(
        &self,
        Parameters(args): Parameters<GroupIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.restart_group(args.id).await)
    }

    // ---- Tunnels (read-only) ------------------------------------------------

    #[tool(
        name = "portbay_list_tunnels",
        description = "List active public tunnels (their trycloudflare share URLs). Each entry \
                       includes the project id, upstream URL, public share URL (or null while \
                       Cloudflare is still assigning one), running state, and origin reachability. \
                       Read-only — start or stop a share from the PortBay app.",
        annotations(
            title = "List tunnels",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_tunnels(&self) -> Result<CallToolResult, McpError> {
        finish(self.ctx.list_tunnels())
    }

    #[tool(
        name = "portbay_tunnel_status",
        description = "Get the tunnel details for one project by id: public share URL, running \
                       state, origin reachability, and when it started. Returns null when no \
                       tunnel exists for the given project. Read-only — start or stop a share \
                       from the PortBay app.",
        annotations(
            title = "Tunnel status",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn tunnel_status(
        &self,
        Parameters(args): Parameters<TunnelStatusArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.tunnel_status(&args.id))
    }

    // ---- Runtimes -----------------------------------------------------------

    #[tool(
        name = "portbay_list_runtimes",
        description = "List every language PortBay knows about (PHP, Node.js, Python, Go, Ruby, \
                       Bun, Flutter) with all detected installs on this machine, their source \
                       (Homebrew, asdf, mise, nvm, system, manual), and which version is the \
                       configured default. No daemon required — all data comes from the local \
                       registry and binary detection. Installing a new language version and \
                       editing PHP FPM/ini config are done from the PortBay app.",
        annotations(
            title = "List runtimes",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_runtimes(&self) -> Result<CallToolResult, McpError> {
        finish(self.ctx.list_runtimes())
    }

    #[tool(
        name = "portbay_set_default_runtime",
        description = "Set (or clear) the default version for a language. The default is \
                       inherited by new projects when no version-manager file (.nvmrc, .tool-versions, \
                       etc.) is detected in the project folder. Omit `version` or pass `null` to \
                       clear the current default. The version must already be detected on this \
                       machine — call `portbay_list_runtimes` to see available versions.",
        annotations(
            title = "Set default runtime",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_default_runtime(
        &self,
        Parameters(args): Parameters<SetDefaultRuntimeArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.set_default_runtime(args.lang, args.version))
    }

    #[tool(
        name = "portbay_add_runtime_path",
        description = "Register an existing binary as a manual runtime install for a language. \
                       PortBay probes the binary for its version string — if it doesn't report \
                       one, the call is rejected. The binary is reused in place (never copied). \
                       Deduplicates by canonical path against already-detected installs. Returns \
                       the updated language list.",
        annotations(
            title = "Add runtime path",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn add_runtime_path(
        &self,
        Parameters(args): Parameters<AddRuntimePathArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.add_runtime_path(args.lang, args.path))
    }

    #[tool(
        name = "portbay_remove_runtime_path",
        description = "Remove a manually-added runtime install by language id and version label. \
                       No-op when the version is not present or was not manually added. Returns \
                       the updated language list.",
        annotations(
            title = "Remove runtime path",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn remove_runtime_path(
        &self,
        Parameters(args): Parameters<RemoveRuntimePathArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.remove_runtime_path(args.lang, args.version))
    }

    // ---- Databases ----------------------------------------------------------

    #[tool(
        name = "portbay_list_database_engines",
        description = "List every database engine PortBay can manage (MySQL, PostgreSQL, MariaDB, \
                       Redis, MongoDB, Memcached), each with whether it's installed on this machine, \
                       its detected version, default port, and CLI-client availability. Check here \
                       before portbay_create_database — installing an engine binary (Homebrew) is \
                       done from the PortBay app; this tool reports the install state and the brew hint.",
        annotations(
            title = "List database engines",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_database_engines(&self) -> Result<CallToolResult, McpError> {
        finish(Ok(self.ctx.list_database_engines()))
    }

    #[tool(
        name = "portbay_list_databases",
        description = "List the database instances PortBay manages, each with engine, port, \
                       connection URL, linked projects, and — when the daemon is running — live \
                       status (running/starting/stopped/errored). `daemon_reachable: false` means \
                       status reflects the registry only.",
        annotations(
            title = "List databases",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_databases(&self) -> Result<CallToolResult, McpError> {
        finish(self.ctx.list_databases().await)
    }

    #[tool(
        name = "portbay_database_connection",
        description = "Get connection details for one database instance: the connection URL plus the \
                       framework env vars (DATABASE_URL, DB_CONNECTION, DB_HOST, DB_PORT, …) PortBay \
                       injects into linked projects. Use this to wire an app up to a database.",
        annotations(
            title = "Database connection details",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn database_connection(
        &self,
        Parameters(args): Parameters<DatabaseIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.database_connection(&args.id))
    }

    #[tool(
        name = "portbay_create_database",
        description = "Provision and register a new database instance: PortBay initializes an \
                       isolated data dir, writes a config, and tracks it. The engine binary must \
                       already be installed (see portbay_list_database_engines). The instance joins \
                       Process Compose after the app's next reconcile (≤30s); start it with \
                       portbay_start_database. Does NOT start it immediately.",
        annotations(
            title = "Create database",
            read_only_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_database(
        &self,
        Parameters(args): Parameters<CreateDatabaseArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.create_database(args))
    }

    #[tool(
        name = "portbay_remove_database",
        description = "Stop (best-effort) and unregister a database instance. By default the on-disk \
                       data is kept; pass `delete_data: true` to also delete the data directory \
                       (irreversible). Confirm with the user before deleting data.",
        annotations(
            title = "Remove database",
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn remove_database(
        &self,
        Parameters(args): Parameters<RemoveDatabaseArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            self.ctx
                .remove_database(&args.id, args.delete_data.unwrap_or(false))
                .await,
        )
    }

    #[tool(
        name = "portbay_start_database",
        description = "Start a database instance's daemon via Process Compose. Requires the PortBay \
                       daemon to be running, and the instance to already exist in its config (true \
                       once the app has reconciled a newly-created instance).",
        annotations(
            title = "Start database",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn start_database(
        &self,
        Parameters(args): Parameters<DatabaseIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.start_database(&args.id).await)
    }

    #[tool(
        name = "portbay_stop_database",
        description = "Stop a running database instance. Requires the PortBay daemon to be running.",
        annotations(
            title = "Stop database",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn stop_database(
        &self,
        Parameters(args): Parameters<DatabaseIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.stop_database(&args.id).await)
    }

    #[tool(
        name = "portbay_restart_database",
        description = "Restart a database instance (stop then start). Requires the daemon to be running.",
        annotations(
            title = "Restart database",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn restart_database(
        &self,
        Parameters(args): Parameters<DatabaseIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.restart_database(&args.id).await)
    }

    #[tool(
        name = "portbay_link_database",
        description = "Link a database instance to a project. PortBay injects the instance's \
                       connection env vars (DATABASE_URL, DB_*) into the linked project's process on \
                       the next reconcile, so the app can reach the database with zero config.",
        annotations(
            title = "Link database to project",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn link_database(
        &self,
        Parameters(args): Parameters<LinkDatabaseArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.link_database(&args.id, &args.project_id))
    }

    #[tool(
        name = "portbay_unlink_database",
        description = "Unlink a database instance from a project, stopping its connection env vars \
                       from being injected into that project.",
        annotations(
            title = "Unlink database from project",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn unlink_database(
        &self,
        Parameters(args): Parameters<LinkDatabaseArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.unlink_database(&args.id, &args.project_id))
    }

    #[tool(
        name = "portbay_set_database_auto_start",
        description = "Set whether a database instance starts automatically when the PortBay daemon \
                       boots.",
        annotations(
            title = "Set database auto-start",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_database_auto_start(
        &self,
        Parameters(args): Parameters<SetDatabaseAutoStartArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            self.ctx
                .set_database_auto_start(&args.id, args.auto_start),
        )
    }

    // ---- DNS / domains ------------------------------------------------------

    #[tool(
        name = "portbay_dns_status",
        description = "Report local DNS state: the active domain suffix, whether the \
                       /etc/resolver/<suffix> file routes wildcard `*.suffix` to PortBay's dnsmasq \
                       (and on which port), whether the privileged helper is installed, and the \
                       persisted dnsmasq tuning. Read-only — starting/restarting dnsmasq and \
                       first-run resolver install are done from the PortBay app.",
        annotations(title = "DNS status", read_only_hint = true, open_world_hint = false)
    )]
    async fn dns_status(&self) -> Result<CallToolResult, McpError> {
        finish(self.ctx.dns_status())
    }

    #[tool(
        name = "portbay_list_dns_records",
        description = "List the names PortBay resolves: the wildcard `*.<suffix>` plus one row per \
                       project hostname, each tagged with how it's currently routed (`dnsmasq` via \
                       the resolver file, or `hosts` via /etc/hosts).",
        annotations(
            title = "List DNS records",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_dns_records(&self) -> Result<CallToolResult, McpError> {
        finish(self.ctx.list_dns_records())
    }

    #[tool(
        name = "portbay_set_domain_suffix",
        description = "Change the local domain suffix (e.g. `test` → `localhost`). Rewrites EVERY \
                       project hostname to the new suffix and drops their HTTPS cert directories \
                       (the app reissues certs and updates /etc/hosts on the next reconcile). \
                       Reserved public TLDs (.com, etc.) are rejected. High blast radius — confirm \
                       with the user first.",
        annotations(
            title = "Set domain suffix",
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_domain_suffix(
        &self,
        Parameters(args): Parameters<SetDomainSuffixArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.set_domain_suffix(&args.suffix))
    }

    // ---- Sandbox ------------------------------------------------------------

    #[tool(
        name = "portbay_sandbox_status",
        description = "Report Sandboxed Run state: per-project policy (enabled, network, ephemeral), \
                       whether this OS supports it (macOS only) and whether `sandbox-exec` is present, \
                       plus the tier's sandbox cap and how many projects are sandboxed. Set `id` for one \
                       project, omit it for all.",
        annotations(
            title = "Sandbox status",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn sandbox_status(
        &self,
        Parameters(args): Parameters<SandboxStatusArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.sandbox_status(args.id.as_deref()))
    }

    #[tool(
        name = "portbay_sandbox_violations",
        description = "List recent sandbox-denial lines from a project's logs (`deny(...)` / \"operation \
                       not permitted\"), so you can see what the profile blocked. Requires the PortBay \
                       daemon (logs come from Process Compose). Defaults to scanning the last 250 lines.",
        annotations(
            title = "Sandbox violations",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn sandbox_violations(
        &self,
        Parameters(args): Parameters<SandboxViolationsArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.sandbox_violations(&args.id, args.limit).await)
    }

    #[tool(
        name = "portbay_enable_sandbox",
        description = "Enable Sandboxed Run on a project (macOS only). Wraps the launch command in a \
                       Seatbelt profile that denies credential stores, browser data, and every `.env` \
                       outside the project. macOS must accept the profile or this fails closed (nothing \
                       persists). The instance is NOT started/restarted here: the app re-wraps the \
                       command on its next reconcile (≤30s), then call portbay_restart to run it \
                       confined. Community tiers cap concurrent sandboxed projects (see \
                       portbay_sandbox_status); Pro is unlimited.",
        annotations(
            title = "Enable Sandboxed Run",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn enable_sandbox(
        &self,
        Parameters(args): Parameters<EnableSandboxArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(
            self.ctx
                .enable_sandbox(&args.id, args.network.as_deref(), args.ephemeral),
        )
    }

    #[tool(
        name = "portbay_disable_sandbox",
        description = "Disable Sandboxed Run on a project (the 'promote to local' action). The change \
                       applies on the next restart. Works on any OS so a synced sandbox flag can always \
                       be cleared.",
        annotations(
            title = "Disable Sandboxed Run",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn disable_sandbox(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.disable_sandbox(&args.id))
    }

    // ---- HTTP request inspector --------------------------------------------

    #[tool(
        name = "portbay_recent_requests",
        description = "List recent HTTP requests Caddy handled (method, host, URI, status, duration, \
                       size, and the matched project), oldest→newest. Reads Caddy's access log off \
                       disk, so it works without the daemon — it's empty until the app has served \
                       traffic. Pass `project` to filter to one project's requests, `limit` to bound \
                       the count (default 200, max 2000).",
        annotations(
            title = "Recent HTTP requests",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn recent_requests(
        &self,
        Parameters(args): Parameters<RecentRequestsArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.recent_requests(args.limit, args.project.as_deref()))
    }

    #[tool(
        name = "portbay_clear_requests",
        description = "Truncate Caddy's access log so the request inspector starts fresh. Safe while \
                       the app is running — the live stream just resumes from the next request.",
        annotations(
            title = "Clear HTTP request log",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn clear_requests(&self) -> Result<CallToolResult, McpError> {
        finish(self.ctx.clear_requests())
    }

    // ---- Certificates -------------------------------------------------------

    #[tool(
        name = "portbay_cert_info",
        description = "Report local-HTTPS certificate metadata — file paths, issued/expiry dates, days \
                       until expiry, and DNS SANs — for one project (set `id`) or every project that \
                       has a cert (omit `id`). Reads the cert files directly (no daemon needed); a \
                       project with no cert yet is simply absent from the result.",
        annotations(
            title = "Certificate info",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn cert_info(
        &self,
        Parameters(args): Parameters<CertInfoArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.cert_info(args.id.as_deref()))
    }

    #[tool(
        name = "portbay_reissue_cert",
        description = "Reissue a project's local-HTTPS certificate: deletes the current cert so the \
                       running PortBay app mints a fresh one and reloads Caddy on its next reconcile \
                       (≤30s). The mkcert CA must already be trusted — installing it into the system \
                       keychain is privileged + interactive, done from the PortBay app.",
        annotations(
            title = "Reissue certificate",
            read_only_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn reissue_cert(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        finish(self.ctx.reissue_cert(&args.id))
    }
}

const INSTRUCTIONS: &str = "\
PortBay manages local development environments — it gives projects friendly hostnames \
(e.g. https://blog.test), local HTTPS, and managed start/stop, backed by Caddy + Process \
Compose. This server lets you drive PortBay for the user.

Typical flows:
- \"Set up the app I just scaffolded at <path>\" → call portbay_setup with that path. It \
  auto-detects the framework, registers the project, and starts it, returning the URL.
- \"Why won't my project start?\" → portbay_status (get the state) → portbay_logs (read the \
  failure) → portbay_doctor (check the environment), then explain the fix.
- Registering without starting → portbay_detect_project (preview) → confirm with the user → \
  portbay_add_project → portbay_start.

Key facts:
- Registry changes (add/update/remove/import) take effect even if the PortBay app isn't \
  running; they're applied when it next runs. Lifecycle actions (start/stop/restart/logs) \
  need the app running — you'll get a SIDECAR_DOWN error otherwise. Pass auto_launch:true on \
  portbay_start only when the user is at their machine and expects the app to open.
- Errors come back as isError results with a structured envelope (code, whatHappened, \
  whyItMatters, actions). Read it and either retry with fixed inputs or tell the user the \
  next step.
- A project's `id` is a stable slug; pass it to lifecycle/update/remove tools. Project caps \
  apply (anonymous 3 / free 6 / Pro unlimited); PROJECT_CAP_REACHED means the user should \
  sign in or upgrade.
- Runtimes: `portbay_list_runtimes` shows every detected language version and the configured \
  default. Use `portbay_set_default_runtime` to change which version new projects inherit. Use \
  `portbay_add_runtime_path` / `portbay_remove_runtime_path` to manage manually-added binaries. \
  Installing a new language version and editing PHP FPM/ini config are done from the PortBay app.
- Databases: PortBay owns isolated database instances. portbay_list_database_engines shows which \
  engines are installed; portbay_create_database provisions one (the engine binary must already be \
  installed — brew installs are done from the app). A new instance joins Process Compose after the \
  app's next reconcile, then portbay_start_database runs it. portbay_link_database wires a project to \
  a database (its connection env is injected); portbay_database_connection returns the URL + env vars.
- DNS: portbay_dns_status and portbay_list_dns_records show how names resolve (resolver file vs \
  /etc/hosts). portbay_set_domain_suffix changes the suffix for every project (destructive — it drops \
  certs, which the app reissues). Starting/restarting dnsmasq and first-run resolver install are done \
  from the PortBay app.
- Sandbox (macOS only): portbay_enable_sandbox confines an untrusted project under a Seatbelt profile \
  that blocks credential/browser/.env reads; it fails closed if macOS rejects the profile. It does NOT \
  launch the project — the app re-wraps the command on its next reconcile (≤30s), then portbay_restart \
  runs it confined. portbay_sandbox_status shows policy + the tier cap; portbay_sandbox_violations \
  lists what the profile blocked; portbay_disable_sandbox lifts it.
- HTTP inspector: portbay_recent_requests reads Caddy's access log (method/host/uri/status/duration \
  + matched project) straight off disk — works without the daemon, empty until traffic flows; filter \
  with `project`. portbay_clear_requests truncates the log. The live request stream is in the app UI.
- Certs: portbay_cert_info shows each project's local-HTTPS cert (paths, expiry, SANs), read off disk. \
  portbay_reissue_cert deletes a cert so the app mints a fresh one on reconcile. Installing the mkcert \
  CA into the system trust store is privileged + interactive — done from the PortBay app.
- Sidecars: portbay_sidecar_status reports what's observable from outside the app — process-compose \
  (live), the dnsmasq resolver file, and managed /etc/hosts; Caddy/mkcert/Mailpit are app-owned and \
  read `unknown`. Restarting a sidecar (caddy/process-compose/dnsmasq) is done from the PortBay app, \
  which owns those processes.";

#[tool_handler(router = self.tool_router)]
impl ServerHandler for PortbayMcp {
    fn get_info(&self) -> ServerInfo {
        let mut instructions = INSTRUCTIONS.to_string();
        if self.read_only {
            instructions.push_str(
                "\n\nNOTE: this server is running in READ-ONLY mode. Only inspection tools are \
                 available; add/update/remove/start/stop and scaffolding are disabled. Tell the \
                 user to restart the server without --read-only if they want to make changes.",
            );
        }
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::from_build_env())
        .with_instructions(instructions)
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let r = |uri: &str, name: &str, desc: &str| -> Resource {
            let mut raw = RawResource::new(uri, name.to_string());
            raw.description = Some(desc.to_string());
            raw.mime_type = Some("application/json".to_string());
            raw.no_annotation()
        };
        Ok(ListResourcesResult {
            resources: vec![
                r(
                    "portbay://registry",
                    "registry",
                    "The full PortBay registry as JSON — every project and its config.",
                ),
                r(
                    "portbay://doctor",
                    "doctor",
                    "Environment health snapshot (same data as portbay_doctor).",
                ),
                r(
                    "portbay://sidecars",
                    "sidecars",
                    "Sidecar status snapshot (same data as portbay_sidecar_status).",
                ),
                r(
                    "portbay://recipes",
                    "recipes",
                    "The stack-recipe catalog (same data as portbay_list_recipes).",
                ),
            ],
            next_cursor: None,
            meta: None,
        })
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        let mut logs = RawResourceTemplate::new("portbay://projects/{id}/logs", "project logs");
        logs.description = Some("Recent log tail for a project, by id.".to_string());
        let mut detail = RawResourceTemplate::new("portbay://projects/{id}", "project");
        detail.description = Some("Live status + config for a single project, by id.".to_string());
        Ok(ListResourceTemplatesResult {
            resource_templates: vec![logs.no_annotation(), detail.no_annotation()],
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _ctx: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let uri = request.uri.clone();
        let body: AppResult<String> = match uri.as_str() {
            "portbay://registry" => self.ctx.registry_json(),
            "portbay://doctor" => self.ctx.doctor().await.and_then(|d| to_json(&d)),
            "portbay://sidecars" => self.ctx.sidecar_status().await.and_then(|s| to_json(&s)),
            "portbay://recipes" => to_json(&self.ctx.list_recipes()),
            other => {
                if let Some(id) = other
                    .strip_prefix("portbay://projects/")
                    .and_then(|rest| rest.strip_suffix("/logs"))
                {
                    self.ctx.logs(id, 200, 0).await.and_then(|l| to_json(&l))
                } else if let Some(id) = other.strip_prefix("portbay://projects/") {
                    self.ctx.status(Some(id)).await.and_then(|s| to_json(&s))
                } else {
                    return Err(McpError::resource_not_found(
                        "unknown resource",
                        Some(json!({ "uri": uri })),
                    ));
                }
            }
        };

        match body {
            Ok(text) => Ok(ReadResourceResult::new(vec![ResourceContents::text(
                text, uri,
            )])),
            // A failed read (e.g. project not found, daemon down) is reported
            // as a protocol error here since resources have no isError channel.
            Err(e) => Err(McpError::internal_error(
                e.to_string(),
                serde_json::to_value(&e).ok(),
            )),
        }
    }
}

fn to_json<T: Serialize>(v: &T) -> AppResult<String> {
    serde_json::to_string_pretty(v).map_err(|e| AppError::Internal(format!("serialise: {e}")))
}
