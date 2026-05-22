use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A short, stable, URL-friendly identifier for a project.
///
/// IDs are also used as `@id` values on Caddy routes and as process names
/// inside Process Compose's YAML, so they must round-trip through HTTP
/// paths and YAML keys cleanly. We don't enforce a regex at this layer —
/// the CLI normalises user input before constructing one.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectId(String);

impl ProjectId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ProjectId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for ProjectId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// The kinds of projects PortBay knows how to launch.
///
/// Unknown / user-supplied launch commands go under `Custom`. We deliberately
/// keep this small in v1; new variants are cheap to add later.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectType {
    Next,
    Vite,
    Php,
    Static,
    Node,
    Custom,
}

/// How PortBay decides a project is "actually serving" rather than just
/// "the process is alive."
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Readiness {
    /// HTTP GET against a path. The most common case for Next, Vite, PHP.
    Http {
        path: String,
        #[serde(default = "default_readiness_timeout")]
        timeout_seconds: u32,
    },
    /// Plain TCP connect — for projects without an HTTP layer.
    Tcp {
        #[serde(default = "default_readiness_timeout")]
        timeout_seconds: u32,
    },
    /// Trust the process — readiness == is_running. Honest about its limits.
    Process,
}

fn default_readiness_timeout() -> u32 {
    75
}

/// A project that PortBay manages.
///
/// JSON field naming intentionally matches the example in
/// `ASSESSMENT_AND_PLAN.md` §7.1 so the doc and the code don't drift.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub path: PathBuf,

    #[serde(rename = "type")]
    pub kind: ProjectType,

    /// Shell command launched by Process Compose for this project's main
    /// dev server. `None` means "service-only" — e.g. a static-file PHP
    /// project that's served entirely by Caddy + PHP-FPM, no separate
    /// dev-server process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_command: Option<String>,

    /// The primary HTTP port the dev server binds to. `None` for projects
    /// served only via Caddy (php_fpm, file_server).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,

    /// Additional ports owned by this project (Vite + API split, multi-port
    /// apps, etc.). PortBay reserves these in the conflict checker.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_ports: Vec<u16>,

    /// The local hostname Caddy routes to this project. Already includes
    /// the domain suffix (e.g. `nour-beiruti.test`).
    pub hostname: String,

    /// Whether Caddy should terminate TLS for this hostname using a
    /// mkcert-issued certificate.
    #[serde(default)]
    pub https: bool,

    /// Shared services the project depends on (e.g. `["caddy", "php-fpm", "mysql"]`).
    /// Resolved against the built-in service catalogue at launch time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<String>,

    /// Environment variables passed to the dev server.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,

    /// How PortBay decides this project is ready to receive traffic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness: Option<Readiness>,

    /// If true, PortBay starts this project automatically when the daemon
    /// comes up. If false, the user must press Play.
    #[serde(default)]
    pub auto_start: bool,

    /// User-supplied tags for filtering / grouping in the UI.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    // ----- PHP-specific (optional) --------------------------------------

    /// For `type: "php"` projects, the document root relative to `path`
    /// (commonly `"public"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_root: Option<String>,

    /// PHP version label to bind to (e.g. `"8.3"`). PHP-FPM service
    /// resolution uses this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub php_version: Option<String>,
}

/// A named cluster of projects (e.g. "Citizen Suite") for batch operations.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub projects: Vec<ProjectId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_id_roundtrips_through_json_as_a_bare_string() {
        let id = ProjectId::new("nour-beiruti");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"nour-beiruti\"");
        let back: ProjectId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn project_type_serialises_snake_case() {
        let v = serde_json::to_string(&ProjectType::Php).unwrap();
        assert_eq!(v, "\"php\"");
    }

    #[test]
    fn readiness_http_uses_tagged_form() {
        let r = Readiness::Http {
            path: "/".into(),
            timeout_seconds: 30,
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["type"], "http");
        assert_eq!(json["path"], "/");
        assert_eq!(json["timeout_seconds"], 30);
    }

    #[test]
    fn readiness_defaults_timeout_when_missing() {
        let json = r#"{ "type": "http", "path": "/" }"#;
        let r: Readiness = serde_json::from_str(json).unwrap();
        match r {
            Readiness::Http {
                path,
                timeout_seconds,
            } => {
                assert_eq!(path, "/");
                assert_eq!(timeout_seconds, 75);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn project_serialises_in_assessment_doc_shape() {
        // Mirrors the Next.js example in ASSESSMENT_AND_PLAN.md §7.1.
        let p = Project {
            id: ProjectId::new("nour-beiruti"),
            name: "Nour Beiruti".into(),
            path: PathBuf::from("/Volumes/DEVSSD/Projects/Clients/Nour Beiruti"),
            kind: ProjectType::Next,
            start_command: Some("pnpm dev".into()),
            port: Some(3010),
            extra_ports: vec![],
            hostname: "nour-beiruti.test".into(),
            https: true,
            services: vec!["caddy".into()],
            env: BTreeMap::new(),
            readiness: Some(Readiness::Http {
                path: "/".into(),
                timeout_seconds: 75,
            }),
            auto_start: false,
            tags: vec!["client".into(), "nextjs".into()],
            document_root: None,
            php_version: None,
        };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["id"], "nour-beiruti");
        assert_eq!(json["type"], "next");
        assert_eq!(json["port"], 3010);
        assert!(json.get("document_root").is_none(), "optional PHP fields should be omitted when empty");
    }
}
