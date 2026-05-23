//! Translates a `Registry` into a Caddy admin-API config document.
//!
//! Only projects with `https: true` are wired in v1; the registry's
//! `hostname` field is the routing key. Per-project cert paths come from
//! the mkcert wrapper (kanban card P1 #4); for now the config generator
//! accepts a `cert_lookup` callback so it can compose without depending
//! on the mkcert module directly.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::json;

use crate::caddy::error::Result;
use crate::caddy::types::{
    AdminConfig, AppsConfig, AutomaticHttps, CaddyConfig, HttpApp, MatchClause, Route, Server,
    TlsApp, TlsCertFile, TlsCertificates,
};
use crate::registry::{Project, ProjectType, Registry};

/// Per-project TLS cert pair, looked up by the caller.
#[derive(Debug, Clone)]
pub struct CertPaths {
    pub certificate: PathBuf,
    pub key: PathBuf,
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
            },
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
pub fn build_config<F>(
    reg: &Registry,
    admin_port: u16,
    https_port: u16,
    cert_lookup: F,
) -> Result<CaddyConfig>
where
    F: Fn(&str) -> Option<CertPaths>,
{
    let mut routes: Vec<Route> = Vec::new();
    let mut cert_files: Vec<TlsCertFile> = Vec::new();

    for p in &reg.projects {
        if !p.https {
            // v1 only wires HTTPS routes. Plain-HTTP could be added later.
            continue;
        }
        routes.push(project_to_route(p));
        if let Some(paths) = cert_lookup(p.id.as_str()) {
            cert_files.push(TlsCertFile {
                certificate: paths.certificate,
                key: paths.key,
                tags: vec![format!("project:{}", p.id)],
            });
        }
    }

    let mut servers = BTreeMap::new();
    servers.insert(
        "portbay".to_string(),
        Server {
            listen: vec![format!(":{https_port}")],
            routes,
            // Quirk-1 fix: don't let Caddy auto-bind :80 for HTTPS redirects.
            automatic_https: AutomaticHttps {
                disable_redirects: true,
            },
        },
    );

    Ok(CaddyConfig {
        admin: AdminConfig {
            listen: format!("localhost:{admin_port}"),
        },
        apps: AppsConfig {
            http: HttpApp {
                // Quirk-1 fix again: explicitly disable Caddy's :80 bind.
                http_port: 0,
                servers,
            },
            tls: TlsApp {
                certificates: TlsCertificates {
                    load_files: cert_files,
                },
            },
        },
    })
}

/// Build a single route for a project. Used both by `build_config` and by
/// runtime `append_route` calls after a project is added live.
pub fn project_to_route(p: &Project) -> Route {
    let id = format!("route_{}", p.id);
    let handler = build_handler(p);
    Route {
        id,
        match_: vec![MatchClause {
            host: vec![p.hostname.clone()],
        }],
        handle: vec![handler],
        terminal: true,
    }
}

fn build_handler(p: &Project) -> serde_json::Value {
    match p.kind {
        ProjectType::Php => php_handler(p),
        _ => reverse_proxy_handler(p),
    }
}

fn reverse_proxy_handler(p: &Project) -> serde_json::Value {
    let port = p.port.unwrap_or(80);
    json!({
        "handler": "reverse_proxy",
        "upstreams": [{ "dial": format!("127.0.0.1:{port}") }]
    })
}

fn php_handler(p: &Project) -> serde_json::Value {
    let doc_root = p
        .document_root
        .as_deref()
        .map(|d| p.path.join(d))
        .unwrap_or_else(|| p.path.clone());
    let doc_root_str = doc_root.to_string_lossy().into_owned();

    let php_socket = match p.php_version.as_deref() {
        Some(ver) => format!("unix//tmp/portbay-php-fpm-{ver}.sock"),
        None => "unix//tmp/portbay-php-fpm.sock".to_string(),
    };

    // Nested subroute: *.php → FastCGI, everything else → file_server.
    json!({
        "handler": "subroute",
        "routes": [
            {
                "match": [{ "path": ["*.php"] }],
                "handle": [{
                    "handler": "reverse_proxy",
                    "transport": { "protocol": "fastcgi" },
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
        }
    }

    fn no_certs(_: &str) -> Option<CertPaths> {
        None
    }

    #[test]
    fn empty_registry_produces_one_server_no_routes() {
        let r = Registry::new("test");
        let c = build_config(&r, 2019, 8443, no_certs).unwrap();
        assert_eq!(c.admin.listen, "localhost:2019");
        assert_eq!(c.apps.http.http_port, 0);
        let s = c.apps.http.servers.get("portbay").unwrap();
        assert_eq!(s.listen, vec![":8443".to_string()]);
        assert!(s.routes.is_empty());
        assert!(s.automatic_https.disable_redirects);
        assert!(c.apps.tls.certificates.load_files.is_empty());
    }

    #[test]
    fn https_project_produces_route_and_cert_entry() {
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
        let c = build_config(&r, 2019, 8443, lookup).unwrap();
        let s = c.apps.http.servers.get("portbay").unwrap();
        assert_eq!(s.routes.len(), 1);
        assert_eq!(s.routes[0].id, "route_marketing-site");
        assert_eq!(s.routes[0].match_[0].host[0], "marketing-site.test");
        let h = &s.routes[0].handle[0];
        assert_eq!(h["handler"], "reverse_proxy");
        assert_eq!(h["upstreams"][0]["dial"], "127.0.0.1:3010");
        let certs = &c.apps.tls.certificates.load_files;
        assert_eq!(certs.len(), 1);
        assert_eq!(certs[0].certificate, PathBuf::from("/c/cert.pem"));
        assert_eq!(certs[0].tags, vec!["project:marketing-site"]);
    }

    #[test]
    fn non_https_project_is_skipped() {
        let mut r = Registry::new("test");
        r.add_project(next_project("plain", 3010, false)).unwrap();
        let c = build_config(&r, 2019, 8443, no_certs).unwrap();
        let s = c.apps.http.servers.get("portbay").unwrap();
        assert!(s.routes.is_empty());
    }

    #[test]
    fn php_project_uses_subroute_with_fastcgi() {
        let mut r = Registry::new("test");
        r.add_project(php_project("api-gateway", "8.3")).unwrap();
        let c = build_config(&r, 2019, 8443, no_certs).unwrap();
        let s = c.apps.http.servers.get("portbay").unwrap();
        let h = &s.routes[0].handle[0];
        assert_eq!(h["handler"], "subroute");
        let sub_routes = &h["routes"];
        // First sub-route: *.php → FastCGI.
        assert_eq!(sub_routes[0]["match"][0]["path"][0], "*.php");
        assert_eq!(sub_routes[0]["handle"][0]["handler"], "reverse_proxy");
        assert_eq!(
            sub_routes[0]["handle"][0]["transport"]["protocol"],
            "fastcgi"
        );
        assert_eq!(
            sub_routes[0]["handle"][0]["upstreams"][0]["dial"],
            "unix//tmp/portbay-php-fpm-8.3.sock"
        );
        // Second sub-route: file_server fallback with index_names.
        assert_eq!(sub_routes[1]["handle"][0]["handler"], "file_server");
        assert_eq!(
            sub_routes[1]["handle"][0]["root"],
            "/tmp/api-gateway/public"
        );
    }

    #[test]
    fn project_to_route_id_matches_format() {
        let p = next_project("abc", 3000, true);
        let r = project_to_route(&p);
        assert_eq!(r.id, "route_abc");
        assert!(r.terminal);
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
}
