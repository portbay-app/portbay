//! Stack-aware database-connection injection.
//!
//! Provisioning a per-project database has to write the connection coordinates
//! where the project's framework will actually read them — and under the key
//! names that framework expects. A one-size-fits-all Laravel `.env` is wrong
//! for most of the stacks PortBay supports.
//!
//! The strategy is a universal denominator plus targeted special-cases:
//!
//! - **`DATABASE_URL` is always written.** Prisma, Drizzle, Knex (Node), Rails,
//!   Django (`dj-database-url`), SQLAlchemy (Python), Doctrine (Symfony), and
//!   most Go setups read this single DSN.
//! - **Laravel** additionally needs the discrete `DB_*` block, and its
//!   `DB_CONNECTION` driver key differs from our engine id (`pgsql`, not
//!   `postgres`) — see [`DatabaseEngine::laravel_driver_id`].
//! - **Next.js** loads `.env.local` (gitignored, takes precedence over `.env`),
//!   so the file target changes for it.
//!
//! Everything is written through `upsert_dotenv`, which preserves unrelated
//! keys and comments, so re-provisioning only rewrites the connection keys.

use std::path::Path;

use crate::registry::{DatabaseEngine, ProjectType};

/// The framework family that decides how connection details are shaped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Framework {
    /// PHP/Laravel — discrete `DB_*` block with a Laravel driver key.
    Laravel,
    /// Next.js — `DATABASE_URL` written into `.env.local`.
    NextJs,
    /// Everything else (Symfony, Rails, Django, plain Node/Vite, Go, Bun,
    /// Python, Flutter server, …) — `DATABASE_URL` in `.env`.
    UrlOnly,
}

/// A resolved injection plan: which file to write, and the key/value pairs.
#[derive(Debug, Clone)]
pub struct DbEnvProfile {
    /// Relative filename under the project root (`.env` or `.env.local`).
    pub file: &'static str,
    pub pairs: Vec<(&'static str, String)>,
}

/// Best-effort framework detection for connection shaping. Cheap substring
/// checks against manifests — a full parse isn't warranted here.
fn detect_framework(project_path: &Path, kind: ProjectType) -> Framework {
    // Next.js is detected up front by project kind; its env file differs.
    if kind == ProjectType::Next {
        return Framework::NextJs;
    }
    // PHP: distinguish Laravel (DB_* block) from Symfony/other (DATABASE_URL).
    if let Ok(body) = std::fs::read_to_string(project_path.join("composer.json")) {
        if body.contains("laravel/framework") || body.contains("laravel/laravel") {
            return Framework::Laravel;
        }
    }
    Framework::UrlOnly
}

/// Build the connection-injection plan for a provisioned SQL database.
///
/// `connection_url` is the full DSN with credentials inline; the discrete
/// `host`/`port`/`database`/`username`/`password` are only consumed by the
/// Laravel `DB_*` block.
#[allow(clippy::too_many_arguments)]
pub fn build(
    project_path: &Path,
    kind: ProjectType,
    engine: DatabaseEngine,
    host: &str,
    port: u16,
    database: &str,
    username: &str,
    password: &str,
    connection_url: &str,
) -> DbEnvProfile {
    let framework = detect_framework(project_path, kind);

    // DATABASE_URL is the universal denominator for every stack.
    let mut pairs: Vec<(&'static str, String)> = vec![("DATABASE_URL", connection_url.to_string())];

    if framework == Framework::Laravel {
        pairs.extend([
            ("DB_CONNECTION", engine.laravel_driver_id().to_string()),
            ("DB_HOST", host.to_string()),
            ("DB_PORT", port.to_string()),
            ("DB_DATABASE", database.to_string()),
            ("DB_USERNAME", username.to_string()),
            ("DB_PASSWORD", password.to_string()),
        ]);
    }

    let file = if framework == Framework::NextJs {
        ".env.local"
    } else {
        ".env"
    };

    DbEnvProfile { file, pairs }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_composer(dir: &Path, body: &str) {
        fs::write(dir.join("composer.json"), body).unwrap();
    }

    fn keys(profile: &DbEnvProfile) -> Vec<&'static str> {
        profile.pairs.iter().map(|(k, _)| *k).collect()
    }

    #[test]
    fn laravel_gets_full_db_block_with_pgsql_driver() {
        let dir = std::env::temp_dir().join(format!("pb-envprof-laravel-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        write_composer(&dir, r#"{ "require": { "laravel/framework": "^11.0" } }"#);

        let p = build(
            &dir,
            ProjectType::Php,
            DatabaseEngine::Postgres,
            "127.0.0.1",
            5432,
            "app_dev",
            "app",
            "secret",
            "postgresql://app:secret@127.0.0.1:5432/app_dev",
        );
        assert_eq!(p.file, ".env");
        assert!(keys(&p).contains(&"DB_CONNECTION"));
        assert!(keys(&p).contains(&"DATABASE_URL"));
        let driver = p.pairs.iter().find(|(k, _)| *k == "DB_CONNECTION").unwrap();
        assert_eq!(driver.1, "pgsql"); // not "postgres"
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn next_js_targets_env_local_url_only() {
        let dir = std::env::temp_dir().join(format!("pb-envprof-next-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = build(
            &dir,
            ProjectType::Next,
            DatabaseEngine::Postgres,
            "127.0.0.1",
            5432,
            "app_dev",
            "app",
            "secret",
            "postgresql://app:secret@127.0.0.1:5432/app_dev",
        );
        assert_eq!(p.file, ".env.local");
        assert_eq!(keys(&p), vec!["DATABASE_URL"]);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn plain_node_gets_url_only_in_env() {
        let dir = std::env::temp_dir().join(format!("pb-envprof-node-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = build(
            &dir,
            ProjectType::Node,
            DatabaseEngine::Mysql,
            "127.0.0.1",
            3306,
            "app_dev",
            "app",
            "secret",
            "mysql://app:secret@127.0.0.1:3306/app_dev",
        );
        assert_eq!(p.file, ".env");
        assert_eq!(keys(&p), vec!["DATABASE_URL"]);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mariadb_laravel_driver_is_mysql() {
        let dir = std::env::temp_dir().join(format!("pb-envprof-maria-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        write_composer(&dir, r#"{ "require": { "laravel/framework": "^10.0" } }"#);
        let p = build(
            &dir,
            ProjectType::Php,
            DatabaseEngine::Mariadb,
            "127.0.0.1",
            3306,
            "app_dev",
            "app",
            "secret",
            "mysql://app:secret@127.0.0.1:3306/app_dev",
        );
        let driver = p.pairs.iter().find(|(k, _)| *k == "DB_CONNECTION").unwrap();
        assert_eq!(driver.1, "mysql");
        fs::remove_dir_all(&dir).ok();
    }
}
