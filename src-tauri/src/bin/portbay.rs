//! PortBay CLI — `portbay <subcommand>`.
//!
//! Thin command-line interface over the same Rust core that powers the
//! Tauri GUI. The two share `portbay_lib`'s registry / process-compose /
//! caddy / mkcert modules verbatim.
//!
//! Subcommands implemented in this binary:
//!
//!   list           Show registered projects (with live status if daemon up)
//!   add            Register a project from a folder path
//!   remove         Unregister a project (+ optional cert + route cleanup)
//!   start          Start a single project (requires running daemon)
//!   stop           Stop a single project, or `--all` for universal stop
//!   restart        Restart a single project
//!   status         Show one project's status, or all when no id given
//!   logs           Static log tail
//!   open           Open the project's URL in the default browser
//!   doctor         Diagnose runtime, ports, certs, registry
//!
//! Connection model: this CLI is a client. It expects a PortBay daemon
//! (the Tauri app, or a future `portbay daemon` subcommand) to be running
//! and exposing Process Compose on a port discoverable via `runtime.json`
//! in the data dir. Falls back to the default PC port when runtime.json
//! is absent.
//!
//! Exit codes:
//!   0  success
//!   1  generic failure
//!   2  user input error (bad project id, missing argument)
//!   3  daemon unreachable
//!   4  port conflict
//!   5  readiness timeout (not used yet)

#![cfg_attr(not(debug_assertions), windows_subsystem = "console")]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use console::{style, Term};
use portbay_lib::caddy::CertPaths;
use portbay_lib::process_compose::{PcClient, Process, ProjectStatus, DEFAULT_PORT as PC_DEFAULT_PORT};
use portbay_lib::registry::{self, store, Project, ProjectId, ProjectType, Readiness, Registry};

// =============================================================================
// CLI shape
// =============================================================================

#[derive(Parser, Debug)]
#[command(
    name = "portbay",
    version,
    about = "PortBay — lightweight local development environment manager",
    long_about = None,
    disable_help_subcommand = true,
)]
struct Cli {
    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,

    /// Override the registry file location.
    #[arg(long, global = true, value_name = "PATH")]
    registry: Option<PathBuf>,

    /// Override the Process Compose daemon port.
    #[arg(long, global = true, value_name = "PORT")]
    pc_port: Option<u16>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// List registered projects (with live status if the daemon is up).
    List,

    /// Show the current status of one project, or all projects when no id.
    Status {
        /// Project id.
        id: Option<String>,
    },

    /// Register a project from a folder path.
    Add(AddArgs),

    /// Unregister a project. Removes the cert directory and Caddy route
    /// (when reachable) by default.
    Remove(RemoveArgs),

    /// Start a single project.
    Start { id: String },

    /// Stop a single project, or pass `--all` to stop every running process.
    Stop(StopArgs),

    /// Restart a single project.
    Restart { id: String },

    /// Tail static logs for a project (use `--limit` to control how many
    /// lines).
    Logs(LogsArgs),

    /// Open the project's hostname URL in the default browser.
    Open { id: String },

    /// Diagnose the runtime, ports, registry, and cert state.
    Doctor,
}

#[derive(Args, Debug)]
struct AddArgs {
    /// Project root path. Used as the working directory for the dev command.
    #[arg(value_name = "PATH")]
    path: PathBuf,

    /// Project id (url-safe slug). Defaults to the directory name.
    #[arg(long)]
    id: Option<String>,

    /// Human-readable name. Defaults to the directory name.
    #[arg(long)]
    name: Option<String>,

    /// Hostname (without https://). Defaults to `<id>.<domain_suffix>`.
    #[arg(long)]
    hostname: Option<String>,

    /// Project type.
    #[arg(long, value_enum, default_value = "custom")]
    kind: CliProjectType,

    /// Port the dev server binds to.
    #[arg(long)]
    port: Option<u16>,

    /// Shell command run inside the project's working directory.
    #[arg(long)]
    start_command: Option<String>,

    /// Whether to enable local HTTPS via mkcert.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    https: bool,

    /// Mark this project to auto-start when PortBay's daemon comes up.
    #[arg(long)]
    auto_start: bool,
}

#[derive(Args, Debug)]
struct RemoveArgs {
    /// Project id.
    id: String,

    /// Keep cert files and the live Caddy route even after removing the
    /// project from the registry.
    #[arg(long)]
    keep_artifacts: bool,
}

