//! Translates a `Registry` into a Caddy admin-API config document.
//!
//! Two servers are emitted: HTTPS-terminating projects (`https: true`) on
//! the TLS port, and plain-HTTP projects (`https: false`) on `:80`. The
//! registry's `hostname` field is the routing key for both; an https
//! project additionally gets an http→https redirect on `:80`. Per-project
//! cert paths come from the mkcert wrapper (kanban card P1 #4); the config
//! generator accepts a `cert_lookup` callback so it can compose without
//! depending on the mkcert module directly.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use serde_json::json;

use crate::caddy::error::Result;
use crate::caddy::types::{
    AdminConfig, AppsConfig, AutomaticHttps, CaddyConfig, HttpApp, MatchClause, Route, Server,
    ServerErrors, TlsApp, TlsCertFile, TlsCertificates,
};
use crate::registry::{Project, ProjectType, Registry, WebServer};

/// Caddy logger name our per-server access logs emit under. The HTTP request
/// inspector tails the file this logger writes.
pub const ACCESS_LOGGER: &str = "portbay_access";

/// Filename (under the logs dir) Caddy writes the JSON access log to.
pub const ACCESS_LOG_FILE: &str = "caddy-access.log";

/// Per-project TLS cert pair, looked up by the caller.
#[derive(Debug, Clone)]
pub struct CertPaths {
    pub certificate: PathBuf,
    pub key: PathBuf,
}

