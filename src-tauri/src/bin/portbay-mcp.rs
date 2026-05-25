//! PortBay MCP server — `portbay-mcp`.
//!
//! Speaks Model Context Protocol over stdio so any MCP-aware agent can drive
//! PortBay: register projects, start/stop them, read logs, diagnose failures.
//! Agents spawn this as a subprocess; see `docs-site/agents/` for per-platform
//! config snippets.
//!
//! Logging goes to STDERR only — stdout is the JSON-RPC transport and must
//! carry nothing else.

#![cfg_attr(not(debug_assertions), windows_subsystem = "console")]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use portbay_lib::mcp::{McpConfig, ToolGroup};

#[derive(Parser, Debug)]
#[command(
    name = "portbay-mcp",
    version,
    about = "PortBay MCP server — drive PortBay from an MCP-aware AI agent over stdio.",
    long_about = "PortBay MCP server. Speaks Model Context Protocol over stdio so agents like \
                  Claude Code, Cursor, Continue, and Zed can register projects, start/stop them, \
                  read logs, and diagnose failures.\n\n\
                  Every flag has an environment-variable equivalent (env takes precedence):\n  \
                  --read-only        PORTBAY_MCP_READ_ONLY=1\n  \
                  --toolsets <list>  PORTBAY_MCP_TOOLSETS=projects,diagnostics\n  \
                  --pc-port <port>   PORTBAY_PC_PORT=9999"
)]
struct Cli {
    /// Override the registry file location (defaults to PortBay's data dir).
    #[arg(long, value_name = "PATH")]
    registry: Option<PathBuf>,

    /// Override the Process Compose daemon port (defaults to the
    /// `PORTBAY_PC_PORT` env var, then 9999).
    #[arg(long, value_name = "PORT")]
    pc_port: Option<u16>,

    /// Expose only inspection tools — disable everything that mutates state
    /// (add/update/remove/start/stop/import/export/scaffold). Safe default for
    /// "let the agent look but not touch". Env: `PORTBAY_MCP_READ_ONLY=1`.
    #[arg(long)]
    read_only: bool,

    /// Comma-separated tool groups to expose: `projects`, `lifecycle`,
    /// `diagnostics`, `scaffold`, or `all`. Defaults to all. Env:
    /// `PORTBAY_MCP_TOOLSETS`.
    #[arg(long, value_name = "LIST")]
    toolsets: Option<String>,

    /// Log verbosity for stderr diagnostics (error, warn, info, debug, trace).
    #[arg(long, default_value = "info")]
    log_level: String,
}

/// Read a boolean env var: set + not "0"/"false"/"no"/"" counts as true.
fn env_bool(key: &str) -> Option<bool> {
    std::env::var(key).ok().map(|v| {
        !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no" | "off"
        )
    })
}

fn build_config(cli: &Cli) -> Result<McpConfig, String> {
    // Env takes precedence over the flag, matching the GitHub MCP server.
    let read_only = env_bool("PORTBAY_MCP_READ_ONLY").unwrap_or(cli.read_only);

    let toolsets = match std::env::var("PORTBAY_MCP_TOOLSETS").ok() {
        Some(s) => ToolGroup::parse_list(&s)?,
        None => match &cli.toolsets {
            Some(s) => ToolGroup::parse_list(s)?,
            None => ToolGroup::all(),
        },
    };

    Ok(McpConfig {
        read_only,
        toolsets,
    })
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Diagnostics → stderr. stdout is reserved for the MCP JSON-RPC stream.
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| format!("portbay={}", cli.log_level));
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .with_ansi(false)
        .init();

    let config = match build_config(&cli) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("portbay-mcp: {e}");
            return ExitCode::from(2);
        }
    };

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("portbay-mcp: failed to start async runtime: {e}");
            return ExitCode::from(1);
        }
    };

    match rt.block_on(portbay_lib::mcp::run(cli.registry, cli.pc_port, config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("portbay-mcp: {e}");
            ExitCode::from(1)
        }
    }
}
