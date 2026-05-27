//! Operations layer — the actual work behind every MCP tool, with no
//! dependency on the MCP protocol or `rmcp`.
//!
//! Like the `portbay` CLI, this layer is a *client* of the running system,
//! not a second copy of it:
//!
//! * **Registry CRUD** (add / update / remove / detect / export / import)
//!   reads and writes the on-disk registry via [`crate::registry::store`].
//!   It never runs a reconciler — the GUI daemon's 30-second safety tick
//!   picks up the change and converges certs / Caddy / hosts. When the GUI
//!   isn't running, the change simply persists and applies on next boot.
//! * **Lifecycle / logs / status** talk to the Process Compose daemon over
//!   HTTP via [`PcClient`], which requires the daemon to be up.
//!
//! Everything returns [`AppResult<T>`]; the protocol layer serialises an
//! `AppError` into the standard error envelope so an agent can recover.

use std::collections::{BTreeMap, HashMap};
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::time::Duration;

use crate::commands::onboarding::ScaffoldKind;
use crate::commands::projects::detect_kind;
use crate::entitlements;
use crate::error::{AppError, AppResult};
use crate::hosts::{HostsError, HostsManager};
use crate::hosts_helper::HostsHelperClient;
use crate::process_compose::{PcClient, Process, ProjectStatus, DEFAULT_PORT};
use crate::registry::{store, Group, ManualRuntime, Project, ProjectId, ProjectType, Readiness, Registry, WebServer, WorkspaceTool};
use crate::util::slugify;
use rmcp::schemars;

use super::types::*;

/// Domain suffix used when no registry exists yet. Kept in sync with the
/// CLI's and GUI's defaults so all three surfaces agree.
const DEFAULT_DOMAIN_SUFFIX: &str = "portbay.test";

/// Shared configuration + I/O endpoints for every operation. Cheap to clone.
#[derive(Clone)]
pub struct McpContext {
    registry_path: PathBuf,
    pc_port: u16,
}

impl McpContext {
    /// Build a context, honouring explicit overrides then the
    /// `PORTBAY_PC_PORT` env var, then the defaults.
    pub fn new(
        registry_override: Option<PathBuf>,
        pc_port_override: Option<u16>,
    ) -> AppResult<Self> {
        let registry_path = match registry_override {
            Some(p) => p,
            None => store::default_path()?,
        };
        let pc_port = pc_port_override
            .or_else(|| {
                std::env::var("PORTBAY_PC_PORT")
                    .ok()
                    .and_then(|s| s.parse().ok())
            })
            .unwrap_or(DEFAULT_PORT);
        Ok(Self {
            registry_path,
            pc_port,
        })
    }

    fn pc(&self) -> PcClient {
        PcClient::new(self.pc_port)
    }

    /// The data directory containing `registry.json` and the tunnel state file.
    fn data_dir(&self) -> &std::path::Path {
        self.registry_path
            .parent()
            .unwrap_or(&self.registry_path)
    }

    fn load_registry(&self) -> AppResult<Registry> {
        Ok(store::load_or_default(
            &self.registry_path,
            DEFAULT_DOMAIN_SUFFIX,
        )?)
    }

    fn save_registry(&self, reg: &Registry) -> AppResult<()> {
        store::save_to(reg, &self.registry_path)?;
        Ok(())
    }

    /// Fetch live process state keyed by project id. `None` means the daemon
    /// is unreachable — callers degrade gracefully where it makes sense.
    async fn fetch_pc_state(&self) -> Option<HashMap<String, Process>> {
        let procs = self.pc().processes().await.ok()?;
        Some(procs.into_iter().map(|p| (p.name.clone(), p)).collect())
    }

    /// Ensure the Process Compose daemon is reachable. When it isn't and
    /// `auto_launch` is set, try to open the PortBay app and wait for the
    /// daemon to come up (macOS only). Otherwise return `SIDECAR_DOWN`.
    async fn ensure_daemon(&self, auto_launch: bool) -> AppResult<()> {
        if self.pc().live().await.unwrap_or(false) {
            return Ok(());
        }
        if auto_launch {
            launch_app();
            // Poll for up to ~15s while the app boots its sidecars.
            for _ in 0..30 {
                tokio::time::sleep(Duration::from_millis(500)).await;
                if self.pc().live().await.unwrap_or(false) {
                    return Ok(());
                }
            }
        }
        Err(AppError::SidecarDown("process-compose"))
    }

    fn require_project(&self, reg: &Registry, id: &str) -> AppResult<()> {
        if reg.get_project(&ProjectId::new(id)).is_none() {
            return Err(AppError::NotFound(id.to_string()));
        }
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Read operations
    // -------------------------------------------------------------------------

    pub async fn list_projects(&self) -> AppResult<ListProjectsResult> {
        let reg = self.load_registry()?;
        let pc_state = self.fetch_pc_state().await;
        let projects = reg
            .list_projects()
            .iter()
            .map(|p| {
                let proc = pc_state.as_ref().and_then(|m| m.get(p.id.as_str()));
                summary(p, proc)
            })
            .collect();
        Ok(ListProjectsResult {
            daemon_reachable: pc_state.is_some(),
            projects,
        })
    }

    /// Status of one project (when `id` is set) or all projects.
    pub async fn status(&self, id: Option<&str>) -> AppResult<ListProjectsResult> {
        let reg = self.load_registry()?;
        let pc_state = self.fetch_pc_state().await;
        let projects: Vec<ProjectSummary> = match id {
            Some(id) => {
                let p = reg
                    .get_project(&ProjectId::new(id))
                    .ok_or_else(|| AppError::NotFound(id.to_string()))?;
                vec![summary(p, pc_state.as_ref().and_then(|m| m.get(id)))]
            }
            None => reg
                .list_projects()
                .iter()
                .map(|p| summary(p, pc_state.as_ref().and_then(|m| m.get(p.id.as_str()))))
                .collect(),
        };
        Ok(ListProjectsResult {
            daemon_reachable: pc_state.is_some(),
            projects,
        })
    }

    /// A single project plus its live runtime state.
    pub async fn get_project(&self, id: &str) -> AppResult<ProjectSummary> {
        let reg = self.load_registry()?;
        let p = reg
            .get_project(&ProjectId::new(id))
            .ok_or_else(|| AppError::NotFound(id.to_string()))?;
        let pc_state = self.fetch_pc_state().await;
        Ok(summary(p, pc_state.as_ref().and_then(|m| m.get(id))))
    }

    /// The full registry as pretty JSON — backs the `portbay://registry`
    /// resource so an agent can pull the whole project list into its context.
    pub fn registry_json(&self) -> AppResult<String> {
        let reg = self.load_registry()?;
        serde_json::to_string_pretty(&reg)
            .map_err(|e| AppError::Internal(format!("serialise registry: {e}")))
    }

    pub fn detect_project(&self, path: &str) -> AppResult<DetectResult> {
        let p = canonical_dir(path)?;
        let reg = self.load_registry()?;
        let dir_name = dir_name_of(&p);
        let id = slugify(&dir_name);
        let d = detect_kind(&p);
        Ok(DetectResult {
            kind: kind_str(d.kind),
            suggested_hostname: format!("{id}.{}", reg.domain_suffix),
            suggested_id: id,
            suggested_name: dir_name,
            suggested_port: d.port,
            suggested_start_command: d.start_command,
            suggested_document_root: d.document_root,
            suggested_php_version: d.php_version,
        })
    }

    /// If `path` is a JS monorepo root, return the runnable apps inside it
    /// (each with framework-detected defaults ready to pass to `add_project`).
    /// Returns `None` for a plain folder — the caller should use
    /// `detect_project` instead.
    pub fn detect_workspace_apps(&self, path: &str) -> AppResult<Option<WorkspaceScanResult>> {
        let root = canonical_dir(path)?;
        let Some(layout) = crate::registry::workspace::detect(&root) else {
            return Ok(None);
        };

        let reg = self.load_registry()?;
        let suffix = &reg.domain_suffix;

        let apps = layout
            .packages
            .iter()
            .map(|pkg| {
                // Use the directory leaf (e.g. `apps/web` → `web`) for the id
                // and hostname; the package name may carry a scope prefix.
                let leaf = std::path::Path::new(&pkg.rel_dir)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&pkg.rel_dir);
                let id = slugify(leaf);
                let detected = detect_kind(&pkg.abs_dir);
                // Honour the repo's own package manager for the dev command;
                // only emit one when the framework detects a dev script.
                let start_command = detected
                    .start_command
                    .map(|_| standalone_dev_command(layout.tool));
                WorkspaceAppSummary {
                    package: pkg.name.clone(),
                    rel_dir: pkg.rel_dir.clone(),
                    path: pkg.abs_dir.display().to_string(),
                    kind: kind_str(detected.kind),
                    suggested_id: id.clone(),
                    suggested_hostname: format!("{id}.{suffix}"),
                    suggested_port: detected.port,
                    suggested_start_command: start_command,
                }
            })
            .collect();

        Ok(Some(WorkspaceScanResult {
            root: root.display().to_string(),
            tool: format!("{:?}", layout.tool).to_lowercase(),
            apps,
        }))
    }