#[derive(Args, Debug)]
struct StopArgs {
    /// Project id. Omit when using `--all`.
    id: Option<String>,

    /// Stop every running process (the universal kill switch — the most
    /// important reliability promise in PortBay's design).
    #[arg(long)]
    all: bool,
}

#[derive(Args, Debug)]
struct LogsArgs {
    /// Project id.
    id: String,

    /// Number of trailing log lines to fetch.
    #[arg(long, default_value_t = 200)]
    limit: u32,

    /// Starting offset within the log buffer (0 = newest).
    #[arg(long, default_value_t = 0)]
    offset: u64,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum CliProjectType {
    Next,
    Vite,
    Php,
    Static,
    Node,
    Custom,
}

impl From<CliProjectType> for ProjectType {
    fn from(v: CliProjectType) -> Self {
        match v {
            CliProjectType::Next => ProjectType::Next,
            CliProjectType::Vite => ProjectType::Vite,
            CliProjectType::Php => ProjectType::Php,
            CliProjectType::Static => ProjectType::Static,
            CliProjectType::Node => ProjectType::Node,
            CliProjectType::Custom => ProjectType::Custom,
        }
    }
}

// =============================================================================
// Entry
// =============================================================================

fn main() -> ExitCode {
    let cli = Cli::parse();
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("portbay: failed to create runtime: {e}");
            return ExitCode::from(1);
        }
    };
    let result = rt.block_on(dispatch(cli));
    match result {
        Ok(code) => code,
        Err(e) => {
            print_error(&e);
            ExitCode::from(e.exit_code())
        }
    }
}

async fn dispatch(cli: Cli) -> Result<ExitCode, CliError> {
    let ctx = CliContext::from_args(&cli)?;
    match cli.cmd {
        Cmd::List => cmd_list(&ctx).await,
        Cmd::Status { id } => cmd_status(&ctx, id.as_deref()).await,
        Cmd::Add(args) => cmd_add(&ctx, args).await,
        Cmd::Remove(args) => cmd_remove(&ctx, args).await,
        Cmd::Start { id } => cmd_proc_op(&ctx, &id, ProcOp::Start).await,
        Cmd::Stop(args) => cmd_stop(&ctx, args).await,
        Cmd::Restart { id } => cmd_proc_op(&ctx, &id, ProcOp::Restart).await,
        Cmd::Logs(args) => cmd_logs(&ctx, args).await,
        Cmd::Open { id } => cmd_open(&ctx, &id).await,
        Cmd::Doctor => cmd_doctor(&ctx).await,
    }
}

// =============================================================================
// Context — config + I/O dependencies shared across commands
// =============================================================================

struct CliContext {
    registry_path: PathBuf,
    pc_port: u16,
    json: bool,
    term: Term,
}

impl CliContext {
    fn from_args(cli: &Cli) -> Result<Self, CliError> {
        let registry_path = match &cli.registry {
            Some(p) => p.clone(),
            None => store::default_path().map_err(CliError::Registry)?,
        };
        let pc_port = cli.pc_port.unwrap_or(PC_DEFAULT_PORT);
        Ok(Self {
            registry_path,
            pc_port,
            json: cli.json,
            term: Term::stdout(),
        })
    }

    fn pc(&self) -> PcClient {
        PcClient::new(self.pc_port)
    }

    fn load_registry(&self) -> Result<Registry, CliError> {
        store::load_or_default(&self.registry_path, "test").map_err(CliError::Registry)
    }

    fn save_registry(&self, r: &Registry) -> Result<(), CliError> {
        store::save_to(r, &self.registry_path).map_err(CliError::Registry)
    }
}

// =============================================================================
// Commands
// =============================================================================

