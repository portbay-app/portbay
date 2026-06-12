//! Web-server overview IPC surface.
//!
//! Backs the `/web-servers` page. PortBay's web-server model is not a set of
//! globally-installed services (the ServBay shape); instead **Caddy** is the
//! always-on edge — host routing, TLS, reverse proxy — and **Nginx / Apache**
//! are per-PHP-project loopback backends whose configs PortBay generates at
//! reconcile time (see `crate::webservers`). This command surfaces that
//! reality: for each server, what role it plays, whether its binary is
//! present, its version, and which projects currently use it.
//!
//! There is intentionally no global per-server config here (ports, modules,
//! server root) — those are derived per project, so a global form would be
//! fiction. The one writable knob, "default for new PHP projects", lives in
//! `Preferences::default_web_server` and is set via `set_preferences`.

use std::path::Path;
use std::process::Command;

use serde::Serialize;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::registry::{store, ProjectType, WebServer};
use crate::state::AppState;
use crate::webservers::{apache_binary, nginx_binary};

/// One project that currently resolves to a given web server.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebServerProjectRef {
    pub id: String,
    pub name: String,
}

/// A single web server's status as shown on the `/web-servers` page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebServerInfo {
    /// Stable id matching `WebServer::id` — "caddy" | "nginx" | "apache".
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// One-line description of the role this server plays in PortBay.
    pub role: &'static str,
    /// True for Caddy — PortBay's public edge. The others are loopback-only.
    pub edge: bool,
    /// True when PortBay ships the binary (Caddy). Nginx/Apache are detected
    /// from the system, never bundled.
    pub bundled: bool,
    /// Whether the binary is available (bundled, or detected on disk).
    pub installed: bool,
    /// Resolved binary path, when one was found on disk. `None` for the
    /// bundled Caddy sidecar (resolved by Tauri at runtime, not a fixed path).
    pub binary_path: Option<String>,
    /// Best-effort version string parsed from `<bin> -v` / `-version`.
    pub version: Option<String>,
    /// PHP projects that currently resolve to this server.
    pub projects: Vec<WebServerProjectRef>,
    /// True when this server is the default for newly-added PHP projects.
    pub is_default: bool,
}

/// Per-server snapshot for the Web Server page.
///
/// Reads the registry (project → server mapping) and the user's default-server
/// preference. Caddy's *live* running state is not included here — the page
/// overlays it from the sidecar-health store so this stays a cheap, pure read
/// that the web demo can mock without a daemon.
#[tauri::command]
pub async fn webserver_overview(state: State<'_, AppState>) -> AppResult<Vec<WebServerInfo>> {
    let registry = store::load_or_default(&state.registry_path, &state.domain_suffix)?;
    let default = state
        .preferences_snapshot()
        .default_web_server
        .unwrap_or(WebServer::Caddy);

    // Group PHP projects by the server they effectively resolve to. Non-PHP
    // projects are proxied by Caddy as the edge but don't "use" a PHP web
    // server backend, so they're excluded from the per-server lists.
    let projects_for = |server: WebServer| -> Vec<WebServerProjectRef> {
        registry
            .list_projects()
            .iter()
            .filter(|p| p.kind == ProjectType::Php && p.web_server_effective() == server)
            .map(|p| WebServerProjectRef {
                id: p.id.as_str().to_string(),
                name: p.name.clone(),
            })
            .collect()
    };

    // Binary discovery walks the filesystem and the version probe spawns
    // `nginx -v` / `httpd -v` — blocking work, so it runs off the shared
    // async workers.
    let (nginx_bin, apache_bin, nginx_version, apache_version) =
        tokio::task::spawn_blocking(|| {
            let nginx_bin = nginx_binary();
            let apache_bin = apache_binary();
            let nginx_version = nginx_bin.as_deref().and_then(|p| binary_version(p, "-v"));
            let apache_version = apache_bin.as_deref().and_then(|p| binary_version(p, "-v"));
            (nginx_bin, apache_bin, nginx_version, apache_version)
        })
        .await
        .map_err(|e| AppError::Internal(format!("web-server probe task failed: {e}")))?;

    Ok(vec![
        WebServerInfo {
            id: "caddy",
            name: "Caddy",
            role: "Edge router — maps your project hostnames to their ports, terminates local HTTPS, and reverse-proxies to Nginx/Apache when a project picks them.",
            edge: true,
            bundled: true,
            installed: true,
            binary_path: None,
            version: None,
            projects: projects_for(WebServer::Caddy),
            is_default: default == WebServer::Caddy,
        },
        WebServerInfo {
            id: "nginx",
            name: "Nginx",
            role: "Per-project PHP backend. PortBay generates the nginx.conf (FastCGI to PHP-FPM) and Caddy reverse-proxies the hostname to it.",
            edge: false,
            bundled: false,
            installed: nginx_bin.is_some(),
            binary_path: nginx_bin.as_ref().map(|p| p.to_string_lossy().into_owned()),
            version: nginx_version,
            projects: projects_for(WebServer::Nginx),
            is_default: default == WebServer::Nginx,
        },
        WebServerInfo {
            id: "apache",
            name: "Apache",
            role: "Per-project PHP backend. PortBay generates the httpd.conf (mod_proxy_fcgi to PHP-FPM) and Caddy reverse-proxies the hostname to it.",
            edge: false,
            bundled: false,
            installed: apache_bin.is_some(),
            binary_path: apache_bin
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            version: apache_version,
            projects: projects_for(WebServer::Apache),
            is_default: default == WebServer::Apache,
        },
    ])
}

/// Run `<bin> <arg>` and pull a version token out of the output.
///
/// Both `nginx -v` (stderr: `nginx version: nginx/1.27.0`) and `httpd -v`
/// (stdout: `Server version: Apache/2.4.58 (Unix)`) embed the version after a
/// `/`. We scan combined stdout+stderr for the first `name/x.y[.z]` token and
/// return the numeric part. Best-effort: any failure yields `None` so a
/// missing/odd binary never breaks the page.
fn binary_version(bin: &Path, arg: &str) -> Option<String> {
    let output = Command::new(bin).arg(arg).output().ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push(' ');
    text.push_str(&String::from_utf8_lossy(&output.stderr));

    text.split_whitespace()
        .filter_map(|tok| tok.split_once('/'))
        .map(|(_, ver)| ver)
        .find(|ver| ver.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|ver| ver.trim_end_matches([',', ';', ')']).to_string())
}
