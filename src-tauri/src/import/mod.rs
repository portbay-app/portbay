//! Migration import from other local-dev tools.
//!
//! Three source tools (`Herd`, `ServBay`, `MAMP`), each with a small
//! detect-and-parse module. Detection probes filesystem paths the tool
//! is known to use; parsing returns a uniform `ImportedSite` list the
//! GUI can show before commit.
//!
//! Best-effort mapping. Where the source tool stores something
//! PortBay doesn't carry (e.g. an explicit Apache document index),
//! we drop it on the floor; the user edits in the detail panel after
//! import.

pub mod error;
pub mod herd;
pub mod mamp;
pub mod servbay;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use error::{ImportError, Result};

use crate::registry::{Project, ProjectId, ProjectType, Readiness, Registry, Runtime, WebServer};

/// Stable identifier for each known source tool. Surface in the GUI and
/// in the registry's `tags` on imported projects (`source:herd` etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportSource {
    Herd,
    ServBay,
    Mamp,
}

impl ImportSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Herd => "Laravel Herd",
            Self::ServBay => "ServBay",
            Self::Mamp => "MAMP",
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Self::Herd => "source:herd",
            Self::ServBay => "source:servbay",
            Self::Mamp => "source:mamp",
        }
    }

    /// Parse a source name (case-insensitive). Accepts the canonical ids the
    /// CLI/MCP use (`herd`, `servbay`, `mamp`). Shared so the terminal and the
    /// MCP agent agree on the accepted spellings.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "herd" => Some(Self::Herd),
            "servbay" => Some(Self::ServBay),
            "mamp" => Some(Self::Mamp),
            _ => None,
        }
    }
}

/// One site extracted from a source tool. The fields are deliberately
/// the same names the registry's `Project` struct uses so the import
/// command can construct a `Project` straight from this shape.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedSite {
    pub source: ImportSource,
    /// Absolute filesystem path to the project root.
    pub path: String,
    /// Hostname as the source tool serves it (e.g. `myapp.test`).
    pub hostname: String,
    /// PHP version string the source tool records (`8.3`, `7.4`, …)
    /// when available. `None` for non-PHP projects.
    pub php_version: Option<String>,
    /// True if the source tool serves this site over HTTPS.
    pub https: bool,
    /// Document root relative to `path`, when the source serves files out of a
    /// sub-directory (e.g. `public` for a Laravel/PHP front-controller app).
    /// `None` when the project is served straight from `path`.
    pub document_root: Option<String>,
    /// Preferred PHP web server when the source implies one.
    pub web_server: Option<WebServer>,
    /// Project type the source clearly implies (e.g. PHP for an FPM vhost,
    /// Static for a plain file-server vhost). `None` lets the import command
    /// fall back to its php-version heuristic.
    pub kind_hint: Option<ProjectType>,
    /// Suggested project id derived from the path's final component.
    pub suggested_id: String,
    /// Suggested project name (human-readable; falls back to id).
    pub suggested_name: String,
}

impl ImportedSite {
    pub(crate) fn from_parts(
        source: ImportSource,
        path: String,
        hostname: String,
        php_version: Option<String>,
        https: bool,
    ) -> Self {
        let id = derive_id(&path);
        Self {
            source,
            suggested_name: derive_name(&path).unwrap_or_else(|| id.clone()),
            suggested_id: id,
            path,
            hostname,
            php_version,
            https,
            document_root: None,
            web_server: match source {
                ImportSource::ServBay => Some(WebServer::Nginx),
                ImportSource::Mamp => Some(WebServer::Apache),
                ImportSource::Herd => Some(WebServer::Caddy),
            },
            kind_hint: None,
        }
    }
}

