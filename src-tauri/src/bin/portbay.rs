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
use portbay_lib::commands::projects::detect_kind;
use portbay_lib::hosts::{HostsError, HostsManager};
use portbay_lib::hosts_helper::HostsHelperClient;
use portbay_lib::process_compose::{
    PcClient, Process, ProjectStatus, DEFAULT_PORT as PC_DEFAULT_PORT,
};
use portbay_lib::registry::workspace as workspace_detect;
use portbay_lib::registry::{
    self, store, DatabaseEngine, DatabaseInstance, DatabaseInstanceId, Group, Project, ProjectId,
    ProjectType, Readiness, Registry, SandboxNetworkPolicy, WorkspaceTool,
};

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

    /// Re-verify the stored session and refresh the cached entitlement from
    /// PortBay Cloud (mirrors the app's startup resync). Use after a license
    /// change to pick up a new tier without reopening the app.
    Resync,

    /// Sign out and clear the saved session.
    Logout,

    /// Show or change anonymized diagnostics consent (usage data + crash
    /// reports). PortBay only ever sends to its own first-party endpoint, never
    /// a third-party analytics SDK, and never collects project names, paths,
    /// source code, environment variables, or logs. `on`/`off` to change it,
    /// or no argument to show the current state.
    Telemetry {
        /// `on` to enable, `off` to disable, or omit to show the current state.
        #[arg(value_enum)]
        action: Option<TelemetryAction>,
    },

    /// Manage project groups (batch start/stop/restart and organisational
    /// clusters of projects).
    #[command(subcommand)]
    Group(GroupCmd),

    /// View active public tunnels (shares) created from the PortBay app.
    /// Read-only — create or stop a share from the app.
    #[command(subcommand)]
    Tunnel(TunnelCmd),

    /// View SSH port-forward tunnels and their live state. Read-only — saving,
    /// starting, and stopping SSH tunnels is done from the PortBay app.
    #[command(subcommand)]
    Ssh(SshCmd),

    /// Manage language runtime installations (detect, set defaults, add/remove paths).
    /// Installing a new language version and editing PHP FPM/ini config are done
    /// from the PortBay app.
    #[command(subcommand)]
    Runtime(RuntimeCmd),

    /// Manage database engines + owned instances (list, create, lifecycle,
    /// link to projects). Installing an engine binary (Homebrew) and opening a
    /// DB shell are done from the PortBay app.
    #[command(subcommand)]
    Db(DbCmd),

    /// Inspect and manage local DNS: resolver status, DNS records, and the
    /// domain suffix. Starting/restarting dnsmasq and first-run resolver
    /// install are done from the PortBay app.
    #[command(subcommand)]
    Dns(DnsCmd),

    /// Manage Sandboxed Run (macOS only): confine a project under a Seatbelt
    /// profile, inspect policy, and read sandbox violations. The confined launch
    /// itself is applied by the PortBay app on its next reconcile.
    #[command(subcommand)]
    Sandbox(SandboxCmd),

    /// Inspect HTTP traffic Caddy handled: list recent requests (read from the
    /// access log) or clear the log. The live stream is in the PortBay app.
    #[command(subcommand)]
    Requests(RequestsCmd),

    /// Local-HTTPS certificates: show per-project cert metadata, or reissue a
    /// cert. Installing the mkcert CA into the system trust store is done from
    /// the PortBay app (privileged + interactive).
    #[command(subcommand)]
    Cert(CertCmd),

    /// Sidecar status (process-compose, dnsmasq resolver, hosts, …). Read-only:
    /// restarting a sidecar is done from the PortBay app, which owns the
    /// processes. Caddy/mkcert/Mailpit live state isn't visible from outside it.
    #[command(subcommand)]
    Sidecar(SidecarStatusCmd),

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

    /// Migrate sites from another local-dev tool (Laravel Herd, ServBay, MAMP)
    /// into PortBay: detect installed sources, preview their sites, or import
    /// them. The PortBay app provisions imported projects on its next reconcile.
    /// (Importing a committed `.portbay.json` is `portbay add <path>`.)
    #[command(subcommand)]
    Import(ImportCmd),

    /// Manage a project's per-project task board (the "Project Context & Task
    /// Authority" board stored as markdown under `<project>/.portbay/tasks/`).
    /// Cards share the same status vocabulary as the in-app Kanban board.
    #[cfg(feature = "tasks")]
    #[command(subcommand)]
    Tasks(TasksCmd),

    /// Regenerate a project's agent-context projections (`AGENTS.md`,
    /// `CLAUDE.md`, …) from PortBay's live, derived environment facts.
    #[cfg(feature = "tasks")]
    #[command(subcommand)]
    Context(ContextCmd),

    /// Show or update a project's session hand-off brief (`.portbay/HANDOFF.md`),
    /// the minimal continuation note the next LLM session resumes from.
    #[cfg(feature = "tasks")]
    #[command(subcommand)]
    Handoff(HandoffCmd),

    /// Show or set a project's board scratchpad (`.portbay/SCRATCHPAD.md`), a
    /// freeform notepad for loose notes and plans.
    #[cfg(feature = "tasks")]
    #[command(subcommand)]
    Scratchpad(ScratchpadCmd),
}

#[cfg(feature = "tasks")]
#[derive(Subcommand, Debug)]
enum ScratchpadCmd {
    /// Print the scratchpad.
    Show {
        /// Project id.
        project: String,
    },
    /// Overwrite the scratchpad. The body is read from `--body` or stdin.
    Set {
        /// Project id.
        project: String,
        /// New scratchpad body. Read from stdin when omitted.
        #[arg(long)]
        body: Option<String>,
    },
}

#[cfg(feature = "tasks")]
#[derive(Subcommand, Debug)]
enum ContextCmd {
    /// Regenerate the enabled projection files for a project.
    Sync {
        /// Project id.
        project: String,
        /// Don't write — just report what would change.
        #[arg(long)]
        dry_run: bool,
        /// Print the rendered managed block for each adapter.
        #[arg(long)]
        diff: bool,
        /// Adopt an adapter whose target file pre-existed with hand-written
        /// content (e.g. `--adopt claude_md`). Repeatable. `--adopt all` adopts
        /// every adapter. Without this, PortBay refuses to graft its block onto
        /// a file you already wrote and reports it as `needs-consent`.
        #[arg(long)]
        adopt: Vec<String>,
    },
    /// Print the derived ProjectContext as JSON.
    Show {
        /// Project id.
        project: String,
    },
}

#[cfg(feature = "tasks")]
#[derive(Subcommand, Debug)]
enum HandoffCmd {
    /// Print the current hand-off brief.
    Show {
        /// Project id.
        project: String,
    },
    /// Replace the hand-off (re-derives the skeleton from board state). The
    /// narrative is read from `--narrative` or stdin.
    Update {
        /// Project id.
        project: String,
        /// New "where we left off" narrative. Read from stdin when omitted.
        #[arg(long)]
        narrative: Option<String>,
    },
}

// Variants wrap clap `Args` structs (which clap's Subcommand derive can't box),
// so the size spread between them is expected and harmless for a CLI enum.
#[cfg(feature = "tasks")]
#[derive(Subcommand, Debug)]
#[allow(clippy::large_enum_variant)]
enum TasksCmd {
    /// List a project's task cards, grouped by column.
    List {
        /// Project id.
        project: String,
        /// Filter query: free text + prefixes `#label`, `priority:high`,
        /// `status:Todo`, `due:overdue`, `label:bug`.
        #[arg(long)]
        filter: Option<String>,
    },
    /// Add a new card (lands in Backlog unless `--status` is given).
    Add(TaskAddArgs),
    /// Tick (or untick with `--undone`) a card's checklist item by index.
    Check {
        /// Project id.
        project: String,
        /// Card id.
        id: String,
        /// Checklist item index.
        idx: u32,
        /// Mark the item not-done instead of done.
        #[arg(long)]
        undone: bool,
    },
    /// Post a comment on a card.
    Comment {
        /// Project id.
        project: String,
        /// Card id.
        id: String,
        /// Comment text.
        text: String,
    },
    /// Append checklist item(s) to a card (the agent's sub-step tracker).
    Checklist {
        /// Project id.
        project: String,
        /// Card id.
        id: String,
        /// One or more item descriptions.
        #[arg(required = true)]
        items: Vec<String>,
    },
    /// Archive (or `--restore`) a card — hidden from the board, recoverable.
    Archive {
        /// Project id.
        project: String,
        /// Card id.
        id: String,
        /// Restore instead of archive.
        #[arg(long)]
        restore: bool,
    },
    /// Move a card to another column (Backlog|Todo|InProgress|Blocked|Review|Done|Rejected).
    Move {
        /// Project id.
        project: String,
        /// Card id.
        id: String,
        /// Destination column.
        to: String,
    },
    /// Add (or `--remove`) a dependency: the card won't auto-dispatch to an
    /// agent until the dependency card reaches Done/Rejected.
    Block {
        /// Project id.
        project: String,
        /// Card id (the dependent card).
        id: String,
        /// Dependency card id it is blocked by.
        dep: String,
        /// Remove the dependency instead of adding it.
        #[arg(long)]
        remove: bool,
    },
    /// Mark a card Done.
    Done {
        /// Project id.
        project: String,
        /// Card id.
        id: String,
    },
    /// Delete a card.
    Rm {
        /// Project id.
        project: String,
        /// Card id.
        id: String,
    },
    /// Show one card's frontmatter + body.
    Show {
        /// Project id.
        project: String,
        /// Card id.
        id: String,
    },
    /// Quick-capture a draft card (not in the board flow until promoted).
    Capture {
        /// Project id.
        project: String,
        /// Card title.
        title: String,
    },
    /// Promote a draft out of the Inbox into the board flow.
    Promote {
        /// Project id.
        project: String,
        /// Card id.
        id: String,
    },
    /// Export the whole board to a JSON snapshot (stdout, or `--out FILE`).
    Export {
        /// Project id.
        project: String,
        /// Write to this file instead of stdout.
        #[arg(long)]
        out: Option<String>,
    },
    /// Import a Trello board JSON export into this project's board.
    ImportTrello {
        /// Project id.
        project: String,
        /// Path to the Trello export `.json` file.
        file: String,
    },
}

#[cfg(feature = "tasks")]
#[derive(Args, Debug)]
struct TaskAddArgs {
    /// Project id.
    project: String,
    /// Card title.
    title: String,
    /// Card body / description.
    #[arg(long)]
    body: Option<String>,
    /// Initial column (default Backlog).
    #[arg(long)]
    status: Option<String>,
    /// Priority: high|medium|low.
    #[arg(long)]
    priority: Option<String>,
    /// Due date (ISO-8601).
    #[arg(long)]
    due: Option<String>,
    /// Acceptance criteria.
    #[arg(long)]
    acceptance: Option<String>,
    /// Affected file/module (repeatable): `--touchpoint src/a.rs --touchpoint src/b.rs`.
    #[arg(long = "touchpoint")]
    touchpoint: Vec<String>,
    /// Label (repeatable): `--label bug --label frontend`.
    #[arg(long = "label")]
    label: Vec<String>,
    /// Numeric effort estimate.
    #[arg(long)]
    estimate: Option<f64>,
    /// Hex accent color (e.g. `#ff6b6b`).
    #[arg(long)]
    color: Option<String>,
    /// External URL (issue / PR / doc).
    #[arg(long)]
    url: Option<String>,
    /// Assign an agent to work this card: claude|codex|cursor|gemini|aider|custom.
    /// Omit to inherit the project's default agent.
    #[arg(long)]
    agent: Option<String>,
    /// Seed from a built-in template: "Implement feature"|"Fix bug"|"Write tests"|"Refactor".
    /// Explicit flags above win; the template fills the rest.
    #[arg(long)]
    template: Option<String>,
}

#[derive(Subcommand, Debug)]
enum ImportCmd {
    /// List which migration sources (Herd, ServBay, MAMP) are installed and how
    /// many sites each exposes.
    Sources,

    /// Preview a source's sites, flagged for id/path collisions with existing
    /// projects. SOURCE is one of: herd, servbay, mamp.
    Preview {
        /// Source tool to scan: herd, servbay, or mamp.
        source: String,
    },

    /// Import sites from a source into PortBay. Give the ids to import (from
    /// `import preview`), or `--all` to import every site.
    Projects {
        /// Source tool to import from: herd, servbay, or mamp.
        source: String,
        /// Suggested ids to import (omit with `--all` to import everything).
        ids: Vec<String>,
        /// Import every site the source exposes, ignoring any listed ids.
        #[arg(long)]
        all: bool,
    },
}