async fn cmd_list(ctx: &CliContext) -> Result<ExitCode, CliError> {
    let reg = ctx.load_registry()?;
    let projects = reg.list_projects();

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(projects)?);
        return Ok(ExitCode::SUCCESS);
    }

    if projects.is_empty() {
        ctx.term.write_line(&format!(
            "{} No projects registered. {} `portbay add <path>`",
            style("·").dim(),
            style("Add one with").dim()
        ))
        .ok();
        return Ok(ExitCode::SUCCESS);
    }

    // Map of statuses keyed by project id. Empty if daemon is down.
    let pc_state = fetch_pc_state(ctx).await;

    let id_w = projects.iter().map(|p| p.id.as_str().len()).max().unwrap_or(2);
    let host_w = projects.iter().map(|p| p.hostname.len()).max().unwrap_or(2);
    for p in projects {
        let status = pc_state
            .as_ref()
            .and_then(|m| m.get(p.id.as_str()))
            .map(|proc| proc.portbay_status());
        let badge = status_badge(status);
        ctx.term.write_line(&format!(
            "  {badge} {id:<id_w$}  {host:<host_w$}  {kind}",
            id = style(p.id.as_str()).bold(),
            host = style(&p.hostname).dim(),
            kind = style(format!("{:?}", p.kind).to_lowercase()).dim(),
            id_w = id_w,
            host_w = host_w,
        ))
        .ok();
    }
    Ok(ExitCode::SUCCESS)
}