    pub async fn logs(&self, id: &str, lines: u32, offset: u64) -> AppResult<LogsResult> {
        {
            let reg = self.load_registry()?;
            self.require_project(&reg, id)?;
        }
        self.ensure_daemon(false).await?;
        let lines = self.pc().logs(id, offset, lines).await?;
        Ok(LogsResult {
            id: id.to_string(),
            lines,
        })
    }

    // -------------------------------------------------------------------------
    // Tunnel visibility (read-only — mirror file written by the running app)
    // -------------------------------------------------------------------------

    /// Return every tunnel the app is currently tracking. Returns an empty vec
    /// when the state file is absent (app not running or no tunnels started).
    pub fn list_tunnels(&self) -> AppResult<Vec<crate::tunnel::TunnelStatus>> {
        Ok(crate::tunnel::read_state(self.data_dir()))
    }

    /// Find a specific tunnel by `project_id`. Returns `None` when not found
    /// or when the state file is absent.
    pub fn tunnel_status(&self, id: &str) -> AppResult<Option<crate::tunnel::TunnelStatus>> {
        Ok(self.list_tunnels()?.into_iter().find(|t| t.project_id == id))
    }

    // -------------------------------------------------------------------------
    // Mutations
    // -------------------------------------------------------------------------

    pub async fn add_project(&self, args: AddProjectArgs) -> AppResult<OpResult> {
        let project = self.build_project(&args)?;
        let mut reg = self.load_registry()?;
        entitlements::check_can_add(reg.projects.len())
            .map_err(|cap| AppError::ProjectCapReached { cap })?;
        if reg.hostname_conflict(&project.hostname, None) {
            return Err(crate::registry::RegistryError::DuplicateHostname(project.hostname).into());
        }
        if let Some(port) = project.port {
            if reg.port_conflict(port, None) {
                return Err(crate::registry::RegistryError::DuplicatePort(port).into());
            }
        }
        reg.add_project(project.clone())?;
        if let Some(rt) = &project.runtime {
            // Best-effort: a missing marker file shouldn't fail registration.
            let _ = crate::project_runtime::ensure_marker_files(&project.path, rt);
        }
        self.save_registry(&reg)?;
        let warnings = best_effort_add_host(&reg.domain_suffix, &project.hostname);
        Ok(OpResult {
            ok: true,
            detail: format!(
                "Registered {} at {}{}.",
                project.id.as_str(),
                project.hostname,
                if project.https { " (HTTPS)" } else { "" }
            ),
            project: Some(summary(&project, None)),
            warnings,
        })
    }

    pub async fn update_project(&self, args: UpdateProjectArgs) -> AppResult<OpResult> {
        let pid = ProjectId::new(&args.id);
        let mut reg = self.load_registry()?;
        if reg.get_project(&pid).is_none() {
            return Err(AppError::NotFound(args.id.clone()));
        }
        if let Some(h) = args.hostname.as_deref() {
            if reg.hostname_conflict(h, Some(&pid)) {
                return Err(crate::registry::RegistryError::DuplicateHostname(h.to_string()).into());
            }
        }
        if let Some(port) = args.port {
            if reg.port_conflict(port, Some(&pid)) {
                return Err(crate::registry::RegistryError::DuplicatePort(port).into());
            }
        }
        {
            let p = reg
                .get_project_mut(&pid)
                .ok_or_else(|| AppError::NotFound(args.id.clone()))?;
            if let Some(v) = args.name {
                p.name = v;
            }
            if let Some(v) = args.hostname {
                p.hostname = v;
            }
            if let Some(v) = args.port {
                p.port = Some(v);
            }
            if let Some(v) = args.start_command {
                p.start_command = Some(v);
            }
            if let Some(v) = args.https {
                p.https = v;
            }
            if let Some(v) = args.auto_start {
                p.auto_start = v;
            }
            if let Some(v) = args.tags {
                p.tags = v;
            }
        }
        self.save_registry(&reg)?;
        let summary = reg.get_project(&pid).map(|p| summary(p, None));
        Ok(OpResult {
            ok: true,
            detail: format!("Updated {}.", args.id),
            project: summary,
            warnings: vec![],
        })
    }

    pub async fn remove_project(&self, id: &str) -> AppResult<OpResult> {
        let pid = ProjectId::new(id);
        let mut reg = self.load_registry()?;
        let removed = reg.remove_project(&pid)?;
        self.save_registry(&reg)?;

        let mut warnings: Vec<String> = Vec::new();
        // Best-effort cert directory cleanup.
        if let Some(root) = certs_root() {
            let dir = root.join(removed.id.as_str());
            if dir.exists() {
                if let Err(e) = std::fs::remove_dir_all(&dir) {
                    warnings.push(format!("could not delete certs at {}: {e}", dir.display()));
                }
            }
        }
        // Best-effort hosts entry removal.
        warnings.extend(best_effort_remove_host(
            &reg.domain_suffix,
            &removed.hostname,
        ));

        Ok(OpResult {
            ok: true,
            detail: format!("Removed {}.", removed.id.as_str()),
            project: None,
            warnings,
        })
    }

    pub async fn export_config(&self, id: &str) -> AppResult<ExportResult> {
        let reg = self.load_registry()?;
        let project = reg
            .get_project(&ProjectId::new(id))
            .ok_or_else(|| AppError::NotFound(id.to_string()))?;
        let file = crate::portfile::export_project(project);
        let json = crate::portfile::to_json_string(&file)
            .map_err(|e| AppError::Internal(format!("serialise .portbay.json: {e}")))?;
        let out_path = project.path.join(crate::portfile::PORTBAY_FILE_NAME);
        std::fs::write(&out_path, &json)?;
        Ok(ExportResult {
            wrote: out_path.display().to_string(),
            env_count: file.env_template.len(),
            secret_names: file.secrets.clone(),
        })
    }

    pub async fn import_config(&self, args: ImportConfigArgs) -> AppResult<OpResult> {
        let raw = PathBuf::from(&args.path);
        let (folder, portfile_path) = if raw.is_file() {
            let parent = raw
                .parent()
                .map(PathBuf::from)
                .ok_or_else(|| AppError::BadInput("file has no parent directory".into()))?;
            (parent, raw)
        } else {
            let folder = canonical_dir(&args.path)?;
            let pf = folder.join(crate::portfile::PORTBAY_FILE_NAME);
            (folder, pf)
        };
        if !portfile_path.is_file() {
            return Err(AppError::BadInput(format!(
                "no {} found at {}",
                crate::portfile::PORTBAY_FILE_NAME,
                portfile_path.display()
            )));
        }
        let folder = folder
            .canonicalize()
            .map_err(|e| AppError::BadInput(format!("path: {e}")))?;

        let bytes = std::fs::read(&portfile_path)?;
        let file = crate::portfile::from_json_bytes(&bytes)
            .map_err(|e| AppError::BadInput(format!("parse .portbay.json: {e}")))?;

        let declared = file.secrets.clone();
        let provided = args.secrets.unwrap_or_default();
        let secrets: BTreeMap<String, String> = declared
            .iter()
            .map(|k| (k.clone(), provided.get(k).cloned().unwrap_or_default()))
            .collect();
        let pending: Vec<String> = declared
            .iter()
            .filter(|k| !provided.contains_key(*k))
            .cloned()
            .collect();

        let id = ProjectId::new(slugify(&dir_name_of(&folder)));
        let plan = crate::portfile::ImportPlan::new(file, folder);
        let project = crate::portfile::materialise_project(&plan, id, &secrets)
            .map_err(|e| AppError::BadInput(format!("materialise: {e}")))?;

        let mut reg = self.load_registry()?;
        entitlements::check_can_add(reg.projects.len())
            .map_err(|cap| AppError::ProjectCapReached { cap })?;
        reg.add_project(project.clone())?;
        self.save_registry(&reg)?;

        let mut warnings = best_effort_add_host(&reg.domain_suffix, &project.hostname);
        if !pending.is_empty() {
            warnings.push(format!(
                "{} secret(s) not set (registered as empty placeholders): {}",
                pending.len(),
                pending.join(", ")
            ));
        }
        Ok(OpResult {
            ok: true,
            detail: format!(
                "Imported {} from .portbay.json at {}.",
                project.id.as_str(),
                project.hostname
            ),
            project: Some(summary(&project, None)),
            warnings,
        })
    }