/// What a `detect_sources` call returns per source — whether anything
/// was found and how many sites are recoverable. The GUI uses
/// `site_count` to gate the "Import N sites from X" button.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedSource {
    pub source: ImportSource,
    pub label: &'static str,
    /// True iff the source tool's config or vhost dir is present.
    pub present: bool,
    /// Number of sites that parsed without error.
    pub site_count: usize,
    /// Free-form note for the GUI (e.g. "uses NGINX vhost format").
    pub note: Option<String>,
}

/// Run every detector and return the per-source summary.
pub fn detect_all() -> Vec<DetectedSource> {
    vec![herd::detect(), servbay::detect(), mamp::detect()]
}

/// Parse all sites from the given source. Errors per site are logged
/// and skipped — the caller gets every site the parser could recover.
pub fn read_all(source: ImportSource) -> Result<Vec<ImportedSite>> {
    match source {
        ImportSource::Herd => herd::read_sites(),
        ImportSource::ServBay => servbay::read_sites(),
        ImportSource::Mamp => mamp::read_sites(),
    }
}

// =============================================================================
// Preview + import-into-registry — the one shared implementation behind the
// GUI commands (commands/import.rs), the CLI (`portbay import`), and the MCP
// `Migrate` toolset, so the site→Project mapping and collision rules can't drift
// between surfaces.
// =============================================================================

/// One row in an import preview: a parsed site plus whether importing it would
/// collide with an existing project's id or path.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreviewRow {
    pub site: ImportedSite,
    /// True if a project with the same id already exists in PortBay.
    pub id_collision: bool,
    /// True if a project with the same path already exists.
    pub path_collision: bool,
}

/// Outcome of an import: the ids that landed and the rows skipped with a reason.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub imported: Vec<String>,
    pub skipped: Vec<SkippedRow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedRow {
    pub site: ImportedSite,
    pub reason: String,
}

/// Build the preview for a source against the current registry: every parsed
/// site, flagged for id/path collisions so a surface can warn before committing.
pub fn preview(source: ImportSource, registry: &Registry) -> Result<Vec<ImportPreviewRow>> {
    Ok(preview_sites(read_all(source)?, registry))
}

/// Pure core of [`preview`]: flag a known set of sites for id/path collisions.
fn preview_sites(sites: Vec<ImportedSite>, registry: &Registry) -> Vec<ImportPreviewRow> {
    let existing_ids: HashSet<String> = registry
        .list_projects()
        .iter()
        .map(|p| p.id.as_str().to_string())
        .collect();
    let existing_paths: HashSet<PathBuf> = registry
        .list_projects()
        .iter()
        .map(|p| p.path.clone())
        .collect();
    sites
        .into_iter()
        .map(|site| ImportPreviewRow {
            id_collision: existing_ids.contains(&site.suggested_id),
            path_collision: existing_paths.contains(&PathBuf::from(&site.path)),
            site,
        })
        .collect()
}

/// The suggested ids for every site a source exposes — the set `import_selected`
/// imports when the caller asks for "all".
pub fn site_ids(source: ImportSource) -> Result<Vec<String>> {
    Ok(read_all(source)?
        .into_iter()
        .map(|s| s.suggested_id)
        .collect())
}

/// Import the chosen sites (by suggested id) from a source into `registry`,
/// translating each to a `Project`. Sites that fail to build or collide are
/// returned in `skipped` with a reason; the caller persists the registry and
/// triggers a reconcile when `imported` is non-empty.
pub fn import_selected(
    source: ImportSource,
    ids: &[String],
    registry: &mut Registry,
) -> Result<ImportResult> {
    // Respect the signed project-cap entitlement (anonymous 3 / free 6 / Pro
    // unlimited) — same gate as the GUI/CLI `add` and MCP `add_project`/
    // `import_config`, so a bulk migration import can't bypass the cap on any
    // surface. `current()` reads the signed cache, so this is honest across
    // processes (the CLI/MCP see the same entitlement the app wrote).
    let max_projects = crate::entitlements::current().entitlements.max_projects;
    Ok(import_sites(
        source,
        read_all(source)?,
        ids,
        registry,
        max_projects,
    ))
}

