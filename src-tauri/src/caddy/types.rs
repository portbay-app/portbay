//! Types describing the bits of Caddy's config we care about.
//!
//! Caddy's config schema is large; we only model what PortBay generates
//! and inspects. Anything beyond that is left as `serde_json::Value` so
//! we don't have to keep up with every Caddy minor version.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Top-level Caddy config payload accepted by `POST /load`.
#[derive(Debug, Clone, Serialize)]
pub struct CaddyConfig {
    pub admin: AdminConfig,
    pub apps: AppsConfig,
    /// Caddy's `logging` block. Populated by [`crate::caddy::with_access_log`]
    /// to route a JSON access log to a file the HTTP request inspector tails.
    /// Kept as raw JSON so we don't model Caddy's full logging schema. `None`
    /// → omitted (no access log).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminConfig {
    pub listen: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppsConfig {
    pub http: HttpApp,
    pub tls: TlsApp,
}

#[derive(Debug, Clone, Serialize)]
pub struct HttpApp {
    /// Setting `http_port: 0` disables Caddy's automatic bind to :80 for
    /// HTTP→HTTPS redirects. This is the spike's Quirk-1 fix.
    pub http_port: u16,
    pub servers: BTreeMap<String, Server>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Server {
    pub listen: Vec<String>,
    pub routes: Vec<Route>,
    /// Disable Caddy's automatic HTTPS redirects — same Quirk-1 fix.
    pub automatic_https: AutomaticHttps,
    /// Error-handling subroute. When a handler errors (e.g. reverse_proxy
    /// can't reach a dev server that's still starting up), Caddy runs these
    /// routes — we serve PortBay's placeholder page here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<ServerErrors>,
    /// Per-server access-log config (`{"default_logger_name": "..."}`).
    /// Populated by [`crate::caddy::with_access_log`]; `None` → omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs: Option<serde_json::Value>,
    /// TLS connection policies. `Some(vec![{}])` (one empty policy) makes Caddy
    /// terminate TLS on this server, auto-selecting the matching cert (by SNI)
    /// from the loaded `tls.certificates.load_files`. REQUIRED for the HTTPS
    /// server: with `automatic_https.disable = true` Caddy won't auto-enable
    /// TLS, so without an explicit policy the listener serves plain HTTP even
    /// on :443 and every `https://` request fails the handshake. `None` for the
    /// plain-HTTP `:80` server.
    #[serde(
        rename = "tls_connection_policies",
        skip_serializing_if = "Option::is_none"
    )]
    pub tls_connection_policies: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServerErrors {
    pub routes: Vec<Route>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AutomaticHttps {
    pub disable_redirects: bool,
    /// Fully disable Caddy's automatic-HTTPS machinery (ACME, on-demand TLS).
    /// PortBay loads its own mkcert certs and serves plain HTTP on :80, so we
    /// never want Caddy reaching out for public certs.
    pub disable: bool,
}

/// A Caddy route. The `@id` field becomes the runtime handle for later
/// `DELETE /id/<route_id>` operations — the spike's "killer feature."
#[derive(Debug, Clone, Serialize)]
pub struct Route {
    #[serde(rename = "@id")]
    pub id: String,
    /// Host matchers. Empty (omitted) means the route matches every request —
    /// used for the catch-all placeholder route.
    #[serde(rename = "match", skip_serializing_if = "Vec::is_empty")]
    pub match_: Vec<MatchClause>,
    pub handle: Vec<serde_json::Value>,
    pub terminal: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchClause {
    pub host: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TlsApp {
    pub certificates: TlsCertificates,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct TlsCertificates {
    #[serde(default)]
    pub load_files: Vec<TlsCertFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TlsCertFile {
    pub certificate: PathBuf,
    pub key: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Subset of Caddy's `/config` response that we read on reconcile.
///
/// Anything beyond the routes list is kept as raw JSON so a new Caddy
/// release that adds fields doesn't break parsing.
#[derive(Debug, Deserialize)]
pub struct LiveConfig {
    #[serde(default)]
    pub apps: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn route_serialises_with_at_id_field() {
        let r = Route {
            id: "route_marketing-site".into(),
            match_: vec![MatchClause {
                host: vec!["marketing-site.test".into()],
            }],
            handle: vec![json!({
                "handler": "reverse_proxy",
                "upstreams": [{ "dial": "127.0.0.1:3010" }]
            })],
            terminal: true,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["@id"], "route_marketing-site");
        assert_eq!(v["match"][0]["host"][0], "marketing-site.test");
        assert_eq!(v["handle"][0]["handler"], "reverse_proxy");
        assert_eq!(v["terminal"], true);
    }

    #[test]
    fn server_serialises_with_disabled_https_redirects() {
        let s = Server {
            listen: vec![":8443".into()],
            routes: vec![],
            automatic_https: AutomaticHttps {
                disable_redirects: true,
                disable: true,
            },
            errors: None,
            logs: None,
            tls_connection_policies: None,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["automatic_https"]["disable_redirects"], true);
        assert_eq!(v["automatic_https"]["disable"], true);
    }

    #[test]
    fn full_config_envelope_uses_http_port_zero() {
        let c = CaddyConfig {
            admin: AdminConfig {
                listen: "localhost:2019".into(),
            },
            apps: AppsConfig {
                http: HttpApp {
                    http_port: 0,
                    servers: BTreeMap::new(),
                },
                tls: TlsApp {
                    certificates: TlsCertificates::default(),
                },
            },
            logging: None,
        };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["apps"]["http"]["http_port"], 0);
        assert_eq!(v["admin"]["listen"], "localhost:2019");
    }
}
