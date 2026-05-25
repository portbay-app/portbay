//! `.portbay.json` — shareable project descriptor.
//!
//! Two complementary halves:
//!
//! - **Export.** Translates a registry `Project` into a portable
//!   `PortbayFile` with absolute paths replaced by `${PROJECT_PATH}`
//!   placeholders and the env-var map split into a non-sensitive
//!   `env_template` plus a `secrets` list the importer fills in.
//! - **Import.** Parses the file, validates the schema version, and
//!   produces an `ImportPlan` carrying both a ready-to-insert Project
//!   stub and the list of secrets the GUI must prompt the user for
//!   before committing.
//!
//! Keep this module pure. Disk I/O lives in `crate::commands::portfile`;
//! that lets unit tests drive both halves against in-memory JSON
//! without touching the filesystem.

pub mod error;
pub mod schema;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::registry::{Project, ProjectId, ProjectType, Readiness};

pub use error::{PortfileError, Result};
pub use schema::{PortbayFile, PORTBAY_FILE_NAME, SCHEMA_VERSION};

/// Plan returned by `read_for_import`. The GUI surfaces `secrets` as
/// a per-key prompt; once filled, `materialise_project` consumes them
/// and produces the final Project for registry insertion.
#[derive(Debug, Clone)]
pub struct ImportPlan {
    pub file: PortbayFile,
    pub project_path: PathBuf,
    pub secrets_required: Vec<String>,
}

impl ImportPlan {
    pub fn new(file: PortbayFile, project_path: PathBuf) -> Self {
        let secrets_required = file.secrets.clone();
        Self {
            file,
            project_path,
            secrets_required,
        }
    }
}

/// Translate a registry Project into a portable PortbayFile.
///
/// Env vars are split via a name heuristic: anything matching
/// [`looks_like_secret`] is dropped from `env_template` and its
/// name (only) is recorded in `secrets`. Everything else is exported
/// verbatim. The heuristic is conservative — false positives just
/// re-prompt the importer; false negatives leak a value into a file
/// users will commit to git, which is the harm to minimise.
pub fn export_project(project: &Project) -> PortbayFile {
    let mut env_template: BTreeMap<String, String> = BTreeMap::new();
    let mut secrets: Vec<String> = Vec::new();
    for (k, v) in &project.env {
        if looks_like_secret(k) {
            secrets.push(k.clone());
        } else {
            env_template.insert(k.clone(), v.clone());
        }
    }
    secrets.sort();

    PortbayFile {
        version: SCHEMA_VERSION,
        name: project.name.clone(),
        kind: project.kind,
        hostname: project.hostname.clone(),
        port: project.port,
        php_version: project.php_version.clone(),
        web_server: project.web_server,
        mobile_run: project.mobile_run.clone(),
        https: project.https,
        auto_start: project.auto_start,
        start_command: project.start_command.clone(),
        document_root: project.document_root.clone(),
        env_template,
        secrets,
        post_install: Vec::new(),
        readiness: project.readiness.clone(),
        tags: project.tags.clone(),
    }
}

/// Conservative classifier for sensitive env-var names. Matches the
/// usual suspects from Laravel / Rails / Node defaults.
pub fn looks_like_secret(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    const NEEDLES: &[&str] = &[
        "PASSWORD",
        "PASSWD",
        "PWD",
        "SECRET",
        "TOKEN",
        "API_KEY",
        "APIKEY",
        "PRIVATE_KEY",
        "ACCESS_KEY",
        "ENCRYPTION_KEY",
        "APP_KEY",
        "JWT",
        "SESSION_KEY",
    ];
    NEEDLES.iter().any(|needle| upper.contains(needle))
}

/// Serialise a PortbayFile to indented JSON.
pub fn to_json_string(file: &PortbayFile) -> Result<String> {
    serde_json::to_string_pretty(file).map_err(PortfileError::Serialise)
}

/// Parse a PortbayFile from JSON bytes and validate its schema version.
pub fn from_json_bytes(bytes: &[u8]) -> Result<PortbayFile> {
    let file: PortbayFile = serde_json::from_slice(bytes).map_err(PortfileError::Parse)?;
    if file.version > SCHEMA_VERSION {
        return Err(PortfileError::UnsupportedVersion {
            found: file.version,
            supported: SCHEMA_VERSION,
        });
    }
    Ok(file)
}