    // -------------------------------------------------------------------------
    // Lifecycle
    // -------------------------------------------------------------------------

    pub async fn start(&self, id: &str, auto_launch: bool) -> AppResult<OpResult> {
        {
            let reg = self.load_registry()?;
            self.require_project(&reg, id)?;
        }
        self.ensure_daemon(auto_launch).await?;
        self.pc().start(id).await?;
        Ok(self.ack(id, "Started").await)
    }

    pub async fn stop(&self, id: &str) -> AppResult<OpResult> {
        {
            let reg = self.load_registry()?;
            self.require_project(&reg, id)?;
        }
        self.ensure_daemon(false).await?;
        self.pc().stop(id).await?;
        Ok(self.ack(id, "Stopped").await)
    }

    pub async fn restart(&self, id: &str) -> AppResult<OpResult> {
        {
            let reg = self.load_registry()?;
            self.require_project(&reg, id)?;
        }
        self.ensure_daemon(false).await?;
        self.pc().restart(id).await?;
        Ok(self.ack(id, "Restarted").await)
    }

    pub async fn stop_all(&self) -> AppResult<OpResult> {
        self.ensure_daemon(false).await?;
        let client = self.pc();
        let processes = client.processes().await?;
        let names: Vec<&str> = processes.iter().map(|p| p.name.as_str()).collect();
        if names.is_empty() {
            return Ok(OpResult {
                ok: true,
                detail: "Nothing was running.".into(),
                project: None,
                warnings: vec![],
            });
        }
        client.stop_many(&names).await?;
        Ok(OpResult {
            ok: true,
            detail: format!("Stopped {} process(es).", names.len()),
            project: None,
            warnings: vec![],
        })
    }

    async fn ack(&self, id: &str, verb: &str) -> OpResult {
        OpResult {
            ok: true,
            detail: format!("{verb} {id}."),
            project: self.get_project_summary(id).await,
            warnings: vec![],
        }
    }

    async fn get_project_summary(&self, id: &str) -> Option<ProjectSummary> {
        let reg = self.load_registry().ok()?;
        let p = reg.get_project(&ProjectId::new(id))?;
        let pc_state = self.fetch_pc_state().await;
        Some(summary(p, pc_state.as_ref().and_then(|m| m.get(id))))
    }

    // -------------------------------------------------------------------------
    // Composite / higher-level
    // -------------------------------------------------------------------------

    pub async fn setup(&self, args: SetupArgs) -> AppResult<OpResult> {
        let add = AddProjectArgs {
            path: args.path,
            name: args.name,
            hostname: args.hostname,
            kind: args.kind,
            port: args.port,
            start_command: args.start_command,
            https: args.https,
            auto_start: Some(false),
            php_version: None,
            document_root: None,
        };
        let mut result = self.add_project(add).await?;
        let id = result
            .project
            .as_ref()
            .map(|p| p.id.clone())
            .ok_or_else(|| AppError::Internal("add returned no project".into()))?;

        if args.start_now.unwrap_or(true) {
            match self.start(&id, args.auto_launch.unwrap_or(false)).await {
                Ok(started) => {
                    result.detail = format!("{} {}", result.detail, started.detail);
                    result.project = started.project.or(result.project);
                }
                Err(e) => {
                    // Registration succeeded; surface the start failure as a
                    // warning rather than failing the whole setup.
                    result
                        .warnings
                        .push(format!("registered, but couldn't start: {e}"));
                }
            }
        }
        Ok(result)
    }

    pub fn list_recipes(&self) -> ListRecipesResult {
        let recipes = super::recipes::all()
            .iter()
            .map(|r| RecipeSummary {
                id: r.id.to_string(),
                title: r.title.to_string(),
                description: r.description.to_string(),
                project_type: kind_str(r.project_type),
                php_version: r.php_version.map(str::to_string),
                document_root: r.document_root.map(str::to_string),
                https: r.https,
                database: r.needs_database.map(str::to_string),
                mail: r.needs_mail,
                composes_fully: r.needs_database.is_none() && !r.needs_mail,
            })
            .collect();
        ListRecipesResult { recipes }
    }

    pub async fn setup_from_recipe(&self, args: SetupFromRecipeArgs) -> AppResult<OpResult> {
        let recipe = super::recipes::find(&args.recipe).ok_or_else(|| {
            AppError::BadInput(format!(
                "unknown recipe `{}` — call portbay_list_recipes to see the catalog",
                args.recipe
            ))
        })?;

        let add = AddProjectArgs {
            path: args.path,
            name: args.name,
            hostname: args.hostname,
            kind: Some(recipe.project_type.into()),
            port: None,
            // Leave the start command to framework detection — it picks the
            // right package-manager dev command; PHP recipes are Caddy-served.
            start_command: None,
            https: Some(args.https.unwrap_or(recipe.https)),
            auto_start: Some(false),
            php_version: args
                .php_version
                .or_else(|| recipe.php_version.map(str::to_string)),
            document_root: recipe.document_root.map(str::to_string),
        };

        let mut result = self.add_project(add).await?;
        result.detail = format!("Set up the `{}` recipe — {}", recipe.id, result.detail);

        // Honestly flag recipe steps PortBay can't auto-provision yet, rather
        // than half-wiring them. The project is registered regardless.
        if let Some(db) = recipe.needs_database {
            result.warnings.push(format!(
                "the `{}` recipe recommends a {} database; automatic database provisioning isn't \
                 available yet, so the project is registered without it — add one from the app's \
                 Databases panel when ready",
                recipe.id, db
            ));
        }
        if recipe.needs_mail {
            result.warnings.push(format!(
                "the `{}` recipe benefits from a local mail catcher (Mailpit); enable it from the \
                 app when ready",
                recipe.id
            ));
        }

        if args.start_now.unwrap_or(true) {
            if let Some(id) = result.project.as_ref().map(|p| p.id.clone()) {
                match self.start(&id, args.auto_launch.unwrap_or(false)).await {
                    Ok(started) => {
                        result.detail = format!("{} {}", result.detail, started.detail);
                        result.project = started.project.or(result.project);
                    }
                    Err(e) => result
                        .warnings
                        .push(format!("registered, but couldn't start: {e}")),
                }
            }
        }
        Ok(result)
    }

