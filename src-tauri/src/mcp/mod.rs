//! PortBay MCP server.
//!
//! Exposes PortBay's project-management surface to any MCP-aware agent
//! (Claude Code, Cursor, Continue, Zed, …) over stdio. The agent spawns
//! `portbay-mcp` as a subprocess; the process boundary is the trust
//! boundary, so there's no extra auth layer.
//!
//! Layering:
//! - [`types`]  — agent-facing input/output schemas (JSON Schema via schemars).
//! - [`ops`]    — the actual work, over `portbay_lib` primitives. No `rmcp`.
//! - [`server`] — `rmcp` adapter: tools, resources, capabilities, instructions.
//!
//! Only compiled when the `mcp` feature is enabled.

use std::path::PathBuf;

use rmcp::{transport::stdio, ServiceExt};

pub mod ops;
pub mod recipes;
pub mod server;
pub mod types;

pub use ops::McpContext;
pub use server::{McpConfig, PortbayMcp, ToolGroup};

/// Build the server and serve it over stdio until the client disconnects.
///
/// `registry_override` / `pc_port_override` mirror the CLI's `--registry` /
/// `--pc-port` flags; pass `None` to use the defaults (and the
/// `PORTBAY_PC_PORT` env var for the port). `config` scopes the tool surface
/// (read-only mode, toolsets).
///
/// All diagnostic logging MUST go to stderr — stdout is the JSON-RPC channel.
pub async fn run(
    registry_override: Option<PathBuf>,
    pc_port_override: Option<u16>,
    config: McpConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let ctx = McpContext::new(registry_override, pc_port_override)?;
    let service = PortbayMcp::with_config(ctx, config).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