/// Pure core of [`import_selected`]: import a chosen subset of an already-parsed
/// site list into `registry`, stopping at `max_projects` (`None` = unlimited).
/// Sites that would exceed the cap are skipped with an upgrade reason.
fn import_sites(
    source: ImportSource,
    sites: Vec<ImportedSite>,
    ids: &[String],
    registry: &mut Registry,
    max_projects: Option<u32>,
) -> ImportResult {
    let by_id: std::collections::HashMap<String, ImportedSite> = sites
        .into_iter()
        .map(|s| (s.suggested_id.clone(), s))
        .collect();

    let mut imported: Vec<String> = Vec::new();
    let mut skipped: Vec<SkippedRow> = Vec::new();

    for id in ids {
        let Some(site) = by_id.get(id) else {
            skipped.push(SkippedRow {
                site: ImportedSite::from_parts(source, String::new(), String::new(), None, false),
                reason: format!("id `{id}` not present in current scan"),
            });
            continue;
        };
        // Stop at the entitlement cap — skip the rest with an upgrade reason
        // rather than silently exceeding the community limit.
        if let Some(cap) = max_projects {
            if registry.list_projects().len() as u32 >= cap {
                skipped.push(SkippedRow {
                    site: site.clone(),
                    reason: format!(
                        "project cap reached ({cap}) — upgrade to PortBay Pro for unlimited projects"
                    ),
                });
                continue;
            }
        }
        let project = match build_project(site) {
            Ok(p) => p,
            Err(reason) => {
                skipped.push(SkippedRow {
                    site: site.clone(),
                    reason,
                });
                continue;
            }
        };
        match registry.add_project(project) {
            Ok(()) => imported.push(site.suggested_id.clone()),
            Err(e) => skipped.push(SkippedRow {
                site: site.clone(),
                reason: e.to_string(),
            }),
        }
    }
    ImportResult { imported, skipped }
}

/// Translate one parsed site into a registry `Project`. Prefers the source's
/// explicit type hint (ServBay knows from the vhost whether it's PHP-FPM or a
/// plain file server); otherwise falls back to a php-version heuristic.
pub fn build_project(site: &ImportedSite) -> std::result::Result<Project, String> {
    let path = PathBuf::from(&site.path);
    if !path.is_absolute() {
        return Err(format!("path is not absolute: {}", site.path));
    }
    let id = ProjectId::new(&site.suggested_id);
    let kind = site.kind_hint.unwrap_or_else(|| {
        if site.php_version.is_some() || path_has_php_entry(&path) {
            ProjectType::Php
        } else {
            ProjectType::Custom
        }
    });
    let runtime = if kind == ProjectType::Php {
        site.php_version
            .clone()
            .or_else(detected_php_version)
            .map(|version| Runtime {
                lang: "php".into(),
                version,
            })
    } else {
        None
    };
    let php_version = if kind == ProjectType::Php {
        site.php_version
            .clone()
            .or_else(|| runtime.as_ref().map(|rt| rt.version.clone()))
    } else {
        None
    };
    Ok(Project {
        id,
        name: site.suggested_name.clone(),
        path,
        kind,
        start_command: None,
        port: None,
        extra_ports: vec![],
        hostname: site.hostname.clone(),
        https: site.https,
        services: match kind {
            ProjectType::Php => vec!["caddy".into(), "php-fpm".into()],
            _ if site.https => vec!["caddy".into()],
            _ => vec![],
        },
        env: Default::default(),
        readiness: Some(Readiness::Process),
        auto_start: false,
        tags: vec![site.source.tag().to_string()],
        document_root: site.document_root.clone(),
        php_version,
        web_server: if kind == ProjectType::Php {
            site.web_server.or(Some(WebServer::Caddy))
        } else {
            None
        },
        mobile_run: None,
        runtime,
        workspace: None,
        cors: None,
        sandbox: None,
        domain: None,
        tunnel: None,
    })
}

