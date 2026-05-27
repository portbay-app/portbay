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

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use console::{style, Term};
use std::net::Ipv4Addr;

use portbay_lib::caddy::CertPaths;
use portbay_lib::hosts::{HostsError, HostsManager};
use portbay_lib::hosts_helper::HostsHelperClient;
use portbay_lib::process_compose::{
    PcClient, Process, ProjectStatus, DEFAULT_PORT as PC_DEFAULT_PORT,
};
use portbay_lib::commands::projects::detect_kind;
use portbay_lib::registry::{self, store, Group, Project, ProjectId, ProjectType, Readiness, Registry, WorkspaceTool};
use portbay_lib::registry::workspace as workspace_detect;

/// Domain suffix used when no registry exists yet. Kept in sync with the
/// GUI's `lib.rs::DEFAULT_DOMAIN_SUFFIX` so the CLI and app agree.
const DEFAULT_DOMAIN_SUFFIX: &str = "portbay.test";

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

    /// Hidden helper used by shell completion scripts to list project ids.
    #[arg(long, hide = true, global = true)]
    complete_projects: bool,

    /// Hidden helper used by shell completion scripts to list running project ids.
    #[arg(long, hide = true, global = true)]
    complete_running_projects: bool,

    #[command(subcommand)]
    cmd: Option<Cmd>,
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

    /// Manage /etc/hosts entries for PortBay projects.
    #[command(subcommand)]
    Hosts(HostsCmd),

    /// Write `<project_path>/.portbay.json` so this project's setup can
    /// be committed to a repo and reproduced by teammates.
    Export {
        /// Project id.
        id: String,
    },

    /// Generate shell completion scripts.
    Completions {
        /// Shell to generate completions for.
        shell: Shell,
    },

    /// Sign in to PortBay Cloud (GitHub by default; `--email <addr>` for a magic link).
    Login(LoginArgs),

    /// Show the current account and entitlement (tier, project cap, features).
    License,

    /// Sign out and clear the saved session.
    Logout,

    /// Manage project groups (batch start/stop/restart and organisational
    /// clusters of projects).
    #[command(subcommand)]
    Group(GroupCmd),

    /// Inspect a folder and print detection results: detected framework,
    /// suggested id, hostname, port, and start command. With `--apps`, list
    /// the runnable apps inside a JS monorepo instead (one row per app).
    Detect {
        /// Absolute or relative path to the project or monorepo root to inspect.
        path: String,

        /// Scan the folder as a JS monorepo and list individual runnable apps.
        /// Prints "not a monorepo" when the folder has no recognised workspace
        /// layout. Without this flag, single-project detection is run instead.
        #[arg(long)]
        apps: bool,
    },
}

#[derive(Args, Debug)]
struct LoginArgs {
    /// Sign in with an email magic link instead of GitHub OAuth.
    #[arg(long)]
    email: Option<String>,
}

#[derive(Subcommand, Debug)]
enum HostsCmd {
    /// List PortBay-managed entries in /etc/hosts.
    List,
    /// Add a hostname → IP mapping (default IP 127.0.0.1). Requires sudo.
    Add {
        hostname: String,
        #[arg(long, default_value = "127.0.0.1")]
        ip: Ipv4Addr,
    },
    /// Remove a hostname. Requires sudo. Missing entries are no-op.
    Remove { hostname: String },
    /// Remove every PortBay-managed entry. Requires sudo.
    Clear,
    /// Reconcile /etc/hosts against the registry — drop entries for
    /// projects that no longer exist, add entries for projects that do.
    /// Requires sudo.
    Reconcile,
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

    /// PHP document root relative to the project path, e.g. public.
    #[arg(long)]
    document_root: Option<String>,

    /// PHP version label to bind to, e.g. 8.3.
    #[arg(long)]
    php_version: Option<String>,

    /// PHP web server for document-root projects.
    #[arg(long, value_enum, default_value = "caddy")]
    web_server: CliWebServer,

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
    Flutter,
    Xcode,
    Android,
    Custom,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum CliWebServer {
    Caddy,
    Nginx,
    Apache,
}

impl From<CliProjectType> for ProjectType {
    fn from(v: CliProjectType) -> Self {
        match v {
            CliProjectType::Next => ProjectType::Next,
            CliProjectType::Vite => ProjectType::Vite,
            CliProjectType::Php => ProjectType::Php,
            CliProjectType::Static => ProjectType::Static,
            CliProjectType::Node => ProjectType::Node,
            CliProjectType::Flutter => ProjectType::Flutter,
            CliProjectType::Xcode => ProjectType::Xcode,
            CliProjectType::Android => ProjectType::Android,
            CliProjectType::Custom => ProjectType::Custom,
        }
    }
}

impl From<CliWebServer> for portbay_lib::registry::WebServer {
    fn from(v: CliWebServer) -> Self {
        match v {
            CliWebServer::Caddy => portbay_lib::registry::WebServer::Caddy,
            CliWebServer::Nginx => portbay_lib::registry::WebServer::Nginx,
            CliWebServer::Apache => portbay_lib::registry::WebServer::Apache,
        }
    }
}

#[derive(Subcommand, Debug)]
enum GroupCmd {
    /// List all project groups.
    List,

    /// Create a new project group.
    Create(GroupCreateArgs),

    /// Update a group's name or member list.
    Update(GroupUpdateArgs),

    /// Remove a group (members are not affected).
    Remove {
        /// Group id (slug).
        id: String,
    },

    /// Start every project in a group (requires running daemon).
    Start {
        /// Group id (slug).
        id: String,
    },

    /// Stop every project in a group (requires running daemon).
    Stop {
        /// Group id (slug).
        id: String,
    },

    /// Restart every project in a group (requires running daemon).
    Restart {
        /// Group id (slug).
        id: String,
    },
}

#[derive(Args, Debug)]
struct GroupCreateArgs {
    /// Human-readable group name (e.g. "Backend services").
    name: String,

    /// Explicit group id (url-safe slug). Defaults to a slug of `name`.
    #[arg(long)]
    id: Option<String>,

    /// Project ids to include in the group (repeatable).
    #[arg(long = "project", value_name = "PROJECT_ID")]
    projects: Vec<String>,
}

#[derive(Args, Debug)]
struct GroupUpdateArgs {
    /// Group id (slug) to update.
    id: String,

    /// New display name.
    #[arg(long)]
    name: Option<String>,