/// Materialise the final Project from an ImportPlan + filled-in
/// secrets. `id` is provided by the caller because the importer is
/// free to override it on collision (e.g. append `-team`); the file
/// itself doesn't carry an id.
///
/// `secrets` is a map keyed by the variable names the file listed as
/// `secrets`. Any required key missing from the map produces a
/// `SecretMissing` error and the caller is expected to re-prompt.
pub fn materialise_project(
    plan: &ImportPlan,
    id: ProjectId,
    secrets: &BTreeMap<String, String>,
) -> Result<Project> {
    // Verify every required secret is filled in.
    for required in &plan.file.secrets {
        if !secrets.contains_key(required) {
            return Err(PortfileError::SecretMissing(required.clone()));
        }
    }

    // Build the final env map: template + secrets, with placeholders
    // expanded against the actual project context.
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    let ctx = PlaceholderContext {
        project_path: &plan.project_path,
        project_name: &plan.file.name,
    };
    for (k, v) in &plan.file.env_template {
        env.insert(k.clone(), expand_placeholders(v, &ctx));
    }
    for (k, v) in secrets {
        env.insert(k.clone(), v.clone());
    }

    let services = if plan.file.https {
        vec!["caddy".to_string()]
    } else {
        vec![]
    };

    let readiness = plan.file.readiness.clone().or(Some(Readiness::Process));

    let runtime = if plan.file.kind == ProjectType::Php {
        plan.file
            .php_version
            .clone()
            .map(|version| crate::registry::Runtime {
                lang: "php".into(),
                version,
            })
    } else {
        None
    };

    Ok(Project {
        id,
        name: plan.file.name.clone(),
        path: plan.project_path.clone(),
        kind: plan.file.kind,
        start_command: plan.file.start_command.clone(),
        port: plan.file.port,
        extra_ports: vec![],
        hostname: plan.file.hostname.clone(),
        https: plan.file.https,
        services: if plan.file.kind == ProjectType::Php && plan.file.start_command.is_none() {
            vec!["caddy".to_string(), "php-fpm".to_string()]
        } else {
            services
        },
        env,
        readiness,
        auto_start: plan.file.auto_start,
        tags: plan.file.tags.clone(),
        document_root: plan.file.document_root.clone(),
        php_version: plan.file.php_version.clone(),
        web_server: plan.file.web_server,
        mobile_run: plan.file.mobile_run.clone(),
        runtime,
        // Portfile import doesn't carry a workspace binding yet — a monorepo
        // app round-trips as a root-path project. Re-set via the add-project
        // workspace flow if needed.
        workspace: None,
        cors: None,
        sandbox: None,
    })
}

struct PlaceholderContext<'a> {
    project_path: &'a Path,
    project_name: &'a str,
}