/// PortBay's "site isn't responding yet" page. Served by the catch-all route
/// (unknown host) and by the error subroute (a known host whose dev server is
/// still starting up or stopped). Self-contained — no external assets — and
/// auto-refreshes so the page flips to the real app the moment it's ready.
const PLACEHOLDER_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="refresh" content="3">
<title>Starting up · PortBay</title>
<style>
  :root { color-scheme: dark; }
  * { box-sizing: border-box; }
  html, body { height: 100%; margin: 0; }
  body {
    font: 15px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    color: #e7ecf3;
    background: radial-gradient(1200px 600px at 50% -10%, #16263b 0%, #0b1118 55%, #070b10 100%);
    display: grid; place-items: center; text-align: center; padding: 24px;
  }
  .card { max-width: 480px; }
  .mark { display: inline-flex; align-items: center; gap: 10px; margin-bottom: 26px; }
  .mark svg { width: 34px; height: 34px; }
  .mark span { font-size: 17px; font-weight: 600; letter-spacing: -0.01em; }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: #36d399;
         box-shadow: 0 0 0 0 rgba(54,211,153,.6); animation: pulse 1.6s infinite; display: inline-block; }
  @keyframes pulse { 0% { box-shadow: 0 0 0 0 rgba(54,211,153,.55); } 70% { box-shadow: 0 0 0 12px rgba(54,211,153,0); } 100% { box-shadow: 0 0 0 0 rgba(54,211,153,0); } }
  h1 { font-size: 22px; font-weight: 650; letter-spacing: -0.02em; margin: 0 0 10px; }
  p { margin: 0 0 8px; color: #9fb0c3; }
  .hint { font-size: 13px; color: #6b7d92; margin-top: 18px; }
  .foot { margin-top: 34px; font-size: 12px; color: #5a6b80; letter-spacing: .02em; }
  code { background: #ffffff10; padding: 1px 6px; border-radius: 6px; font-size: 13px; }
</style>
</head>
<body>
  <div class="card">
    <div class="mark">
      <svg viewBox="0 0 24 24" fill="none" stroke="#36d399" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M12 2.5 9.5 9h5L12 2.5Z"/><path d="M9.7 9h4.6l1.2 11.5H8.5L9.7 9Z"/><path d="M7 20.5h10"/><path d="M14.8 6.2l3 1.2M9.2 6.2l-3 1.2"/>
      </svg>
      <span>PortBay</span>
    </div>
    <h1><span class="dot"></span>&nbsp; Waking up your site</h1>
    <p>PortBay is connecting to this project.</p>
    <p>This page refreshes automatically — it'll switch to your app as soon as the dev server responds.</p>
    <p class="hint">If it doesn't load, the project may be stopped. Start it from the PortBay app, then come back here.</p>
    <div class="foot">Served locally by PortBay</div>
  </div>
</body>
</html>
"##;

/// `static_response` handler that serves [`PLACEHOLDER_HTML`] with a 503 +
/// short `Retry-After`, so clients (and our auto-refresh) retry quickly.
fn placeholder_handler() -> serde_json::Value {
    json!({
        "handler": "static_response",
        "status_code": 503,
        "headers": {
            "Content-Type": ["text/html; charset=utf-8"],
            "Cache-Control": ["no-store"],
            "Retry-After": ["3"]
        },
        "body": PLACEHOLDER_HTML
    })
}

/// Catch-all route (no host matcher → matches everything) that serves the
/// placeholder. Goes last in a server's route list.
fn placeholder_route(id: &str) -> Route {
    Route {
        id: id.to_string(),
        match_: vec![],
        handle: vec![placeholder_handler()],
        terminal: true,
    }
}

/// Error subroute that serves the placeholder when an upstream errors (e.g. a
/// dev server that's still starting up answers no connection).
fn server_errors(error_route_id: &str) -> ServerErrors {
    ServerErrors {
        routes: vec![Route {
            id: error_route_id.to_string(),
            match_: vec![],
            handle: vec![placeholder_handler()],
            terminal: true,
        }],
    }
}

/// On the `:80` server, send an https project's host straight to https so the
/// browser lands on the TLS listener.
fn https_redirect_route(p: &Project) -> Route {
    Route {
        id: format!("redirect_{}", p.id),
        match_: vec![MatchClause {
            host: vec![p.hostname.clone()],
        }],
        handle: vec![json!({
            "handler": "static_response",
            "status_code": 308,
            "headers": { "Location": ["https://{http.request.host}{http.request.uri}"] }
        })],
        terminal: true,
    }
}

/// Minimal admin-only config used to bring Caddy up at app start.
///
/// One server named `portbay` listens on `https_port` with no routes. The
/// admin endpoint is bound to `localhost:<admin_port>` so the reconcile
/// loop can push the real registry-derived config once projects exist.
///
/// Quirk-1 (see `claudedocs/spike-caddy.md`) is honoured: `http_port: 0`
/// and `automatic_https.disable_redirects: true` keep Caddy off `:80`.
pub fn bootstrap_config(admin_port: u16, https_port: u16) -> CaddyConfig {
    let mut servers = BTreeMap::new();
    servers.insert(
        "portbay".to_string(),
        Server {
            listen: vec![format!(":{https_port}")],
            routes: vec![],
            automatic_https: AutomaticHttps {
                disable_redirects: true,
                disable: true,
            },
            errors: None,
            logs: None,
        },
    );

    CaddyConfig {
        admin: AdminConfig {
            listen: format!("localhost:{admin_port}"),
        },
        apps: AppsConfig {
            http: HttpApp {
                http_port: 0,
                servers,
            },
            tls: TlsApp {
                certificates: TlsCertificates { load_files: vec![] },
            },
        },
        logging: None,
    }
}

/// Build the full Caddy config document from a registry.
///
/// `https_port` is the port the public-facing server listens on (default
/// `:443`; PortBay falls back to `:8443` when 443 is held — see
/// `caddy::lifecycle::find_free_port`).
///
/// `cert_lookup(project_id) -> Option<CertPaths>` lets the caller plug in
/// the mkcert wrapper without us depending on it.
///
/// No projects are considered actively shared — equivalent to calling
/// [`build_config_filtered`] with an empty `shared_project_ids` and empty
/// `suppressed` set.
pub fn build_config<F>(
    reg: &Registry,
    admin_port: u16,
    http_port: u16,
    https_port: u16,
    php_socket_dir: &std::path::Path,
    cert_lookup: F,
) -> Result<CaddyConfig>
where
    F: Fn(&str) -> Option<CertPaths>,
{
    build_config_filtered(
        reg,
        admin_port,
        http_port,
        https_port,
        php_socket_dir,
        &HashSet::new(),
        &HashSet::new(),
        cert_lookup,
    )
}

/// Like [`build_config`] but omits the routes of any project id in
/// `suppressed`, and enables full Origin/Host header normalisation for any
/// project id in `shared_project_ids` (those with an active Cloudflare
/// tunnel).
///
/// The reconciler uses `suppressed` to drop `expose_when_running` projects
/// that aren't currently up, so their hostname stops claiming the edge until
/// the process is back.
///
/// `shared_project_ids` is the set of project ids that currently have an
/// active tunnel. For those projects `normalize_all = true` is passed to
/// `project_to_route`, which adds Origin/Host/X-Forwarded-Host rewrites on
/// plain (non-WebSocket) requests so Next.js `allowedDevOrigins` and Vite
/// `allowedHosts` accept traffic arriving via the public tunnel URL. For all
/// other projects the output is **byte-for-byte identical** to the no-tunnel
/// path.
pub fn build_config_filtered<F>(
    reg: &Registry,
    admin_port: u16,
    http_port: u16,
    https_port: u16,
    php_socket_dir: &std::path::Path,
    suppressed: &HashSet<String>,
    shared_project_ids: &HashSet<String>,
    cert_lookup: F,
) -> Result<CaddyConfig>
where
    F: Fn(&str) -> Option<CertPaths>,
{
    // Two servers: HTTPS-terminating projects on `https_port` (:443), plain
    // HTTP projects on `http_port` (:80). Each ends with a catch-all that
    // serves PortBay's placeholder, and an error subroute that serves the
    // same page when a routed-but-not-yet-running upstream refuses the
    // connection. https projects also get an http→https redirect on :80.
    let mut https_routes: Vec<Route> = Vec::new();
    let mut http_routes: Vec<Route> = Vec::new();
    let mut cert_files: Vec<TlsCertFile> = Vec::new();

    for p in &reg.projects {
        // `expose_when_running` projects that aren't up are skipped entirely,
        // so their hostname falls through to PortBay's catch-all rather than
        // claiming the edge while the app is down.
        if suppressed.contains(p.id.as_str()) {
            continue;
        }
        // Enable full header normalisation only for projects with an active
        // tunnel. The no-tunnel path is byte-for-byte identical to before.
        let normalize_all = shared_project_ids.contains(p.id.as_str());
        if p.https {
            https_routes.push(project_to_route(p, php_socket_dir, reg, normalize_all));
            // While a project is actively shared over a tunnel, also serve it on
            // :80 (with normalisation) instead of the usual https-redirect, so
            // cloudflared can reach the origin over plain HTTP. Dialing Caddy's
            // TLS port by IP can't send SNI, so Caddy has no cert to present and
            // the handshake fails — routing the share through :80 sidesteps that
            // entirely. Local https access still works via the :8443 route above.
            // No active tunnel ⇒ the redirect, so the no-tunnel output is unchanged.
            if normalize_all {
                http_routes.push(project_to_route(p, php_socket_dir, reg, true));
            } else {
                http_routes.push(https_redirect_route(p));
            }
            if let Some(paths) = cert_lookup(p.id.as_str()) {
                cert_files.push(TlsCertFile {
                    certificate: paths.certificate,
                    key: paths.key,
                    tags: vec![format!("project:{}", p.id)],
                });
            }
        } else {
            http_routes.push(project_to_route(p, php_socket_dir, reg, normalize_all));
        }
    }

    https_routes.push(placeholder_route("route_fallback_https"));
    http_routes.push(placeholder_route("route_fallback_http"));

    let mut servers = BTreeMap::new();
    servers.insert(
        "portbay".to_string(),
        Server {
            listen: vec![format!(":{https_port}")],
            routes: https_routes,
            automatic_https: AutomaticHttps {
                disable_redirects: true,
                disable: true,
            },
            errors: Some(server_errors("route_error_https")),
            logs: None,
        },
    );
    servers.insert(
        "portbay_http".to_string(),
        Server {
            listen: vec![format!(":{http_port}")],
            routes: http_routes,
            automatic_https: AutomaticHttps {
                disable_redirects: true,
                disable: true,
            },
            errors: Some(server_errors("route_error_http")),
            logs: None,
        },
    );

    Ok(CaddyConfig {
        admin: AdminConfig {
            listen: format!("localhost:{admin_port}"),
        },
        apps: AppsConfig {
            http: HttpApp {
                // We bind :80 ourselves via the `portbay_http` server, so keep
                // Caddy's automatic-HTTP machinery off.
                http_port: 0,
                servers,
            },
            tls: TlsApp {
                certificates: TlsCertificates {
                    load_files: cert_files,
                },
            },
        },
        logging: None,
    })
}

/// Enable a JSON access log on every server in `cfg`, writing to `log_path`.
///
/// Each server gets `logs.default_logger_name = ACCESS_LOGGER`, and the
/// top-level `logging` block routes that access namespace to a file (JSON
/// encoder), excluding it from Caddy's default stderr log. The HTTP request
/// inspector tails `log_path`. Applied by the reconciler after `build_config`
/// so the access log survives every `/load`.
pub fn with_access_log(mut cfg: CaddyConfig, log_path: &std::path::Path) -> CaddyConfig {
    let server_logs = json!({ "default_logger_name": ACCESS_LOGGER });
    for server in cfg.apps.http.servers.values_mut() {
        server.logs = Some(server_logs.clone());
    }

    let access_ns = format!("http.log.access.{ACCESS_LOGGER}");
    let mut logs = serde_json::Map::new();
    // Keep the access entries out of Caddy's default (stderr) log…
    logs.insert(
        "default".to_string(),
        json!({ "exclude": [access_ns.clone()] }),
    );
    // …and route them to our JSON file instead.
    logs.insert(
        ACCESS_LOGGER.to_string(),
        json!({
            "writer": { "output": "file", "filename": log_path.to_string_lossy() },
            "encoder": { "format": "json" },
            "include": [access_ns],
        }),
    );
    cfg.logging = Some(json!({ "logs": logs }));
    cfg
}

/// Build a single route for a project. Used both by `build_config` and by
/// runtime `append_route` calls after a project is added live.
///
/// `php_socket_dir` is the parent directory under which PortBay
/// expects per-version FPM sockets at `<dir>/<version>/php-fpm.sock`
/// (matching [`crate::php::lifecycle::fpm_socket_path`]).
///
/// `normalize_all` — when `true` the plain (non-WebSocket) reverse-proxy
/// route also rewrites `Origin`, `Host`, and `X-Forwarded-Host` to the
/// loopback dev origin. Set to `true` only for projects with an active
/// Cloudflare tunnel so that Next.js/Vite `allowedDevOrigins`/
/// `allowedHosts` checks are satisfied for the duration of a share.
/// When `false` the output is byte-for-byte identical to the pre-tunnel
/// path (CSRF intact, plain requests untouched).
pub fn project_to_route(
    p: &Project,
    php_socket_dir: &std::path::Path,
    reg: &Registry,
    normalize_all: bool,
) -> Route {
    let id = format!("route_{}", p.id);
    let handler = build_handler(p, php_socket_dir, reg, normalize_all);
    // Pro: wrap in a CORS subroute when the project has an active policy. The
    // basic listen port is never gated; only this custom cross-origin policy
    // is (see `Project::cors`). Inactive/absent → the handler is untouched, so
    // free projects behave exactly as before.
    let handler = match &p.cors {
        Some(cors) if cors.is_active() => cors_wrap(handler, cors),
        _ => handler,
    };
    // Path Prefix (Domains page): serve the app under a sub-path, stripping the
    // prefix before it reaches the upstream so the app still sees `/`. `None`
    // for the default root path, so unprefixed projects are untouched.
    let handler = match p.path_prefix() {
        Some(prefix) => path_prefix_wrap(handler, prefix),
        None => handler,
    };
    // Wildcard subdomains (Domains page): also match `*.hostname`. The cert
    // SAN and (where needed) the resolver are handled in the cert/hosts
    // reconcilers; here we just widen the host matcher.
    let mut hosts = vec![p.hostname.clone()];
    if p.include_wildcard_subdomains() {
        hosts.push(format!("*.{}", p.hostname));
    }
    Route {
        id,
        match_: vec![MatchClause { host: hosts }],
        handle: vec![handler],
        terminal: true,
    }
}

/// Wrap a handler so the host only serves requests under `prefix`, stripping
/// that prefix before the inner handler runs (Caddy's `handle_path` semantics:
/// a path match plus `rewrite`/`strip_path_prefix`). Requests outside the
/// prefix fall through to nothing and get PortBay's catch-all. `prefix` is
/// normalised to a single leading slash and no trailing slash.
fn path_prefix_wrap(inner: serde_json::Value, prefix: &str) -> serde_json::Value {
    let trimmed = prefix.trim().trim_end_matches('/');
    let norm = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    };
    json!({
        "handler": "subroute",
        "routes": [{
            "match": [{ "path": [norm.clone(), format!("{norm}/*")] }],
            "handle": [
                { "handler": "rewrite", "strip_path_prefix": norm },
                inner
            ]
        }]
    })
}

/// Wrap a project's handler in a CORS subroute (Pro). For requests whose
/// `Origin` is in the allowlist it echoes that origin into
/// `Access-Control-Allow-Origin` (plus `Vary: Origin`, and credentials when
/// configured) and answers preflight `OPTIONS` with `204`; all other requests
/// fall straight through to `inner` with no added headers. Echoing the exact
/// matched origin keeps an allowlist honest — never a blanket `*`.
fn cors_wrap(inner: serde_json::Value, cors: &crate::registry::CorsConfig) -> serde_json::Value {
    let origins = cors.allowed_origins.clone();

    let mut preflight = json!({
        "Access-Control-Allow-Origin": ["{http.request.header.Origin}"],
        "Access-Control-Allow-Methods": ["GET, POST, PUT, PATCH, DELETE, OPTIONS"],
        "Access-Control-Allow-Headers": ["{http.request.header.Access-Control-Request-Headers}"],
        "Access-Control-Max-Age": ["86400"],
        "Vary": ["Origin"]
    });
    let mut actual_set = json!({
        "Access-Control-Allow-Origin": ["{http.request.header.Origin}"],
        "Vary": ["Origin"]
    });
    if cors.allow_credentials {
        preflight["Access-Control-Allow-Credentials"] = json!(["true"]);
        actual_set["Access-Control-Allow-Credentials"] = json!(["true"]);
    }

    json!({
        "handler": "subroute",
        "routes": [
            // Preflight: an allow-listed OPTIONS gets a 204 with the CORS
            // headers and goes no further.
            {
                "match": [{ "method": ["OPTIONS"], "header": { "Origin": origins } }],
                "handle": [{
                    "handler": "static_response",
                    "status_code": 204,
                    "headers": preflight
                }],
                "terminal": true
            },
            // Actual requests from an allow-listed origin: register the
            // response headers (non-terminal middleware), then fall through.
            {
                "match": [{ "header": { "Origin": origins } }],
                "handle": [{
                    "handler": "headers",
                    "response": { "set": actual_set }
                }]
            },
            // Everything else (and the fall-through above) hits the real handler.
            { "handle": [inner] }
        ]
    })
}

fn build_handler(
    p: &Project,
    php_socket_dir: &std::path::Path,
    reg: &Registry,
    normalize_all: bool,
) -> serde_json::Value {
    match p.kind {
        // PHP can run in two modes:
        // - command + port: a framework/router dev server that PortBay starts
        //   through Process Compose, then reverse-proxies to;
        // - no command: a pure Caddy/PHP-FPM site served from disk.
        ProjectType::Php
            if (p.start_command.is_some()
                || matches!(
                    p.web_server_effective(),
                    WebServer::Nginx | WebServer::Apache
                ))
                && p.port.is_some() =>
        {
            reverse_proxy_handler(p, normalize_all)
        }
        ProjectType::Php => php_handler(p, php_socket_dir, reg),
        // Static sites have no dev server — serve their files straight off
        // disk. Routing them through reverse_proxy (the old `_` arm) dialed
        // 127.0.0.1:80, i.e. Caddy itself, and never served anything.
        ProjectType::Static => static_handler(p),
        _ => reverse_proxy_handler(p, normalize_all),
    }
}

/// `file_server` handler rooted at the project's document root (or its path
/// when no doc root is set). Serves a Static project's files directly, with the
/// usual index fallbacks so `/` lands on `index.html`.
fn static_handler(p: &Project) -> serde_json::Value {
    let root = p
        .document_root
        .as_deref()
        .map(|d| p.path.join(d))
        .unwrap_or_else(|| p.path.clone());
    json!({
        "handler": "file_server",
        "root": root.to_string_lossy().into_owned(),
        "index_names": ["index.html", "index.htm"]
    })
}

/// Build the `reverse_proxy` subroute handler for a dev-server project.
///
/// `normalize_all` controls whether plain (non-WebSocket) requests also
/// have their `Origin`, `Host`, and `X-Forwarded-Host` rewritten to the
/// loopback dev origin.
///
/// **`normalize_all = false` (default, no active tunnel):**
/// - WebSocket upgrades get `Origin` rewritten (HMR fix, unchanged).
/// - Plain requests are forwarded as-is (CSRF intact).
/// Output is byte-for-byte identical to the pre-tunnel implementation.
///
/// **`normalize_all = true` (project has an active Cloudflare tunnel):**
/// - WebSocket upgrades: same as above.
/// - Plain requests: also rewrite `Origin`, `Host`, `X-Forwarded-Host` →
///   loopback, so Next.js `allowedDevOrigins` and Vite `allowedHosts`
///   accept tunnel traffic. Scoped to the active-share window only.
fn reverse_proxy_handler(p: &Project, normalize_all: bool) -> serde_json::Value {
    let port = p.port.unwrap_or(80);
    let dial = format!("127.0.0.1:{port}");
    let loopback_origin = format!("http://localhost:{port}");
    let loopback_host = format!("localhost:{port}");

    // Dev-server HMR / live-reload WebSockets: Next.js (via `allowedDevOrigins`),
    // Vite, and webpack-dev-server all reject upgrade requests whose `Origin`
    // isn't a loopback dev origin. Behind PortBay's pretty hostname the browser
    // sends `Origin: http(s)://<host>`, so the upstream closes the socket and
    // HMR dies — the page loads, but live-reload spins forever. Rewrite `Origin`
    // to the loopback dev origin for WebSocket upgrades ONLY (matched on the
    // `Upgrade: websocket` header), so HMR works with zero per-project config.
    // Plain (non-upgrade) requests keep their real `Origin` when normalize_all
    // is false, so app-level CSRF/CORS is left untouched. When normalize_all
    // is true (active tunnel), we also rewrite Origin/Host/X-Forwarded-Host on
    // plain requests so the dev server's cross-origin checks accept tunnel traffic.
    let plain_proxy = if normalize_all {
        json!({
            "handle": [{
                "handler": "reverse_proxy",
                "headers": { "request": { "set": {
                    "Origin": [loopback_origin],
                    "Host": [loopback_host.clone()],
                    "X-Forwarded-Host": [loopback_host]
                }}},
                "upstreams": [{ "dial": &dial }]
            }]
        })
    } else {
        json!({
            "handle": [{
                "handler": "reverse_proxy",
                "upstreams": [{ "dial": &dial }]
            }]
        })
    };

    json!({
        "handler": "subroute",
        "routes": [
            {
                "match": [{ "header": { "Upgrade": ["websocket"] } }],
                "handle": [{
                    "handler": "reverse_proxy",
                    "headers": { "request": { "set": { "Origin": [format!("http://localhost:{port}")] } } },
                    "upstreams": [{ "dial": &dial }]
                }]
            },
            plain_proxy
        ]
    })
}

fn php_handler(p: &Project, php_socket_dir: &std::path::Path, reg: &Registry) -> serde_json::Value {
    let doc_root = p
        .document_root
        .as_deref()
        .map(|d| p.path.join(d))
        .unwrap_or_else(|| p.path.clone());
    let doc_root_str = doc_root.to_string_lossy().into_owned();

    // Match `crate::php::lifecycle::fpm_socket_path` exactly. The version is
    // resolved through `php_version_effective` (runtime pin first, legacy
    // `php_version` fallback) — the same source the FPM-pool reconciler uses,
    // so Caddy never dials a socket the reconciler didn't spawn. When a project
    // has no version we fall back to a sentinel directory under the same parent
    // so a future "default PHP" lookup still finds the socket via the scheme.
    let version = p.php_version_effective().unwrap_or("default");
    let socket_path = php_socket_dir.join(version).join("php-fpm.sock");
    let tuning = reg
        .runtimes
        .php
        .get(version)
        .map(|cfg| &cfg.fpm)
        .cloned()
        .unwrap_or_default();
    let php_socket = crate::php::lifecycle::fpm_fastcgi_dial(&tuning, &socket_path);

    // Nested subroute:
    // 1. rewrite extensionless/directory URLs to a project front controller
    //    when present (`public/router.php` or `index.php`);
    // 2. execute PHP through FastCGI;
    // 3. serve static assets from the document root.
    json!({
        "handler": "subroute",
        "routes": [
            {
                "handle": [{
                    "handler": "rewrite",
                    "try_files": [
                        "{http.request.uri.path}/index.php",
                        "{http.request.uri.path}",
                        "/router.php",
                        "/index.php"
                    ]
                }]
            },
            {
                "match": [{ "path": ["*.php"] }],
                "handle": [{
                    "handler": "reverse_proxy",
                    "transport": { "protocol": "fastcgi", "split_path": [".php"] },
                    "upstreams": [{ "dial": php_socket }]
                }]
            },
            {
                "handle": [{
                    "handler": "file_server",
                    "root": doc_root_str,
                    "index_names": ["index.php", "index.html"]
                }]
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Project, ProjectId, ProjectType, Registry};
    use std::path::Path;
    use std::path::PathBuf;

    fn next_project(id: &str, port: u16, https: bool) -> Project {
        Project {
            id: ProjectId::new(id),
            name: id.into(),
            path: PathBuf::from(format!("/tmp/{id}")),
            kind: ProjectType::Next,
            start_command: Some("pnpm dev".into()),
            port: Some(port),
            extra_ports: vec![],
            hostname: format!("{id}.test"),
            https,
            services: vec!["caddy".into()],
            env: Default::default(),
            readiness: None,
            auto_start: false,
            tags: vec![],
            document_root: None,
            php_version: None,
            web_server: None,
            mobile_run: None,
            runtime: None,
            workspace: None,
            cors: None,
            sandbox: None,
            domain: None,
        }
    }

    fn php_project(id: &str, php: &str) -> Project {
        Project {
            id: ProjectId::new(id),
            name: id.into(),
            path: PathBuf::from(format!("/tmp/{id}")),
            kind: ProjectType::Php,
            start_command: None,
            port: None,
            extra_ports: vec![],
            hostname: format!("{id}.test"),
            https: true,
            services: vec!["caddy".into(), "php-fpm".into()],
            env: Default::default(),
            readiness: None,
            auto_start: false,
            tags: vec![],
            document_root: Some("public".into()),
            php_version: Some(php.into()),
            web_server: None,
            mobile_run: None,
            runtime: None,
            workspace: None,
            cors: None,
            sandbox: None,
            domain: None,
        }
    }

    fn no_certs(_: &str) -> Option<CertPaths> {
        None
    }

    #[test]
    fn access_log_wires_every_server_and_routes_to_file() {
        let mut r = Registry::new("test");
        r.add_project(next_project("a", 3010, true)).unwrap();
        let cfg =
            build_config(&r, 2019, 80, 8443, Path::new("/tmp/portbay-php"), no_certs).unwrap();
        let cfg = with_access_log(cfg, Path::new("/tmp/logs/caddy-access.log"));
        let v = serde_json::to_value(&cfg).unwrap();

        // Every HTTP server emits its access log under our logger name.
        let servers = v["apps"]["http"]["servers"].as_object().unwrap();
        assert!(!servers.is_empty());
        for (_name, server) in servers {
            assert_eq!(server["logs"]["default_logger_name"], ACCESS_LOGGER);
        }

        // The top-level logging block routes that namespace to our JSON file
        // and keeps it out of the default (stderr) log.
        let logs = &v["logging"]["logs"];
        assert_eq!(logs[ACCESS_LOGGER]["writer"]["output"], "file");
        assert_eq!(
            logs[ACCESS_LOGGER]["writer"]["filename"],
            "/tmp/logs/caddy-access.log"
        );
        assert_eq!(logs[ACCESS_LOGGER]["encoder"]["format"], "json");
        assert_eq!(
            logs[ACCESS_LOGGER]["include"][0],
            "http.log.access.portbay_access"
        );
        assert_eq!(
            logs["default"]["exclude"][0],
            "http.log.access.portbay_access"
        );
    }

    #[test]
    fn no_cors_policy_leaves_handler_untouched() {
        // A project with no CORS policy must route exactly as before — no CORS
        // headers anywhere in the emitted route.
        let p = next_project("plain", 3010, true);
        let mut r = Registry::new("test");
        r.add_project(p.clone()).unwrap();
        let route = project_to_route(&p, Path::new("/tmp/portbay-php"), &r, false);
        let json = serde_json::to_string(&route).unwrap();
        assert!(!json.contains("Access-Control-Allow-Origin"));
    }

    #[test]
    fn active_cors_policy_wraps_handler_with_allow_origin_and_preflight() {
        let mut p = next_project("api", 3010, true);
        p.cors = Some(crate::registry::CorsConfig {
            allowed_origins: vec!["https://app.example.test".into()],
            allow_credentials: true,
        });
        let mut r = Registry::new("test");
        r.add_project(p.clone()).unwrap();
        let route = project_to_route(&p, Path::new("/tmp/portbay-php"), &r, false);
        let v = serde_json::to_value(&route).unwrap();
        let sub = &v["handle"][0];
        assert_eq!(sub["handler"], "subroute");

        // Route 0 is the preflight: OPTIONS + allow-listed Origin → 204.
        let preflight = &sub["routes"][0];
        assert_eq!(preflight["match"][0]["method"][0], "OPTIONS");
        assert_eq!(
            preflight["match"][0]["header"]["Origin"][0],
            "https://app.example.test"
        );
        assert_eq!(preflight["handle"][0]["status_code"], 204);
        assert_eq!(
            preflight["handle"][0]["headers"]["Access-Control-Allow-Origin"][0],
            "{http.request.header.Origin}"
        );
        assert_eq!(
            preflight["handle"][0]["headers"]["Access-Control-Allow-Credentials"][0],
            "true"
        );

        // Route 1 adds the response header for allow-listed actual requests.
        assert_eq!(sub["routes"][1]["handle"][0]["handler"], "headers");
        assert_eq!(
            sub["routes"][1]["handle"][0]["response"]["set"]["Access-Control-Allow-Origin"][0],
            "{http.request.header.Origin}"
        );

        // Route 2 is the real project handler (the reverse-proxy subroute).
        assert!(sub["routes"][2]["handle"][0].get("handler").is_some());
    }

    #[test]
    fn empty_registry_produces_two_servers_with_only_placeholders() {
        let r = Registry::new("test");
        let c = build_config(&r, 2019, 80, 8443, Path::new("/tmp/portbay-php"), no_certs).unwrap();
        assert_eq!(c.admin.listen, "localhost:2019");
        assert_eq!(c.apps.http.http_port, 0);

        let https = c.apps.http.servers.get("portbay").unwrap();
        assert_eq!(https.listen, vec![":8443".to_string()]);
        // No projects → just the catch-all placeholder route.
        assert_eq!(https.routes.len(), 1);
        assert_eq!(https.routes[0].id, "route_fallback_https");
        assert!(https.routes[0].match_.is_empty());
        assert!(https.automatic_https.disable);
        assert!(https.errors.is_some());

        let http = c.apps.http.servers.get("portbay_http").unwrap();
        assert_eq!(http.listen, vec![":80".to_string()]);
        assert_eq!(http.routes.len(), 1);
        assert_eq!(http.routes[0].id, "route_fallback_http");

        assert!(c.apps.tls.certificates.load_files.is_empty());
    }

    #[test]
    fn placeholder_route_serves_html_status_503() {
        let r = Registry::new("test");
        let c = build_config(&r, 2019, 80, 8443, Path::new("/tmp/portbay-php"), no_certs).unwrap();
        let https = c.apps.http.servers.get("portbay").unwrap();
        let h = &https.routes[0].handle[0];
        assert_eq!(h["handler"], "static_response");
        assert_eq!(h["status_code"], 503);
        assert!(h["body"].as_str().unwrap().contains("PortBay"));
        // The error subroute serves the same placeholder.
        let err = &https.errors.as_ref().unwrap().routes[0].handle[0];
        assert_eq!(err["handler"], "static_response");
    }

    #[test]
    fn https_project_routes_on_443_and_redirects_on_80() {
        let mut r = Registry::new("test");
        r.add_project(next_project("marketing-site", 3010, true))
            .unwrap();
        let lookup = |id: &str| {
            if id == "marketing-site" {
                Some(CertPaths {
                    certificate: PathBuf::from("/c/cert.pem"),
                    key: PathBuf::from("/c/key.pem"),
                })
            } else {
                None
            }
        };
        let c = build_config(&r, 2019, 80, 8443, Path::new("/tmp/portbay-php"), lookup).unwrap();
        let https = c.apps.http.servers.get("portbay").unwrap();
        // project route + catch-all
        assert_eq!(https.routes.len(), 2);
        assert_eq!(https.routes[0].id, "route_marketing-site");
        assert_eq!(https.routes[0].match_[0].host[0], "marketing-site.test");
        // The project handler is a subroute; its non-upgrade branch proxies to
        // the dev server.
        let h = &https.routes[0].handle[0];
        assert_eq!(h["handler"], "subroute");
        assert_eq!(
            h["routes"][1]["handle"][0]["upstreams"][0]["dial"],
            "127.0.0.1:3010"
        );

        // :80 server redirects the https host to https.
        let http = c.apps.http.servers.get("portbay_http").unwrap();
        assert_eq!(http.routes[0].id, "redirect_marketing-site");
        assert_eq!(http.routes[0].handle[0]["status_code"], 308);

        let certs = &c.apps.tls.certificates.load_files;
        assert_eq!(certs.len(), 1);
        assert_eq!(certs[0].certificate, PathBuf::from("/c/cert.pem"));
        assert_eq!(certs[0].tags, vec!["project:marketing-site"]);
    }

    #[test]
    fn http_project_is_routed_on_port_80() {
        let mut r = Registry::new("test");
        r.add_project(next_project("plain", 3010, false)).unwrap();
        let c = build_config(&r, 2019, 80, 8443, Path::new("/tmp/portbay-php"), no_certs).unwrap();

        // https server has only the catch-all.
        let https = c.apps.http.servers.get("portbay").unwrap();
        assert_eq!(https.routes.len(), 1);
        assert_eq!(https.routes[0].id, "route_fallback_https");

        // http server reverse-proxies the plain project via a subroute.
        let http = c.apps.http.servers.get("portbay_http").unwrap();
        assert_eq!(http.routes.len(), 2); // project + catch-all
        assert_eq!(http.routes[0].id, "route_plain");
        assert_eq!(http.routes[0].handle[0]["handler"], "subroute");
        assert_eq!(
            http.routes[0].handle[0]["routes"][1]["handle"][0]["upstreams"][0]["dial"],
            "127.0.0.1:3010"
        );
        // No cert needed for a plain-http project.
        assert!(c.apps.tls.certificates.load_files.is_empty());
    }

    #[test]
    fn reverse_proxy_rewrites_origin_on_websocket_upgrade_only() {
        let mut r = Registry::new("test");
        r.add_project(next_project("hmr", 3010, false)).unwrap();
        let c = build_config(&r, 2019, 80, 8443, Path::new("/tmp/portbay-php"), no_certs).unwrap();
        let http = c.apps.http.servers.get("portbay_http").unwrap();
        let sub = &http.routes[0].handle[0];
        assert_eq!(sub["handler"], "subroute");

        // Branch 0: WebSocket upgrades, matched on the Upgrade header, get
        // Origin rewritten to the loopback dev origin so the dev server's
        // cross-origin HMR guard accepts the connection.
        let ws = &sub["routes"][0];
        assert_eq!(ws["match"][0]["header"]["Upgrade"][0], "websocket");
        assert_eq!(
            ws["handle"][0]["headers"]["request"]["set"]["Origin"][0],
            "http://localhost:3010"
        );
        assert_eq!(ws["handle"][0]["upstreams"][0]["dial"], "127.0.0.1:3010");

        // Branch 1: everything else proxies through with no Origin rewrite.
        let plain = &sub["routes"][1];
        assert!(plain["match"].is_null());
        assert!(plain["handle"][0]["headers"].is_null());
        assert_eq!(plain["handle"][0]["upstreams"][0]["dial"], "127.0.0.1:3010");
    }

    #[test]
    fn php_project_uses_subroute_with_fastcgi() {
        let mut r = Registry::new("test");
        r.add_project(php_project("api-gateway", "8.3")).unwrap();
        let c = build_config(&r, 2019, 80, 8443, Path::new("/tmp/portbay-php"), no_certs).unwrap();
        let s = c.apps.http.servers.get("portbay").unwrap();
        let h = &s.routes[0].handle[0];
        assert_eq!(h["handler"], "subroute");
        let sub_routes = &h["routes"];
        // First sub-route: front-controller rewrite.
        assert_eq!(sub_routes[0]["handle"][0]["handler"], "rewrite");
        assert_eq!(sub_routes[0]["handle"][0]["try_files"][2], "/router.php");
        // Second sub-route: *.php → FastCGI.
        assert_eq!(sub_routes[1]["match"][0]["path"][0], "*.php");
        assert_eq!(sub_routes[1]["handle"][0]["handler"], "reverse_proxy");
        assert_eq!(
            sub_routes[1]["handle"][0]["transport"]["protocol"],
            "fastcgi"
        );
        assert_eq!(
            sub_routes[1]["handle"][0]["upstreams"][0]["dial"],
            "unix//tmp/portbay-php/8.3/php-fpm.sock"
        );
        // Third sub-route: file_server fallback with index_names.
        assert_eq!(sub_routes[2]["handle"][0]["handler"], "file_server");
        assert_eq!(
            sub_routes[2]["handle"][0]["root"],
            "/tmp/api-gateway/public"
        );
    }

    #[test]
    fn php_route_resolves_version_from_runtime_pin() {
        // A project pinned via `runtime` with NO legacy php_version must still
        // dial the correct per-version socket — proving the route reads through
        // php_version_effective, not the raw field.
        let mut r = Registry::new("test");
        let mut p = php_project("api-gateway", "0.0"); // placeholder legacy value
        p.php_version = None;
        p.runtime = Some(crate::registry::Runtime {
            lang: "php".into(),
            version: "8.4".into(),
        });
        r.add_project(p).unwrap();
        let c = build_config(&r, 2019, 80, 8443, Path::new("/tmp/portbay-php"), no_certs).unwrap();
        let s = c.apps.http.servers.get("portbay").unwrap();
        let sub = &s.routes[0].handle[0]["routes"];
        assert_eq!(
            sub[1]["handle"][0]["upstreams"][0]["dial"],
            "unix//tmp/portbay-php/8.4/php-fpm.sock"
        );
    }

    #[test]
    fn php_fastcgi_dial_follows_tcp_fpm_tuning() {
        let mut r = Registry::new("test");
        r.runtimes.php.insert(
            "8.3".into(),
            crate::registry::PhpVersionConfig {
                fpm: crate::registry::FpmTuning {
                    listen: "tcp".into(),
                    tcp_port: 9103,
                    ..Default::default()
                },
                ini: Default::default(),
            },
        );
        r.add_project(php_project("api-gateway", "8.3")).unwrap();
        let c = build_config(&r, 2019, 80, 8443, Path::new("/tmp/portbay-php"), no_certs).unwrap();
        let s = c.apps.http.servers.get("portbay").unwrap();
        let sub = &s.routes[0].handle[0]["routes"];
        assert_eq!(
            sub[1]["handle"][0]["upstreams"][0]["dial"],
            "127.0.0.1:9103"
        );
    }

    #[test]
    fn php_project_with_dev_command_reverse_proxies_to_port() {
        let mut r = Registry::new("test");
        let mut p = php_project("cms", "8.3");
        p.start_command = Some("php -S 127.0.0.1:8000 router.php".into());
        p.port = Some(8000);
        r.add_project(p).unwrap();
        let c = build_config(&r, 2019, 80, 8443, Path::new("/tmp/portbay-php"), no_certs).unwrap();
        let s = c.apps.http.servers.get("portbay").unwrap();
        let h = &s.routes[0].handle[0];
        assert_eq!(h["handler"], "subroute");
        assert_eq!(
            h["routes"][1]["handle"][0]["upstreams"][0]["dial"],
            "127.0.0.1:8000"
        );
    }

    #[test]
    fn php_project_with_nginx_reverse_proxies_to_generated_server_port() {
        let mut r = Registry::new("test");
        let mut p = php_project("cms", "8.3");
        p.web_server = Some(WebServer::Nginx);
        p.port = Some(9080);
        r.add_project(p).unwrap();
        let c = build_config(&r, 2019, 80, 8443, Path::new("/tmp/portbay-php"), no_certs).unwrap();
        let s = c.apps.http.servers.get("portbay").unwrap();
        let h = &s.routes[0].handle[0];
        assert_eq!(h["handler"], "subroute");
        assert_eq!(
            h["routes"][1]["handle"][0]["upstreams"][0]["dial"],
            "127.0.0.1:9080"
        );
    }

    #[test]
    fn static_project_is_served_by_file_server_not_reverse_proxy() {
        let mut r = Registry::new("test");
        let mut p = next_project("docs", 0, false);
        p.kind = ProjectType::Static;
        p.start_command = None;
        p.port = None;
        p.document_root = Some("public".into());
        r.add_project(p).unwrap();
        let c = build_config(&r, 2019, 80, 8443, Path::new("/tmp/portbay-php"), no_certs).unwrap();
        let http = c.apps.http.servers.get("portbay_http").unwrap();
        let h = &http.routes[0].handle[0];
        assert_eq!(h["handler"], "file_server");
        assert_eq!(h["root"], "/tmp/docs/public");
        assert_eq!(h["index_names"][0], "index.html");
    }

    #[test]
    fn project_to_route_id_matches_format() {
        let p = next_project("abc", 3000, true);
        let mut reg = Registry::new("test");
        reg.add_project(p.clone()).unwrap();
        let route = project_to_route(&p, Path::new("/tmp/portbay-php"), &reg, false);
        assert_eq!(route.id, "route_abc");
        assert!(route.terminal);
    }

    #[test]
    fn bootstrap_config_has_admin_endpoint_and_no_routes() {
        let c = bootstrap_config(2021, 8443);
        assert_eq!(c.admin.listen, "localhost:2021");
        assert_eq!(c.apps.http.http_port, 0);
        let s = c.apps.http.servers.get("portbay").unwrap();
        assert_eq!(s.listen, vec![":8443".to_string()]);
        assert!(s.routes.is_empty());
        assert!(s.automatic_https.disable_redirects);
        assert!(c.apps.tls.certificates.load_files.is_empty());
    }

    #[test]
    fn bootstrap_config_serialises_to_admin_only_json() {
        let c = bootstrap_config(2019, 443);
        let v = serde_json::to_value(&c).unwrap();
        // The admin endpoint is what Caddy needs to come up; the rest is
        // empty scaffolding that POST /load can refill at any time.
        assert_eq!(v["admin"]["listen"], "localhost:2019");
        assert_eq!(
            v["apps"]["http"]["servers"]["portbay"]["routes"],
            serde_json::json!([])
        );
    }

    /// Hard constraint: when no project has an active tunnel, `build_config_filtered`
    /// with an empty `shared_project_ids` MUST produce byte-for-byte identical
    /// output to `build_config` (which always passes an empty shared set).
    #[test]
    fn no_tunnel_output_is_identical_to_build_config() {
        let mut r = Registry::new("test");
        r.add_project(next_project("site", 3010, true)).unwrap();

        let cfg_base =
            build_config(&r, 2019, 80, 8443, Path::new("/tmp/portbay-php"), no_certs).unwrap();
        let cfg_filtered = build_config_filtered(
            &r,
            2019,
            80,
            8443,
            Path::new("/tmp/portbay-php"),
            &HashSet::new(), // suppressed
            &HashSet::new(), // shared_project_ids (empty — no tunnels)
            no_certs,
        )
        .unwrap();

        let base_json = serde_json::to_vec(&cfg_base).unwrap();
        let filtered_json = serde_json::to_vec(&cfg_filtered).unwrap();
        assert_eq!(
            base_json, filtered_json,
            "no-tunnel output must be byte-for-byte identical"
        );
    }

    /// When a project id is in `shared_project_ids`, the plain (non-WebSocket)
    /// reverse-proxy route must include Origin, Host, and X-Forwarded-Host
    /// header rewrites pointing at the loopback dev origin.
    #[test]
    fn active_tunnel_adds_origin_host_normalization_on_plain_route() {
        let p = next_project("shared", 3010, false);
        let mut r = Registry::new("test");
        r.add_project(p.clone()).unwrap();

        let mut shared = HashSet::new();
        shared.insert("shared".to_string());

        let cfg = build_config_filtered(
            &r,
            2019,
            80,
            8443,
            Path::new("/tmp/portbay-php"),
            &HashSet::new(),
            &shared,
            no_certs,
        )
        .unwrap();

        let http = cfg.apps.http.servers.get("portbay_http").unwrap();
        let sub = &http.routes[0].handle[0];
        assert_eq!(sub["handler"], "subroute");

        // WebSocket branch (route 0): unchanged — still rewrites only Origin.
        let ws = &sub["routes"][0];
        assert_eq!(ws["match"][0]["header"]["Upgrade"][0], "websocket");
        assert_eq!(
            ws["handle"][0]["headers"]["request"]["set"]["Origin"][0],
            "http://localhost:3010"
        );

        // Plain branch (route 1): now also rewrites Origin, Host, X-Forwarded-Host.
        let plain = &sub["routes"][1];
        let set = &plain["handle"][0]["headers"]["request"]["set"];
        assert_eq!(set["Origin"][0], "http://localhost:3010");
        assert_eq!(set["Host"][0], "localhost:3010");
        assert_eq!(set["X-Forwarded-Host"][0], "localhost:3010");
        assert_eq!(plain["handle"][0]["upstreams"][0]["dial"], "127.0.0.1:3010");
    }

    /// Without a tunnel the plain route must have NO header rewrite block.
    #[test]
    fn no_tunnel_plain_route_has_no_header_rewrite() {
        let p = next_project("local", 3010, false);
        let mut r = Registry::new("test");
        r.add_project(p.clone()).unwrap();

        let cfg =
            build_config(&r, 2019, 80, 8443, Path::new("/tmp/portbay-php"), no_certs).unwrap();

        let http = cfg.apps.http.servers.get("portbay_http").unwrap();
        let sub = &http.routes[0].handle[0];
        let plain = &sub["routes"][1];
        // No "headers" key at all on the plain route's handler.
        assert!(plain["handle"][0]["headers"].is_null());
        assert_eq!(plain["handle"][0]["upstreams"][0]["dial"], "127.0.0.1:3010");
    }
}