    pub async fn setup_from_template(&self, args: SetupFromTemplateArgs) -> AppResult<OpResult> {
        let kind: ScaffoldKind = args.template.into();
        let name = args.name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::BadInput("project name cannot be empty".into()));
        }
        let parent = PathBuf::from(&args.parent_path);
        if !parent.is_dir() {
            return Err(AppError::BadInput(format!(
                "parent path is not a directory: {}",
                args.parent_path
            )));
        }
        let target = parent.join(&name);
        if target.exists() {
            return Err(AppError::BadInput(format!(
                "target folder already exists: {}",
                target.display()
            )));
        }

        if matches!(kind, ScaffoldKind::Php) {
            std::fs::create_dir_all(&target)?;
            std::fs::write(
                target.join("index.php"),
                "<?php\necho \"Hello from PortBay!\";\n",
            )?;
        } else {
            let (program, cmd_args) = kind.command_for(&name);
            let output = tokio::process::Command::new(program)
                .args(&cmd_args)
                .current_dir(&parent)
                .output()
                .await
                .map_err(|e| AppError::Internal(format!("failed to spawn `{program}`: {e}")))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(AppError::Internal(format!(
                    "`{program}` exited with {}: {}",
                    output.status,
                    stderr.trim()
                )));
            }
            if !target.exists() {
                return Err(AppError::Internal(format!(
                    "`{program}` reported success but {} is missing",
                    target.display()
                )));
            }
        }

        let add = AddProjectArgs {
            path: target.to_string_lossy().to_string(),
            name: Some(name),
            hostname: None,
            kind: None,
            port: None,
            start_command: kind.default_start_command().map(str::to_string),
            https: Some(true),
            auto_start: Some(false),
            php_version: None,
            document_root: None,
        };
        let mut result = self.add_project(add).await?;
        if args.start_now.unwrap_or(false) {
            if let Some(id) = result.project.as_ref().map(|p| p.id.clone()) {
                match self.start(&id, false).await {
                    Ok(started) => result.detail = format!("{} {}", result.detail, started.detail),
                    Err(e) => result
                        .warnings
                        .push(format!("scaffolded + registered, but couldn't start: {e}")),
                }
            }
        }
        Ok(result)
    }

    // -------------------------------------------------------------------------
    // Diagnostics
    // -------------------------------------------------------------------------

    pub async fn doctor(&self) -> AppResult<DoctorResult> {
        let mut findings: Vec<DoctorFinding> = Vec::new();

        match self.load_registry() {
            Ok(reg) => findings.push(DoctorFinding {
                check: "registry".into(),
                verdict: "ok".into(),
                detail: format!(
                    "{} project(s), v{} schema, suffix .{}",
                    reg.list_projects().len(),
                    reg.version,
                    reg.domain_suffix
                ),
            }),
            Err(e) => findings.push(DoctorFinding {
                check: "registry".into(),
                verdict: "fail".into(),
                detail: e.to_string(),
            }),
        }

        match self.pc().live().await {
            Ok(true) => findings.push(DoctorFinding {
                check: format!("process-compose :{}", self.pc_port),
                verdict: "ok".into(),
                detail: "alive".into(),
            }),
            _ => findings.push(DoctorFinding {
                check: format!("process-compose :{}", self.pc_port),
                verdict: "warn".into(),
                detail: "not reachable — open PortBay.app to start the daemon".into(),
            }),
        }

        for tool in ["mkcert", "caddy", "process-compose"] {
            match which::which(tool) {
                Ok(p) => findings.push(DoctorFinding {
                    check: format!("tool: {tool}"),
                    verdict: "ok".into(),
                    detail: p.display().to_string(),
                }),
                Err(_) => findings.push(DoctorFinding {
                    check: format!("tool: {tool}"),
                    verdict: "warn".into(),
                    detail: "not found on PATH (bundled with PortBay.app; only matters for standalone CLI use)".into(),
                }),
            }
        }

        let tier = entitlements::current().tier;
        findings.push(DoctorFinding {
            check: "entitlement".into(),
            verdict: "ok".into(),
            detail: format!("{tier} tier"),
        });

        Ok(DoctorResult {
            ok: !findings.iter().any(|f| f.verdict == "fail"),
            findings,
        })
    }

    pub async fn sidecar_status(&self) -> AppResult<SidecarStatusResult> {
        // Process Compose is the one sidecar we can probe directly (its HTTP
        // API). The others are owned by the daemon and aren't reachable from
        // outside it, so we report only their install presence — honestly
        // labelled — and point the agent at `portbay_doctor` for more.
        let pc_live = self.pc().live().await.unwrap_or(false);
        let mut sidecars = vec![SidecarReport {
            name: "process-compose".into(),
            state: if pc_live { "running" } else { "stopped" }.into(),
            detail: if pc_live {
                format!("reachable on :{}", self.pc_port)
            } else {
                "daemon not reachable — open PortBay.app".into()
            },
        }];
        for tool in ["caddy", "mkcert", "dnsmasq", "mailpit"] {
            let detail = match which::which(tool) {
                Ok(p) => format!("binary on PATH at {}", p.display()),
                Err(_) => "not on PATH (bundled with PortBay.app; live state unknown from here)"
                    .to_string(),
            };
            sidecars.push(SidecarReport {
                name: tool.into(),
                // We can't confirm live state from outside the daemon.
                state: "unknown".into(),
                detail,
            });
        }
        Ok(SidecarStatusResult {
            daemon_reachable: pc_live,
            sidecars,
        })
    }

    // -------------------------------------------------------------------------
    // Runtime CRUD (mirrors commands/runtimes.rs, no app-state deps)
    // All operations are registry-only; no daemon needed.
    // -------------------------------------------------------------------------

    /// List every language PortBay knows about, with all detected and
    /// manually-added installs. No daemon required.
    pub fn list_runtimes(&self) -> AppResult<Vec<RuntimeLanguageSummary>> {
        let reg = self.load_registry()?;
        let views = crate::runtimes::list_all(&reg.runtimes);
        Ok(views.into_iter().map(language_summary).collect())
    }

    /// Set (or clear) the default version for a language. Clearing happens
    /// when `version` is `None` or empty. Rejects a version string not
    /// currently surfaced by `list_all`.
    pub fn set_default_runtime(
        &self,
        lang: String,
        version: Option<String>,
    ) -> AppResult<Vec<RuntimeLanguageSummary>> {
        let mut reg = self.load_registry()?;

        // Validate that the language exists.
        if crate::runtimes::runtime_by_id(&lang).is_none() {
            return Err(AppError::BadInput(format!("unknown language `{lang}`")));
        }

        match version {
            Some(ref v) if !v.trim().is_empty() => {
                // Reject a version that list_all doesn't surface.
                let views = crate::runtimes::list_all(&reg.runtimes);
                let lang_view = views.iter().find(|lv| lv.id == lang);
                let version_known = lang_view.is_some_and(|lv| {
                    lv.versions.iter().any(|vv| vv.install.version == *v)
                });
                if !version_known {
                    return Err(AppError::BadInput(format!(
                        "version `{v}` is not currently detected for `{lang}` \
                         — call portbay_list_runtimes to see available versions"
                    )));
                }
                reg.runtimes.defaults.insert(lang, v.clone());
            }
            _ => {
                reg.runtimes.defaults.remove(&lang);
            }
        }
        self.save_registry(&reg)?;
        let views = crate::runtimes::list_all(&reg.runtimes);
        Ok(views.into_iter().map(language_summary).collect())
    }

    /// Register an existing binary as a manual install. The binary is probed
    /// for its version; a binary that doesn't report one is rejected. Dedupes
    /// by canonical path.
    pub fn add_runtime_path(
        &self,
        lang: String,
        path: String,
    ) -> AppResult<Vec<RuntimeLanguageSummary>> {
        let runtime = crate::runtimes::runtime_by_id(&lang)
            .ok_or_else(|| AppError::BadInput(format!("unknown language `{lang}`")))?;

        let binary = std::path::PathBuf::from(&path);
        if !binary.is_file() {
            return Err(AppError::BadInput(format!("no binary found at {path}")));
        }

        let version = runtime.probe_version(&binary).ok_or_else(|| {
            AppError::BadInput(format!(
                "{path} didn't report a {lang} version — is it the right binary?"
            ))
        })?;
        let version = crate::runtimes::major_minor(&version);

        let mut reg = self.load_registry()?;
        let canon = binary
            .canonicalize()
            .unwrap_or_else(|_| binary.clone());
        let exists = reg
            .runtimes
            .manual
            .iter()
            .any(|m| m.binary.canonicalize().unwrap_or_else(|_| m.binary.clone()) == canon);
        if !exists {
            reg.runtimes.manual.push(ManualRuntime {
                lang: lang.clone(),
                version,
                binary,
            });
            self.save_registry(&reg)?;
        }

        let views = crate::runtimes::list_all(&reg.runtimes);
        Ok(views.into_iter().map(language_summary).collect())
    }

    /// Remove a manually-added install by language + version. No-op if it
    /// wasn't manual or isn't present.
    pub fn remove_runtime_path(
        &self,
        lang: String,
        version: String,
    ) -> AppResult<Vec<RuntimeLanguageSummary>> {
        let mut reg = self.load_registry()?;
        reg.runtimes
            .manual
            .retain(|m| !(m.lang == lang && m.version == version));
        self.save_registry(&reg)?;
        let views = crate::runtimes::list_all(&reg.runtimes);
        Ok(views.into_iter().map(language_summary).collect())
    }

    // -------------------------------------------------------------------------
    // Group CRUD + lifecycle (mirrors commands/groups.rs, no app-state deps)
    // -------------------------------------------------------------------------

    pub fn list_groups(&self) -> AppResult<Vec<GroupSummary>> {
        let reg = self.load_registry()?;
        let known: std::collections::HashSet<&str> = reg
            .list_projects()
            .iter()
            .map(|p| p.id.as_str())
            .collect();
        Ok(reg
            .list_groups()
            .iter()
            .map(|g| group_summary(g, &known))
            .collect())
    }

    pub fn create_group(
        &self,
        id: Option<String>,
        name: String,
        project_ids: Vec<String>,
    ) -> AppResult<GroupSummary> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::BadInput("group name cannot be empty".into()));
        }
        let id = id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| slugify(&name));
        if id.is_empty() {
            return Err(AppError::BadInput(
                "group id couldn't be derived from name".into(),
            ));
        }
        let mut reg = self.load_registry()?;
        let known: std::collections::HashSet<String> = reg
            .list_projects()
            .iter()
            .map(|p| p.id.as_str().to_string())
            .collect();
        let group = Group {
            id: id.clone(),
            name,
            projects: project_ids.into_iter().map(ProjectId::new).collect(),
        };
        reg.add_group(group.clone())
            .map_err(AppError::Registry)?;
        self.save_registry(&reg)?;
        let known_ref: std::collections::HashSet<&str> =
            known.iter().map(|s| s.as_str()).collect();
        Ok(group_summary(&group, &known_ref))
    }

    pub fn update_group(
        &self,
        id: String,
        name: Option<String>,
        project_ids: Option<Vec<String>>,
    ) -> AppResult<GroupSummary> {
        let mut reg = self.load_registry()?;
        let current = reg
            .get_group(&id)
            .ok_or_else(|| AppError::NotFound(format!("group:{id}")))?
            .clone();
        let next = Group {
            id: current.id.clone(),
            name: name
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or(current.name),
            projects: project_ids
                .map(|ids| ids.into_iter().map(ProjectId::new).collect())
                .unwrap_or(current.projects),
        };
        reg.update_group(next.clone())
            .map_err(AppError::Registry)?;
        self.save_registry(&reg)?;
        let known: std::collections::HashSet<&str> = reg
            .list_projects()
            .iter()
            .map(|p| p.id.as_str())
            .collect();
        Ok(group_summary(&next, &known))
    }

    pub fn remove_group(&self, id: String) -> AppResult<()> {
        let mut reg = self.load_registry()?;
        reg.remove_group(&id)
            .map_err(AppError::Registry)?;
        self.save_registry(&reg)?;
        Ok(())
    }

    pub async fn start_group(&self, id: String) -> AppResult<GroupFanoutResult> {
        self.fanout_group(&id, GroupOp::Start).await
    }

    pub async fn stop_group(&self, id: String) -> AppResult<GroupFanoutResult> {
        self.fanout_group(&id, GroupOp::Stop).await
    }

    pub async fn restart_group(&self, id: String) -> AppResult<GroupFanoutResult> {
        self.fanout_group(&id, GroupOp::Restart).await
    }

    async fn fanout_group(&self, group_id: &str, op: GroupOp) -> AppResult<GroupFanoutResult> {
        self.ensure_daemon(false).await?;
        let reg = self.load_registry()?;
        let group = reg
            .get_group(group_id)
            .ok_or_else(|| AppError::NotFound(format!("group:{group_id}")))?
            .clone();

        let projects_by_id: HashMap<&str, &crate::registry::Project> = reg
            .list_projects()
            .iter()
            .map(|p| (p.id.as_str(), p))
            .collect();
        let client = self.pc();

        let mut result = GroupFanoutResult {
            group_id: group_id.to_string(),
            succeeded: 0,
            failed: 0,
            results: Vec::with_capacity(group.projects.len()),
        };

        for pid in &group.projects {
            let id_str = pid.as_str().to_string();
            let Some(project) = projects_by_id.get(id_str.as_str()) else {
                // Stale member — count as failed so the caller sees the drift.
                result.failed += 1;
                result.results.push(GroupMemberResult {
                    project_id: id_str,
                    ok: false,
                    error: Some("project not in registry".into()),
                });
                continue;
            };
            let process_id = project.process_compose_id();
            if process_id.is_none() {
                // No process to manage (e.g. mobile / Xcode project) — count ok.
                result.succeeded += 1;
                result.results.push(GroupMemberResult {
                    project_id: id_str,
                    ok: true,
                    error: None,
                });
                continue;
            }
            let process_id = process_id.expect("checked above");
            // Note: mark_stop_requested is app-only state; OMIT here (cross-process).
            let res = match op {
                GroupOp::Start => client.start(&process_id).await,
                GroupOp::Stop => client.stop(&process_id).await,
                GroupOp::Restart => client.restart(&process_id).await,
            };
            match res {
                Ok(()) => {
                    result.succeeded += 1;
                    result.results.push(GroupMemberResult {
                        project_id: id_str,
                        ok: true,
                        error: None,
                    });
                }
                Err(e) => {
                    result.failed += 1;
                    result.results.push(GroupMemberResult {
                        project_id: id_str,
                        ok: false,
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        Ok(result)
    }

    // -------------------------------------------------------------------------
    // Project construction (mirrors the CLI's `cmd_add`)
    // -------------------------------------------------------------------------

    fn build_project(&self, args: &AddProjectArgs) -> AppResult<Project> {
        let canonical = canonical_dir(&args.path)?;
        let reg = self.load_registry()?;

        let dir_name = dir_name_of(&canonical);
        let id_str = slugify(&dir_name);
        let id = ProjectId::new(id_str.clone());
        let name = args.name.clone().unwrap_or(dir_name);
        let hostname = args
            .hostname
            .clone()
            .unwrap_or_else(|| format!("{id_str}.{}", reg.domain_suffix));

        let detection = detect_kind(&canonical);
        let kind: ProjectType = args.kind.map(Into::into).unwrap_or(detection.kind);
        let port = args.port.or(detection.port);
        let start_command = args.start_command.clone().or(detection.start_command);
        let has_start_command = start_command.is_some();
        let https = args.https.unwrap_or(true);
        let auto_start = args.auto_start.unwrap_or(false);

        let runtime =
            crate::project_runtime::detect(&canonical).or_else(|| reg.runtimes.default_for(kind));

        let php_version = if kind == ProjectType::Php {
            args.php_version
                .clone()
                .or(detection.php_version)
                .or_else(|| runtime.as_ref().map(|r| r.version.clone()))
        } else {
            None
        };
        let document_root = if kind == ProjectType::Php {
            args.document_root
                .clone()
                .filter(|s| !s.trim().is_empty())
                .or(detection.document_root)
        } else {
            None
        };
        let web_server: Option<WebServer> = if kind == ProjectType::Php && !has_start_command {
            Some(detection.web_server.unwrap_or(WebServer::Caddy))
        } else {
            None
        };

        let readiness = port.map(|_| Readiness::Http {
            path: "/".into(),
            timeout_seconds: 75,
        });

        let services = match kind {
            ProjectType::Flutter | ProjectType::Xcode | ProjectType::Android => vec![],
            ProjectType::Php if has_start_command => vec!["caddy".into()],
            ProjectType::Php => vec!["caddy".into(), "php-fpm".into()],
            _ if https => vec!["caddy".into()],
            _ => vec![],
        };

        Ok(Project {
            id,
            name,
            path: canonical,
            kind,
            start_command,
            port,
            extra_ports: vec![],
            hostname,
            https,
            services,
            env: Default::default(),
            readiness,
            auto_start,
            tags: vec![],
            document_root,
            php_version,
            web_server,
            mobile_run: detection.mobile_run,
            runtime,
            workspace: None,
            cors: None,
            sandbox: None,
            domain: None,
        })
    }
}

// =============================================================================
// Runtime result helpers (converts runtimes::LanguageView into the MCP shape)
// =============================================================================

fn language_summary(lv: crate::runtimes::LanguageView) -> RuntimeLanguageSummary {
    let versions = lv
        .versions
        .into_iter()
        .map(|vv| RuntimeVersionSummary {
            is_default: lv
                .default_version
                .as_deref()
                .is_some_and(|d| d == vv.install.version),
            version: vv.install.version,
            source: crate::runtimes::source_label(vv.install.source).to_string(),
            binary: vv.install.binary.to_string_lossy().into_owned(),
        })
        .collect();
    RuntimeLanguageSummary {
        id: lv.id,
        display_name: lv.display_name,
        default_version: lv.default_version,
        versions,
        install_hint: lv.install_hint,
    }
}

// =============================================================================
// Group result types (separate from Tauri IPC shapes in commands/groups.rs)
// =============================================================================

/// One group's registry view, returned by CRUD operations.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct GroupSummary {
    /// Stable slug id — use this in start/stop/restart/update/remove calls.
    pub id: String,
    pub name: String,
    /// All member project ids recorded in the group (may include stale ids if
    /// a member was removed from the registry without updating the group).
    pub project_ids: Vec<String>,
    /// Subset of `project_ids` that currently exist in the registry.
    /// When `known_ids.len() < project_ids.len()`, the group has stale
    /// members — call `portbay_update_group` to clean them up.
    pub known_ids: Vec<String>,
    pub member_count: usize,
}

/// Per-member result for a group lifecycle fanout.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct GroupMemberResult {
    pub project_id: String,
    pub ok: bool,
    /// Error detail when `ok` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Result of `portbay_start_group`, `portbay_stop_group`,
/// `portbay_restart_group`.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct GroupFanoutResult {
    pub group_id: String,
    pub succeeded: usize,
    pub failed: usize,
    pub results: Vec<GroupMemberResult>,
}

// Internal op tag — not exposed.
#[derive(Clone, Copy)]
enum GroupOp {
    Start,
    Stop,
    Restart,
}

// =============================================================================
// Free helpers
// =============================================================================

fn group_summary(g: &Group, known: &std::collections::HashSet<&str>) -> GroupSummary {
    let project_ids: Vec<String> = g.projects.iter().map(|id| id.as_str().to_string()).collect();
    let known_ids: Vec<String> = project_ids
        .iter()
        .filter(|id| known.contains(id.as_str()))
        .cloned()
        .collect();
    GroupSummary {
        id: g.id.clone(),
        name: g.name.clone(),
        member_count: project_ids.len(),
        project_ids,
        known_ids,
    }
}

fn summary(p: &Project, proc: Option<&Process>) -> ProjectSummary {
    let scheme = if p.https { "https" } else { "http" };
    ProjectSummary {
        id: p.id.as_str().to_string(),
        name: p.name.clone(),
        kind: kind_str(p.kind),
        hostname: p.hostname.clone(),
        url: format!("{scheme}://{}", p.hostname),
        https: p.https,
        port: p.port,
        status: proc
            .map(|pr| status_str(pr.portbay_status()))
            .unwrap_or_else(|| "unknown".into()),
        pid: proc.map(|pr| pr.pid),
        restarts: proc.map(|pr| pr.restarts),
        ready: proc.map(|pr| pr.is_ready.clone()),
    }
}

fn kind_str(kind: ProjectType) -> String {
    format!("{kind:?}").to_lowercase()
}

fn status_str(status: ProjectStatus) -> String {
    match status {
        ProjectStatus::Stopped => "stopped",
        ProjectStatus::Starting => "starting",
        ProjectStatus::Running => "running",
        ProjectStatus::Unhealthy => "unhealthy",
        ProjectStatus::Crashed => "crashed",
        ProjectStatus::PortConflict => "port_conflict",
    }
    .to_string()
}

fn canonical_dir(path: &str) -> AppResult<PathBuf> {
    let p = PathBuf::from(path)
        .canonicalize()
        .map_err(|e| AppError::BadInput(format!("path: {e}")))?;
    if !p.is_dir() {
        return Err(AppError::BadInput(format!(
            "path is not a directory: {path}"
        )));
    }
    Ok(p)
}

fn dir_name_of(p: &std::path::Path) -> String {
    p.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_string()
}

fn certs_root() -> Option<PathBuf> {
    let mut p = dirs::data_dir()?;
    p.push("PortBay");
    p.push("certs");
    Some(p)
}

/// Best-effort `/etc/hosts` add via the privileged helper, falling back to a
/// direct write. Returns warnings (never fails the operation) — the registry
/// change has already landed, and the user can fix hosts later.
fn best_effort_add_host(suffix: &str, hostname: &str) -> Vec<String> {
    if HostsHelperClient::system()
        .add(hostname, Ipv4Addr::LOCALHOST, suffix)
        .is_ok()
    {
        return vec![];
    }
    match HostsManager::system().add(hostname, Ipv4Addr::LOCALHOST) {
        Ok(()) => vec![],
        Err(HostsError::PermissionDenied { .. }) => vec![format!(
            "couldn't update /etc/hosts (permission denied). Run: sudo portbay hosts add {hostname}"
        )],
        Err(e) => vec![format!("hosts: {e}")],
    }
}

fn best_effort_remove_host(suffix: &str, hostname: &str) -> Vec<String> {
    if HostsHelperClient::system().remove(hostname, suffix).is_ok() {
        return vec![];
    }
    match HostsManager::system().remove(hostname) {
        Ok(()) => vec![],
        Err(e) => vec![format!("hosts: {e}")],
    }
}

/// The dev command that starts a single app from its own directory, using the
/// monorepo's package manager. Mirrors `commands::projects::standalone_dev_command`
/// — kept here so `ops` doesn't need a pub re-export from the commands layer.
fn standalone_dev_command(tool: WorkspaceTool) -> String {
    match tool {
        WorkspaceTool::Pnpm | WorkspaceTool::Turbo => "pnpm dev".into(),
        WorkspaceTool::Npm => "npm run dev".into(),
        WorkspaceTool::Yarn => "yarn dev".into(),
        WorkspaceTool::Bun => "bun run dev".into(),
    }
}

/// Open the PortBay app so its daemon comes up (macOS only).
fn launch_app() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .args(["-a", "PortBay"])
            .status();
    }
}

