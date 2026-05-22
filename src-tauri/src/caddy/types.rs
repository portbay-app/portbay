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
}

#[derive(Debug, Clone, Serialize)]
pub struct AutomaticHttps {
    pub disable_redirects: bool,
}

/// A Caddy route. The `@id` field becomes the runtime handle for later
/// `DELETE /id/<route_id>` operations — the spike's "killer feature."
#[derive(Debug, Clone, Serialize)]
pub struct Route {
    #[serde(rename = "@id")]
    pub id: String,
    #[serde(rename = "match")]
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
            id: "route_nour-beiruti".into(),
            match_: vec![MatchClause {
                host: vec!["nour-beiruti.test".into()],
            }],
            handle: vec![json!({
                "handler": "reverse_proxy",
                "upstreams": [{ "dial": "127.0.0.1:3010" }]
            })],
            terminal: true,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["@id"], "route_nour-beiruti");
        assert_eq!(v["match"][0]["host"][0], "nour-beiruti.test");
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
            },
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["automatic_https"]["disable_redirects"], true);
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
        };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["apps"]["http"]["http_port"], 0);
        assert_eq!(v["admin"]["listen"], "localhost:2019");
    }
}