fn path_has_php_entry(path: &Path) -> bool {
    path.join("index.php").exists() || path.join("public").join("index.php").exists()
}

fn detected_php_version() -> Option<String> {
    crate::php::detect_all()
        .into_iter()
        .next()
        .map(|p| p.version)
}

fn derive_id(path: &str) -> String {
    let last = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("imported");
    let mut out = String::with_capacity(last.len());
    let mut last_dash = true;
    for ch in last.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "imported".to_string()
    } else {
        trimmed
    }
}

fn derive_name(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_id_lowercases_and_hyphenates() {
        assert_eq!(derive_id("/Users/x/Sites/API Gateway"), "api-gateway");
        assert_eq!(derive_id("/Users/x/MyApp"), "myapp");
        assert_eq!(derive_id("/Users/x/__weird__"), "weird");
    }

    #[test]
    fn derive_id_falls_back_to_imported_for_empty() {
        assert_eq!(derive_id("/"), "imported");
        assert_eq!(derive_id(""), "imported");
    }

    #[test]
    fn imported_site_has_id_and_name_derived_from_path() {
        let s = ImportedSite::from_parts(
            ImportSource::Herd,
            "/Users/x/MyApp".into(),
            "myapp.test".into(),
            Some("8.3".into()),
            true,
        );
        assert_eq!(s.suggested_id, "myapp");
        assert_eq!(s.suggested_name, "MyApp");
        assert!(s.https);
    }

    #[test]
    fn detect_all_returns_one_entry_per_source() {
        let result = detect_all();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].source, ImportSource::Herd);
        assert_eq!(result[1].source, ImportSource::ServBay);
        assert_eq!(result[2].source, ImportSource::Mamp);
    }

    #[test]
    fn source_parse_is_case_insensitive_and_rejects_unknown() {
        assert_eq!(ImportSource::parse("herd"), Some(ImportSource::Herd));
        assert_eq!(ImportSource::parse("ServBay"), Some(ImportSource::ServBay));
        assert_eq!(ImportSource::parse(" MAMP "), Some(ImportSource::Mamp));
        assert_eq!(ImportSource::parse("valet"), None);
    }

    // --- build_project (the site→Project mapping shared by every surface) ---

    #[test]
    fn build_project_marks_php_kind_when_version_present() {
        let site = ImportedSite::from_parts(
            ImportSource::Herd,
            "/tmp/myapp".into(),
            "myapp.test".into(),
            Some("8.3".into()),
            true,
        );
        let p = build_project(&site).unwrap();
        assert!(matches!(p.kind, ProjectType::Php));
        assert_eq!(p.php_version.as_deref(), Some("8.3"));
        assert!(p.https);
        assert_eq!(p.tags, vec!["source:herd"]);
    }

    #[test]
    fn build_project_marks_custom_when_no_php() {
        let site = ImportedSite::from_parts(
            ImportSource::Mamp,
            "/tmp/static-site".into(),
            "static.test".into(),
            None,
            false,
        );
        let p = build_project(&site).unwrap();
        assert!(matches!(p.kind, ProjectType::Custom));
        assert!(p.php_version.is_none());
        assert_eq!(p.tags, vec!["source:mamp"]);
    }

    #[test]
    fn build_project_honors_kind_hint_and_document_root() {
        let mut site = ImportedSite::from_parts(
            ImportSource::ServBay,
            "/Volumes/x/Tribal House/tribal-house-cms".into(),
            "tribal-house.localhost".into(),
            None,
            false,
        );
        site.kind_hint = Some(ProjectType::Php);
        site.document_root = Some("public".into());
        let p = build_project(&site).unwrap();
        assert!(matches!(p.kind, ProjectType::Php));
        assert_eq!(p.document_root.as_deref(), Some("public"));
        assert_eq!(p.name, "tribal-house-cms");
        assert_eq!(p.tags, vec!["source:servbay"]);
    }

    #[test]
    fn build_project_static_hint_maps_to_static_not_custom() {
        let mut site = ImportedSite::from_parts(
            ImportSource::ServBay,
            "/Users/x/Sites/brochure".into(),
            "brochure.localhost".into(),
            None,
            false,
        );
        site.kind_hint = Some(ProjectType::Static);
        let p = build_project(&site).unwrap();
        assert!(matches!(p.kind, ProjectType::Static));
    }

    #[test]
    fn build_project_rejects_relative_path() {
        let site = ImportedSite::from_parts(
            ImportSource::Herd,
            "relative/path".into(),
            "x.test".into(),
            None,
            false,
        );
        assert!(build_project(&site).is_err());
    }

    // --- preview + import collision/skip logic (pure cores) ---

    fn site(id_path: &str, https: bool) -> ImportedSite {
        ImportedSite::from_parts(
            ImportSource::Herd,
            id_path.into(),
            format!("{}.test", id_path.rsplit('/').next().unwrap_or("x")),
            Some("8.3".into()),
            https,
        )
    }

    #[test]
    fn preview_flags_id_and_path_collisions() {
        let mut reg = Registry::new("portbay.test");
        let existing = build_project(&site("/Users/x/alpha", true)).unwrap();
        reg.add_project(existing).unwrap();

        let rows = preview_sites(
            vec![site("/Users/x/alpha", true), site("/Users/x/beta", true)],
            &reg,
        );
        let alpha = rows
            .iter()
            .find(|r| r.site.suggested_id == "alpha")
            .unwrap();
        assert!(alpha.id_collision && alpha.path_collision);
        let beta = rows.iter().find(|r| r.site.suggested_id == "beta").unwrap();
        assert!(!beta.id_collision && !beta.path_collision);
    }

    #[test]
    fn import_sites_imports_selected_and_skips_unknown_and_duplicate() {
        let mut reg = Registry::new("portbay.test");
        let sites = vec![site("/Users/x/alpha", true), site("/Users/x/beta", true)];
        // First import alpha + beta (no cap).
        let r1 = import_sites(
            ImportSource::Herd,
            sites.clone(),
            &["alpha".into(), "beta".into()],
            &mut reg,
            None,
        );
        assert_eq!(r1.imported, vec!["alpha", "beta"]);
        assert!(r1.skipped.is_empty());
        assert_eq!(reg.list_projects().len(), 2);

        // Re-import alpha (duplicate id) + a missing id → both skipped, nothing added.
        let r2 = import_sites(
            ImportSource::Herd,
            sites,
            &["alpha".into(), "ghost".into()],
            &mut reg,
            None,
        );
        assert!(r2.imported.is_empty());
        assert_eq!(r2.skipped.len(), 2);
        assert!(r2.skipped.iter().any(|s| s.reason.contains("ghost")));
        assert_eq!(reg.list_projects().len(), 2);
    }

    #[test]
    fn import_sites_enforces_the_project_cap() {
        // A community cap of 2: importing three sites lands two and skips the
        // third with an upgrade reason — a bulk import can't exceed the cap.
        let mut reg = Registry::new("portbay.test");
        let sites = vec![
            site("/Users/x/alpha", true),
            site("/Users/x/beta", true),
            site("/Users/x/gamma", true),
        ];
        let result = import_sites(
            ImportSource::Herd,
            sites,
            &["alpha".into(), "beta".into(), "gamma".into()],
            &mut reg,
            Some(2),
        );
        assert_eq!(result.imported, vec!["alpha", "beta"]);
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].site.suggested_id, "gamma");
        assert!(result.skipped[0].reason.contains("cap reached"));
        assert_eq!(reg.list_projects().len(), 2);
    }
}