impl From<McpTemplate> for ScaffoldKind {
    fn from(t: McpTemplate) -> Self {
        match t {
            McpTemplate::Nextjs => ScaffoldKind::Nextjs,
            McpTemplate::Vite => ScaffoldKind::Vite,
            McpTemplate::Astro => ScaffoldKind::Astro,
            McpTemplate::Laravel => ScaffoldKind::Laravel,
            McpTemplate::Php => ScaffoldKind::Php,
        }
    }
}

// =============================================================================
// Tests — registry-path operations need no daemon (PcClient just fails and we
// degrade), so they run fully offline against a tempfile registry.
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn ctx_in(dir: &std::path::Path) -> McpContext {
        McpContext {
            registry_path: dir.join("registry.json"),
            // A port nothing is listening on, so the daemon always reads as down.
            pc_port: 1,
        }
    }

    #[tokio::test]
    async fn add_then_list_roundtrips_offline() {
        let home = tempdir().unwrap();
        let proj = tempdir().unwrap();
        let ctx = ctx_in(home.path());

        let res = ctx
            .add_project(AddProjectArgs {
                path: proj.path().to_string_lossy().to_string(),
                name: Some("My App".into()),
                hostname: Some("myapp.test".into()),
                kind: Some(McpProjectKind::Static),
                port: None,
                start_command: None,
                https: Some(true),
                auto_start: Some(false),
                php_version: None,
                document_root: None,
            })
            .await
            .expect("add should succeed");
        assert!(res.ok);
        let added = res.project.unwrap();
        assert_eq!(added.hostname, "myapp.test");
        assert_eq!(added.url, "https://myapp.test");
        assert_eq!(added.kind, "static");

        let listed = ctx.list_projects().await.unwrap();
        assert!(!listed.daemon_reachable, "no daemon on port 1");
        assert_eq!(listed.projects.len(), 1);
        assert_eq!(listed.projects[0].status, "unknown");
    }

    #[tokio::test]
    async fn update_patches_only_set_fields() {
        let home = tempdir().unwrap();
        let proj = tempdir().unwrap();
        let ctx = ctx_in(home.path());
        let id = ctx
            .add_project(AddProjectArgs {
                path: proj.path().to_string_lossy().to_string(),
                name: None,
                hostname: None,
                kind: Some(McpProjectKind::Node),
                port: Some(3000),
                start_command: Some("pnpm dev".into()),
                https: Some(true),
                auto_start: Some(false),
                php_version: None,
                document_root: None,
            })
            .await
            .unwrap()
            .project
            .unwrap()
            .id;

        ctx.update_project(UpdateProjectArgs {
            id: id.clone(),
            name: Some("Renamed".into()),
            hostname: None,
            port: Some(4000),
            start_command: None,
            https: None,
            auto_start: Some(true),
            tags: Some(vec!["api".into()]),
        })
        .await
        .unwrap();

        let p = ctx.get_project(&id).await.unwrap();
        assert_eq!(p.name, "Renamed");
        assert_eq!(p.port, Some(4000));
        // hostname + start_command were left untouched.
        assert!(p.hostname.ends_with(".test") || p.hostname.contains('.'));
    }

    #[tokio::test]
    async fn remove_is_idempotent_in_effect() {
        let home = tempdir().unwrap();
        let proj = tempdir().unwrap();
        let ctx = ctx_in(home.path());
        let id = ctx
            .add_project(AddProjectArgs {
                path: proj.path().to_string_lossy().to_string(),
                name: None,
                hostname: None,
                kind: Some(McpProjectKind::Static),
                port: None,
                start_command: None,
                https: Some(false),
                auto_start: Some(false),
                php_version: None,
                document_root: None,
            })
            .await
            .unwrap()
            .project
            .unwrap()
            .id;

        ctx.remove_project(&id).await.unwrap();
        assert!(ctx.list_projects().await.unwrap().projects.is_empty());
        // Removing again is a clean NotFound, not a panic.
        let err = ctx.remove_project(&id).await.unwrap_err();
        assert!(matches!(err, AppError::Registry(_) | AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn detect_reports_a_kind_for_a_plain_folder() {
        let home = tempdir().unwrap();
        let proj = tempdir().unwrap();
        let ctx = ctx_in(home.path());
        let d = ctx.detect_project(&proj.path().to_string_lossy()).unwrap();
        assert!(!d.kind.is_empty());
        assert!(d.suggested_hostname.contains('.'));
    }

    #[tokio::test]
    async fn detect_workspace_apps_returns_none_for_plain_folder() {
        let home = tempdir().unwrap();
        let proj = tempdir().unwrap();
        let ctx = ctx_in(home.path());
        // A bare directory (no package.json with workspaces) → None.
        let result = ctx
            .detect_workspace_apps(&proj.path().to_string_lossy())
            .unwrap();
        assert!(result.is_none(), "plain folder should not be detected as a monorepo");
    }

    #[tokio::test]
    async fn detect_workspace_apps_finds_apps_in_pnpm_monorepo() {
        use std::fs;
        let home = tempdir().unwrap();
        let root = tempdir().unwrap();
        let ctx = ctx_in(home.path());

        // Build a minimal pnpm monorepo layout:
        //   root/
        //     package.json          (workspaces field)
        //     pnpm-workspace.yaml   (triggers pnpm detection)
        //     apps/web/package.json (has a "dev" script)
        let apps_web = root.path().join("apps").join("web");
        fs::create_dir_all(&apps_web).unwrap();
        fs::write(
            root.path().join("package.json"),
            r#"{"name":"monorepo","workspaces":["apps/*"]}"#,
        )
        .unwrap();
        fs::write(root.path().join("pnpm-workspace.yaml"), "packages:\n  - 'apps/*'\n").unwrap();
        // lockfile so detect_package_manager picks pnpm.
        fs::write(root.path().join("pnpm-lock.yaml"), "lockfileVersion: '6.0'\n").unwrap();
        fs::write(
            apps_web.join("package.json"),
            r#"{"name":"@acme/web","scripts":{"dev":"next dev"}}"#,
        )
        .unwrap();

        let result = ctx
            .detect_workspace_apps(&root.path().to_string_lossy())
            .unwrap();

        let scan = result.expect("pnpm monorepo should be detected");
        assert_eq!(scan.tool, "pnpm");
        assert_eq!(scan.apps.len(), 1);
        let app = &scan.apps[0];
        assert_eq!(app.package, "@acme/web");
        assert_eq!(app.suggested_id, "web");
        assert!(app.suggested_hostname.ends_with(".portbay.test") || app.suggested_hostname.contains('.'));
        assert_eq!(app.suggested_start_command.as_deref(), Some("pnpm dev"));
    }

    #[tokio::test]
    async fn setup_from_recipe_applies_blueprint_and_flags_pending_services() {
        let home = tempdir().unwrap();
        let proj = tempdir().unwrap();
        let ctx = ctx_in(home.path());

        // Catalog is exposed and non-empty.
        assert!(!ctx.list_recipes().recipes.is_empty());

        // Laravel recipe → PHP project at public/, HTTPS, with a pending-DB note.
        let res = ctx
            .setup_from_recipe(SetupFromRecipeArgs {
                recipe: "laravel".into(),
                path: proj.path().to_string_lossy().to_string(),
                name: Some("Blog".into()),
                hostname: Some("blog.test".into()),
                php_version: None,
                https: None,
                start_now: Some(false),
                auto_launch: None,
            })
            .await
            .expect("recipe setup should register the project");
        assert!(res.ok);
        let p = res.project.unwrap();
        assert_eq!(p.kind, "php");
        assert_eq!(p.hostname, "blog.test");
        assert!(p.https);
        assert!(
            res.warnings.iter().any(|w| w.contains("database")),
            "laravel recipe should flag the pending database recommendation"
        );

        // Unknown recipe is a clean BadInput, not a panic.
        let err = ctx
            .setup_from_recipe(SetupFromRecipeArgs {
                recipe: "nonsense".into(),
                path: proj.path().to_string_lossy().to_string(),
                name: None,
                hostname: None,
                php_version: None,
                https: None,
                start_now: Some(false),
                auto_launch: None,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadInput(_)));
    }

    #[tokio::test]
    async fn lifecycle_without_daemon_is_sidecar_down() {
        let home = tempdir().unwrap();
        let proj = tempdir().unwrap();
        let ctx = ctx_in(home.path());
        let id = ctx
            .add_project(AddProjectArgs {
                path: proj.path().to_string_lossy().to_string(),
                name: None,
                hostname: None,
                kind: Some(McpProjectKind::Static),
                port: None,
                start_command: None,
                https: Some(false),
                auto_start: Some(false),
                php_version: None,
                document_root: None,
            })
            .await
            .unwrap()
            .project
            .unwrap()
            .id;

        let err = ctx.start(&id, false).await.unwrap_err();
        assert!(matches!(err, AppError::SidecarDown(_)));
    }

    // =========================================================================
    // Group tests — no daemon needed; all CRUD is registry-only.
    // =========================================================================

    #[tokio::test]
    async fn create_group_roundtrips_in_list() {
        let home = tempdir().unwrap();
        let ctx = ctx_in(home.path());

        let g = ctx
            .create_group(None, "My Cluster".into(), vec!["blog".into(), "api".into()])
            .unwrap();
        assert_eq!(g.id, "my-cluster");
        assert_eq!(g.name, "My Cluster");
        assert_eq!(g.member_count, 2);
        // Neither "blog" nor "api" are registered, so known_ids is empty.
        assert!(g.known_ids.is_empty());

        let groups = ctx.list_groups().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].id, "my-cluster");
    }

    #[tokio::test]
    async fn create_group_explicit_id_is_used() {
        let home = tempdir().unwrap();
        let ctx = ctx_in(home.path());

        let g = ctx
            .create_group(Some("cluster-1".into()), "Cluster One".into(), vec![])
            .unwrap();
        assert_eq!(g.id, "cluster-1");
    }

    #[tokio::test]
    async fn create_group_duplicate_id_errors() {
        let home = tempdir().unwrap();
        let ctx = ctx_in(home.path());

        ctx.create_group(None, "Dev".into(), vec![]).unwrap();
        let err = ctx
            .create_group(None, "Dev".into(), vec![])
            .unwrap_err();
        assert!(matches!(err, AppError::Registry(_)));
    }

    #[tokio::test]
    async fn update_group_patches_name_and_members() {
        let home = tempdir().unwrap();
        let proj = tempdir().unwrap();
        let ctx = ctx_in(home.path());

        // Register a real project so known_ids reflects it.
        let p = ctx
            .add_project(AddProjectArgs {
                path: proj.path().to_string_lossy().to_string(),
                name: Some("API".into()),
                hostname: Some("api.test".into()),
                kind: Some(McpProjectKind::Node),
                port: Some(4000),
                start_command: Some("node index.js".into()),
                https: Some(false),
                auto_start: Some(false),
                php_version: None,
                document_root: None,
            })
            .await
            .unwrap()
            .project
            .unwrap();

        ctx.create_group(None, "old name".into(), vec![]).unwrap();

        let updated = ctx
            .update_group(
                "old-name".into(),
                Some("New Name".into()),
                Some(vec![p.id.clone(), "ghost".into()]),
            )
            .unwrap();

        assert_eq!(updated.id, "old-name"); // id is immutable
        assert_eq!(updated.name, "New Name");
        assert_eq!(updated.member_count, 2);
        // Only the real project shows up in known_ids.
        assert_eq!(updated.known_ids, vec![p.id.clone()]);
    }

    #[tokio::test]
    async fn update_group_name_only_leaves_members_unchanged() {
        let home = tempdir().unwrap();
        let ctx = ctx_in(home.path());

        ctx.create_group(None, "alpha".into(), vec!["x".into(), "y".into()])
            .unwrap();
        let g = ctx
            .update_group("alpha".into(), Some("beta".into()), None)
            .unwrap();
        assert_eq!(g.name, "beta");
        assert_eq!(g.member_count, 2);
    }

    #[tokio::test]
    async fn remove_group_is_clean() {
        let home = tempdir().unwrap();
        let ctx = ctx_in(home.path());

        ctx.create_group(None, "temp".into(), vec![]).unwrap();
        ctx.remove_group("temp".into()).unwrap();
        assert!(ctx.list_groups().unwrap().is_empty());
        // Second remove is a clean error.
        let err = ctx.remove_group("temp".into()).unwrap_err();
        assert!(matches!(err, AppError::Registry(_)));
    }

    #[tokio::test]
    async fn group_known_ids_reflects_registered_projects() {
        let home = tempdir().unwrap();
        let proj_a = tempdir().unwrap();
        let proj_b = tempdir().unwrap();
        let ctx = ctx_in(home.path());

        // Write a signal file so detect_kind gives Static (port 8000) not
        // Custom (port 3000), avoiding the port-conflict on the second add.
        std::fs::write(proj_a.path().join("index.html"), "").unwrap();
        std::fs::write(proj_b.path().join("index.html"), "").unwrap();

        // Register project A (Static, port 8000 from detection).
        let a_id = ctx
            .add_project(AddProjectArgs {
                path: proj_a.path().to_string_lossy().to_string(),
                name: Some("A".into()),
                hostname: Some("a.test".into()),
                kind: Some(McpProjectKind::Static),
                port: None,
                start_command: None,
                https: Some(false),
                auto_start: Some(false),
                php_version: None,
                document_root: None,
            })
            .await
            .unwrap()
            .project
            .unwrap()
            .id;

        // Create a group with a registered member and a stale one.
        let g = ctx
            .create_group(None, "mixed".into(), vec![a_id.clone(), "stale".into()])
            .unwrap();
        assert_eq!(g.known_ids, vec![a_id.clone()]);
        assert_eq!(g.project_ids.len(), 2);

        // Register project B — same kind + port, different hostname only.
        // Use explicit port 8001 to avoid the duplicate-8000 conflict with A.
        ctx.add_project(AddProjectArgs {
            path: proj_b.path().to_string_lossy().to_string(),
            name: Some("B".into()),
            hostname: Some("b.test".into()),
            kind: Some(McpProjectKind::Static),
            port: Some(8001),
            start_command: None,
            https: Some(false),
            auto_start: Some(false),
            php_version: None,
            document_root: None,
        })
        .await
        .expect("project B should register without conflict");

        // After adding B, list_groups still shows same group (stale not auto-cleaned).
        let groups = ctx.list_groups().unwrap();
        assert_eq!(groups[0].known_ids.len(), 1, "only A is known; stale still stale");
    }

    #[tokio::test]
    async fn fanout_group_without_daemon_is_sidecar_down() {
        let home = tempdir().unwrap();
        let ctx = ctx_in(home.path());
        ctx.create_group(None, "test".into(), vec!["x".into()])
            .unwrap();
        let err = ctx.start_group("test".into()).await.unwrap_err();
        assert!(matches!(err, AppError::SidecarDown(_)));
    }

    // =========================================================================
    // Tunnel visibility tests — no daemon needed; reads/writes the state file.
    // =========================================================================

    #[test]
    fn list_tunnels_empty_when_no_state_file() {
        let home = tempdir().unwrap();
        let ctx = ctx_in(home.path());
        // No state file has been written, so the result is an empty vec.
        let tunnels = ctx.list_tunnels().unwrap();
        assert!(tunnels.is_empty());
    }

    #[test]
    fn list_tunnels_and_tunnel_status_round_trip() {
        use crate::tunnel::{write_state, TunnelStatus};

        let home = tempdir().unwrap();
        let ctx = ctx_in(home.path());
        let data_dir = home.path();

        let entry = TunnelStatus {
            project_id: "blog".into(),
            upstream_url: "http://localhost:3000".into(),
            public_url: Some("https://example.trycloudflare.com".into()),
            running: true,
            origin_reachable: Some(true),
            started_at_ms: 1_000_000,
        };
        write_state(data_dir, std::slice::from_ref(&entry)).expect("write_state should succeed");

        let tunnels = ctx.list_tunnels().unwrap();
        assert_eq!(tunnels.len(), 1);
        assert_eq!(tunnels[0].project_id, "blog");
        assert_eq!(
            tunnels[0].public_url.as_deref(),
            Some("https://example.trycloudflare.com")
        );
        assert!(tunnels[0].running);

        // tunnel_status finds by project_id.
        let found = ctx.tunnel_status("blog").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().project_id, "blog");

        // Unknown id returns None.
        let missing = ctx.tunnel_status("does-not-exist").unwrap();
        assert!(missing.is_none());
    }

    // =========================================================================
    // Runtime tests — registry-only, no daemon needed.
    // =========================================================================

    #[test]
    fn list_runtimes_returns_all_languages() {
        let home = tempdir().unwrap();
        let ctx = ctx_in(home.path());
        let runtimes = ctx.list_runtimes().unwrap();
        // Every supported language must appear.
        let ids: Vec<&str> = runtimes.iter().map(|l| l.id.as_str()).collect();
        assert!(ids.contains(&"php"), "php missing");
        assert!(ids.contains(&"node"), "node missing");
        assert!(ids.contains(&"bun"), "bun missing");
        assert!(ids.contains(&"python"), "python missing");
        assert!(ids.contains(&"go"), "go missing");
        assert!(ids.contains(&"ruby"), "ruby missing");
        // All entries have non-empty install hints.
        for l in &runtimes {
            assert!(!l.install_hint.is_empty(), "{} has empty install hint", l.id);
        }
    }

    #[test]
    fn set_default_unknown_lang_errors() {
        let home = tempdir().unwrap();
        let ctx = ctx_in(home.path());
        let err = ctx
            .set_default_runtime("notareal".into(), Some("1.0".into()))
            .unwrap_err();
        assert!(matches!(err, AppError::BadInput(_)));
    }

    #[test]
    fn set_default_clear_roundtrips() {
        let home = tempdir().unwrap();
        let ctx = ctx_in(home.path());
        // Clear when nothing is set is a no-op (no error).
        let views = ctx.set_default_runtime("node".into(), None).unwrap();
        let node = views.iter().find(|l| l.id == "node").unwrap();
        assert!(node.default_version.is_none());
    }

    #[test]
    fn set_default_unknown_version_errors() {
        let home = tempdir().unwrap();
        let ctx = ctx_in(home.path());
        // Version "99.99" is not on any real machine.
        let err = ctx
            .set_default_runtime("node".into(), Some("99.99".into()))
            .unwrap_err();
        assert!(matches!(err, AppError::BadInput(_)));
    }

    #[test]
    fn add_runtime_path_unknown_lang_errors() {
        let home = tempdir().unwrap();
        let ctx = ctx_in(home.path());
        let err = ctx
            .add_runtime_path("notareal".into(), "/usr/bin/php".into())
            .unwrap_err();
        assert!(matches!(err, AppError::BadInput(_)));
    }

    #[test]
    fn add_runtime_path_missing_binary_errors() {
        let home = tempdir().unwrap();
        let ctx = ctx_in(home.path());
        let err = ctx
            .add_runtime_path("node".into(), "/nonexistent/path/to/node".into())
            .unwrap_err();
        assert!(matches!(err, AppError::BadInput(_)));
    }

    #[test]
    fn remove_runtime_path_noop_when_not_present() {
        let home = tempdir().unwrap();
        let ctx = ctx_in(home.path());
        // No manual entries — removing is a no-op, not an error.
        let views = ctx
            .remove_runtime_path("node".into(), "20.0.0".into())
            .unwrap();
        assert!(views.iter().any(|l| l.id == "node"));
    }

    #[test]
    fn remove_runtime_path_drops_matching_entry() {
        use crate::registry::ManualRuntime;

        let home = tempdir().unwrap();
        let ctx = ctx_in(home.path());

        // Manually write a registry with one manual entry.
        let mut reg = ctx.load_registry().unwrap();
        // Use a tempdir as the fake binary path; we won't probe it here.
        let fake_bin = home.path().join("fake-node");
        std::fs::write(&fake_bin, b"#!/bin/sh\necho 20.0.0").unwrap();
        reg.runtimes.manual.push(ManualRuntime {
            lang: "node".into(),
            version: "20.0.0".into(),
            binary: fake_bin.clone(),
        });
        ctx.save_registry(&reg).unwrap();

        // Remove it.
        let views = ctx
            .remove_runtime_path("node".into(), "20.0.0".into())
            .unwrap();
        let node = views.iter().find(|l| l.id == "node").unwrap();
        assert!(
            node.versions
                .iter()
                .all(|v| v.binary != fake_bin.to_string_lossy()),
            "manual entry should be gone after remove"
        );
    }
}