    /// Full replacement member list (repeatable). When provided, replaces the
    /// entire member list — it is not a merge. Pass multiple times to set
    /// several members: `--project blog --project api`.
    #[arg(long = "project", value_name = "PROJECT_ID")]
    projects: Vec<String>,
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
    if cli.complete_projects {
        return cmd_complete_projects(&cli, false).await;
    }
    if cli.complete_running_projects {
        return cmd_complete_projects(&cli, true).await;
    }
    let ctx = CliContext::from_args(&cli)?;
    let Some(cmd) = cli.cmd else {
        let mut command = Cli::command();
        command
            .print_help()
            .map_err(|e| CliError::Other(e.to_string()))?;
        println!();
        return Ok(ExitCode::SUCCESS);
    };
    match cmd {
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
        Cmd::Hosts(sub) => cmd_hosts(&ctx, sub).await,
        Cmd::Export { id } => cmd_export(&ctx, &id).await,
        Cmd::Completions { shell } => cmd_completions(shell),
        Cmd::Login(args) => cmd_login(args).await,
        Cmd::License => cmd_license(),
        Cmd::Logout => cmd_logout(),
        Cmd::Group(sub) => cmd_group(&ctx, sub).await,
        Cmd::Detect { path, apps } => cmd_detect(&ctx, &path, apps).await,
    }
}

/// `portbay login` — drive the flow+poll handshake from the terminal, then
/// store the session in the OS keychain (shared with the GUI).
async fn cmd_login(args: LoginArgs) -> Result<ExitCode, CliError> {
    use portbay_lib::auth::{self, PollOutcome, CLOUD_BASE_URL};
    use portbay_lib::entitlements;

    let method = if args.email.is_some() {
        "email"
    } else {
        "github"
    };
    let init = auth::init(CLOUD_BASE_URL, method, args.email.as_deref())
        .await
        .map_err(CliError::Other)?;

    match (&init.authorize_url, &args.email) {
        (Some(url), _) => {
            println!("Opening your browser to sign in. If it doesn't open, visit:\n  {url}\n");
            let _ = std::process::Command::new("open").arg(url).status();
        }
        (None, Some(email)) => {
            println!("We emailed a sign-in link to {email}. Open it to continue…")
        }
        (None, None) => {}
    }
    println!("Waiting for you to finish signing in (Ctrl-C to cancel)…");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    loop {
        if std::time::Instant::now() > deadline {
            return Err(CliError::Other(
                "sign-in timed out — run `portbay login` again".into(),
            ));
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        match auth::poll(CLOUD_BASE_URL, &init.poll_token)
            .await
            .map_err(CliError::Other)?
        {
            PollOutcome::Pending => continue,
            PollOutcome::Expired => {
                return Err(CliError::Other("sign-in link expired — try again".into()))
            }
            PollOutcome::Ready(session) => {
                auth::store_session(&session).map_err(CliError::Other)?;
                let eff = entitlements::refresh(CLOUD_BASE_URL, &session.access_token)
                    .await
                    .map_err(CliError::Other)?;
                let who = eff
                    .account
                    .as_ref()
                    .map(|a| a.login.clone())
                    .unwrap_or_default();
                println!("\u{2713} Signed in as {who} — {} tier.", eff.tier);
                return Ok(ExitCode::SUCCESS);
            }
        }
    }
}

/// `portbay license` — print the cached effective entitlement.
fn cmd_license() -> Result<ExitCode, CliError> {
    use portbay_lib::entitlements;
    let eff = entitlements::current();
    match &eff.account {
        Some(a) => println!("Account: {}", a.login),
        None => println!("Account: not signed in (anonymous)"),
    }
    let cap = eff
        .entitlements
        .max_projects
        .map(|n| n.to_string())
        .unwrap_or_else(|| "unlimited".into());
    println!("Tier:     {}", eff.tier);
    println!("Projects: {cap}");
    println!(
        "Sync:     {}",
        if eff.entitlements.sync { "yes" } else { "no" }
    );
    println!("Mail:     {}", eff.entitlements.mail);
    if eff.tier != "pro" {
        println!("\nUpgrade: support PortBay with a donation or a merged PR to unlock Pro.");
    }
    Ok(ExitCode::SUCCESS)
}

/// `portbay logout` — clear the saved session and cached entitlement.
fn cmd_logout() -> Result<ExitCode, CliError> {
    use portbay_lib::{auth, entitlements};
    let _ = auth::clear_session();
    let _ = entitlements::clear_cache();
    println!("Signed out.");
    Ok(ExitCode::SUCCESS)
}

fn cmd_completions(shell: Shell) -> Result<ExitCode, CliError> {
    let mut command = Cli::command();
    let mut stdout = std::io::stdout();
    generate(shell, &mut command, "portbay", &mut stdout);
    Ok(ExitCode::SUCCESS)
}

async fn cmd_complete_projects(cli: &Cli, running_only: bool) -> Result<ExitCode, CliError> {
    let ctx = CliContext::from_args(cli)?;
    let reg = ctx.load_registry()?;
    let running = if running_only {
        fetch_pc_state(&ctx).await
    } else {
        None
    };
    let mut ids: Vec<String> = reg
        .list_projects()
        .iter()
        .filter(|p| {
            if !running_only {
                return true;
            }
            running
                .as_ref()
                .and_then(|m| m.get(p.id.as_str()))
                .map(|p| p.is_running)
                .unwrap_or(false)
        })
        .map(|p| p.id.as_str().to_owned())
        .collect();
    ids.sort();
    println!("{}", ids.join(" "));
    Ok(ExitCode::SUCCESS)
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
        store::load_or_default(&self.registry_path, DEFAULT_DOMAIN_SUFFIX)
            .map_err(CliError::Registry)
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
        ctx.term
            .write_line(&format!(
                "{} No projects registered. {} `portbay add <path>`",
                style("·").dim(),
                style("Add one with").dim()
            ))
            .ok();
        return Ok(ExitCode::SUCCESS);
    }

    // Map of statuses keyed by project id. Empty if daemon is down.
    let pc_state = fetch_pc_state(ctx).await;

    let id_w = projects
        .iter()
        .map(|p| p.id.as_str().len())
        .max()
        .unwrap_or(2);
    let host_w = projects.iter().map(|p| p.hostname.len()).max().unwrap_or(2);
    for p in projects {
        let status = pc_state
            .as_ref()
            .and_then(|m| m.get(p.id.as_str()))
            .map(|proc| proc.portbay_status());
        let badge = status_badge(status);
        ctx.term
            .write_line(&format!(
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
            .map(|p| (p, pc_state.as_ref().and_then(|m| m.get(p.id.as_str()))))
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
        ctx.term
            .write_line(&format!(
                "{} Daemon not reachable on port {}. Status reflects registry only.",
                style("!").yellow(),
                ctx.pc_port
            ))
            .ok();
    }

    for (p, proc) in entries {
        let status = proc.map(|p| p.portbay_status());
        ctx.term
            .write_line(&format!(
                "  {} {}  {}",
                status_badge(status),
                style(p.id.as_str()).bold(),
                style(&p.hostname).dim(),
            ))
            .ok();
        if let Some(proc) = proc {
            ctx.term
                .write_line(&format!(
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

/// Shared project-cap gate for the CLI add paths (anonymous 3 / free 6 / pro
/// unlimited). Reads the same cached entitlement the GUI does, so the limit is
/// consistent across both surfaces.
fn enforce_project_cap(current_count: usize) -> Result<(), CliError> {
    portbay_lib::entitlements::check_can_add(current_count).map_err(|cap| {
        CliError::BadInput(format!(
            "you've reached your {cap}-project limit. Sign in with `portbay login` for 6 projects, \
             or support PortBay (donate / merged PR) for unlimited Pro projects."
        ))
    })
}

async fn cmd_add(ctx: &CliContext, args: AddArgs) -> Result<ExitCode, CliError> {
    let canonical = args
        .path
        .canonicalize()
        .map_err(|e| CliError::BadInput(format!("path: {e}")))?;

    // Auto-detect a committed `.portbay.json` before falling back to the
    // standard flow. The file's contents win for every field; CLI `--`
    // overrides (--name, --id) still apply.
    let portfile_path = canonical.join(portbay_lib::portfile::PORTBAY_FILE_NAME);
    if portfile_path.is_file() {
        return cmd_add_from_portfile(ctx, &canonical, &portfile_path, args).await;
    }

    let mut reg = ctx.load_registry()?;
    enforce_project_cap(reg.projects.len())?;

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

    // Prefer the project's own version-manager files, then fall back to the
    // language default from the Languages panel. This keeps the CLI aligned
    // with the GUI add flow and avoids global runtime conflicts.
    let kind: ProjectType = args.kind.into();
    let runtime =
        portbay_lib::project_runtime::detect(&canonical).or_else(|| reg.runtimes.default_for(kind));
    let php_version = if kind == ProjectType::Php {
        args.php_version
            .clone()
            .or_else(|| runtime.as_ref().map(|r| r.version.clone()))
    } else {
        None
    };
    let document_root = if kind == ProjectType::Php {
        args.document_root.filter(|s| !s.trim().is_empty())
    } else {
        None
    };
    let has_start_command = args.start_command.is_some();
    let web_server = if kind == ProjectType::Php && !has_start_command {
        Some(args.web_server.into())
    } else {
        None
    };

    let project = Project {
        id,
        name,
        path: canonical,
        kind,
        start_command: args.start_command,
        port: args.port,
        extra_ports: vec![],
        hostname,
        https: args.https,
        services: match kind {
            ProjectType::Flutter | ProjectType::Xcode | ProjectType::Android => vec![],
            ProjectType::Php if has_start_command => vec!["caddy".into()],
            ProjectType::Php => vec!["caddy".into(), "php-fpm".into()],
            _ if args.https => vec!["caddy".into()],
            _ => vec![],
        },
        env: Default::default(),
        readiness,
        auto_start: args.auto_start,
        tags: vec![],
        document_root,
        php_version,
        web_server,
        mobile_run: None,
        runtime,
        workspace: None,
        cors: None,
        sandbox: None,
        domain: None,
    };

    reg.add_project(project.clone())
        .map_err(CliError::Registry)?;
    if let Some(runtime) = &project.runtime {
        if let Err(err) = portbay_lib::project_runtime::ensure_marker_files(&project.path, runtime)
        {
            eprintln!(
                "warning: failed to write project runtime marker files for {}: {err}",
                project.id
            );
        }
    }
    ctx.save_registry(&reg)?;

    // Best-effort hosts write. Permission-denied is reported as a hint, not
    // an error — the project is registered either way, and the user can
    // catch up with `sudo portbay hosts add <hostname>`.
    let hosts_outcome = add_host_best_effort(ctx, &project.hostname, Ipv4Addr::LOCALHOST);

    if ctx.json {
        let warnings = hosts_warnings(&hosts_outcome);
        println!(
            "{}",
            serde_json::json!({
                "project": project,
                "warnings": warnings,
            })
        );
    } else {
        ctx.term
            .write_line(&format!(
                "{} {} registered as {}",
                style("✓").green(),
                project.id.as_str(),
                style(&project.hostname).dim()
            ))
            .ok();
        if project.https {
            ctx.term
                .write_line(&format!(
                    "  {} cert issuance + Caddy wiring will happen when the daemon picks it up.",
                    style("·").dim()
                ))
                .ok();
        }
        emit_hosts_hint(&ctx.term, &project.hostname, &hosts_outcome, true);
    }
    Ok(ExitCode::SUCCESS)
}

/// Path taken by `cmd_add` when the picked folder already contains a
/// `.portbay.json`. The file's values win; CLI `--id` and `--name`
/// overrides still apply. Any secrets the file lists are inserted
/// into the project's env as empty placeholders — the user fills them
/// in via the GUI or `--secret KEY=VALUE` flags (latter is a future
/// extension; today's surface is the minimal one).
async fn cmd_add_from_portfile(
    ctx: &CliContext,
    canonical: &Path,
    portfile_path: &Path,
    args: AddArgs,
) -> Result<ExitCode, CliError> {
    use std::collections::BTreeMap;

    let bytes = std::fs::read(portfile_path)
        .map_err(|e| CliError::BadInput(format!("read {}: {e}", portfile_path.display())))?;
    let file = portbay_lib::portfile::from_json_bytes(&bytes)
        .map_err(|e| CliError::BadInput(format!("parse .portbay.json: {e}")))?;

    let dir_name = canonical
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_string();
    let id_str = args.id.unwrap_or_else(|| slugify(&dir_name));
    let id = ProjectId::new(id_str.clone());

    // CLI cannot prompt interactively for secret values. We materialise
    // the project with every listed secret set to an empty placeholder;
    // the user fills them in later via the GUI's detail panel or by
    // editing the registry JSON directly. The names alone are useful —
    // running `portbay logs <id>` will show `[KEY] not set` warnings
    // from frameworks expecting them.
    let secrets_for_materialise: BTreeMap<String, String> = file
        .secrets
        .iter()
        .map(|k| (k.clone(), String::new()))
        .collect();

    let plan = portbay_lib::portfile::ImportPlan::new(file.clone(), canonical.to_path_buf());
    let project = portbay_lib::portfile::materialise_project(&plan, id, &secrets_for_materialise)
        .map_err(|e| CliError::BadInput(format!("materialise: {e}")))?;

    let mut reg = ctx.load_registry()?;
    enforce_project_cap(reg.projects.len())?;
    reg.add_project(project.clone())
        .map_err(CliError::Registry)?;
    ctx.save_registry(&reg)?;

    // Best-effort hosts add. Same UX as cmd_add's main path.
    let hosts_outcome = add_host_best_effort(ctx, &project.hostname, Ipv4Addr::LOCALHOST);

    if ctx.json {
        let warnings = hosts_warnings(&hosts_outcome);
        println!(
            "{}",
            serde_json::json!({
                "project": project,
                "source": ".portbay.json",
                "secrets_pending": file.secrets,
                "warnings": warnings,
            })
        );
    } else {
        ctx.term
            .write_line(&format!(
                "{} {} imported from {} as {}",
                style("✓").green(),
                id_str,
                style(".portbay.json").dim(),
                style(&project.hostname).dim(),
            ))
            .ok();
        if !file.secrets.is_empty() {
            ctx.term
                .write_line(&format!(
                    "  {} {} secret(s) not set: {}",
                    style("·").yellow(),
                    file.secrets.len(),
                    file.secrets.join(", "),
                ))
                .ok();
            ctx.term
                .write_line(&format!(
                    "  {} fill them via the GUI's detail panel before starting.",
                    style("·").dim(),
                ))
                .ok();
        }
        for w in &hosts_warnings(&hosts_outcome) {
            ctx.term
                .write_line(&format!("  {} {}", style("·").yellow(), w))
                .ok();
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn cmd_export(ctx: &CliContext, id: &str) -> Result<ExitCode, CliError> {
    let reg = ctx.load_registry()?;
    let project = reg
        .get_project(&ProjectId::new(id))
        .ok_or_else(|| CliError::BadInput(format!("project `{id}` not found")))?;
    let file = portbay_lib::portfile::export_project(project);
    let json = portbay_lib::portfile::to_json_string(&file)
        .map_err(|e| CliError::BadInput(format!("serialise: {e}")))?;
    let out_path = project.path.join(portbay_lib::portfile::PORTBAY_FILE_NAME);
    std::fs::write(&out_path, &json)
        .map_err(|e| CliError::BadInput(format!("write {}: {e}", out_path.display())))?;
    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "wrote": out_path.display().to_string(),
                "secrets_count": file.secrets.len(),
            })
        );
    } else {
        ctx.term
            .write_line(&format!(
                "{} wrote {} ({} env, {} secret name(s))",
                style("✓").green(),
                style(out_path.display().to_string()).dim(),
                file.env_template.len(),
                file.secrets.len(),
            ))
            .ok();
        ctx.term
            .write_line(&format!(
                "  {} commit this file so teammates can reproduce the setup.",
                style("·").dim(),
            ))
            .ok();
    }
    Ok(ExitCode::SUCCESS)
}

async fn cmd_remove(ctx: &CliContext, args: RemoveArgs) -> Result<ExitCode, CliError> {
    let mut reg = ctx.load_registry()?;
    let pid = ProjectId::new(args.id.clone());
    let removed = reg.remove_project(&pid).map_err(CliError::Registry)?;
    ctx.save_registry(&reg)?;

    let mut warnings: Vec<String> = Vec::new();
    let mut hosts_outcome: Option<std::result::Result<(), HostsError>> = None;

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

        // Best-effort hosts entry removal — permission-denied reported as
        // a hint, not an error. The registry change has already landed.
        hosts_outcome = Some(remove_host_best_effort(ctx, &removed.hostname));

        // Live Caddy routes are left alone — the reconciler drops orphans
        // on next daemon boot.
    }

    if ctx.json {
        let mut all_warnings = warnings.clone();
        if let Some(Err(e)) = &hosts_outcome {
            all_warnings.push(format!("hosts: {e}"));
        }
        println!(
            "{}",
            serde_json::json!({
                "removed": removed.id.as_str(),
                "warnings": all_warnings,
            })
        );
    } else {
        ctx.term
            .write_line(&format!(
                "{} {} removed.",
                style("✓").green(),
                removed.id.as_str()
            ))
            .ok();
        for w in &warnings {
            ctx.term
                .write_line(&format!("  {} {w}", style("!").yellow()))
                .ok();
        }
        if let Some(outcome) = &hosts_outcome {
            emit_hosts_hint(&ctx.term, &removed.hostname, outcome, false);
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
                println!(
                    "{}",
                    serde_json::json!({ "ok": true, "id": id, "verb": verb })
                );
            } else {
                ctx.term
                    .write_line(&format!("{} {} {}", style("✓").green(), verb, id))
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
                ctx.term
                    .write_line(&format!("{} Nothing to stop.", style("·").dim()))
                    .ok();
            }
            return Ok(ExitCode::SUCCESS);
        }
        let result = client.stop_many(&names).await.map_err(CliError::Pc)?;
        if ctx.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            ctx.term
                .write_line(&format!(
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
    let status = std::process::Command::new("open").arg(&url).status();
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
                .write_line(&format!(
                    "{} `open` exited non-zero for {url}",
                    style("!").yellow()
                ))
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
        let exists = root.exists();
        let count = std::fs::read_dir(&root).map(|d| d.count()).unwrap_or(0);
        let detail = if exists {
            format!("{} ({} entries)", root.display(), count)
        } else {
            format!("{} (not created yet)", root.display())
        };
        findings.push((
            "certs root".into(),
            if exists { Verdict::Ok } else { Verdict::Warn },
            detail,
        ));
    }

    // /etc/hosts managed entries
    match HostsManager::system().list_managed() {
        Ok(entries) => {
            let reg = ctx.load_registry().ok();
            let expected: std::collections::HashSet<String> = reg
                .as_ref()
                .map(|r| {
                    r.list_projects()
                        .iter()
                        .map(|p| p.hostname.clone())
                        .collect()
                })
                .unwrap_or_default();
            let present: std::collections::HashSet<String> =
                entries.iter().map(|e| e.hostname.clone()).collect();
            let missing: Vec<&String> = expected.difference(&present).collect();
            let orphan: Vec<&String> = present.difference(&expected).collect();
            let verdict = if missing.is_empty() && orphan.is_empty() {
                Verdict::Ok
            } else {
                Verdict::Warn
            };
            let detail = if missing.is_empty() && orphan.is_empty() {
                format!("{} entries, all match registry", entries.len())
            } else {
                format!(
                    "{} entries (missing: {}, orphan: {}). Run `sudo portbay hosts reconcile` to fix.",
                    entries.len(),
                    missing.len(),
                    orphan.len()
                )
            };
            findings.push(("/etc/hosts".into(), verdict, detail));
        }
        Err(e) => findings.push(("/etc/hosts".into(), Verdict::Warn, e.to_string())),
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

async fn cmd_hosts(ctx: &CliContext, sub: HostsCmd) -> Result<ExitCode, CliError> {
    let mgr = HostsManager::system();
    let helper = HostsHelperClient::system();
    match sub {
        HostsCmd::List => {
            let entries = match helper.list() {
                Ok(entries) => entries
                    .into_iter()
                    .map(|entry| portbay_lib::hosts::HostsEntry {
                        ip: entry.ip,
                        hostname: entry.hostname,
                    })
                    .collect(),
                Err(_) => mgr.list_managed().map_err(CliError::Hosts)?,
            };
            if ctx.json {
                let out: Vec<_> = entries
                    .iter()
                    .map(|e| serde_json::json!({ "ip": e.ip.to_string(), "hostname": e.hostname }))
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else if entries.is_empty() {
                ctx.term
                    .write_line(&format!("{} No managed hosts entries.", style("·").dim()))
                    .ok();
            } else {
                for e in &entries {
                    ctx.term
                        .write_line(&format!("  {}\t{}", style(e.ip).green(), e.hostname))
                        .ok();
                }
            }
        }
        HostsCmd::Add { hostname, ip } => {
            add_host_best_effort(ctx, &hostname, ip).map_err(CliError::Hosts)?;
            cli_say(ctx, &format!("added {hostname} → {ip}"));
        }
        HostsCmd::Remove { hostname } => {
            remove_host_best_effort(ctx, &hostname).map_err(CliError::Hosts)?;
            cli_say(ctx, &format!("removed {hostname}"));
        }
        HostsCmd::Clear => {
            match helper.clear() {
                Ok(()) => {}
                Err(_) => mgr.clear().map_err(CliError::Hosts)?,
            }
            cli_say(ctx, "cleared all PortBay-managed hosts entries");
        }
        HostsCmd::Reconcile => {
            let reg = ctx.load_registry()?;
            let pairs: Vec<(String, Ipv4Addr)> = reg
                .list_projects()
                .iter()
                .map(|p| (p.hostname.clone(), Ipv4Addr::LOCALHOST))
                .collect();
            let n = pairs.len();
            match helper.replace_all(pairs.clone(), &reg.domain_suffix) {
                Ok(()) => {}
                Err(_) => mgr.replace_all(pairs).map_err(CliError::Hosts)?,
            }
            cli_say(ctx, &format!("reconciled {n} entry(ies) from registry"));
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `portbay detect <path>` — single-project or monorepo app detection.
///
/// Without `--apps`: detect the framework at `path` and print the suggested
/// registration defaults (kind, id, hostname, port, start command).
///
/// With `--apps`: treat `path` as a JS monorepo root and list the individual
/// runnable apps inside it. Prints "not a monorepo" when no workspace layout
/// is found (the caller should retry without `--apps`).
async fn cmd_detect(ctx: &CliContext, path: &str, apps: bool) -> Result<ExitCode, CliError> {
    let canonical = PathBuf::from(path)
        .canonicalize()
        .map_err(|e| CliError::BadInput(format!("path: {e}")))?;

    if apps {
        // --- Monorepo / workspace app detection ---
        let Some(layout) = workspace_detect::detect(&canonical) else {
            if ctx.json {
                println!("{}", serde_json::json!({ "monorepo": false, "apps": [] }));
            } else {
                ctx.term
                    .write_line(&format!(
                        "{} Not a monorepo (no recognised workspace layout).",
                        style("·").dim()
                    ))
                    .ok();
                ctx.term
                    .write_line(&format!(
                        "  {} Try `portbay detect {}` without --apps for single-project detection.",
                        style("hint:").dim(),
                        path
                    ))
                    .ok();
            }
            return Ok(ExitCode::SUCCESS);
        };

        let reg = ctx.load_registry()?;
        let suffix = &reg.domain_suffix;

        // Build the per-app summary list, mirroring detect_workspace_apps in ops.rs.
        let detected_apps: Vec<serde_json::Value> = layout
            .packages
            .iter()
            .map(|pkg| {
                let leaf = std::path::Path::new(&pkg.rel_dir)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&pkg.rel_dir);
                let id = slugify(leaf);
                let detected = detect_kind(&pkg.abs_dir);
                let start_command = detected
                    .start_command
                    .map(|_| cli_standalone_dev_command(layout.tool));
                serde_json::json!({
                    "package":                pkg.name,
                    "rel_dir":                pkg.rel_dir,
                    "path":                   pkg.abs_dir.display().to_string(),
                    "kind":                   format!("{:?}", detected.kind).to_lowercase(),
                    "suggested_id":           id.clone(),
                    "suggested_hostname":     format!("{id}.{suffix}"),
                    "suggested_port":         detected.port,
                    "suggested_start_command": start_command,
                })
            })
            .collect();

        if ctx.json {
            println!(
                "{}",
                serde_json::json!({
                    "monorepo": true,
                    "root":     canonical.display().to_string(),
                    "tool":     format!("{:?}", layout.tool).to_lowercase(),
                    "apps":     detected_apps,
                })
            );
        } else {
            ctx.term
                .write_line(&format!(
                    "{} Monorepo detected ({} app(s), tool: {:?})",
                    style("✓").green(),
                    detected_apps.len(),
                    layout.tool,
                ))
                .ok();
            for app in &detected_apps {
                ctx.term
                    .write_line(&format!(
                        "  {id}  {host}  {kind}{cmd}",
                        id   = style(app["suggested_id"].as_str().unwrap_or("")).bold(),
                        host = style(app["suggested_hostname"].as_str().unwrap_or("")).dim(),
                        kind = style(app["kind"].as_str().unwrap_or("")).dim(),
                        cmd  = app["suggested_start_command"]
                            .as_str()
                            .map(|c| format!("  ({})", style(c).dim()))
                            .unwrap_or_default(),
                    ))
                    .ok();
            }
        }
    } else {
        // --- Single-project detection ---
        let dir_name = canonical
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("project")
            .to_string();
        let reg = ctx.load_registry()?;
        let id = slugify(&dir_name);
        let detected = detect_kind(&canonical);
        let hostname = format!("{id}.{}", reg.domain_suffix);

        if ctx.json {
            println!(
                "{}",
                serde_json::json!({
                    "kind":                    format!("{:?}", detected.kind).to_lowercase(),
                    "suggested_id":            id,
                    "suggested_name":          dir_name,
                    "suggested_hostname":      hostname,
                    "suggested_port":          detected.port,
                    "suggested_start_command": detected.start_command,
                    "suggested_document_root": detected.document_root,
                    "suggested_php_version":   detected.php_version,
                })
            );
        } else {
            ctx.term
                .write_line(&format!(
                    "{} {} detected as {}",
                    style("✓").green(),
                    style(&id).bold(),
                    style(format!("{:?}", detected.kind).to_lowercase()).dim(),
                ))
                .ok();
            ctx.term
                .write_line(&format!(
                    "  hostname:  {}",
                    style(&hostname).dim()
                ))
                .ok();
            if let Some(port) = detected.port {
                ctx.term
                    .write_line(&format!("  port:      {}", style(port).dim()))
                    .ok();
            }
            if let Some(cmd) = &detected.start_command {
                ctx.term
                    .write_line(&format!("  start:     {}", style(cmd).dim()))
                    .ok();
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}

/// Standalone dev command for a single app by its package manager, mirroring
/// `commands::projects::standalone_dev_command` and `mcp::ops::standalone_dev_command`.
fn cli_standalone_dev_command(tool: WorkspaceTool) -> String {
    match tool {
        WorkspaceTool::Pnpm | WorkspaceTool::Turbo => "pnpm dev".into(),
        WorkspaceTool::Npm => "npm run dev".into(),
        WorkspaceTool::Yarn => "yarn dev".into(),
        WorkspaceTool::Bun => "bun run dev".into(),
    }
}

async fn cmd_group(ctx: &CliContext, sub: GroupCmd) -> Result<ExitCode, CliError> {
    // Group operations follow the same pattern as all other CliContext commands:
    // registry CRUD via ctx.load_registry/save_registry, lifecycle via ctx.pc().
    // The MCP server does the same through McpContext (same underlying calls).
    match sub {
        GroupCmd::List => {
            let reg = ctx.load_registry()?;
            let known: std::collections::HashSet<&str> =
                reg.list_projects().iter().map(|p| p.id.as_str()).collect();
            let groups: Vec<serde_json::Value> = reg
                .list_groups()
                .iter()
                .map(|g| {
                    let project_ids: Vec<&str> =
                        g.projects.iter().map(|id| id.as_str()).collect();
                    let known_ids: Vec<&str> = project_ids
                        .iter()
                        .filter(|id| known.contains(*id))
                        .copied()
                        .collect();
                    serde_json::json!({
                        "id": g.id,
                        "name": g.name,
                        "project_ids": project_ids,
                        "known_ids": known_ids,
                        "member_count": project_ids.len(),
                    })
                })
                .collect();

            if ctx.json {
                println!("{}", serde_json::to_string_pretty(&groups)?);
            } else if groups.is_empty() {
                ctx.term
                    .write_line(&format!(
                        "{} No groups. Create one with `portbay group create <name>`.",
                        style("·").dim()
                    ))
                    .ok();
            } else {
                for g in &groups {
                    ctx.term
                        .write_line(&format!(
                            "  {id}  {name}  ({n} member(s))",
                            id = style(g["id"].as_str().unwrap_or("")).bold(),
                            name = style(g["name"].as_str().unwrap_or("")).dim(),
                            n = g["member_count"].as_u64().unwrap_or(0),
                        ))
                        .ok();
                }
            }
        }

        GroupCmd::Create(args) => {
            let name = args.name.trim().to_string();
            if name.is_empty() {
                return Err(CliError::BadInput("group name cannot be empty".into()));
            }
            let id = args
                .id
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| slugify(&name));
            if id.is_empty() {
                return Err(CliError::BadInput(
                    "group id couldn't be derived from name".into(),
                ));
            }
            let mut reg = ctx.load_registry()?;
            let group = Group {
                id: id.clone(),
                name: name.clone(),
                projects: args.projects.into_iter().map(ProjectId::new).collect(),
            };
            reg.add_group(group.clone()).map_err(CliError::Registry)?;
            ctx.save_registry(&reg)?;

            if ctx.json {
                let known: std::collections::HashSet<&str> =
                    reg.list_projects().iter().map(|p| p.id.as_str()).collect();
                let project_ids: Vec<&str> =
                    group.projects.iter().map(|id| id.as_str()).collect();
                let known_ids: Vec<&str> = project_ids
                    .iter()
                    .filter(|id| known.contains(*id))
                    .copied()
                    .collect();
                println!(
                    "{}",
                    serde_json::json!({
                        "id": group.id,
                        "name": group.name,
                        "project_ids": project_ids,
                        "known_ids": known_ids,
                        "member_count": project_ids.len(),
                    })
                );
            } else {
                ctx.term
                    .write_line(&format!(
                        "{} group {} created ({} member(s)).",
                        style("\u{2713}").green(),
                        style(&id).bold(),
                        group.projects.len(),
                    ))
                    .ok();
            }
        }

        GroupCmd::Update(args) => {
            let mut reg = ctx.load_registry()?;
            let current = reg
                .get_group(&args.id)
                .ok_or_else(|| CliError::BadInput(format!("group `{}` not found", args.id)))?
                .clone();
            let next = Group {
                id: current.id.clone(),
                name: args
                    .name
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(current.name),
                projects: if args.projects.is_empty() {
                    current.projects
                } else {
                    args.projects.into_iter().map(ProjectId::new).collect()
                },
            };
            reg.update_group(next.clone())
                .map_err(CliError::Registry)?;
            ctx.save_registry(&reg)?;

            if ctx.json {
                let known: std::collections::HashSet<&str> =
                    reg.list_projects().iter().map(|p| p.id.as_str()).collect();
                let project_ids: Vec<&str> =
                    next.projects.iter().map(|id| id.as_str()).collect();
                let known_ids: Vec<&str> = project_ids
                    .iter()
                    .filter(|id| known.contains(*id))
                    .copied()
                    .collect();
                println!(
                    "{}",
                    serde_json::json!({
                        "id": next.id,
                        "name": next.name,
                        "project_ids": project_ids,
                        "known_ids": known_ids,
                        "member_count": project_ids.len(),
                    })
                );
            } else {
                ctx.term
                    .write_line(&format!(
                        "{} group {} updated ({} member(s)).",
                        style("\u{2713}").green(),
                        style(&next.id).bold(),
                        next.projects.len(),
                    ))
                    .ok();
            }
        }

        GroupCmd::Remove { id } => {
            let mut reg = ctx.load_registry()?;
            reg.remove_group(&id).map_err(CliError::Registry)?;
            ctx.save_registry(&reg)?;
            if ctx.json {
                println!("{}", serde_json::json!({ "ok": true, "removed": id }));
            } else {
                ctx.term
                    .write_line(&format!(
                        "{} group {} removed.",
                        style("\u{2713}").green(),
                        style(&id).bold(),
                    ))
                    .ok();
            }
        }

        GroupCmd::Start { id } => {
            let r = cmd_group_fanout(ctx, &id, GroupFanoutOp::Start).await?;
            print_group_fanout_result(ctx, &r)?;
        }

        GroupCmd::Stop { id } => {
            let r = cmd_group_fanout(ctx, &id, GroupFanoutOp::Stop).await?;
            print_group_fanout_result(ctx, &r)?;
        }

        GroupCmd::Restart { id } => {
            let r = cmd_group_fanout(ctx, &id, GroupFanoutOp::Restart).await?;
            print_group_fanout_result(ctx, &r)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Local result type for group fanout — mirrors the MCP layer's GroupFanoutResult
/// so JSON output is identical across both surfaces.
#[derive(serde::Serialize)]
struct CliFanoutResult {
    group_id: String,
    succeeded: usize,
    failed: usize,
    results: Vec<CliFanoutMember>,
}

#[derive(serde::Serialize)]
struct CliFanoutMember {
    project_id: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Clone, Copy)]
enum GroupFanoutOp {
    Start,
    Stop,
    Restart,
}

/// Fan out a start/stop/restart over every member of a group, mirroring the
/// logic in `McpContext::fanout_group`. The daemon must be reachable.
async fn cmd_group_fanout(
    ctx: &CliContext,
    group_id: &str,
    op: GroupFanoutOp,
) -> Result<CliFanoutResult, CliError> {
    let client = ctx.pc();
    // Check daemon reachability — surface a Pc error if it's down.
    let is_live = client.live().await.unwrap_or(false);
    if !is_live {
        return Err(CliError::Other(
            "daemon not reachable — open PortBay.app to start the daemon".into(),
        ));
    }

    let reg = ctx.load_registry()?;
    let group = reg
        .get_group(group_id)
        .ok_or_else(|| CliError::BadInput(format!("group `{group_id}` not found")))?
        .clone();

    let projects_by_id: std::collections::HashMap<&str, &Project> = reg
        .list_projects()
        .iter()
        .map(|p| (p.id.as_str(), p))
        .collect();

    let mut result = CliFanoutResult {
        group_id: group_id.to_string(),
        succeeded: 0,
        failed: 0,
        results: Vec::with_capacity(group.projects.len()),
    };

    for pid in &group.projects {
        let id_str = pid.as_str().to_string();
        let Some(project) = projects_by_id.get(id_str.as_str()) else {
            result.failed += 1;
            result.results.push(CliFanoutMember {
                project_id: id_str,
                ok: false,
                error: Some("project not in registry".into()),
            });
            continue;
        };
        let process_id = project.process_compose_id();
        if process_id.is_none() {
            // No managed process (e.g. mobile/Xcode) — count as ok.
            result.succeeded += 1;
            result.results.push(CliFanoutMember {
                project_id: id_str,
                ok: true,
                error: None,
            });
            continue;
        }
        let process_id = process_id.expect("checked above");
        let res = match op {
            GroupFanoutOp::Start => client.start(&process_id).await,
            GroupFanoutOp::Stop => client.stop(&process_id).await,
            GroupFanoutOp::Restart => client.restart(&process_id).await,
        };
        match res {
            Ok(()) => {
                result.succeeded += 1;
                result.results.push(CliFanoutMember {
                    project_id: id_str,
                    ok: true,
                    error: None,
                });
            }
            Err(e) => {
                result.failed += 1;
                result.results.push(CliFanoutMember {
                    project_id: id_str,
                    ok: false,
                    error: Some(e.to_string()),
                });
            }
        }
    }

    Ok(result)
}

fn print_group_fanout_result(ctx: &CliContext, r: &CliFanoutResult) -> Result<(), CliError> {
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(r)?);
        return Ok(());
    }
    ctx.term
        .write_line(&format!(
            "{} group {}: {} ok, {} failed.",
            if r.failed == 0 {
                style("\u{2713}").green()
            } else {
                style("!").yellow()
            },
            style(&r.group_id).bold(),
            r.succeeded,
            r.failed,
        ))
        .ok();
    for m in &r.results {
        if !m.ok {
            ctx.term
                .write_line(&format!(
                    "    {} {}  {}",
                    style("\u{2717}").red(),
                    style(&m.project_id).bold(),
                    style(m.error.as_deref().unwrap_or("unknown error")).dim(),
                ))
                .ok();
        }
    }
    Ok(())
}

fn add_host_best_effort(
    ctx: &CliContext,
    hostname: &str,
    ip: Ipv4Addr,
) -> std::result::Result<(), HostsError> {
    let suffix = ctx
        .load_registry()
        .map(|reg| reg.domain_suffix)
        .unwrap_or_else(|_| DEFAULT_DOMAIN_SUFFIX.into());
    if HostsHelperClient::system()
        .add(hostname, ip, &suffix)
        .is_ok()
    {
        return Ok(());
    }
    HostsManager::system().add(hostname, ip)
}

fn remove_host_best_effort(
    ctx: &CliContext,
    hostname: &str,
) -> std::result::Result<(), HostsError> {
    let suffix = ctx
        .load_registry()
        .map(|reg| reg.domain_suffix)
        .unwrap_or_else(|_| DEFAULT_DOMAIN_SUFFIX.into());
    if HostsHelperClient::system()
        .remove(hostname, &suffix)
        .is_ok()
    {
        return Ok(());
    }
    HostsManager::system().remove(hostname)
}

fn cli_say(ctx: &CliContext, msg: &str) {
    if ctx.json {
        println!("{}", serde_json::json!({ "ok": true, "message": msg }));
    } else {
        ctx.term
            .write_line(&format!("{} {msg}", style("✓").green()))
            .ok();
    }
}

fn hosts_warnings(outcome: &std::result::Result<(), HostsError>) -> Vec<String> {
    match outcome {
        Ok(()) => vec![],
        Err(e) => vec![format!("hosts: {e}")],
    }
}

/// Print a friendly note explaining why /etc/hosts couldn't be updated and
/// what the user should do — only when there's something to say.
fn emit_hosts_hint(
    term: &Term,
    hostname: &str,
    outcome: &std::result::Result<(), HostsError>,
    is_add: bool,
) {
    match outcome {
        Ok(()) => { /* silent — hosts is in sync */ }
        Err(HostsError::PermissionDenied { .. }) => {
            let cmd = if is_add { "add" } else { "remove" };
            let _ = term.write_line(&format!(
                "  {} couldn't update /etc/hosts (permission denied). Run: {}",
                style("!").yellow(),
                style(format!("sudo portbay hosts {cmd} {hostname}"))
                    .cyan()
                    .underlined()
            ));
        }
        Err(other) => {
            let _ = term.write_line(&format!(
                "  {} hosts file update failed: {other}",
                style("!").yellow()
            ));
        }
    }
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
    Some(processes.into_iter().map(|p| (p.name.clone(), p)).collect())
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

// Shared slugifier — same ids as the GUI's project/group commands.
use portbay_lib::util::slugify;

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
    Hosts(HostsError),
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
            CliError::Hosts(HostsError::PermissionDenied { .. }) => 6,
            CliError::Registry(_) | CliError::Json(_) | CliError::Other(_) | CliError::Hosts(_) => {
                1
            }
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
            CliError::Hosts(e) => write!(f, "hosts: {e}"),
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
    match e {
        CliError::Pc(_) => {
            let _ = term.write_line(&format!(
                "  {} The daemon may not be running. Start PortBay.app, or pass --pc-port if it's on a non-default port.",
                style("hint:").dim()
            ));
        }
        CliError::Hosts(HostsError::PermissionDenied { .. }) => {
            let _ = term.write_line(&format!(
                "  {} Re-run with sudo. (Future PortBay versions will install a privileged helper so this prompt is one-time.)",
                style("hint:").dim()
            ));
        }
        _ => {}
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parses_list() {
        let cli = Cli::try_parse_from(["portbay", "list"]).unwrap();
        assert!(matches!(cli.cmd, Some(Cmd::List)));
        assert!(!cli.json);
    }

    #[test]
    fn cli_parses_add_with_defaults() {
        let cli = Cli::try_parse_from(["portbay", "add", "/tmp/x"]).unwrap();
        let Some(Cmd::Add(args)) = cli.cmd else {
            panic!("expected Add")
        };
        assert_eq!(args.path, PathBuf::from("/tmp/x"));
        assert!(args.https);
        assert!(matches!(args.kind, CliProjectType::Custom));
        assert!(matches!(args.web_server, CliWebServer::Caddy));
    }

    #[test]
    fn cli_parses_php_web_server_flag() {
        let cli = Cli::try_parse_from([
            "portbay",
            "add",
            "/tmp/x",
            "--kind",
            "php",
            "--web-server",
            "nginx",
        ])
        .unwrap();
        let Some(Cmd::Add(args)) = cli.cmd else {
            panic!("expected Add")
        };
        assert!(matches!(args.web_server, CliWebServer::Nginx));
    }

    #[test]
    fn cli_parses_stop_all() {
        let cli = Cli::try_parse_from(["portbay", "stop", "--all"]).unwrap();
        let Some(Cmd::Stop(args)) = cli.cmd else {
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
        let cli = Cli::try_parse_from(["portbay", "--pc-port", "9000", "status"]).unwrap();
        assert_eq!(cli.pc_port, Some(9000));
    }

    #[test]
    fn cli_project_type_round_trip() {
        let t: ProjectType = CliProjectType::Next.into();
        assert!(matches!(t, ProjectType::Next));
    }

    #[test]
    fn cli_parses_group_list() {
        let cli = Cli::try_parse_from(["portbay", "group", "list"]).unwrap();
        assert!(matches!(cli.cmd, Some(Cmd::Group(GroupCmd::List))));
    }

    #[test]
    fn cli_parses_group_create_with_members() {
        let cli = Cli::try_parse_from([
            "portbay",
            "group",
            "create",
            "Backend",
            "--project",
            "api",
            "--project",
            "worker",
        ])
        .unwrap();
        let Some(Cmd::Group(GroupCmd::Create(args))) = cli.cmd else {
            panic!("expected Group::Create")
        };
        assert_eq!(args.name, "Backend");
        assert_eq!(args.projects, vec!["api", "worker"]);
        assert!(args.id.is_none());
    }

    #[test]
    fn cli_parses_group_create_with_explicit_id() {
        let cli =
            Cli::try_parse_from(["portbay", "group", "create", "Dev", "--id", "dev-group"])
                .unwrap();
        let Some(Cmd::Group(GroupCmd::Create(args))) = cli.cmd else {
            panic!("expected Group::Create")
        };
        assert_eq!(args.id, Some("dev-group".into()));
    }

    #[test]
    fn cli_parses_group_update() {
        let cli = Cli::try_parse_from([
            "portbay",
            "group",
            "update",
            "my-group",
            "--name",
            "Renamed",
            "--project",
            "blog",
        ])
        .unwrap();
        let Some(Cmd::Group(GroupCmd::Update(args))) = cli.cmd else {
            panic!("expected Group::Update")
        };
        assert_eq!(args.id, "my-group");
        assert_eq!(args.name, Some("Renamed".into()));
        assert_eq!(args.projects, vec!["blog"]);
    }

    #[test]
    fn cli_parses_group_remove() {
        let cli =
            Cli::try_parse_from(["portbay", "group", "remove", "old-group"]).unwrap();
        let Some(Cmd::Group(GroupCmd::Remove { id })) = cli.cmd else {
            panic!("expected Group::Remove")
        };
        assert_eq!(id, "old-group");
    }

    #[test]
    fn cli_parses_group_start_stop_restart() {
        for verb in ["start", "stop", "restart"] {
            let cli =
                Cli::try_parse_from(["portbay", "group", verb, "g1"]).unwrap();
            assert!(matches!(
                cli.cmd,
                Some(Cmd::Group(
                    GroupCmd::Start { .. } | GroupCmd::Stop { .. } | GroupCmd::Restart { .. }
                ))
            ));
        }
    }
}
