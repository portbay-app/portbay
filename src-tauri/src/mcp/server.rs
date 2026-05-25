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
}

impl ToolGroup {
    pub fn all() -> Vec<ToolGroup> {
        vec![
            ToolGroup::Projects,
            ToolGroup::Lifecycle,
            ToolGroup::Diagnostics,
            ToolGroup::Scaffold,
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
            other => Err(format!(
                "unknown toolset `{other}` (valid: projects, lifecycle, diagnostics, scaffold, all)"
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
  sign in or upgrade.";

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