fn expand_placeholders(value: &str, ctx: &PlaceholderContext<'_>) -> String {
    value
        .replace("${PROJECT_PATH}", &ctx.project_path.to_string_lossy())
        .replace("${PROJECT_NAME}", ctx.project_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ProjectType;
    use std::collections::BTreeMap;

    fn sample_project() -> Project {
        let mut env = BTreeMap::new();
        env.insert("APP_ENV".into(), "local".into());
        env.insert("APP_DEBUG".into(), "true".into());

        Project {
            cors: None,
            sandbox: None,
            id: ProjectId::new("demo-cms"),
            name: "Demo CMS".into(),
            path: PathBuf::from("/Users/me/code/demo-cms"),
            kind: ProjectType::Php,
            start_command: None,
            port: Some(8000),
            extra_ports: vec![],
            hostname: "demo-cms.test".into(),
            https: true,
            services: vec!["caddy".into()],
            env,
            readiness: None,
            auto_start: false,
            tags: vec!["client:demo".into()],
            document_root: Some("public".into()),
            php_version: Some("8.3".into()),
            web_server: None,
            mobile_run: None,
            runtime: None,
            workspace: None,
        }
    }

    #[test]
    fn export_roundtrips_through_json() {
        let p = sample_project();
        let file = export_project(&p);
        let json = to_json_string(&file).unwrap();
        let parsed = from_json_bytes(json.as_bytes()).unwrap();
        assert_eq!(parsed.name, "Demo CMS");
        assert_eq!(parsed.hostname, "demo-cms.test");
        assert_eq!(parsed.port, Some(8000));
        assert_eq!(parsed.php_version.as_deref(), Some("8.3"));
        assert_eq!(parsed.document_root.as_deref(), Some("public"));
        assert!(parsed.https);
        assert_eq!(parsed.env_template.len(), 2);
        assert!(parsed.secrets.is_empty());
    }

    #[test]
    fn looks_like_secret_flags_common_patterns() {
        assert!(looks_like_secret("DB_PASSWORD"));
        assert!(looks_like_secret("APP_KEY"));
        assert!(looks_like_secret("STRIPE_API_KEY"));
        assert!(looks_like_secret("GITHUB_TOKEN"));
        assert!(looks_like_secret("SESSION_KEY"));
        assert!(looks_like_secret("JWT_SECRET"));
        assert!(!looks_like_secret("APP_ENV"));
        assert!(!looks_like_secret("DB_DATABASE"));
        assert!(!looks_like_secret("APP_URL"));
    }

    #[test]
    fn export_moves_sensitive_keys_into_secrets_list() {
        let mut p = sample_project();
        p.env.insert("DB_PASSWORD".into(), "hunter2".into());
        p.env.insert("APP_KEY".into(), "base64:abcd=".into());
        p.env.insert("STRIPE_TOKEN".into(), "sk_test_x".into());
        let file = export_project(&p);
        // Sensitive keys never appear in env_template…
        assert!(!file.env_template.contains_key("DB_PASSWORD"));
        assert!(!file.env_template.contains_key("APP_KEY"));
        assert!(!file.env_template.contains_key("STRIPE_TOKEN"));
        // …and their names are recorded in secrets, sorted.
        assert_eq!(
            file.secrets,
            vec![
                "APP_KEY".to_string(),
                "DB_PASSWORD".to_string(),
                "STRIPE_TOKEN".to_string(),
            ]
        );
        // Non-sensitive keys stay in env_template.
        assert!(file.env_template.contains_key("APP_ENV"));
        assert!(file.env_template.contains_key("APP_DEBUG"));
    }

    #[test]
    fn export_omits_id_and_path() {
        let p = sample_project();
        let file = export_project(&p);
        let json = to_json_string(&file).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        // The file carries no id or path — both come from the
        // importer's context.
        assert!(v.get("id").is_none());
        assert!(v.get("path").is_none());
    }

    #[test]
    fn import_expands_placeholders_in_env() {
        let mut env_template = BTreeMap::new();
        env_template.insert("DB_DATABASE".into(), "${PROJECT_NAME}_dev".into());
        env_template.insert("APP_LOG_PATH".into(), "${PROJECT_PATH}/storage/logs".into());
        let file = PortbayFile {
            version: SCHEMA_VERSION,
            name: "MyApp".into(),
            kind: ProjectType::Php,
            hostname: "myapp.test".into(),
            port: Some(8000),
            php_version: Some("8.3".into()),
            web_server: None,
            mobile_run: None,
            https: true,
            auto_start: false,
            start_command: None,
            document_root: Some("public".into()),
            env_template,
            secrets: vec![],
            post_install: vec![],
            readiness: None,
            tags: vec![],
        };
        let plan = ImportPlan::new(file, PathBuf::from("/Users/u/sites/myapp"));
        let project =
            materialise_project(&plan, ProjectId::new("myapp"), &BTreeMap::new()).unwrap();
        assert_eq!(project.env["DB_DATABASE"], "MyApp_dev");
        assert_eq!(
            project.env["APP_LOG_PATH"],
            "/Users/u/sites/myapp/storage/logs"
        );
    }

    #[test]
    fn import_requires_every_listed_secret() {
        let file = PortbayFile {
            version: SCHEMA_VERSION,
            name: "X".into(),
            kind: ProjectType::Php,
            hostname: "x.test".into(),
            port: None,
            php_version: None,
            web_server: None,
            mobile_run: None,
            https: true,
            auto_start: false,
            start_command: None,
            document_root: None,
            env_template: BTreeMap::new(),
            secrets: vec!["DB_PASSWORD".into(), "APP_KEY".into()],
            post_install: vec![],
            readiness: None,
            tags: vec![],
        };
        let plan = ImportPlan::new(file, PathBuf::from("/tmp/x"));
        let mut partial = BTreeMap::new();
        partial.insert("DB_PASSWORD".into(), "secret".into());
        match materialise_project(&plan, ProjectId::new("x"), &partial) {
            Err(PortfileError::SecretMissing(name)) => assert_eq!(name, "APP_KEY"),
            other => panic!("expected SecretMissing(APP_KEY), got {other:?}"),
        }
    }

    #[test]
    fn import_writes_secrets_into_env() {
        let file = PortbayFile {
            version: SCHEMA_VERSION,
            name: "X".into(),
            kind: ProjectType::Php,
            hostname: "x.test".into(),
            port: None,
            php_version: None,
            web_server: None,
            mobile_run: None,
            https: true,
            auto_start: false,
            start_command: None,
            document_root: None,
            env_template: BTreeMap::new(),
            secrets: vec!["APP_KEY".into()],
            post_install: vec![],
            readiness: None,
            tags: vec![],
        };
        let plan = ImportPlan::new(file, PathBuf::from("/tmp/x"));
        let mut secrets = BTreeMap::new();
        secrets.insert("APP_KEY".into(), "base64:abc=".into());
        let project = materialise_project(&plan, ProjectId::new("x"), &secrets).unwrap();
        assert_eq!(project.env["APP_KEY"], "base64:abc=");
    }

    #[test]
    fn unsupported_schema_version_is_rejected() {
        let json = serde_json::json!({
            "version": SCHEMA_VERSION + 5,
            "name": "X",
            "type": "custom",
            "hostname": "x.test",
            "https": true,
            "autoStart": false,
            "envTemplate": {},
            "secrets": [],
            "postInstall": [],
            "tags": []
        });
        let bytes = serde_json::to_vec(&json).unwrap();
        match from_json_bytes(&bytes) {
            Err(PortfileError::UnsupportedVersion { found, supported }) => {
                assert_eq!(found, SCHEMA_VERSION + 5);
                assert_eq!(supported, SCHEMA_VERSION);
            }
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
    }
}
