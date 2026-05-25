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
use crate::registry::{store, Project, ProjectId, ProjectType, Readiness, Registry, WebServer};
use crate::util::slugify;

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
    // Mutations
    // -------------------------------------------------------------------------

    pub async fn add_project(&self, args: AddProjectArgs) -> AppResult<OpResult> {
        let project = self.build_project(&args)?;
        let mut reg = self.load_registry()?;
        entitlements::check_can_add(reg.projects.len())
            .map_err(|cap| AppError::ProjectCapReached { cap })?;
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
// Free helpers
// =============================================================================

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
}