#[derive(Args, Debug)]
struct LoginArgs {
    /// Sign in with an email magic link instead of GitHub OAuth.
    #[arg(long)]
    email: Option<String>,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum TelemetryAction {
    /// Enable anonymized usage data + crash reports.
    On,
    /// Disable all diagnostics.
    Off,
    /// Print the current state (the default when no argument is given).
    Status,
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

    /// Skip the confirmation prompt.
    #[arg(long)]
    force: bool,
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
    Expo,
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
            CliProjectType::Expo => ProjectType::Expo,
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

#[derive(Subcommand, Debug)]
enum TunnelCmd {
    /// List all active public tunnels.
    List,

    /// Show the tunnel for one project by id.
    Status {
        /// Project id (slug) whose tunnel you want.
        id: String,
    },
}

#[derive(Subcommand, Debug)]
enum SshCmd {
    /// List all saved SSH tunnels and their live state.
    List,

    /// Show one SSH tunnel by id.
    Status {
        /// SSH tunnel id (slug).
        id: String,
    },

    /// List saved SSH connections (hosts) from the registry.
    Connections,
}

#[derive(Subcommand, Debug)]
enum RuntimeCmd {
    /// List detected language runtimes (optionally filter to one language).
    List {
        /// Filter output to one language id (e.g. php, node, python).
        #[arg(long, value_name = "LANG")]
        lang: Option<String>,
    },

    /// Set (or clear) the default version for a language.
    SetDefault {
        /// Language id (e.g. php, node).
        lang: String,
        /// Version label to set as the default (e.g. 8.3, 20).
        /// Omit (or combine with --clear) to clear the current default.
        version: Option<String>,
        /// Clear the current default for this language.
        #[arg(long)]
        clear: bool,
    },

    /// Register an existing binary as a manual runtime install.
    AddPath {
        /// Language id (e.g. php, node).
        lang: String,
        /// Absolute path to the runtime binary.
        path: String,
    },

    /// Remove a manually-added runtime install by language and version.
    RemovePath {
        /// Language id (e.g. php, node).
        lang: String,
        /// Version label as shown by `portbay runtime list` (e.g. 8.3).
        version: String,
    },
}

#[derive(Subcommand, Debug)]
enum DbCmd {
    /// List supported database engines and their install state.
    Engines,

    /// List managed database instances (with live status if the daemon is up).
    List,

    /// Show connection details (URL + framework env) for an instance.
    Info {
        /// Database instance id (slug).
        id: String,
    },

    /// Provision + register a new instance. The engine binary must already be
    /// installed (see `portbay db engines`).
    Create(DbCreateArgs),

    /// Stop + unregister an instance (optionally delete its data dir).
    Remove(DbRemoveArgs),

    /// Start an instance (requires running daemon; instance must already be in
    /// the daemon's config — true once the app has reconciled a new instance).
    Start {
        /// Database instance id (slug).
        id: String,
    },

    /// Stop an instance (requires running daemon).
    Stop {
        /// Database instance id (slug).
        id: String,
    },

    /// Restart an instance (requires running daemon).
    Restart {
        /// Database instance id (slug).
        id: String,
    },

    /// Link an instance to a project (its connection env is injected on the
    /// next reconcile).
    Link {
        /// Database instance id (slug).
        id: String,
        /// Project id (slug) to link.
        project_id: String,
    },

    /// Unlink an instance from a project.
    Unlink {
        /// Database instance id (slug).
        id: String,
        /// Project id (slug) to unlink.
        project_id: String,
    },

    /// Set whether an instance auto-starts when the daemon boots.
    AutoStart {
        /// Database instance id (slug).
        id: String,
        /// `true` to auto-start, `false` to disable.
        #[arg(action = clap::ArgAction::Set)]
        enabled: bool,
    },
}

#[derive(Args, Debug)]
struct DbCreateArgs {
    /// Engine id: mysql, postgres, mariadb, sqlite, redis, mongo, memcached.
    engine: String,

    /// Human-readable name (slugified into the instance id).
    name: String,

    /// Port to bind. Omit to auto-allocate from the engine default upward.
    /// Ignored for file-based engines (sqlite).
    #[arg(long)]
    port: Option<u16>,

    /// For file-based engines (sqlite): adopt an existing database file at this
    /// path instead of creating a fresh managed one.
    #[arg(long)]
    file: Option<String>,

    /// Start the instance when the daemon boots.
    #[arg(long)]
    auto_start: bool,
}

#[derive(Args, Debug)]
struct DbRemoveArgs {
    /// Database instance id (slug).
    id: String,

    /// Also delete the on-disk data directory (irreversible).
    #[arg(long)]
    delete_data: bool,

    /// Skip the confirmation prompt.
    #[arg(long)]
    force: bool,
}

#[derive(Subcommand, Debug)]
enum DnsCmd {
    /// Show the domain suffix, resolver-file state + port, helper availability,
    /// and dnsmasq tuning.
    Status,

    /// List resolvable names (wildcard + per-project), tagged by how each is
    /// routed (dnsmasq vs hosts).
    Records,

    /// Change the domain suffix. Rewrites every project hostname and drops
    /// their HTTPS certs (the app reissues them on the next reconcile).
    Suffix {
        /// New suffix (e.g. test, localhost, portbay.test). Reserved public
        /// TLDs like .com are rejected.
        suffix: String,

        /// Skip the confirmation prompt.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand, Debug)]
enum SandboxCmd {
    /// Show sandbox policy for one project (pass an id) or all projects, plus
    /// host capability (sandbox-exec) and the tier's sandbox cap.
    Status {
        /// Project id to report on. Omit to list every project.
        id: Option<String>,
    },

    /// Enable Sandboxed Run on a project (macOS only). Verifies the generated
    /// profile, then the app re-wraps the command on its next reconcile (≤30s);
    /// run `portbay restart <id>` to launch it confined.
    Enable(SandboxEnableArgs),

    /// Disable Sandboxed Run on a project. Restart the project to apply.
    Disable {
        /// Project id to unconfine.
        id: String,
    },

    /// Print recent sandbox-denial lines from a project's logs (requires the
    /// daemon).
    Violations {
        /// Project id whose logs to scan.
        id: String,
        /// How many recent log lines to scan (default 250).
        #[arg(long)]
        limit: Option<u32>,
    },
}

#[derive(Args, Debug)]
struct SandboxEnableArgs {
    /// Project id to sandbox.
    id: String,

    /// Network access granted inside the sandbox.
    #[arg(long, value_enum, default_value_t = CliSandboxNetwork::LoopbackOnly)]
    network: CliSandboxNetwork,

    /// Keep the per-run cache/temp scratch dir between runs instead of wiping it
    /// before each sandboxed start (ephemeral mode is on by default).
    #[arg(long)]
    no_ephemeral: bool,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum CliSandboxNetwork {
    /// Loopback bind/connect only — safest useful default for a local dev server.
    LoopbackOnly,
    /// Loopback plus outbound (package-manager fetches).
    Outbound,
    /// All networking.
    Full,
    /// No networking.
    Blocked,
}

impl From<CliSandboxNetwork> for SandboxNetworkPolicy {
    fn from(v: CliSandboxNetwork) -> Self {
        match v {
            CliSandboxNetwork::LoopbackOnly => SandboxNetworkPolicy::LoopbackOnly,
            CliSandboxNetwork::Outbound => SandboxNetworkPolicy::Outbound,
            CliSandboxNetwork::Full => SandboxNetworkPolicy::Full,
            CliSandboxNetwork::Blocked => SandboxNetworkPolicy::Blocked,
        }
    }
}

#[derive(Subcommand, Debug)]
enum RequestsCmd {
    /// List recent HTTP requests Caddy handled (oldest→newest), read from the
    /// access log. Works without the daemon; empty until the app serves traffic.
    Recent {
        /// How many recent requests to show (default 200, capped at 2000).
        #[arg(long)]
        limit: Option<u32>,
        /// Only show requests routed to this project id.
        #[arg(long)]
        project: Option<String>,
    },

    /// Truncate the access log so the inspector starts fresh.
    Clear,
}

#[derive(Subcommand, Debug)]
enum CertCmd {
    /// Show certificate metadata (paths, issue/expiry, days left, SANs) for one
    /// project, or all projects that have a cert when no id is given.
    Info {
        /// Project id to report on. Omit to list every project's cert.
        id: Option<String>,
    },

    /// Reissue a project's cert: delete it so the app mints a fresh one and
    /// reloads Caddy on its next reconcile (≤30s).
    Reissue {
        /// Project id whose cert to reissue.
        id: String,
    },
}

#[derive(Subcommand, Debug)]
enum SidecarStatusCmd {
    /// Report each sidecar's state as seen from outside the app: process-compose
    /// (live), the dnsmasq resolver file, and managed /etc/hosts. Caddy, mkcert,
    /// and Mailpit are app-owned — their live state shows `unknown` here.
    Status,

    /// Reclaim orphaned PortBay sidecars (caddy, dnsmasq, mailpit,
    /// process-compose) left squatting their ports after a crash or rebuild.
    /// Only processes orphaned to launchd (PPID 1) and carrying PortBay's own
    /// config paths are reaped — a live app's sidecars and any foreign
    /// caddy/dnsmasq/mailpit (ServBay, Homebrew) are never touched. Safe to run
    /// while the app is open. Normally the app self-heals on launch; this is the
    /// manual escape hatch.
    Reclaim,
}

// =============================================================================
// Entry
// =============================================================================

fn main() -> ExitCode {
    // Restore SIGPIPE to its default disposition so a broken pipe on stdout/
    // stderr terminates the write (EPIPE) rather than returning Err, which
    // Rust's print! machinery turns into a panic. Rust sets SIGPIPE=SIG_IGN
    // at startup; for a short-lived CLI the default (terminate) is correct.
    #[cfg(unix)]
    // SAFETY: signal(2) is async-signal-safe; we call it before any threads
    // or signal handlers are registered, and we only change SIGPIPE.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let cli = Cli::parse();

    // Capture CLI panics to the shared crash spool (same store the GUI uses).
    // Delivery is consent-gated and deferred to the next run via `flush_outbox`
    // below — a panic aborts this process, so the report it writes is uploaded
    // on the user's next `portbay` command, not during the unwind.
    portbay_lib::telemetry::install_panic_hook(env!("CARGO_PKG_VERSION"));

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

    // Completion helpers fire on every shell TAB — keep them free of any prefs
    // read, network, or spool write so they stay instant.
    let is_completion_helper = cli.complete_projects || cli.complete_running_projects;
    let cmd_name = cli.cmd.as_ref().map(command_name);

    let result = rt.block_on(async move {
        let prefs = portbay_lib::preferences::Preferences::load();
        if !is_completion_helper {
            // Best-effort delivery of anything queued from prior runs (crashes +
            // usage events). Near-instant when the queue is empty or consent is
            // off; bounded so it can never hang the command.
            portbay_lib::telemetry::flush_outbox(&prefs).await;
        }
        let outcome = dispatch(cli).await;
        if prefs.telemetry_enabled && !is_completion_helper {
            if let Some(name) = cmd_name {
                record_cli_command(name, &outcome);
            }
        }
        outcome
    });

    match result {
        Ok(code) => code,
        Err(e) => {
            print_error(&e);
            ExitCode::from(e.exit_code())
        }
    }
}

/// Stable, PII-free analytics label for a subcommand — the top-level verb only
/// (never args, ids, or paths). Subcommand groups collapse to their group name
/// (`db`, `dns`, …); that's the right granularity for "which commands get used".
fn command_name(cmd: &Cmd) -> &'static str {
    match cmd {
        Cmd::List => "list",
        Cmd::Status { .. } => "status",
        Cmd::Add(_) => "add",
        Cmd::Remove(_) => "remove",
        Cmd::Start { .. } => "start",
        Cmd::Stop(_) => "stop",
        Cmd::Restart { .. } => "restart",
        Cmd::Logs(_) => "logs",
        Cmd::Open { .. } => "open",
        Cmd::Doctor => "doctor",
        Cmd::Hosts(_) => "hosts",
        Cmd::Export { .. } => "export",
        Cmd::Completions { .. } => "completions",
        Cmd::Login(_) => "login",
        Cmd::License => "license",
        Cmd::Resync => "resync",
        Cmd::Logout => "logout",
        Cmd::Telemetry { .. } => "telemetry",
        Cmd::Group(_) => "group",
        Cmd::Tunnel(_) => "tunnel",
        Cmd::Ssh(_) => "ssh",
        Cmd::Runtime(_) => "runtime",
        Cmd::Db(_) => "db",
        Cmd::Dns(_) => "dns",
        Cmd::Sandbox(_) => "sandbox",
        Cmd::Requests(_) => "requests",
        Cmd::Cert(_) => "cert",
        Cmd::Sidecar(_) => "sidecar",
        Cmd::Detect { .. } => "detect",
        Cmd::Import(_) => "import",
        #[cfg(feature = "tasks")]
        Cmd::Tasks(_) => "tasks",
        #[cfg(feature = "tasks")]
        Cmd::Context(_) => "context",
        #[cfg(feature = "tasks")]
        Cmd::Handoff(_) => "handoff",
        #[cfg(feature = "tasks")]
        Cmd::Scratchpad(_) => "scratchpad",
    }
}

/// Spool one usage event for this command run (delivered on the next run by
/// `flush_outbox`). Only reached when telemetry consent is on. Best-effort: a
/// failure to write the spool file is swallowed — analytics never affects the
/// command's result.
fn record_cli_command(name: &str, outcome: &Result<ExitCode, CliError>) {
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let event = portbay_lib::telemetry::TelemetryEvent {
        command_name: name.to_string(),
        ok: outcome.is_ok(),
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        created_at,
    };
    let _ = portbay_lib::telemetry::spool_telemetry_event(&event);
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
        Cmd::Resync => cmd_resync().await,
        Cmd::Logout => cmd_logout(),
        Cmd::Telemetry { action } => cmd_telemetry(action),
        Cmd::Group(sub) => cmd_group(&ctx, sub).await,
        Cmd::Tunnel(sub) => cmd_tunnel(&ctx, sub).await,
        Cmd::Ssh(sub) => cmd_ssh(&ctx, sub).await,
        Cmd::Runtime(sub) => cmd_runtime(&ctx, sub).await,
        Cmd::Db(sub) => cmd_db(&ctx, sub).await,
        Cmd::Dns(sub) => cmd_dns(&ctx, sub).await,
        Cmd::Sandbox(sub) => cmd_sandbox(&ctx, sub).await,
        Cmd::Requests(sub) => cmd_requests(&ctx, sub).await,
        Cmd::Cert(sub) => cmd_cert(&ctx, sub).await,
        Cmd::Sidecar(sub) => cmd_sidecar(&ctx, sub).await,
        Cmd::Detect { path, apps } => cmd_detect(&ctx, &path, apps).await,
        Cmd::Import(sub) => cmd_import(&ctx, sub).await,
        #[cfg(feature = "tasks")]
        Cmd::Tasks(sub) => cmd_tasks(&ctx, sub).await,
        #[cfg(feature = "tasks")]
        Cmd::Context(sub) => cmd_context(&ctx, sub).await,
        #[cfg(feature = "tasks")]
        Cmd::Handoff(sub) => cmd_handoff(&ctx, sub).await,
        #[cfg(feature = "tasks")]
        Cmd::Scratchpad(sub) => cmd_scratchpad(&ctx, sub).await,
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
                maybe_prompt_telemetry_consent();
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

/// `portbay resync` — re-verify the stored session and refresh the cached
/// entitlement from PortBay Cloud. Mirrors the app's startup `account_resync`:
/// rotate the (likely-expired) access token via the refresh token, then
/// re-fetch the signed license. Use after a license change to pick up a new
/// tier without reopening the app.
async fn cmd_resync() -> Result<ExitCode, CliError> {
    use portbay_lib::auth::{self, RefreshOutcome, CLOUD_BASE_URL};
    use portbay_lib::entitlements;

    let Some(session) = auth::load_session() else {
        return Err(CliError::BadInput(
            "not signed in — run `portbay login` first.".into(),
        ));
    };

    let eff = match auth::refresh_session(CLOUD_BASE_URL, &session.refresh_token).await {
        RefreshOutcome::Rotated(new_session) => {
            auth::store_session(&new_session).map_err(CliError::Other)?;
            entitlements::refresh(CLOUD_BASE_URL, &new_session.access_token)
                .await
                .map_err(CliError::Other)?
        }
        RefreshOutcome::Unauthorized => {
            let _ = auth::clear_session();
            let _ = entitlements::clear_cache();
            return Err(CliError::Other(
                "session expired — run `portbay login` again.".into(),
            ));
        }
        RefreshOutcome::Transient => {
            return Err(CliError::Other(
                "couldn't reach PortBay Cloud — try again.".into(),
            ));
        }
    };

    let who = eff
        .account
        .as_ref()
        .map(|a| a.login.clone())
        .unwrap_or_default();
    println!("\u{2713} Resynced {who} — {} tier.", eff.tier);
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

/// `portbay telemetry [on|off]` — show or change the standing diagnostics
/// consent. Writes the shared `preferences.json` the GUI reads, so the choice
/// is consistent across both surfaces. Setting it either way also marks the
/// first-run consent prompt as answered (`telemetry_consent_prompted`), so a
/// user who flips this before ever logging in is never asked again.
fn cmd_telemetry(action: Option<TelemetryAction>) -> Result<ExitCode, CliError> {
    use portbay_lib::preferences::Preferences;

    let mut prefs = Preferences::load();
    match action.unwrap_or(TelemetryAction::Status) {
        TelemetryAction::On => {
            prefs.telemetry_enabled = true;
            prefs.telemetry_consent_prompted = true;
            prefs.save().map_err(|e| CliError::Other(e.to_string()))?;
            println!(
                "{} Anonymized diagnostics are ON. Turn them off with `portbay telemetry off`.",
                style("✓").green()
            );
        }
        TelemetryAction::Off => {
            prefs.telemetry_enabled = false;
            prefs.telemetry_consent_prompted = true;
            prefs.save().map_err(|e| CliError::Other(e.to_string()))?;
            println!(
                "{} Anonymized diagnostics are OFF. Turn them on with `portbay telemetry on`.",
                style("✓").green()
            );
        }
        TelemetryAction::Status => {
            let state = if prefs.telemetry_enabled {
                style("on").green()
            } else {
                style("off").dim()
            };
            println!("Anonymized diagnostics: {state}");
            println!("Change with `portbay telemetry on` or `portbay telemetry off`.");
            println!("Details: https://docs.portbay.app/legal/privacy");
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// First-run diagnostics consent, modelled on the gcloud SDK's usage-stats
/// prompt. Shown once, right after the first successful `portbay login`.
/// Recording the choice — yes or no — sets `telemetry_consent_prompted` so we
/// never ask again; a "yes" is standing consent that lets crash reports upload
/// without a per-incident prompt. Skipped silently when stdin isn't a terminal
/// (CI / piped logins) so the prompt simply waits for the next interactive
/// login rather than blocking, and skipped when already answered.
fn maybe_prompt_telemetry_consent() {
    use portbay_lib::preferences::Preferences;
    use std::io::{IsTerminal, Write};

    let mut prefs = Preferences::load();
    if prefs.telemetry_consent_prompted {
        return;
    }
    if !std::io::stdin().is_terminal() {
        return;
    }

    println!();
    println!(
        "To help improve PortBay, we can collect anonymized usage data and anonymized\n\
         crash stacktraces when something goes wrong. We never collect project names,\n\
         paths, source code, environment variables, or logs, and we only ever send to\n\
         PortBay's own endpoint — never a third-party analytics SDK.\n\
         \n\
         Details: https://docs.portbay.app/legal/privacy\n\
         \n\
         You can change this at any time by running `portbay telemetry on` (or `off`)."
    );
    eprint!("\nEnable anonymized usage data & crash reports? [y/N]: ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    let yes = matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes");

    prefs.telemetry_enabled = yes;
    prefs.telemetry_consent_prompted = true;
    if let Err(e) = prefs.save() {
        // Sign-in already succeeded; this is non-fatal. Warn and move on — the
        // marker didn't persist, so we'll simply ask again on the next login.
        eprintln!("warning: couldn't save your diagnostics choice: {e}");
        return;
    }

    if yes {
        println!(
            "{} Thanks — anonymized diagnostics are on. Turn them off any time with `portbay telemetry off`.",
            style("✓").green()
        );
    } else {
        println!(
            "{} No diagnostics will be sent. Enable later with `portbay telemetry on`.",
            style("·").dim()
        );
    }
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
        framework: None,
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
        pre_start: Vec::new(),
        post_start: Vec::new(),
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
        tunnel: None,
        deploy: None,
    };

    // Reject hostname/port collisions up front (mirrors the MCP add_project
    // path). Without this, two projects could silently share a hostname or port
    // and Caddy would route only one of them, leaving the other unreachable with
    // no visible error.
    if reg.hostname_conflict(&project.hostname, None) {
        return Err(CliError::Registry(
            portbay_lib::registry::RegistryError::DuplicateHostname(project.hostname.clone()),
        ));
    }
    if let Some(port) = project.port {
        if reg.port_conflict(port, None) {
            return Err(CliError::Registry(
                portbay_lib::registry::RegistryError::DuplicatePort(port),
            ));
        }
    }

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
    confirm_destructive(
        args.force,
        &format!(
            "This unregisters project '{}'{}.",
            args.id,
            if args.keep_artifacts {
                ""
            } else {
                " and deletes its certs + hosts entries"
            }
        ),
    )?;
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

/// CLI status badge for a single doctor check row.
fn doctor_badge(v: portbay_lib::doctor::Verdict) -> console::StyledObject<&'static str> {
    use portbay_lib::doctor::Verdict;
    match v {
        Verdict::Ok => style("✓").green(),
        Verdict::Warn => style("!").yellow(),
        Verdict::Fail => style("✗").red(),
    }
}

/// CLI status badge for a doctor category header.
fn doctor_header_badge(v: portbay_lib::doctor::Verdict) -> console::StyledObject<&'static str> {
    use portbay_lib::doctor::Verdict;
    match v {
        Verdict::Ok => style("[✓]").green(),
        Verdict::Warn => style("[!]").yellow(),
        Verdict::Fail => style("[✗]").red(),
    }
}

/// `portbay doctor` — flutter-doctor-style environment report. Renders the
/// shared [`portbay_lib::doctor`] report (the same data the MCP `portbay_doctor`
/// tool returns) as grouped categories with a header verdict and indented
/// sub-checks. Exits non-zero only on a fatal (Fail) check; warnings exit 0.
async fn cmd_doctor(ctx: &CliContext) -> Result<ExitCode, CliError> {
    use portbay_lib::doctor;

    let reg = ctx.load_registry().map_err(|e| e.to_string());
    let data_dir = ctx
        .registry_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| ctx.registry_path.clone());
    let report = doctor::report(reg.as_ref().map_err(|e| e.as_str()), ctx.pc_port, &data_dir).await;

    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        for cat in &report.categories {
            ctx.term
                .write_line(&format!(
                    "{} {}",
                    doctor_header_badge(cat.verdict),
                    style(&cat.title).bold()
                ))
                .ok();
            for c in &cat.checks {
                ctx.term
                    .write_line(&format!(
                        "    {} {:<16} {}",
                        doctor_badge(c.verdict),
                        c.check,
                        style(&c.detail).dim()
                    ))
                    .ok();
            }
            ctx.term.write_line("").ok();
        }
        let (warns, fails) = report.counts();
        if warns == 0 && fails == 0 {
            ctx.term
                .write_line(&format!("{} No issues found.", style("•").green()))
                .ok();
        } else {
            ctx.term
                .write_line(&format!(
                    "{} {} issue(s) found ({} fatal).",
                    style("•").yellow(),
                    warns + fails,
                    fails
                ))
                .ok();
        }
    }

    Ok(if report.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
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

/// `portbay tunnel list` / `portbay tunnel status <id>` — read-only view of
/// the active public tunnels mirrored by the running app to a state file.
/// Creating or stopping a share is done from the PortBay app.
async fn cmd_tunnel(ctx: &CliContext, sub: TunnelCmd) -> Result<ExitCode, CliError> {
    use portbay_lib::tunnel::read_state;

    // The data dir is the parent of registry.json.
    let data_dir = ctx
        .registry_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| ctx.registry_path.clone());

    match sub {
        TunnelCmd::List => {
            let tunnels = read_state(&data_dir);
            if ctx.json {
                println!("{}", serde_json::to_string_pretty(&tunnels)?);
                return Ok(ExitCode::SUCCESS);
            }
            if tunnels.is_empty() {
                ctx.term
                    .write_line(&format!(
                        "{} No active tunnels. Start a share from the PortBay app.",
                        style("·").dim()
                    ))
                    .ok();
                return Ok(ExitCode::SUCCESS);
            }
            let id_w = tunnels
                .iter()
                .map(|t| t.project_id.len())
                .max()
                .unwrap_or(2);
            for t in &tunnels {
                let running_badge = if t.running {
                    style("●").green()
                } else {
                    style("○").dim()
                };
                let origin = match t.origin_reachable {
                    Some(true) => style("origin ok").dim(),
                    Some(false) => style("origin unreachable").yellow(),
                    None => style("origin unknown").dim(),
                };
                let public = t.public_url.as_deref().unwrap_or("(assigning…)");
                ctx.term
                    .write_line(&format!(
                        "  {running_badge} {id:<id_w$}  {pub_url}  {origin}",
                        id = style(&t.project_id).bold(),
                        pub_url = style(public).dim(),
                    ))
                    .ok();
            }
        }

        TunnelCmd::Status { id } => {
            let tunnels = read_state(&data_dir);
            let t = tunnels.into_iter().find(|t| t.project_id == id);
            if ctx.json {
                println!("{}", serde_json::to_string_pretty(&t)?);
                return Ok(ExitCode::SUCCESS);
            }
            match t {
                None => {
                    ctx.term
                        .write_line(&format!(
                            "{} No active tunnel for `{id}`. Start a share from the PortBay app.",
                            style("·").dim()
                        ))
                        .ok();
                }
                Some(ref t) => {
                    let running_badge = if t.running {
                        style("●").green()
                    } else {
                        style("○").dim()
                    };
                    let public = t.public_url.as_deref().unwrap_or("(assigning…)");
                    ctx.term
                        .write_line(&format!(
                            "  {running_badge} {}  {}",
                            style(&t.project_id).bold(),
                            style(public).dim(),
                        ))
                        .ok();
                    let origin = match t.origin_reachable {
                        Some(true) => "reachable",
                        Some(false) => "unreachable",
                        None => "unknown",
                    };
                    ctx.term
                        .write_line(&format!(
                            "      running={}  origin={}  upstream={}",
                            t.running,
                            origin,
                            style(&t.upstream_url).dim(),
                        ))
                        .ok();
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `portbay ssh list` / `portbay ssh status <id>` / `portbay ssh connections` —
/// read-only view of saved SSH tunnels (live state, mirrored by the running app
/// to a state file) and saved SSH connections (hosts, read from the registry).
/// Saving / starting / stopping tunnels and adding / editing hosts is done from
/// the PortBay app.
async fn cmd_ssh(ctx: &CliContext, sub: SshCmd) -> Result<ExitCode, CliError> {
    use portbay_lib::ssh::manager::SshTunnelState;
    use portbay_lib::ssh::{read_state, SshTunnelRuntimeStatus};

    // The data dir is the parent of registry.json.
    let data_dir = ctx
        .registry_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| ctx.registry_path.clone());

    fn badge(t: &SshTunnelRuntimeStatus) -> console::StyledObject<&'static str> {
        match t.state {
            SshTunnelState::Live => style("●").green(),
            SshTunnelState::Reconnecting => style("◐").yellow(),
            SshTunnelState::Down => style("○").dim(),
        }
    }
    fn forward(t: &SshTunnelRuntimeStatus) -> String {
        format!(
            "{}:{} → {}:{}",
            t.local_host, t.local_port, t.remote_host, t.remote_port
        )
    }
    fn dest(t: &SshTunnelRuntimeStatus) -> String {
        if t.ssh_user.is_empty() {
            t.ssh_host.clone()
        } else {
            format!("{}@{}", t.ssh_user, t.ssh_host)
        }
    }

    match sub {
        SshCmd::List => {
            let tunnels = read_state(&data_dir);
            if ctx.json {
                println!("{}", serde_json::to_string_pretty(&tunnels)?);
                return Ok(ExitCode::SUCCESS);
            }
            if tunnels.is_empty() {
                ctx.term
                    .write_line(&format!(
                        "{} No saved SSH tunnels. Add one from the PortBay app.",
                        style("·").dim()
                    ))
                    .ok();
                return Ok(ExitCode::SUCCESS);
            }
            let id_w = tunnels.iter().map(|t| t.id.len()).max().unwrap_or(2);
            for t in &tunnels {
                ctx.term
                    .write_line(&format!(
                        "  {badge} {id:<id_w$}  {fwd}  {via}",
                        badge = badge(t),
                        id = style(&t.id).bold(),
                        fwd = style(forward(t)).dim(),
                        via = style(format!("via {}", dest(t))).dim(),
                    ))
                    .ok();
            }
        }

        SshCmd::Status { id } => {
            let tunnels = read_state(&data_dir);
            let t = tunnels.into_iter().find(|t| t.id == id);
            if ctx.json {
                println!("{}", serde_json::to_string_pretty(&t)?);
                return Ok(ExitCode::SUCCESS);
            }
            match t {
                None => {
                    ctx.term
                        .write_line(&format!(
                            "{} No SSH tunnel `{id}`. Add one from the PortBay app.",
                            style("·").dim()
                        ))
                        .ok();
                }
                Some(ref t) => {
                    ctx.term
                        .write_line(&format!(
                            "  {} {}  {}",
                            badge(t),
                            style(&t.name).bold(),
                            style(forward(t)).dim(),
                        ))
                        .ok();
                    ctx.term
                        .write_line(&format!(
                            "      running={}  via={}:{}  command={}",
                            t.running,
                            dest(t),
                            t.ssh_port,
                            style(&t.command).dim(),
                        ))
                        .ok();
                }
            }
        }

        SshCmd::Connections => {
            let reg = ctx.load_registry()?;
            let conns = reg.list_ssh_connections();
            if ctx.json {
                println!("{}", serde_json::to_string_pretty(&conns)?);
                return Ok(ExitCode::SUCCESS);
            }
            if conns.is_empty() {
                ctx.term
                    .write_line(&format!(
                        "{} No saved SSH connections. Add one from the PortBay app.",
                        style("·").dim()
                    ))
                    .ok();
                return Ok(ExitCode::SUCCESS);
            }
            let id_w = conns.iter().map(|c| c.id.as_str().len()).max().unwrap_or(2);
            for c in conns {
                let mut target = if c.ssh_user.is_empty() {
                    c.ssh_host.clone()
                } else {
                    format!("{}@{}", c.ssh_user, c.ssh_host)
                };
                if c.ssh_port != 22 {
                    target.push_str(&format!(":{}", c.ssh_port));
                }
                let os = c
                    .metadata
                    .detected_os
                    .as_deref()
                    .map(|o| format!("  {o}"))
                    .unwrap_or_default();
                ctx.term
                    .write_line(&format!(
                        "  {id:<id_w$}  {name}  {target}{os}",
                        id = style(c.id.as_str()).bold(),
                        name = style(&c.name).cyan(),
                        target = style(target).dim(),
                        os = style(os).dim(),
                    ))
                    .ok();
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `portbay runtime <...>` — language runtime management.
///
/// All operations are registry-only and do not require the daemon.
/// Installing a new language version and editing PHP FPM/ini config are
/// done from the PortBay app.
async fn cmd_runtime(ctx: &CliContext, sub: RuntimeCmd) -> Result<ExitCode, CliError> {
    use portbay_lib::registry::ManualRuntime;
    use portbay_lib::runtimes::{self, major_minor, runtime_by_id};

    match sub {
        RuntimeCmd::List { lang } => {
            let reg = ctx.load_registry()?;
            let mut views = runtimes::list_all(&reg.runtimes);
            if let Some(ref filter_lang) = lang {
                views.retain(|v| &v.id == filter_lang);
                if views.is_empty() {
                    return Err(CliError::BadInput(format!(
                        "unknown language `{filter_lang}` — valid ids: php, node, bun, python, go, ruby, flutter"
                    )));
                }
            }
            if ctx.json {
                let out: Vec<serde_json::Value> = views
                    .iter()
                    .map(|lv| {
                        serde_json::json!({
                            "id": lv.id,
                            "display_name": lv.display_name,
                            "default_version": lv.default_version,
                            "install_hint": lv.install_hint,
                            "versions": lv.versions.iter().map(|vv| serde_json::json!({
                                "version": vv.install.version,
                                "source": runtimes::source_label(vv.install.source),
                                "binary": vv.install.binary.to_string_lossy(),
                                "is_default": lv.default_version.as_deref()
                                    .is_some_and(|d| d == vv.install.version),
                            })).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(ExitCode::SUCCESS);
            }
            if views.is_empty() {
                ctx.term
                    .write_line(&format!(
                        "{} No language runtimes detected.",
                        style("·").dim()
                    ))
                    .ok();
                return Ok(ExitCode::SUCCESS);
            }
            for lv in &views {
                let default_label = lv
                    .default_version
                    .as_deref()
                    .map(|v| format!("  (default: {})", style(v).bold()))
                    .unwrap_or_default();
                ctx.term
                    .write_line(&format!(
                        "  {} {}{}",
                        style(&lv.id).bold(),
                        style(&lv.display_name).dim(),
                        default_label,
                    ))
                    .ok();
                if lv.versions.is_empty() {
                    ctx.term
                        .write_line(&format!(
                            "      {} no versions detected  {}",
                            style("·").dim(),
                            style(&lv.install_hint).dim(),
                        ))
                        .ok();
                } else {
                    for vv in &lv.versions {
                        let is_default = lv
                            .default_version
                            .as_deref()
                            .is_some_and(|d| d == vv.install.version);
                        let def_mark = if is_default {
                            style(" *").green().to_string()
                        } else {
                            String::new()
                        };
                        ctx.term
                            .write_line(&format!(
                                "      {} {}  {}{}",
                                style("·").dim(),
                                style(&vv.install.version).bold(),
                                style(runtimes::source_label(vv.install.source)).dim(),
                                def_mark,
                            ))
                            .ok();
                    }
                }
            }
        }

        RuntimeCmd::SetDefault {
            lang,
            version,
            clear,
        } => {
            // If --clear is set or no version provided, remove the default.
            let effective_version: Option<String> = if clear {
                None
            } else {
                version.filter(|v| !v.trim().is_empty())
            };

            if runtime_by_id(&lang).is_none() {
                return Err(CliError::BadInput(format!("unknown language `{lang}`")));
            }

            let mut reg = ctx.load_registry()?;

            if let Some(ref v) = effective_version {
                // Validate the version is currently detected.
                let views = runtimes::list_all(&reg.runtimes);
                let lang_view = views.iter().find(|lv| lv.id == lang);
                let version_known = lang_view
                    .is_some_and(|lv| lv.versions.iter().any(|vv| vv.install.version == *v));
                if !version_known {
                    return Err(CliError::BadInput(format!(
                        "version `{v}` is not currently detected for `{lang}` \
                         — run `portbay runtime list --lang {lang}` to see available versions"
                    )));
                }
                reg.runtimes.defaults.insert(lang.clone(), v.clone());
            } else {
                reg.runtimes.defaults.remove(&lang);
            }
            ctx.save_registry(&reg)?;

            if ctx.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": true,
                        "lang": lang,
                        "default_version": effective_version,
                    })
                );
            } else {
                match effective_version {
                    Some(v) => ctx
                        .term
                        .write_line(&format!(
                            "{} default for {} set to {}",
                            style("\u{2713}").green(),
                            style(&lang).bold(),
                            style(&v).bold(),
                        ))
                        .ok(),
                    None => ctx
                        .term
                        .write_line(&format!(
                            "{} default for {} cleared",
                            style("\u{2713}").green(),
                            style(&lang).bold(),
                        ))
                        .ok(),
                };
            }
        }

        RuntimeCmd::AddPath { lang, path } => {
            let runtime = runtime_by_id(&lang)
                .ok_or_else(|| CliError::BadInput(format!("unknown language `{lang}`")))?;

            let binary = std::path::PathBuf::from(&path);
            if !binary.is_file() {
                return Err(CliError::BadInput(format!("no binary found at {path}")));
            }

            let version = runtime.probe_version(&binary).ok_or_else(|| {
                CliError::BadInput(format!(
                    "{path} didn't report a {lang} version — is it the right binary?"
                ))
            })?;
            let version = major_minor(&version);

            let mut reg = ctx.load_registry()?;
            let canon = binary.canonicalize().unwrap_or_else(|_| binary.clone());
            let exists = reg
                .runtimes
                .manual
                .iter()
                .any(|m| m.binary.canonicalize().unwrap_or_else(|_| m.binary.clone()) == canon);
            if !exists {
                reg.runtimes.manual.push(ManualRuntime {
                    lang: lang.clone(),
                    version: version.clone(),
                    binary: binary.clone(),
                });
                ctx.save_registry(&reg)?;
            }

            if ctx.json {
                let views = runtimes::list_all(&reg.runtimes);
                let lang_view = views.iter().find(|lv| lv.id == lang);
                let out: Vec<serde_json::Value> = lang_view
                    .into_iter()
                    .map(|lv| {
                        serde_json::json!({
                            "id": lv.id,
                            "display_name": lv.display_name,
                            "default_version": lv.default_version,
                            "versions": lv.versions.iter().map(|vv| serde_json::json!({
                                "version": vv.install.version,
                                "source": runtimes::source_label(vv.install.source),
                                "binary": vv.install.binary.to_string_lossy(),
                            })).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                ctx.term
                    .write_line(&format!(
                        "{} added {} {} at {}",
                        style("\u{2713}").green(),
                        style(&lang).bold(),
                        style(&version).bold(),
                        style(&path).dim(),
                    ))
                    .ok();
            }
        }

        RuntimeCmd::RemovePath { lang, version } => {
            let mut reg = ctx.load_registry()?;
            let before = reg.runtimes.manual.len();
            reg.runtimes
                .manual
                .retain(|m| !(m.lang == lang && m.version == version));
            let removed = reg.runtimes.manual.len() != before;
            if removed {
                ctx.save_registry(&reg)?;
            }

            if ctx.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": true,
                        "lang": lang,
                        "version": version,
                        "removed": removed,
                    })
                );
            } else if removed {
                ctx.term
                    .write_line(&format!(
                        "{} removed manual {} {} install",
                        style("\u{2713}").green(),
                        style(&lang).bold(),
                        style(&version).bold(),
                    ))
                    .ok();
            } else {
                ctx.term
                    .write_line(&format!(
                        "{} no manual {} {} install found (nothing changed)",
                        style("·").dim(),
                        style(&lang).bold(),
                        style(&version).bold(),
                    ))
                    .ok();
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `portbay db <...>` — database engine catalogue + owned-instance lifecycle.
/// Mirrors `commands/databases.rs` over the registry + Process Compose; the
/// running app's reconcile loop adds the `db-<id>` process after a create.
/// Installing an engine binary (brew) and opening a DB shell are app-only.
async fn cmd_db(ctx: &CliContext, sub: DbCmd) -> Result<ExitCode, CliError> {
    use portbay_lib::databases as engine;

    let app_data = ctx
        .registry_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| ctx.registry_path.clone());

    match sub {
        DbCmd::Engines => {
            let rows: Vec<serde_json::Value> = DB_ENGINES
                .iter()
                .map(|&e| {
                    let det = engine::detect(e);
                    serde_json::json!({
                        "id": e.id(),
                        "label": e.label(),
                        "installed": det.installed,
                        "version": det.version,
                        "default_port": e.default_port(),
                        "client_available": det.client.is_some(),
                        "install_hint": db_install_hint(e),
                    })
                })
                .collect();
            if ctx.json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
                return Ok(ExitCode::SUCCESS);
            }
            for r in &rows {
                let badge = if r["installed"].as_bool().unwrap_or(false) {
                    style("●").green()
                } else {
                    style("○").dim()
                };
                ctx.term
                    .write_line(&format!(
                        "  {badge} {id:<10} {ver:<10} {hint}",
                        id = style(r["id"].as_str().unwrap_or("")).bold(),
                        ver = style(r["version"].as_str().unwrap_or("")).dim(),
                        hint = style(r["install_hint"].as_str().unwrap_or("")).dim(),
                    ))
                    .ok();
            }
        }

        DbCmd::List => {
            let reg = ctx.load_registry()?;
            let pc = fetch_pc_state(ctx).await;
            let instances = reg.list_databases();
            if ctx.json {
                let out: Vec<_> = instances
                    .iter()
                    .map(|inst| {
                        let proc = pc.as_ref().and_then(|m| m.get(&format!("db-{}", inst.id)));
                        serde_json::json!({
                            "id": inst.id.to_string(),
                            "name": inst.name,
                            "engine": inst.engine.id(),
                            "version": inst.version,
                            "port": inst.port,
                            "status": db_status_str(proc),
                            "auto_start": inst.auto_start,
                            "connection_url": inst.connection_url(),
                            "linked_projects": inst.linked_projects.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(ExitCode::SUCCESS);
            }
            if instances.is_empty() {
                ctx.term
                    .write_line(&format!(
                        "{} No database instances. Create one: `portbay db create <engine> <name>`.",
                        style("·").dim()
                    ))
                    .ok();
                return Ok(ExitCode::SUCCESS);
            }
            if pc.is_none() {
                ctx.term
                    .write_line(&format!(
                        "{} Daemon not reachable; status reflects the registry only.",
                        style("!").yellow()
                    ))
                    .ok();
            }
            for inst in instances {
                // File-based engines (sqlite) have no daemon or port: they're
                // always available once the file exists. Show "file" + the
                // file's presence instead of a misleading `:0 stopped`.
                let (badge, port_col, status) = if inst.engine.is_file_based() {
                    let present = inst
                        .file_path
                        .as_ref()
                        .map(|p| p.is_file())
                        .unwrap_or(false);
                    if present {
                        (
                            style("●").green(),
                            "file".to_string(),
                            "available".to_string(),
                        )
                    } else {
                        (style("○").dim(), "file".to_string(), "missing".to_string())
                    }
                } else {
                    let proc = pc.as_ref().and_then(|m| m.get(&format!("db-{}", inst.id)));
                    let status = db_status_str(proc);
                    let badge = if status == "running" {
                        style("●").green()
                    } else {
                        style("○").dim()
                    };
                    (badge, format!(":{}", inst.port), status)
                };
                ctx.term
                    .write_line(&format!(
                        "  {badge} {id:<16} {eng:<10} {port_col:<7} {status}",
                        id = style(inst.id.to_string()).bold(),
                        eng = style(inst.engine.id()).dim(),
                        status = style(&status).dim(),
                    ))
                    .ok();
            }
        }

        DbCmd::Info { id } => {
            let reg = ctx.load_registry()?;
            let inst = reg
                .get_database(&DatabaseInstanceId::new(id.clone()))
                .ok_or_else(|| CliError::BadInput(format!("database `{id}` not found")))?;
            let env = inst.connection_env();
            if ctx.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "id": inst.id.to_string(),
                        "engine": inst.engine.id(),
                        "connection_url": inst.connection_url(),
                        "account": inst.default_account(),
                        "env": env,
                    })
                );
                return Ok(ExitCode::SUCCESS);
            }
            ctx.term
                .write_line(&format!(
                    "  {} {}",
                    style(inst.id.to_string()).bold(),
                    style(inst.connection_url()).dim()
                ))
                .ok();
            for (k, v) in &env {
                ctx.term
                    .write_line(&format!("      {}={}", style(k).dim(), v))
                    .ok();
            }
        }

        DbCmd::Create(args) => {
            let eng = DatabaseEngine::from_id(&args.engine)
                .ok_or_else(|| CliError::BadInput(format!("unknown engine: {}", args.engine)))?;
            let name = args.name.trim();
            if name.is_empty() {
                return Err(CliError::BadInput("a database name is required".into()));
            }
            let mut reg = ctx.load_registry()?;
            let id = db_unique_id(&reg, name);

            // File-based engines (sqlite): no daemon, no port. Adopt an existing
            // file or create a fresh managed one.
            let (port, detection_version, file_path) = if eng.is_file_based() {
                let managed_bin = reg
                    .managed_engine(eng)
                    .map(|m| engine::managed_bin_dir(&m.dir));
                let file = match args.file.as_deref().map(str::trim) {
                    Some(p) if !p.is_empty() => {
                        let path = std::path::PathBuf::from(p);
                        if !path.is_file() {
                            return Err(CliError::BadInput(format!(
                                "no database file at {} to adopt",
                                path.display()
                            )));
                        }
                        path
                    }
                    _ => {
                        engine::provision(eng, std::path::Path::new(""), &app_data, &id, 0, None)
                            .map_err(|e| CliError::Other(format!("provision: {e}")))?;
                        engine::sqlite_file(&app_data, &id)
                    }
                };
                (
                    0u16,
                    engine::detect_resolved(eng, managed_bin.as_deref()).version,
                    Some(file),
                )
            } else {
                // Prefer a PortBay-managed engine install, falling back to Homebrew/system.
                let managed_bin = reg
                    .managed_engine(eng)
                    .map(|m| engine::managed_bin_dir(&m.dir));
                let daemon = engine::daemon_binary_resolved(eng, managed_bin.as_deref()).ok_or_else(|| {
                    CliError::BadInput(format!(
                        "{} isn't installed ({}). Install the engine binary from the PortBay app, then retry.",
                        eng.label(),
                        db_install_hint(eng)
                    ))
                })?;
                let port = match args.port {
                    Some(p) => {
                        if reg.database_port_in_use(p, None) {
                            return Err(CliError::BadInput(format!(
                                "port {p} is already used by another database instance"
                            )));
                        }
                        p
                    }
                    None => db_alloc_port(&reg, eng),
                };
                let detection = engine::detect_resolved(eng, managed_bin.as_deref());
                engine::provision(eng, &daemon, &app_data, &id, port, managed_bin.as_deref())
                    .map_err(|e| CliError::Other(format!("provision: {e}")))?;
                (port, detection.version, None)
            };
            let instance = DatabaseInstance {
                id: DatabaseInstanceId::new(id.clone()),
                name: name.to_string(),
                engine: eng,
                version: detection_version,
                port,
                data_dir: engine::data_dir(&app_data, &id),
                config_path: engine::config_path(eng, &app_data, &id),
                socket_path: engine::socket_path(eng, &app_data, &id),
                file_path,
                auto_start: args.auto_start,
                linked_projects: vec![],
            };
            reg.add_database(instance.clone())
                .map_err(CliError::Registry)?;
            ctx.save_registry(&reg)?;
            if ctx.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "id": instance.id.to_string(),
                        "engine": eng.id(),
                        "port": port,
                        "connection_url": instance.connection_url(),
                    })
                );
            } else if eng.is_file_based() {
                let file = instance
                    .file_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                ctx.term
                    .write_line(&format!(
                        "{} provisioned {} ({} at {})",
                        style("✓").green(),
                        instance.id,
                        eng.label(),
                        style(file).dim(),
                    ))
                    .ok();
                ctx.term
                    .write_line(&format!(
                        "  {} file-based — no daemon to start. Link it: `portbay db link {} <project>`.",
                        style("·").dim(),
                        instance.id
                    ))
                    .ok();
            } else {
                ctx.term
                    .write_line(&format!(
                        "{} provisioned {} ({} on :{})",
                        style("✓").green(),
                        instance.id,
                        eng.label(),
                        port
                    ))
                    .ok();
                ctx.term
                    .write_line(&format!(
                        "  {} joins Process Compose after the app reconciles (≤30s); then `portbay db start {}`.",
                        style("·").dim(),
                        instance.id
                    ))
                    .ok();
            }
        }

        DbCmd::Remove(args) => {
            let did = DatabaseInstanceId::new(args.id.clone());
            confirm_destructive(
                args.force,
                &format!(
                    "This removes database instance '{}'{}.",
                    args.id,
                    if args.delete_data {
                        " AND permanently deletes its on-disk data"
                    } else {
                        ""
                    }
                ),
            )?;
            // Best-effort stop so we don't orphan a running daemon.
            let _ = ctx.pc().stop(&format!("db-{}", args.id)).await;
            let mut reg = ctx.load_registry()?;
            let removed = reg.remove_database(&did).map_err(CliError::Registry)?;
            ctx.save_registry(&reg)?;
            let mut warnings: Vec<String> = Vec::new();
            if args.delete_data {
                let dir = engine::instance_dir(&app_data, removed.id.as_str());
                if dir.starts_with(engine::instances_root(&app_data)) && dir.exists() {
                    if let Err(e) = std::fs::remove_dir_all(&dir) {
                        warnings.push(format!("could not delete data dir {}: {e}", dir.display()));
                    }
                }
            }
            if ctx.json {
                println!(
                    "{}",
                    serde_json::json!({ "removed": removed.id.to_string(), "warnings": warnings })
                );
            } else {
                ctx.term
                    .write_line(&format!(
                        "{} removed database {}",
                        style("✓").green(),
                        removed.id
                    ))
                    .ok();
                for w in &warnings {
                    ctx.term
                        .write_line(&format!("  {} {w}", style("!").yellow()))
                        .ok();
                }
            }
        }

        DbCmd::Start { id } => return db_lifecycle(ctx, &id, ProcOp::Start).await,
        DbCmd::Stop { id } => return db_lifecycle(ctx, &id, ProcOp::Stop).await,
        DbCmd::Restart { id } => return db_lifecycle(ctx, &id, ProcOp::Restart).await,

        DbCmd::Link { id, project_id } => {
            let mut reg = ctx.load_registry()?;
            if reg
                .get_project(&ProjectId::new(project_id.clone()))
                .is_none()
            {
                return Err(CliError::ProjectNotFound(project_id));
            }
            let pid = ProjectId::new(project_id.clone());
            let inst = reg
                .get_database_mut(&DatabaseInstanceId::new(id.clone()))
                .ok_or_else(|| CliError::BadInput(format!("database `{id}` not found")))?;
            if !inst.linked_projects.contains(&pid) {
                inst.linked_projects.push(pid);
            }
            ctx.save_registry(&reg)?;
            cli_say(ctx, &format!("linked {id} to {project_id}"));
        }

        DbCmd::Unlink { id, project_id } => {
            let mut reg = ctx.load_registry()?;
            let pid = ProjectId::new(project_id.clone());
            let inst = reg
                .get_database_mut(&DatabaseInstanceId::new(id.clone()))
                .ok_or_else(|| CliError::BadInput(format!("database `{id}` not found")))?;
            inst.linked_projects.retain(|p| p != &pid);
            ctx.save_registry(&reg)?;
            cli_say(ctx, &format!("unlinked {project_id} from {id}"));
        }

        DbCmd::AutoStart { id, enabled } => {
            let mut reg = ctx.load_registry()?;
            let inst = reg
                .get_database_mut(&DatabaseInstanceId::new(id.clone()))
                .ok_or_else(|| CliError::BadInput(format!("database `{id}` not found")))?;
            inst.auto_start = enabled;
            ctx.save_registry(&reg)?;
            cli_say(ctx, &format!("set auto-start={enabled} for {id}"));
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Shared start/stop/restart for a DB instance: confirm it exists, then drive
/// Process Compose on its `db-<id>` process.
async fn db_lifecycle(ctx: &CliContext, id: &str, op: ProcOp) -> Result<ExitCode, CliError> {
    {
        let reg = ctx.load_registry()?;
        if reg.get_database(&DatabaseInstanceId::new(id)).is_none() {
            return Err(CliError::BadInput(format!("database `{id}` not found")));
        }
    }
    cmd_proc_op(ctx, &format!("db-{id}"), op).await
}

/// Engines PortBay can manage (mirrors `commands::databases::ALL_ENGINES`).
const DB_ENGINES: &[DatabaseEngine] = &[
    DatabaseEngine::Mysql,
    DatabaseEngine::Postgres,
    DatabaseEngine::Mariadb,
    DatabaseEngine::Sqlite,
    DatabaseEngine::Redis,
    DatabaseEngine::Mongo,
    DatabaseEngine::Memcached,
];

fn db_install_hint(e: DatabaseEngine) -> &'static str {
    match e {
        DatabaseEngine::Mysql => "brew install mysql",
        DatabaseEngine::Postgres => "brew install postgresql@16",
        DatabaseEngine::Mariadb => "brew install mariadb",
        DatabaseEngine::Redis => "brew install redis",
        DatabaseEngine::Mongo => "brew install mongodb-community",
        DatabaseEngine::Memcached => "brew install memcached",
        DatabaseEngine::Sqlite => "ships with macOS (brew install sqlite)",
    }
}

fn db_unique_id(reg: &Registry, name: &str) -> String {
    let base = {
        let s = slugify(name);
        if s.is_empty() {
            "db".to_string()
        } else {
            s
        }
    };
    let mut candidate = base.clone();
    let mut n = 2;
    while reg
        .get_database(&DatabaseInstanceId::new(candidate.clone()))
        .is_some()
    {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    candidate
}

fn db_alloc_port(reg: &Registry, eng: DatabaseEngine) -> u16 {
    let mut port = eng.default_port();
    for _ in 0..500 {
        if !reg.database_port_in_use(port, None) && portbay_lib::port_holder::find(port).is_none() {
            return port;
        }
        port = port.saturating_add(1);
        if port == u16::MAX {
            break;
        }
    }
    eng.default_port()
}

fn db_status_str(proc: Option<&Process>) -> String {
    match proc {
        None => "stopped".into(),
        Some(p) => {
            let s = p.status.to_lowercase();
            if p.is_running && (s.contains("running") || s.contains("ready")) {
                "running".into()
            } else if s.contains("launching") || s.contains("starting") {
                "starting".into()
            } else if s.contains("error") || s.contains("failed") {
                "errored".into()
            } else {
                "stopped".into()
            }
        }
    }
}

/// `portbay dns <...>` — local DNS inspection + domain-suffix change. The
/// resolver file is read cross-process; starting/restarting dnsmasq and
/// first-run resolver install are done from the PortBay app.
async fn cmd_dns(ctx: &CliContext, sub: DnsCmd) -> Result<ExitCode, CliError> {
    use portbay_lib::dnsmasq::resolver;

    match sub {
        DnsCmd::Status => {
            let reg = ctx.load_registry()?;
            let suffix = reg.domain_suffix.clone();
            let path = resolver::resolver_file_path(&suffix);
            let contents = resolver::read_installed(&suffix);
            let installed = contents
                .as_deref()
                .is_some_and(dns_resolver_points_to_portbay);
            let port = contents.as_deref().and_then(dns_resolver_port);
            let helper = HostsHelperClient::system().is_available();
            if ctx.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "suffix": suffix,
                        "resolver_installed": installed,
                        "resolver_path": path.to_string_lossy(),
                        "resolver_port": port,
                        "helper_available": helper,
                        "dnsmasq": {
                            "cache_size": reg.dnsmasq.cache_size,
                            "local_ttl": reg.dnsmasq.local_ttl,
                            "disable_negative_cache": reg.dnsmasq.disable_negative_cache,
                        },
                    })
                );
                return Ok(ExitCode::SUCCESS);
            }
            ctx.term
                .write_line(&format!("  suffix:   {}", style(&suffix).bold()))
                .ok();
            let badge = if installed {
                style("●").green()
            } else {
                style("○").dim()
            };
            let port_str = port.map(|p| format!(" (port {p})")).unwrap_or_default();
            ctx.term
                .write_line(&format!(
                    "  {badge} resolver: {}{}",
                    if installed {
                        "installed"
                    } else {
                        "not installed (names resolve via /etc/hosts)"
                    },
                    port_str
                ))
                .ok();
            ctx.term
                .write_line(&format!("      {}", style(path.to_string_lossy()).dim()))
                .ok();
            ctx.term
                .write_line(&format!(
                    "  helper:   {}",
                    if helper { "available" } else { "not installed" }
                ))
                .ok();
            ctx.term
                .write_line(&format!(
                    "  dnsmasq:  cache={} ttl={} no-negcache={}",
                    reg.dnsmasq.cache_size,
                    reg.dnsmasq.local_ttl,
                    reg.dnsmasq.disable_negative_cache
                ))
                .ok();
        }

        DnsCmd::Records => {
            let reg = ctx.load_registry()?;
            let suffix = reg.domain_suffix.clone();
            let dns_routing = resolver::read_installed(&suffix)
                .as_deref()
                .is_some_and(dns_resolver_points_to_portbay);
            let suffix_tail = format!(".{suffix}");
            if ctx.json {
                let mut out = vec![serde_json::json!({
                    "hostname": format!("*.{suffix}"),
                    "target": "127.0.0.1",
                    "kind": "wildcard",
                    "routed_via": "dnsmasq",
                })];
                for p in reg.list_projects() {
                    let in_suffix = p.hostname.ends_with(&suffix_tail);
                    out.push(serde_json::json!({
                        "hostname": p.hostname,
                        "target": "127.0.0.1",
                        "kind": "project",
                        "project_id": p.id.as_str(),
                        "project_name": p.name,
                        "routed_via": if dns_routing && in_suffix { "dnsmasq" } else { "hosts" },
                    }));
                }
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(ExitCode::SUCCESS);
            }
            ctx.term
                .write_line(&format!(
                    "  {} {}",
                    style(format!("*.{suffix}")).bold(),
                    style("dnsmasq").dim()
                ))
                .ok();
            for p in reg.list_projects() {
                let in_suffix = p.hostname.ends_with(&suffix_tail);
                let via = if dns_routing && in_suffix {
                    "dnsmasq"
                } else {
                    "hosts"
                };
                ctx.term
                    .write_line(&format!(
                        "  {} {}",
                        style(&p.hostname).bold(),
                        style(via).dim()
                    ))
                    .ok();
            }
        }

        DnsCmd::Suffix { suffix, force } => {
            let mut reg = ctx.load_registry()?;
            confirm_destructive(
                force,
                &format!(
                    "This rewrites every project hostname to '.{}' and drops all HTTPS certs (reissued on next reconcile).",
                    suffix.trim().trim_start_matches('.')
                ),
            )?;
            let migration =
                portbay_lib::domain::migrate_registry_suffix(&mut reg, &suffix, certs_root())
                    .map_err(|e| CliError::BadInput(e.to_string()))?;
            ctx.save_registry(&reg)?;
            if ctx.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "old_suffix": migration.old_suffix,
                        "new_suffix": migration.new_suffix,
                        "changed_projects": migration.changed_projects,
                        "cert_dirs_removed": migration.cert_dirs_removed,
                    })
                );
            } else {
                ctx.term
                    .write_line(&format!(
                        "{} suffix {} → {}",
                        style("✓").green(),
                        style(&migration.old_suffix).dim(),
                        style(&migration.new_suffix).bold()
                    ))
                    .ok();
                ctx.term
                    .write_line(&format!(
                        "  {} {} hostname(s) rewritten, {} cert dir(s) removed; the app reissues \
                         certs + updates /etc/hosts on the next reconcile.",
                        style("·").dim(),
                        migration.changed_projects,
                        migration.cert_dirs_removed
                    ))
                    .ok();
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn dns_resolver_points_to_portbay(contents: &str) -> bool {
    contents.contains("nameserver 127.0.0.1") || contents.contains("DNS=127.0.0.1:")
}

/// Parse the target port out of the platform resolver file body.
fn dns_resolver_port(contents: &str) -> Option<u16> {
    contents.lines().find_map(|l| {
        let line = l.trim();
        line.strip_prefix("port ")
            .map(str::trim)
            .or_else(|| line.strip_prefix("DNS=127.0.0.1:").map(str::trim))
            .and_then(|n| n.split_whitespace().next()?.parse().ok())
    })
}

async fn cmd_sandbox(ctx: &CliContext, sub: SandboxCmd) -> Result<ExitCode, CliError> {
    use portbay_lib::{entitlements, sandbox};

    // The data dir is the parent of registry.json — same dir the app reads the
    // generated `.sb` profile from, so the preflight below validates the real one.
    let data_dir = ctx
        .registry_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| ctx.registry_path.clone());

    match sub {
        SandboxCmd::Status { id } => {
            let reg = ctx.load_registry()?;
            if let Some(ref want) = id {
                if reg.get_project(&ProjectId::new(want)).is_none() {
                    return Err(CliError::ProjectNotFound(want.clone()));
                }
            }
            let available = sandbox::is_available();
            let cap = entitlements::current().entitlements.max_sandbox_projects();
            let enabled_count = reg
                .projects
                .iter()
                .filter(|p| sandbox::is_enabled(p))
                .count();
            let rows: Vec<&Project> = reg
                .projects
                .iter()
                .filter(|p| id.as_deref().is_none_or(|w| p.id.as_str() == w))
                .collect();

            if ctx.json {
                let out: Vec<_> = rows
                    .iter()
                    .map(|p| {
                        let cfg = sandbox::config(p);
                        serde_json::json!({
                            "id": p.id.as_str(),
                            "name": p.name,
                            "enabled": sandbox::is_enabled(p),
                            "network": sandbox::network_policy_key(cfg.network),
                            "ephemeral": cfg.ephemeral,
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::json!({
                        "platform_supported": cfg!(target_os = "macos"),
                        "sandbox_available": available,
                        "community_cap": cap,
                        "enabled_count": enabled_count,
                        "projects": out,
                    })
                );
                return Ok(ExitCode::SUCCESS);
            }

            let cap_str = cap
                .map(|c| c.to_string())
                .unwrap_or_else(|| "unlimited".into());
            ctx.term
                .write_line(&format!(
                    "  sandbox-exec: {}",
                    if available {
                        "available"
                    } else {
                        "unavailable"
                    }
                ))
                .ok();
            ctx.term
                .write_line(&format!("  enabled:      {enabled_count} / {cap_str}"))
                .ok();
            for p in rows {
                let on = sandbox::is_enabled(p);
                let badge = if on {
                    style("●").green()
                } else {
                    style("○").dim()
                };
                let detail = if on {
                    let cfg = sandbox::config(p);
                    style(format!(
                        "network={} ephemeral={}",
                        sandbox::network_policy_key(cfg.network),
                        cfg.ephemeral
                    ))
                    .dim()
                } else {
                    style("off".to_string()).dim()
                };
                ctx.term
                    .write_line(&format!(
                        "  {badge} {} {detail}",
                        style(p.id.as_str()).bold()
                    ))
                    .ok();
            }
        }

        SandboxCmd::Enable(args) => {
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (&args, &data_dir);
                return Err(CliError::Other(
                    "Sandboxed Run is only available on macOS.".into(),
                ));
            }

            #[cfg(target_os = "macos")]
            {
                let policy: SandboxNetworkPolicy = args.network.into();
                let ephemeral = !args.no_ephemeral;
                let pid = ProjectId::new(args.id.clone());
                let mut reg = ctx.load_registry()?;

                // Community sandbox cap (Pro unlimited): only a newly sandboxed
                // project counts, measured against the others already sandboxed.
                let already_on = reg
                    .get_project(&pid)
                    .map(sandbox::is_enabled)
                    .unwrap_or(false);
                if !already_on {
                    let others = reg
                        .projects
                        .iter()
                        .filter(|p| p.id != pid && sandbox::is_enabled(p))
                        .count();
                    if let Err(cap) = entitlements::check_can_sandbox(others) {
                        return Err(CliError::BadInput(format!(
                            "sandbox cap reached: this tier allows {cap} sandboxed project(s) at \
                             once. Disable sandbox on another project, or sign in / upgrade for \
                             unlimited."
                        )));
                    }
                }

                let project = reg
                    .get_project_mut(&pid)
                    .ok_or_else(|| CliError::ProjectNotFound(args.id.clone()))?;
                if project.start_command.is_none() && project.workspace.is_none() {
                    return Err(CliError::BadInput(
                        "Sandboxed Run requires a project command to supervise".into(),
                    ));
                }
                sandbox::enable(project, policy, ephemeral);
                // Fail closed: prove macOS accepts this profile before persisting.
                sandbox::preflight(&data_dir, project)
                    .map_err(|e| CliError::Other(format!("sandbox could not be activated: {e}")))?;
                sandbox::reset_ephemeral_state(&data_dir, project)
                    .map_err(|e| CliError::Other(format!("sandbox reset failed: {e}")))?;
                let net = sandbox::network_policy_key(policy);
                ctx.save_registry(&reg)?;

                if ctx.json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "ok": true,
                            "id": args.id,
                            "enabled": true,
                            "network": net,
                            "ephemeral": ephemeral,
                        })
                    );
                } else {
                    cli_say(
                        ctx,
                        &format!(
                            "sandboxed {} (network={net}, ephemeral={ephemeral})",
                            args.id
                        ),
                    );
                    ctx.term
                        .write_line(&format!(
                            "  {} macOS accepted the profile. The app re-wraps the command on its \
                             next reconcile (≤30s); then `portbay restart {}` to run it confined.",
                            style("·").dim(),
                            args.id
                        ))
                        .ok();
                }
            }
        }

        SandboxCmd::Disable { id } => {
            let pid = ProjectId::new(id.clone());
            let mut reg = ctx.load_registry()?;
            let project = reg
                .get_project_mut(&pid)
                .ok_or_else(|| CliError::ProjectNotFound(id.clone()))?;
            sandbox::disable(project);
            ctx.save_registry(&reg)?;
            cli_say(
                ctx,
                &format!("disabled sandbox for {id} — restart the project to apply"),
            );
        }

        SandboxCmd::Violations { id, limit } => {
            {
                let reg = ctx.load_registry()?;
                if reg.get_project(&ProjectId::new(&id)).is_none() {
                    return Err(CliError::ProjectNotFound(id));
                }
            }
            let lines = ctx
                .pc()
                .logs(&id, 0, limit.unwrap_or(250))
                .await
                .map_err(CliError::Pc)?;
            let violations = sandbox::violation_lines(&lines);
            if ctx.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "id": id,
                        "scanned_lines": lines.len(),
                        "violations": violations,
                    })
                );
            } else if violations.is_empty() {
                ctx.term
                    .write_line(&format!(
                        "{} no sandbox violations in the last {} log line(s) for {id}",
                        style("·").dim(),
                        lines.len()
                    ))
                    .ok();
            } else {
                ctx.term
                    .write_line(&format!(
                        "{} {} sandbox violation(s) for {id}:",
                        style("⚠").yellow(),
                        violations.len()
                    ))
                    .ok();
                for v in &violations {
                    println!("  {v}");
                }
            }
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
                        id = style(app["suggested_id"].as_str().unwrap_or("")).bold(),
                        host = style(app["suggested_hostname"].as_str().unwrap_or("")).dim(),
                        kind = style(app["kind"].as_str().unwrap_or("")).dim(),
                        cmd = app["suggested_start_command"]
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
                .write_line(&format!("  hostname:  {}", style(&hostname).dim()))
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
                    let project_ids: Vec<&str> = g.projects.iter().map(|id| id.as_str()).collect();
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
                let project_ids: Vec<&str> = group.projects.iter().map(|id| id.as_str()).collect();
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
            reg.update_group(next.clone()).map_err(CliError::Registry)?;
            ctx.save_registry(&reg)?;

            if ctx.json {
                let known: std::collections::HashSet<&str> =
                    reg.list_projects().iter().map(|p| p.id.as_str()).collect();
                let project_ids: Vec<&str> = next.projects.iter().map(|id| id.as_str()).collect();
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

async fn cmd_requests(ctx: &CliContext, sub: RequestsCmd) -> Result<ExitCode, CliError> {
    use portbay_lib::commands::http_inspector;

    // The access log lives at <data_dir>/logs/caddy-access.log; data_dir is the
    // registry's parent (same convention as tunnels/databases).
    let data_dir = ctx
        .registry_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| ctx.registry_path.clone());
    let log = http_inspector::access_log_path(&data_dir);

    match sub {
        RequestsCmd::Recent { limit, project } => {
            let reg = ctx.load_registry()?;
            if let Some(ref pid) = project {
                if reg.get_project(&ProjectId::new(pid)).is_none() {
                    return Err(CliError::ProjectNotFound(pid.clone()));
                }
            }
            let mut entries = http_inspector::read_recent(&log, limit, &reg);
            if let Some(ref pid) = project {
                entries.retain(|e| e.project_id.as_deref() == Some(pid.as_str()));
            }

            if ctx.json {
                println!("{}", serde_json::to_string_pretty(&entries)?);
                return Ok(ExitCode::SUCCESS);
            }
            if entries.is_empty() {
                ctx.term
                    .write_line(&format!(
                        "{} No requests recorded yet. Serve traffic through a project, then re-run.",
                        style("·").dim()
                    ))
                    .ok();
                return Ok(ExitCode::SUCCESS);
            }
            for e in &entries {
                let status = match e.status {
                    200..=299 => style(e.status).green(),
                    300..=399 => style(e.status).cyan(),
                    400..=499 => style(e.status).yellow(),
                    _ => style(e.status).red(),
                };
                ctx.term
                    .write_line(&format!(
                        "  {status} {method:<6} {host}{uri}  {dur:.1}ms",
                        method = e.method,
                        host = style(&e.host).dim(),
                        uri = e.uri,
                        dur = e.duration_ms,
                    ))
                    .ok();
            }
        }

        RequestsCmd::Clear => {
            http_inspector::clear_access_log(&log)
                .map_err(|e| CliError::Other(format!("clear access log: {e}")))?;
            cli_say(ctx, "cleared the HTTP request log");
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn cmd_cert(ctx: &CliContext, sub: CertCmd) -> Result<ExitCode, CliError> {
    use portbay_lib::commands::certs::{read_cert_info, CertInfo};
    use portbay_lib::mkcert;

    let root = certs_root()
        .ok_or_else(|| CliError::Other("could not resolve the certs directory".into()))?;

    match sub {
        CertCmd::Info { id } => {
            let reg = ctx.load_registry()?;
            let infos: Vec<CertInfo> = match &id {
                Some(id) => {
                    if reg.get_project(&ProjectId::new(id)).is_none() {
                        return Err(CliError::ProjectNotFound(id.clone()));
                    }
                    read_cert_info(&root, id)
                        .map_err(|e| CliError::Other(e.to_string()))?
                        .into_iter()
                        .collect()
                }
                None => reg
                    .list_projects()
                    .iter()
                    .filter_map(|p| read_cert_info(&root, p.id.as_str()).ok().flatten())
                    .collect(),
            };

            if ctx.json {
                println!("{}", serde_json::to_string_pretty(&infos)?);
                return Ok(ExitCode::SUCCESS);
            }
            if infos.is_empty() {
                let scope = id
                    .as_deref()
                    .map(|i| format!("for {i} "))
                    .unwrap_or_default();
                ctx.term
                    .write_line(&format!(
                        "{} No certificate {scope}issued yet. Start an HTTPS project so the app mints one.",
                        style("·").dim()
                    ))
                    .ok();
                return Ok(ExitCode::SUCCESS);
            }
            for c in &infos {
                let expiry = match c.days_until_expiry {
                    Some(d) if d < 0 => style(format!("expired {} day(s) ago", -d)).red(),
                    Some(d) if d <= 14 => style(format!("expires in {d} day(s)")).yellow(),
                    Some(d) => style(format!("expires in {d} day(s)")).green(),
                    None => style("expiry unknown".to_string()).dim(),
                };
                ctx.term
                    .write_line(&format!("  {} {}", style(&c.project_id).bold(), expiry))
                    .ok();
                if !c.sans.is_empty() {
                    ctx.term
                        .write_line(&format!(
                            "    {} {}",
                            style("SANs:").dim(),
                            c.sans.join(", ")
                        ))
                        .ok();
                }
                ctx.term
                    .write_line(&format!(
                        "    {} {}",
                        style("cert:").dim(),
                        style(&c.certificate_path).dim()
                    ))
                    .ok();
            }
        }

        CertCmd::Reissue { id } => {
            let reg = ctx.load_registry()?;
            if reg.get_project(&ProjectId::new(&id)).is_none() {
                return Err(CliError::ProjectNotFound(id));
            }
            mkcert::remove_cert_dir(&root, &id)
                .map_err(|e| CliError::Other(format!("remove cert: {e}")))?;
            cli_say(
                ctx,
                &format!(
                    "removed {id}'s certificate — the app reissues it + reloads Caddy on its next \
                     reconcile (≤30s)"
                ),
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn cmd_sidecar(ctx: &CliContext, sub: SidecarStatusCmd) -> Result<ExitCode, CliError> {
    use portbay_lib::sidecar_probe::{self, ProbeState};

    match sub {
        SidecarStatusCmd::Status => {
            let suffix = ctx
                .load_registry()
                .map(|r| r.domain_suffix)
                .unwrap_or_else(|_| DEFAULT_DOMAIN_SUFFIX.to_string());
            let probes = sidecar_probe::probe(ctx.pc_port, &suffix).await;

            if ctx.json {
                let out: Vec<_> = probes
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "name": p.name,
                            "state": p.state.as_str(),
                            "detail": p.detail,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(ExitCode::SUCCESS);
            }

            let name_w = probes.iter().map(|p| p.name.len()).max().unwrap_or(4);
            for p in &probes {
                let badge = match p.state {
                    ProbeState::Running => style("●").green(),
                    ProbeState::Stopped => style("○").red(),
                    ProbeState::Unknown => style("·").dim(),
                };
                ctx.term
                    .write_line(&format!(
                        "  {badge} {name:<name_w$}  {detail}",
                        name = style(p.name).bold(),
                        detail = style(&p.detail).dim(),
                        name_w = name_w,
                    ))
                    .ok();
            }
            ctx.term
                .write_line(&format!(
                    "  {} restarting a sidecar is done from the PortBay app (it owns the processes).",
                    style("·").dim()
                ))
                .ok();
        }
        SidecarStatusCmd::Reclaim => {
            use portbay_lib::sidecar_reclaim::{self, SidecarKind, SweepMode};

            // OrphansOnly: never reach into a live app's sidecar tree. If the
            // app is up, its sidecars are parented to it (not launchd) and are
            // skipped; only genuine crash-leftovers are reaped.
            let mut total = 0usize;
            let mut rows: Vec<(String, usize)> = Vec::new();
            for kind in SidecarKind::ALL {
                let n = sidecar_reclaim::reclaim_stale(kind, SweepMode::OrphansOnly);
                total += n;
                if n > 0 {
                    rows.push((kind.display_name().to_string(), n));
                }
            }

            if ctx.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "reclaimed": total,
                        "by_kind": rows
                            .iter()
                            .map(|(k, n)| serde_json::json!({ "kind": k, "count": n }))
                            .collect::<Vec<_>>(),
                        "app_running": sidecar_reclaim::app_running(),
                    })
                );
                return Ok(ExitCode::SUCCESS);
            }

            if total == 0 {
                ctx.term
                    .write_line(&format!(
                        "{} No orphaned PortBay sidecars found — nothing to reclaim.",
                        style("✓").green()
                    ))
                    .ok();
            } else {
                for (kind, n) in &rows {
                    ctx.term
                        .write_line(&format!(
                            "  {} reclaimed {n} orphaned {kind}",
                            style("✓").green()
                        ))
                        .ok();
                }
                ctx.term
                    .write_line(&format!(
                        "{} Reclaimed {total} orphaned sidecar(s).",
                        style("•").green()
                    ))
                    .ok();
            }
            if let Some(pid) = sidecar_reclaim::app_running() {
                ctx.term
                    .write_line(&format!(
                        "  {} PortBay is running (pid {pid}); its live sidecars were left untouched.",
                        style("·").dim()
                    ))
                    .ok();
            }
        }
    }
    Ok(ExitCode::SUCCESS)
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

/// `portbay import …` — migrate sites from Herd / ServBay / MAMP. Writes
/// Projects to the registry; the running app provisions them on its next
/// reconcile. The site→Project mapping is shared with the GUI + MCP via
/// `portbay_lib::import`.
async fn cmd_import(ctx: &CliContext, sub: ImportCmd) -> Result<ExitCode, CliError> {
    use portbay_lib::import::{self, ImportSource};

    let parse_source = |s: &str| -> Result<ImportSource, CliError> {
        ImportSource::parse(s).ok_or_else(|| {
            CliError::BadInput(format!("unknown source `{s}` (valid: herd, servbay, mamp)"))
        })
    };

    match sub {
        ImportCmd::Sources => {
            let sources = import::detect_all();
            if ctx.json {
                println!("{}", serde_json::to_string_pretty(&sources)?);
                return Ok(ExitCode::SUCCESS);
            }
            for s in &sources {
                let head = if s.present {
                    style(format!("{} ({} site(s))", s.label, s.site_count)).bold()
                } else {
                    style(format!("{} — not installed", s.label)).dim()
                };
                ctx.term.write_line(&format!("  {head}")).ok();
                if let Some(note) = &s.note {
                    ctx.term
                        .write_line(&format!("    {}", style(note).dim()))
                        .ok();
                }
            }
        }

        ImportCmd::Preview { source } => {
            let src = parse_source(&source)?;
            let reg = ctx.load_registry()?;
            let rows = import::preview(src, &reg).map_err(|e| CliError::Other(e.to_string()))?;
            if ctx.json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
                return Ok(ExitCode::SUCCESS);
            }
            if rows.is_empty() {
                ctx.term
                    .write_line(&format!(
                        "{} No sites found for {}.",
                        style("·").dim(),
                        src.label()
                    ))
                    .ok();
                return Ok(ExitCode::SUCCESS);
            }
            for r in &rows {
                let flag = match (r.id_collision, r.path_collision) {
                    (false, false) => style("new".to_string()).green(),
                    (true, _) => style("id exists".to_string()).yellow(),
                    (_, true) => style("path exists".to_string()).yellow(),
                };
                ctx.term
                    .write_line(&format!(
                        "  {} {} {} [{}]",
                        style(&r.site.suggested_id).bold(),
                        style(&r.site.hostname).dim(),
                        style(&r.site.path).dim(),
                        flag
                    ))
                    .ok();
            }
        }

        ImportCmd::Projects { source, ids, all } => {
            let src = parse_source(&source)?;
            let mut reg = ctx.load_registry()?;
            let ids: Vec<String> = if all || ids.is_empty() {
                import::site_ids(src).map_err(|e| CliError::Other(e.to_string()))?
            } else {
                ids
            };
            let result = import::import_selected(src, &ids, &mut reg)
                .map_err(|e| CliError::Other(e.to_string()))?;
            if !result.imported.is_empty() {
                ctx.save_registry(&reg)?;
            }
            if ctx.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
                return Ok(ExitCode::SUCCESS);
            }
            if result.imported.is_empty() && result.skipped.is_empty() {
                ctx.term
                    .write_line(&format!(
                        "{} Nothing to import from {}.",
                        style("·").dim(),
                        src.label()
                    ))
                    .ok();
                return Ok(ExitCode::SUCCESS);
            }
            if !result.imported.is_empty() {
                ctx.term
                    .write_line(&format!(
                        "{} Imported {}: {}",
                        style("✓").green(),
                        result.imported.len(),
                        result.imported.join(", ")
                    ))
                    .ok();
                ctx.term
                    .write_line(&format!(
                        "  {}",
                        style("the PortBay app provisions them on its next reconcile (≤30s)").dim()
                    ))
                    .ok();
            }
            for s in &result.skipped {
                ctx.term
                    .write_line(&format!(
                        "  {} {} — {}",
                        style("skipped").yellow(),
                        s.site.suggested_id,
                        s.reason
                    ))
                    .ok();
            }
        }
    }
    Ok(ExitCode::SUCCESS)
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
            // Port conflict gets its own documented code (4) so scripts can
            // distinguish it from a generic registry error.
            CliError::Registry(registry::RegistryError::DuplicatePort(_)) => 4,
            CliError::Hosts(HostsError::PermissionDenied { .. }) => 6,
            CliError::Registry(_) | CliError::Json(_) | CliError::Other(_) | CliError::Hosts(_) => {
                1
            }
        }
    }
}

/// Confirm an irreversible action. `--force` bypasses the prompt. On an
/// interactive terminal the user is asked [y/N]; in a non-interactive context
/// (script/CI) without `--force` we refuse rather than silently destroy data —
/// a mistyped id then aborts cleanly instead of wiping the wrong thing.
fn confirm_destructive(force: bool, what: &str) -> Result<(), CliError> {
    use std::io::{IsTerminal, Write};
    if force {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(CliError::BadInput(format!(
            "{what} is destructive; re-run with --force to confirm (no terminal to prompt on)"
        )));
    }
    eprint!("{what}\nProceed? [y/N]: ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    if matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(CliError::BadInput("aborted by user".into()))
    }
}

impl From<serde_json::Error> for CliError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

#[cfg(feature = "tasks")]
impl From<portbay_lib::context::ContextError> for CliError {
    fn from(e: portbay_lib::context::ContextError) -> Self {
        use portbay_lib::context::ContextError as CE;
        match e {
            CE::ProjectNotFound(id) | CE::CardNotFound(id) => CliError::ProjectNotFound(id),
            CE::BadInput(s) => CliError::BadInput(s),
            CE::IllegalTransition { .. }
            | CE::StaleRun { .. }
            | CE::HandoffTooLarge { .. }
            | CE::AgentNotFound(_)
            | CE::ProRequired(_) => CliError::BadInput(e.to_string()),
            other => CliError::Other(other.to_string()),
        }
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
// Tasks board (`portbay tasks …`)
// =============================================================================

#[cfg(feature = "tasks")]
fn tasks_project_path(ctx: &CliContext, project: &str) -> Result<PathBuf, CliError> {
    let reg = ctx.load_registry()?;
    reg.get_project(&registry::ProjectId::new(project))
        .map(|p| p.path.clone())
        .ok_or_else(|| CliError::ProjectNotFound(project.to_string()))
}

#[cfg(feature = "tasks")]
fn parse_cli_priority(
    s: Option<String>,
) -> Result<Option<portbay_lib::context::board::Priority>, CliError> {
    use portbay_lib::context::board::Priority;
    match s {
        None => Ok(None),
        Some(v) => match v.trim().to_ascii_lowercase().as_str() {
            "high" => Ok(Some(Priority::High)),
            "medium" | "med" => Ok(Some(Priority::Medium)),
            "low" => Ok(Some(Priority::Low)),
            other => Err(CliError::BadInput(format!(
                "unknown priority '{other}' (expected high|medium|low)"
            ))),
        },
    }
}

#[cfg(feature = "tasks")]
async fn cmd_tasks(ctx: &CliContext, sub: TasksCmd) -> Result<ExitCode, CliError> {
    use portbay_lib::context::audit::Actor;
    use portbay_lib::context::board::{self, BoardStatus};
    use portbay_lib::context::{clock, ops};

    match sub {
        TasksCmd::List { project, filter } => {
            let path = tasks_project_path(ctx, &project)?;
            let mut cards = board::list_cards(&path)?;
            let show_archived = filter.as_deref().is_some_and(|f| f.contains("is:archived"));
            if !show_archived {
                cards.retain(|c| !c.card.archived);
            }
            if let Some(f) = filter.as_deref() {
                let today = clock::now_iso8601();
                let today = &today[..10];
                cards.retain(|c| board::card_matches(&c.card, &c.body, f, today));
            }

            if ctx.json {
                #[derive(serde::Serialize)]
                struct Row<'a> {
                    #[serde(flatten)]
                    card: &'a board::TaskCard,
                    body: &'a str,
                }
                let rows: Vec<Row> = cards
                    .iter()
                    .map(|c| Row {
                        card: &c.card,
                        body: &c.body,
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&rows)?);
                return Ok(ExitCode::SUCCESS);
            }

            if cards.is_empty() {
                ctx.term
                    .write_line(&format!(
                        "{} No tasks yet. {} `portbay tasks add {project} \"<title>\"`",
                        style("·").dim(),
                        style("Add one with").dim()
                    ))
                    .ok();
                return Ok(ExitCode::SUCCESS);
            }

            for col in BoardStatus::flow_order() {
                let in_col: Vec<_> = cards.iter().filter(|c| c.card.status == col).collect();
                if in_col.is_empty() {
                    continue;
                }
                ctx.term
                    .write_line(&format!(
                        "{} {}",
                        style(col.as_str()).bold(),
                        style(format!("({})", in_col.len())).dim()
                    ))
                    .ok();
                for c in in_col {
                    let pri = c
                        .card
                        .priority
                        .map(|p| format!("{p:?}").to_lowercase())
                        .unwrap_or_default();
                    ctx.term
                        .write_line(&format!(
                            "  {}  {:<7} {}",
                            style(&c.card.id).dim(),
                            pri,
                            c.card.title
                        ))
                        .ok();
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        TasksCmd::Add(args) => {
            let path = tasks_project_path(ctx, &args.project)?;
            let status = match args.status {
                Some(s) => Some(BoardStatus::parse(&s)?),
                None => None,
            };
            let priority = parse_cli_priority(args.priority)?;
            let agent = match args
                .agent
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(s) => Some(
                    portbay_lib::context::config::AgentKind::parse(s)
                        .ok_or_else(|| CliError::Other(format!("unknown agent: {s}")))?,
                ),
                None => None,
            };
            let mut create = ops::CreateInput {
                title: args.title,
                body: args.body.unwrap_or_default(),
                status,
                priority,
                due: args.due,
                acceptance: args.acceptance,
                touchpoints: args.touchpoint,
                automation: None,
                agent,
                labels: args.label,
                estimate: args.estimate,
                color: args.color,
                url: args.url,
                links: Vec::new(),
                checklist: None,
            };
            if let Some(name) = args.template.as_deref() {
                let tpl = portbay_lib::context::templates::get(name)
                    .ok_or_else(|| CliError::Other(format!("unknown template: {name}")))?;
                tpl.seed(&mut create);
            }
            let pc = ops::create_card(&path, create, Actor::Cli, &clock::now_iso8601())?;
            if ctx.json {
                println!("{}", serde_json::to_string_pretty(&pc.card)?);
            } else {
                ctx.term
                    .write_line(&format!(
                        "{} added {} in {}",
                        style("✓").green(),
                        style(&pc.card.id).bold(),
                        pc.card.status.as_str()
                    ))
                    .ok();
            }
            Ok(ExitCode::SUCCESS)
        }

        TasksCmd::Move { project, id, to } => {
            let path = tasks_project_path(ctx, &project)?;
            let to = BoardStatus::parse(&to)?;
            let pc = ops::move_card(
                &path,
                &id,
                to,
                false,
                None,
                Actor::Cli,
                &clock::now_iso8601(),
            )?;
            ctx.term
                .write_line(&format!(
                    "{} {} → {}",
                    style("✓").green(),
                    style(&pc.card.id).bold(),
                    pc.card.status.as_str()
                ))
                .ok();
            Ok(ExitCode::SUCCESS)
        }

        TasksCmd::Block {
            project,
            id,
            dep,
            remove,
        } => {
            let path = tasks_project_path(ctx, &project)?;
            let mut deps = board::read_card(&path, &id)?.card.blocked_by;
            if remove {
                deps.retain(|d| d != &dep);
            } else if dep != id && !deps.contains(&dep) {
                deps.push(dep.clone());
            }
            ops::update_card(
                &path,
                &id,
                ops::UpdatePatch {
                    blocked_by: Some(deps),
                    ..Default::default()
                },
                Actor::Cli,
                &clock::now_iso8601(),
            )?;
            ctx.term
                .write_line(&format!(
                    "{} {} {} dependency {}",
                    style("✓").green(),
                    style(&id).bold(),
                    if remove { "removed" } else { "blocked by" },
                    style(&dep).bold()
                ))
                .ok();
            Ok(ExitCode::SUCCESS)
        }

        TasksCmd::Done { project, id } => {
            let path = tasks_project_path(ctx, &project)?;
            let pc = ops::move_card(
                &path,
                &id,
                BoardStatus::Done,
                false,
                None,
                Actor::Cli,
                &clock::now_iso8601(),
            )?;
            ctx.term
                .write_line(&format!(
                    "{} {} done",
                    style("✓").green(),
                    style(&pc.card.id).bold()
                ))
                .ok();
            Ok(ExitCode::SUCCESS)
        }

        TasksCmd::Rm { project, id } => {
            let path = tasks_project_path(ctx, &project)?;
            ops::delete_card(&path, &id, Actor::Cli, &clock::now_iso8601())?;
            ctx.term
                .write_line(&format!(
                    "{} removed {}",
                    style("✓").green(),
                    style(&id).bold()
                ))
                .ok();
            Ok(ExitCode::SUCCESS)
        }

        TasksCmd::Show { project, id } => {
            let path = tasks_project_path(ctx, &project)?;
            let pc = board::read_card(&path, &id)?;
            if ctx.json {
                #[derive(serde::Serialize)]
                struct Row<'a> {
                    #[serde(flatten)]
                    card: &'a board::TaskCard,
                    body: &'a str,
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(&Row {
                        card: &pc.card,
                        body: &pc.body
                    })?
                );
            } else {
                println!("{}", board::render_card(&pc.card, &pc.body)?);
            }
            Ok(ExitCode::SUCCESS)
        }

        TasksCmd::Capture { project, title } => {
            let path = tasks_project_path(ctx, &project)?;
            let now = clock::now_iso8601();
            let created = ops::create_card(
                &path,
                ops::CreateInput {
                    title,
                    ..Default::default()
                },
                Actor::Cli,
                &now,
            )?;
            ops::update_card(
                &path,
                &created.card.id,
                ops::UpdatePatch {
                    draft: Some(true),
                    ..Default::default()
                },
                Actor::Cli,
                &now,
            )?;
            ctx.term
                .write_line(&format!(
                    "{} captured {} → Drafts",
                    style("✓").green(),
                    style(&created.card.id).bold()
                ))
                .ok();
            Ok(ExitCode::SUCCESS)
        }

        TasksCmd::Promote { project, id } => {
            let path = tasks_project_path(ctx, &project)?;
            ops::update_card(
                &path,
                &id,
                ops::UpdatePatch {
                    draft: Some(false),
                    ..Default::default()
                },
                Actor::Cli,
                &clock::now_iso8601(),
            )?;
            ctx.term
                .write_line(&format!(
                    "{} promoted {} to the board",
                    style("✓").green(),
                    style(&id).bold()
                ))
                .ok();
            Ok(ExitCode::SUCCESS)
        }

        TasksCmd::Export { project, out } => {
            let path = tasks_project_path(ctx, &project)?;
            let json = portbay_lib::context::portage::export_board(&path)?;
            match out {
                Some(file) => {
                    std::fs::write(&file, &json)
                        .map_err(|e| CliError::Other(format!("write {file}: {e}")))?;
                    ctx.term
                        .write_line(&format!(
                            "{} board exported → {}",
                            style("✓").green(),
                            style(&file).bold()
                        ))
                        .ok();
                }
                None => println!("{json}"),
            }
            Ok(ExitCode::SUCCESS)
        }

        TasksCmd::ImportTrello { project, file } => {
            let path = tasks_project_path(ctx, &project)?;
            let json = std::fs::read_to_string(&file)
                .map_err(|e| CliError::Other(format!("read {file}: {e}")))?;
            let n =
                portbay_lib::context::portage::import_trello(&path, &json, &clock::now_iso8601())?;
            ctx.term
                .write_line(&format!(
                    "{} imported {n} card(s) from Trello",
                    style("✓").green()
                ))
                .ok();
            Ok(ExitCode::SUCCESS)
        }

        TasksCmd::Check {
            project,
            id,
            idx,
            undone,
        } => {
            let path = tasks_project_path(ctx, &project)?;
            let pc = ops::check_item(&path, &id, idx, !undone, Actor::Cli, &clock::now_iso8601())?;
            let (done, total) = pc
                .card
                .checklist
                .as_ref()
                .map(|c| c.progress())
                .unwrap_or((0, 0));
            ctx.term
                .write_line(&format!(
                    "{} item {idx} {} ({done}/{total})",
                    style("✓").green(),
                    if undone { "reopened" } else { "done" }
                ))
                .ok();
            Ok(ExitCode::SUCCESS)
        }

        TasksCmd::Comment { project, id, text } => {
            let path = tasks_project_path(ctx, &project)?;
            ops::comment(&path, &id, &text, Actor::Cli, &clock::now_iso8601())?;
            ctx.term
                .write_line(&format!(
                    "{} comment added to {}",
                    style("✓").green(),
                    style(&id).bold()
                ))
                .ok();
            Ok(ExitCode::SUCCESS)
        }

        TasksCmd::Checklist { project, id, items } => {
            let path = tasks_project_path(ctx, &project)?;
            let n = items.len();
            ops::add_checklist_items(&path, &id, items, None, Actor::Cli, &clock::now_iso8601())?;
            ctx.term
                .write_line(&format!(
                    "{} added {n} checklist item(s) to {}",
                    style("✓").green(),
                    style(&id).bold()
                ))
                .ok();
            Ok(ExitCode::SUCCESS)
        }

        TasksCmd::Archive {
            project,
            id,
            restore,
        } => {
            let path = tasks_project_path(ctx, &project)?;
            ops::set_archived(&path, &id, !restore, Actor::Cli, &clock::now_iso8601())?;
            ctx.term
                .write_line(&format!(
                    "{} {} {}",
                    style("✓").green(),
                    style(&id).bold(),
                    if restore { "restored" } else { "archived" }
                ))
                .ok();
            Ok(ExitCode::SUCCESS)
        }
    }
}

// =============================================================================
// Context projections (`portbay context …`) + hand-off (`portbay handoff …`)
// =============================================================================

#[cfg(feature = "tasks")]
async fn cmd_context(ctx: &CliContext, sub: ContextCmd) -> Result<ExitCode, CliError> {
    use portbay_lib::context::sync;

    match sub {
        ContextCmd::Sync {
            project,
            dry_run,
            diff,
            adopt,
        } => {
            use portbay_lib::context::{adapters, config};
            let reg = ctx.load_registry()?;
            let p = reg
                .get_project(&registry::ProjectId::new(&project))
                .ok_or_else(|| CliError::ProjectNotFound(project.clone()))?;

            // Record adoptions before syncing so the gated files get written
            // this run. `all` adopts every shipped adapter.
            if !adopt.is_empty() && !dry_run {
                let mut cfg = config::load(&p.path)?;
                for id in &adopt {
                    if id == "all" {
                        for spec in adapters::ALL {
                            cfg.adapter_adopt.insert(spec.id.to_string());
                        }
                    } else if adapters::by_id(id).is_some() {
                        cfg.adapter_adopt.insert(id.clone());
                    } else {
                        return Err(CliError::Other(format!("unknown adapter id: {id}")));
                    }
                }
                config::save(&p.path, &cfg)?;
            }

            if diff || dry_run {
                let preview = sync::sync_project(&reg, p, true)?;
                if ctx.json {
                    println!("{}", serde_json::to_string_pretty(&preview)?);
                    if dry_run {
                        return Ok(ExitCode::SUCCESS);
                    }
                } else {
                    for r in &preview.results {
                        if let Some(pv) = &r.preview {
                            ctx.term
                                .write_line(&format!(
                                    "{} {}",
                                    style("──").dim(),
                                    style(&r.path).bold()
                                ))
                                .ok();
                            println!("{pv}");
                        }
                    }
                    if dry_run {
                        return Ok(ExitCode::SUCCESS);
                    }
                }
            }

            let report = sync::sync_project(&reg, p, false)?;
            if ctx.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
                return Ok(ExitCode::SUCCESS);
            }
            for r in &report.results {
                if r.action == sync::SyncAction::NeedsConsent {
                    ctx.term
                        .write_line(&format!(
                            "{} {:<12} {} — adopt with `portbay context sync {} --adopt {}`",
                            style("!").yellow(),
                            "needs-consent",
                            r.path,
                            project,
                            r.id,
                        ))
                        .ok();
                    continue;
                }
                let action = format!("{:?}", r.action).to_lowercase();
                if r.enabled || r.action != sync::SyncAction::Skipped {
                    ctx.term
                        .write_line(&format!("{} {:<12} {}", style("·").dim(), action, r.path))
                        .ok();
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        ContextCmd::Show { project } => {
            let reg = ctx.load_registry()?;
            let p = reg
                .get_project(&registry::ProjectId::new(&project))
                .ok_or_else(|| CliError::ProjectNotFound(project.clone()))?;
            let context = sync::derive_context(&reg, p);
            println!("{}", serde_json::to_string_pretty(&context)?);
            Ok(ExitCode::SUCCESS)
        }
    }
}

#[cfg(feature = "tasks")]
async fn cmd_handoff(ctx: &CliContext, sub: HandoffCmd) -> Result<ExitCode, CliError> {
    use portbay_lib::context::{board, clock, config, handoff, sync};

    match sub {
        HandoffCmd::Show { project } => {
            let path = tasks_project_path(ctx, &project)?;
            match handoff::read(&path)? {
                Some(h) => {
                    if ctx.json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "updated": h.meta.updated,
                                "tokenBudget": h.meta.token_budget,
                                "autoGenerated": h.meta.auto_generated,
                                "tokens": handoff::estimate_tokens(&h.body),
                                "body": h.body,
                            })
                        );
                    } else {
                        println!("{}", h.body);
                    }
                }
                None => {
                    ctx.term
                        .write_line(&format!("{} no hand-off yet", style("·").dim()))
                        .ok();
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        HandoffCmd::Update { project, narrative } => {
            let reg = ctx.load_registry()?;
            let p = reg
                .get_project(&registry::ProjectId::new(&project))
                .ok_or_else(|| CliError::ProjectNotFound(project.clone()))?;
            let path = p.path.clone();
            let cfg = config::load(&path)?;
            let context = sync::derive_context(&reg, p);
            let cards = board::list_cards(&path)?;

            let text = match narrative {
                Some(n) => n,
                None => {
                    use std::io::Read;
                    let mut s = String::new();
                    std::io::stdin().read_to_string(&mut s).ok();
                    s
                }
            };

            let up = handoff::update(
                &path,
                Some(&text),
                "you (CLI)",
                &context,
                &cards,
                cfg.handoff.max_chars,
                &clock::now_iso8601(),
                false,
            )?;
            let _ = sync::sync_project(&reg, p, false);

            ctx.term
                .write_line(&format!(
                    "{} hand-off entry added ({}/{} chars{})",
                    style("✓").green(),
                    up.chars,
                    up.handoff.meta.max_chars,
                    if up.trimmed {
                        ", oldest entries pruned"
                    } else {
                        ""
                    }
                ))
                .ok();
            Ok(ExitCode::SUCCESS)
        }
    }
}

#[cfg(feature = "tasks")]
async fn cmd_scratchpad(ctx: &CliContext, sub: ScratchpadCmd) -> Result<ExitCode, CliError> {
    use portbay_lib::context::scratchpad;

    match sub {
        ScratchpadCmd::Show { project } => {
            let path = tasks_project_path(ctx, &project)?;
            let body = scratchpad::read(&path)?;
            if ctx.json {
                println!("{}", serde_json::json!({ "body": body }));
            } else if body.trim().is_empty() {
                ctx.term
                    .write_line(&format!("{} scratchpad is empty", style("·").dim()))
                    .ok();
            } else {
                println!("{body}");
            }
            Ok(ExitCode::SUCCESS)
        }

        ScratchpadCmd::Set { project, body } => {
            let path = tasks_project_path(ctx, &project)?;
            let text = match body {
                Some(b) => b,
                None => {
                    use std::io::Read;
                    let mut s = String::new();
                    std::io::stdin().read_to_string(&mut s).ok();
                    s
                }
            };
            scratchpad::write(&path, &text)?;
            ctx.term
                .write_line(&format!(
                    "{} scratchpad saved ({} chars)",
                    style("✓").green(),
                    text.chars().count()
                ))
                .ok();
            Ok(ExitCode::SUCCESS)
        }
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
    fn cli_parses_doctor() {
        let cli = Cli::try_parse_from(["portbay", "doctor"]).unwrap();
        assert!(matches!(cli.cmd, Some(Cmd::Doctor)));
    }

    #[test]
    fn cli_parses_doctor_json() {
        let cli = Cli::try_parse_from(["portbay", "--json", "doctor"]).unwrap();
        assert!(matches!(cli.cmd, Some(Cmd::Doctor)));
        assert!(cli.json);
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
    fn cli_parses_sandbox_enable_with_network_and_ephemeral_flags() {
        let cli = Cli::try_parse_from([
            "portbay",
            "sandbox",
            "enable",
            "blog",
            "--network",
            "outbound",
            "--no-ephemeral",
        ])
        .unwrap();
        let Some(Cmd::Sandbox(SandboxCmd::Enable(args))) = cli.cmd else {
            panic!("expected Sandbox Enable")
        };
        assert_eq!(args.id, "blog");
        assert!(matches!(args.network, CliSandboxNetwork::Outbound));
        assert!(args.no_ephemeral);
        // Default network is loopback-only when the flag is omitted.
        let cli = Cli::try_parse_from(["portbay", "sandbox", "enable", "blog"]).unwrap();
        let Some(Cmd::Sandbox(SandboxCmd::Enable(args))) = cli.cmd else {
            panic!("expected Sandbox Enable")
        };
        assert!(matches!(args.network, CliSandboxNetwork::LoopbackOnly));
        assert!(!args.no_ephemeral);
    }

    #[test]
    fn cli_parses_sandbox_status_optional_id() {
        let cli = Cli::try_parse_from(["portbay", "sandbox", "status"]).unwrap();
        assert!(matches!(
            cli.cmd,
            Some(Cmd::Sandbox(SandboxCmd::Status { id: None }))
        ));
    }

    #[test]
    fn cli_parses_requests_recent_with_filters_and_clear() {
        let cli = Cli::try_parse_from([
            "portbay",
            "requests",
            "recent",
            "--limit",
            "50",
            "--project",
            "blog",
        ])
        .unwrap();
        let Some(Cmd::Requests(RequestsCmd::Recent { limit, project })) = cli.cmd else {
            panic!("expected Requests Recent")
        };
        assert_eq!(limit, Some(50));
        assert_eq!(project.as_deref(), Some("blog"));

        let cli = Cli::try_parse_from(["portbay", "requests", "clear"]).unwrap();
        assert!(matches!(cli.cmd, Some(Cmd::Requests(RequestsCmd::Clear))));
    }

    #[test]
    fn cli_parses_cert_info_optional_id_and_reissue() {
        let cli = Cli::try_parse_from(["portbay", "cert", "info"]).unwrap();
        assert!(matches!(
            cli.cmd,
            Some(Cmd::Cert(CertCmd::Info { id: None }))
        ));
        let cli = Cli::try_parse_from(["portbay", "cert", "reissue", "blog"]).unwrap();
        let Some(Cmd::Cert(CertCmd::Reissue { id })) = cli.cmd else {
            panic!("expected Cert Reissue")
        };
        assert_eq!(id, "blog");
    }

    #[test]
    fn cli_parses_sidecar_status() {
        let cli = Cli::try_parse_from(["portbay", "sidecar", "status"]).unwrap();
        assert!(matches!(
            cli.cmd,
            Some(Cmd::Sidecar(SidecarStatusCmd::Status))
        ));
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
        let cli = Cli::try_parse_from(["portbay", "group", "create", "Dev", "--id", "dev-group"])
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
        let cli = Cli::try_parse_from(["portbay", "group", "remove", "old-group"]).unwrap();
        let Some(Cmd::Group(GroupCmd::Remove { id })) = cli.cmd else {
            panic!("expected Group::Remove")
        };
        assert_eq!(id, "old-group");
    }

    #[test]
    fn cli_parses_group_start_stop_restart() {
        for verb in ["start", "stop", "restart"] {
            let cli = Cli::try_parse_from(["portbay", "group", verb, "g1"]).unwrap();
            assert!(matches!(
                cli.cmd,
                Some(Cmd::Group(
                    GroupCmd::Start { .. } | GroupCmd::Stop { .. } | GroupCmd::Restart { .. }
                ))
            ));
        }
    }

    #[test]
    fn cli_parses_tunnel_list() {
        let cli = Cli::try_parse_from(["portbay", "tunnel", "list"]).unwrap();
        assert!(matches!(cli.cmd, Some(Cmd::Tunnel(TunnelCmd::List))));
    }

    #[test]
    fn cli_parses_tunnel_status() {
        let cli = Cli::try_parse_from(["portbay", "tunnel", "status", "blog"]).unwrap();
        let Some(Cmd::Tunnel(TunnelCmd::Status { id })) = cli.cmd else {
            panic!("expected Tunnel::Status")
        };
        assert_eq!(id, "blog");
    }

    #[test]
    fn cli_parses_ssh_list() {
        let cli = Cli::try_parse_from(["portbay", "ssh", "list"]).unwrap();
        assert!(matches!(cli.cmd, Some(Cmd::Ssh(SshCmd::List))));
    }

    #[test]
    fn cli_parses_ssh_status() {
        let cli = Cli::try_parse_from(["portbay", "ssh", "status", "prod-db"]).unwrap();
        let Some(Cmd::Ssh(SshCmd::Status { id })) = cli.cmd else {
            panic!("expected Ssh::Status")
        };
        assert_eq!(id, "prod-db");
    }

    #[test]
    fn cli_parses_ssh_connections() {
        let cli = Cli::try_parse_from(["portbay", "ssh", "connections"]).unwrap();
        assert!(matches!(cli.cmd, Some(Cmd::Ssh(SshCmd::Connections))));
    }

    #[test]
    fn cli_parses_runtime_list() {
        let cli = Cli::try_parse_from(["portbay", "runtime", "list"]).unwrap();
        assert!(matches!(
            cli.cmd,
            Some(Cmd::Runtime(RuntimeCmd::List { lang: None }))
        ));
    }

    #[test]
    fn cli_parses_runtime_list_with_lang_filter() {
        let cli = Cli::try_parse_from(["portbay", "runtime", "list", "--lang", "php"]).unwrap();
        let Some(Cmd::Runtime(RuntimeCmd::List { lang })) = cli.cmd else {
            panic!("expected Runtime::List")
        };
        assert_eq!(lang.as_deref(), Some("php"));
    }

    #[test]
    fn cli_parses_runtime_set_default() {
        let cli = Cli::try_parse_from(["portbay", "runtime", "set-default", "node", "20"]).unwrap();
        let Some(Cmd::Runtime(RuntimeCmd::SetDefault {
            lang,
            version,
            clear,
        })) = cli.cmd
        else {
            panic!("expected Runtime::SetDefault")
        };
        assert_eq!(lang, "node");
        assert_eq!(version.as_deref(), Some("20"));
        assert!(!clear);
    }

    #[test]
    fn cli_parses_runtime_set_default_clear_flag() {
        let cli =
            Cli::try_parse_from(["portbay", "runtime", "set-default", "php", "--clear"]).unwrap();
        let Some(Cmd::Runtime(RuntimeCmd::SetDefault { lang, clear, .. })) = cli.cmd else {
            panic!("expected Runtime::SetDefault")
        };
        assert_eq!(lang, "php");
        assert!(clear);
    }

    #[test]
    fn cli_parses_runtime_add_path() {
        let cli = Cli::try_parse_from([
            "portbay",
            "runtime",
            "add-path",
            "node",
            "/usr/local/bin/node",
        ])
        .unwrap();
        let Some(Cmd::Runtime(RuntimeCmd::AddPath { lang, path })) = cli.cmd else {
            panic!("expected Runtime::AddPath")
        };
        assert_eq!(lang, "node");
        assert_eq!(path, "/usr/local/bin/node");
    }

    #[test]
    fn cli_parses_runtime_remove_path() {
        let cli = Cli::try_parse_from(["portbay", "runtime", "remove-path", "php", "8.3"]).unwrap();
        let Some(Cmd::Runtime(RuntimeCmd::RemovePath { lang, version })) = cli.cmd else {
            panic!("expected Runtime::RemovePath")
        };
        assert_eq!(lang, "php");
        assert_eq!(version, "8.3");
    }
}