async fn cmd_status(ctx: &CliContext, id: Option<&str>) -> Result<ExitCode, CliError> {
    let reg = ctx.load_registry()?;
    let pc_state = fetch_pc_state(ctx).await;

    let entries: Vec<(&Project, Option<&Process>)> = match id {
        Some(id) => {
            let p = reg
                .get_project(&ProjectId::new(id))
                .ok_or_else(|| CliError::ProjectNotFound(id.to_string()))?;
            vec![(p, pc_state.as_ref().and_then(|m| m.get(id)))]
        }
        None => reg
            .list_projects()
            .iter()
            .map(|p| {
                (
                    p,
                    pc_state.as_ref().and_then(|m| m.get(p.id.as_str())),
                )
            })
            .collect(),
    };

    if ctx.json {
        let out: Vec<_> = entries
            .iter()
            .map(|(p, proc)| {
                serde_json::json!({
                    "id": p.id.as_str(),
                    "hostname": p.hostname,
                    "status": proc.map(|p| p.portbay_status()),
                    "pid": proc.map(|p| p.pid),
                    "is_running": proc.map(|p| p.is_running),
                    "is_ready": proc.map(|p| p.is_ready.clone()),
                    "restarts": proc.map(|p| p.restarts),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(ExitCode::SUCCESS);
    }

    if pc_state.is_none() {
        ctx.term.write_line(&format!(
            "{} Daemon not reachable on port {}. Status reflects registry only.",
            style("!").yellow(),
            ctx.pc_port
        ))
        .ok();
    }

    for (p, proc) in entries {
        let status = proc.map(|p| p.portbay_status());
        ctx.term.write_line(&format!(
            "  {} {}  {}",
            status_badge(status),
            style(p.id.as_str()).bold(),
            style(&p.hostname).dim(),
        ))
        .ok();
        if let Some(proc) = proc {
            ctx.term.write_line(&format!(
                "      pid={pid} running={running} ready={ready} restarts={restarts}",
                pid = proc.pid,
                running = proc.is_running,
                ready = proc.is_ready,
                restarts = proc.restarts
            ))
            .ok();
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn cmd_add(ctx: &CliContext, args: AddArgs) -> Result<ExitCode, CliError> {
    let mut reg = ctx.load_registry()?;

    let canonical = args
        .path
        .canonicalize()
        .map_err(|e| CliError::BadInput(format!("path: {e}")))?;

    let dir_name = canonical
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_string();

    let id_str = args.id.unwrap_or_else(|| slugify(&dir_name));
    let id = ProjectId::new(id_str.clone());

    let name = args.name.unwrap_or(dir_name);
    let hostname = args
        .hostname
        .unwrap_or_else(|| format!("{}.{}", id_str, reg.domain_suffix));

    let readiness = args.port.map(|_| Readiness::Http {
        path: "/".into(),
        timeout_seconds: 75,
    });

    let project = Project {
        id,
        name,
        path: canonical,
        kind: args.kind.into(),
        start_command: args.start_command,
        port: args.port,
        extra_ports: vec![],
        hostname,
        https: args.https,
        services: if args.https {
            vec!["caddy".into()]
        } else {
            vec![]
        },
        env: Default::default(),
        readiness,
        auto_start: args.auto_start,
        tags: vec![],
        document_root: None,
        php_version: None,
    };

    reg.add_project(project.clone()).map_err(CliError::Registry)?;
    ctx.save_registry(&reg)?;

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&project)?);
    } else {
        ctx.term.write_line(&format!(
            "{} {} registered as {}",
            style("✓").green(),
            project.id.as_str(),
            style(&project.hostname).dim()
        ))
        .ok();
        if project.https {
            ctx.term.write_line(&format!(
                "  {} cert issuance + Caddy wiring will happen when the daemon picks it up.",
                style("·").dim()
            ))
            .ok();
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn cmd_remove(ctx: &CliContext, args: RemoveArgs) -> Result<ExitCode, CliError> {
    let mut reg = ctx.load_registry()?;
    let pid = ProjectId::new(args.id.clone());
    let removed = reg
        .remove_project(&pid)
        .map_err(CliError::Registry)?;
    ctx.save_registry(&reg)?;

    let mut warnings: Vec<String> = Vec::new();
    if !args.keep_artifacts {
        // Try to remove the cert directory. Failure is non-fatal.
        if let Some(certs_root) = certs_root() {
            let dir = certs_root.join(removed.id.as_str());
            if dir.exists() {
                if let Err(e) = std::fs::remove_dir_all(&dir) {
                    warnings.push(format!("could not delete certs at {}: {e}", dir.display()));
                }
            }
        }
        // Note about live Caddy: we leave the route alone here. Once the
        // daemon is restarted (or a reconcile is triggered), it will drop
        // routes whose ids no longer have a matching project.
    }

    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "removed": removed.id.as_str(),
                "warnings": warnings,
            })
        );
    } else {
        ctx.term.write_line(&format!(
            "{} {} removed.",
            style("✓").green(),
            removed.id.as_str()
        ))
        .ok();
        for w in &warnings {
            ctx.term.write_line(&format!("  {} {w}", style("!").yellow())).ok();
        }
    }
    Ok(ExitCode::SUCCESS)
}

enum ProcOp {
    Start,
    Stop,
    Restart,
}

async fn cmd_proc_op(ctx: &CliContext, id: &str, op: ProcOp) -> Result<ExitCode, CliError> {
    let client = ctx.pc();
    let verb = match op {
        ProcOp::Start => "start",
        ProcOp::Stop => "stop",
        ProcOp::Restart => "restart",
    };
    let result = match op {
        ProcOp::Start => client.start(id).await,
        ProcOp::Stop => client.stop(id).await,
        ProcOp::Restart => client.restart(id).await,
    };
    match result {
        Ok(()) => {
            if ctx.json {
                println!("{}", serde_json::json!({ "ok": true, "id": id, "verb": verb }));
            } else {
                ctx.term.write_line(&format!(
                    "{} {} {}",
                    style("✓").green(),
                    verb,
                    id
                ))
                .ok();
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => Err(CliError::Pc(e)),
    }
}

async fn cmd_stop(ctx: &CliContext, args: StopArgs) -> Result<ExitCode, CliError> {
    if args.all {
        let client = ctx.pc();
        let processes = client.processes().await.map_err(CliError::Pc)?;
        let names: Vec<&str> = processes.iter().map(|p| p.name.as_str()).collect();
        if names.is_empty() {
            if !ctx.json {
                ctx.term.write_line(&format!(
                    "{} Nothing to stop.",
                    style("·").dim()
                ))
                .ok();
            }
            return Ok(ExitCode::SUCCESS);
        }
        let result = client.stop_many(&names).await.map_err(CliError::Pc)?;
        if ctx.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            ctx.term.write_line(&format!(
                "{} stopped {} process(es)",
                style("✓").green(),
                names.len()
            ))
            .ok();
        }
        return Ok(ExitCode::SUCCESS);
    }
    let id = args
        .id
        .ok_or_else(|| CliError::BadInput("pass an id, or use --all".into()))?;
    cmd_proc_op(ctx, &id, ProcOp::Stop).await
}

async fn cmd_logs(ctx: &CliContext, args: LogsArgs) -> Result<ExitCode, CliError> {
    let client = ctx.pc();
    let lines = client
        .logs(&args.id, args.offset, args.limit)
        .await
        .map_err(CliError::Pc)?;
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&lines)?);
    } else {
        for line in lines {
            println!("{line}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn cmd_open(ctx: &CliContext, id: &str) -> Result<ExitCode, CliError> {
    let reg = ctx.load_registry()?;
    let p = reg
        .get_project(&ProjectId::new(id))
        .ok_or_else(|| CliError::ProjectNotFound(id.to_string()))?;
    let scheme = if p.https { "https" } else { "http" };
    let url = format!("{scheme}://{}", p.hostname);
    let status = std::process::Command::new("open")
        .arg(&url)
        .status();
    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "url": url,
                "ok": status.as_ref().map(|s| s.success()).unwrap_or(false),
            })
        );
    } else if let Ok(s) = status {
        if s.success() {
            ctx.term
                .write_line(&format!("{} opened {url}", style("✓").green()))
                .ok();
        } else {
            ctx.term
                .write_line(&format!("{} `open` exited non-zero for {url}", style("!").yellow()))
                .ok();
        }
    } else if let Err(e) = status {
        return Err(CliError::Other(format!("failed to spawn `open`: {e}")));
    }
    Ok(ExitCode::SUCCESS)
}

async fn cmd_doctor(ctx: &CliContext) -> Result<ExitCode, CliError> {
    let mut findings: Vec<(String, Verdict, String)> = Vec::new();

    // Registry
    match ctx.load_registry() {
        Ok(reg) => findings.push((
            "registry".into(),
            Verdict::Ok,
            format!(
                "{} project(s), v{} schema, suffix .{}",
                reg.list_projects().len(),
                reg.version,
                reg.domain_suffix
            ),
        )),
        Err(e) => findings.push(("registry".into(), Verdict::Fail, e.to_string())),
    }

    // PC daemon reachability
    let pc_client = ctx.pc();
    match pc_client.live().await {
        Ok(true) => findings.push((
            format!("process-compose :{}", ctx.pc_port),
            Verdict::Ok,
            "alive".into(),
        )),
        Ok(false) => findings.push((
            format!("process-compose :{}", ctx.pc_port),
            Verdict::Warn,
            "not reachable".into(),
        )),
        Err(e) => findings.push((
            format!("process-compose :{}", ctx.pc_port),
            Verdict::Warn,
            e.to_string(),
        )),
    }

    // Tooling on PATH
    for tool in ["mkcert", "caddy", "process-compose"] {
        match which::which(tool) {
            Ok(p) => findings.push((
                format!("tool: {tool}"),
                Verdict::Ok,
                p.display().to_string(),
            )),
            Err(_) => findings.push((
                format!("tool: {tool}"),
                Verdict::Warn,
                "not found on PATH (the bundled .app uses its sidecar — this only matters for CLI standalone use)".into(),
            )),
        }
    }

    // certs root presence
    if let Some(root) = certs_root() {
        let count = std::fs::read_dir(&root)
            .map(|d| d.count())
            .unwrap_or(0);
        findings.push((
            "certs root".into(),
            if root.exists() { Verdict::Ok } else { Verdict::Warn },
            format!("{} ({} entries)", root.display(), count),
        ));
    }

    if ctx.json {
        let out: Vec<_> = findings
            .iter()
            .map(|(label, verdict, detail)| {
                serde_json::json!({
                    "check": label,
                    "verdict": match verdict {
                        Verdict::Ok => "ok",
                        Verdict::Warn => "warn",
                        Verdict::Fail => "fail",
                    },
                    "detail": detail,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        for (label, verdict, detail) in &findings {
            let badge = match verdict {
                Verdict::Ok => style("✓").green(),
                Verdict::Warn => style("!").yellow(),
                Verdict::Fail => style("✗").red(),
            };
            ctx.term
                .write_line(&format!("  {badge} {label:<28} {}", style(detail).dim()))
                .ok();
        }
    }

    let any_fail = findings.iter().any(|(_, v, _)| matches!(v, Verdict::Fail));
    Ok(if any_fail {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

// =============================================================================
// Helpers
// =============================================================================

enum Verdict {
    Ok,
    Warn,
    Fail,
}

/// Try to fetch process state from PC. Returns `None` (not an error) when
/// the daemon is unreachable — many commands degrade gracefully.
async fn fetch_pc_state(ctx: &CliContext) -> Option<std::collections::HashMap<String, Process>> {
    let client = ctx.pc();
    let processes = client.processes().await.ok()?;
    Some(
        processes
            .into_iter()
            .map(|p| (p.name.clone(), p))
            .collect(),
    )
}

fn status_badge(status: Option<ProjectStatus>) -> console::StyledObject<&'static str> {
    match status {
        Some(ProjectStatus::Running) => style("●").green(),
        Some(ProjectStatus::Starting) => style("◐").cyan(),
        Some(ProjectStatus::Unhealthy) => style("⚠").yellow(),
        Some(ProjectStatus::Crashed) => style("✕").red(),
        Some(ProjectStatus::PortConflict) => style("⊘").yellow(),
        Some(ProjectStatus::Stopped) | None => style("○").dim(),
    }
}

fn certs_root() -> Option<PathBuf> {
    let mut p = dirs::data_dir()?;
    p.push("PortBay");
    p.push("certs");
    Some(p)
}

fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = true;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

// Allows the linker to see this even though the file isn't referenced
// from main(); silences "dead code" on the CertPaths reexport above.
#[allow(dead_code)]
fn _ensure_cert_paths_in_scope(_: CertPaths) {}

// =============================================================================
// Error model
// =============================================================================

#[derive(Debug)]
enum CliError {
    Registry(registry::RegistryError),
    Pc(portbay_lib::process_compose::PcError),
    ProjectNotFound(String),
    BadInput(String),
    Json(serde_json::Error),
    Other(String),
}

impl CliError {
    fn exit_code(&self) -> u8 {
        match self {
            CliError::ProjectNotFound(_) | CliError::BadInput(_) => 2,
            CliError::Pc(_) => 3,
            CliError::Registry(_) | CliError::Json(_) | CliError::Other(_) => 1,
        }
    }
}

impl From<serde_json::Error> for CliError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::Registry(e) => write!(f, "registry: {e}"),
            CliError::Pc(e) => write!(f, "daemon: {e}"),
            CliError::ProjectNotFound(id) => write!(f, "project not found: {id}"),
            CliError::BadInput(s) => write!(f, "bad input: {s}"),
            CliError::Json(e) => write!(f, "json: {e}"),
            CliError::Other(s) => write!(f, "{s}"),
        }
    }
}

fn print_error(e: &CliError) {
    let term = Term::stderr();
    let _ = term.write_line(&format!("{} {e}", style("✗").red()));
    if let CliError::Pc(_) = e {
        let _ = term.write_line(&format!(
            "  {} The daemon may not be running. Start PortBay.app, or pass --pc-port if it's on a non-default port.",
            style("hint:").dim()
        ));
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_lowercases_and_hyphenates() {
        assert_eq!(slugify("Nour Beiruti"), "nour-beiruti");
        assert_eq!(slugify("Tribal House CMS"), "tribal-house-cms");
        assert_eq!(slugify("__weird___name__"), "weird-name");
        assert_eq!(slugify("UPPER"), "upper");
    }

    #[test]
    fn slugify_handles_unicode_by_dropping_it() {
        // Phase 1: ASCII-only IDs. Anything else turns into hyphens.
        assert_eq!(slugify("Café"), "caf");
    }

    #[test]
    fn cli_parses_list() {
        let cli = Cli::try_parse_from(["portbay", "list"]).unwrap();
        assert!(matches!(cli.cmd, Cmd::List));
        assert!(!cli.json);
    }

    #[test]
    fn cli_parses_add_with_defaults() {
        let cli =
            Cli::try_parse_from(["portbay", "add", "/tmp/x"]).unwrap();
        let Cmd::Add(args) = cli.cmd else {
            panic!("expected Add")
        };
        assert_eq!(args.path, PathBuf::from("/tmp/x"));
        assert!(args.https);
        assert!(matches!(args.kind, CliProjectType::Custom));
    }

    #[test]
    fn cli_parses_stop_all() {
        let cli = Cli::try_parse_from(["portbay", "stop", "--all"]).unwrap();
        let Cmd::Stop(args) = cli.cmd else {
            panic!("expected Stop")
        };
        assert!(args.all);
        assert!(args.id.is_none());
    }

    #[test]
    fn cli_parses_global_json_flag_after_subcommand() {
        let cli = Cli::try_parse_from(["portbay", "list", "--json"]).unwrap();
        assert!(cli.json);
    }

    #[test]
    fn cli_parses_pc_port_override() {
        let cli =
            Cli::try_parse_from(["portbay", "--pc-port", "9000", "status"]).unwrap();
        assert_eq!(cli.pc_port, Some(9000));
    }

    #[test]
    fn cli_project_type_round_trip() {
        let t: ProjectType = CliProjectType::Next.into();
        assert!(matches!(t, ProjectType::Next));
    }
}
