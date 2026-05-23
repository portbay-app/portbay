//! Database-connection surfacing for the project detail panel.
//!
//! Reads a project's on-disk `.env` (the Laravel/Symfony convention) and
//! turns its `DB_*` variables into structured connections the GUI renders
//! inline — answering the "what's the DB connection again?" lookup every
//! debug session starts with, without a trip to the project folder.
//!
//! Read-only: we never write the project's `.env`. The default `DB_*` set
//! becomes the "Default" connection; any `<PREFIX>_DB_HOST` / `_DB_DATABASE`
//! keys define additional named connections (e.g. a read replica).

use serde::Serialize;
use tauri::State;
use url::Url;

use crate::commands::projects::load_registry;
use crate::commands::system::parse_dotenv;
use crate::error::{AppError, AppResult};
use crate::registry::ProjectId;
use crate::state::AppState;

/// One database connection parsed from a project's `.env`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbConnection {
    /// Display label — "Default" for the primary `DB_*` set, otherwise the
    /// connection prefix (e.g. "READ" from `READ_DB_HOST`).
    pub name: String,
    /// Driver from `<prefix>DB_CONNECTION` (e.g. "mysql", "pgsql"); empty if unset.
    pub driver: String,
    pub host: String,
    pub port: String,
    pub database: String,
    pub username: String,
    pub password: String,
    /// A scheme URL a DB client (TablePlus, Sequel Ace, DBeaver) can open,
    /// or `None` when there's no host to connect to (e.g. sqlite).
    pub url: Option<String>,
}

/// `project_db_connections(id)` — parse the project's `.env` for `DB_*`
/// connection details. A missing or unreadable `.env` yields an empty list
/// (the common case for non-PHP projects), never an error.
#[tauri::command]
pub async fn project_db_connections(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Vec<DbConnection>> {
    let registry = load_registry(&state)?;
    let project = registry
        .get_project(&ProjectId::new(id.clone()))
        .ok_or_else(|| AppError::NotFound(id.clone()))?;

    let env_path = project.path.join(".env");
    let Ok(text) = std::fs::read_to_string(&env_path) else {
        return Ok(Vec::new());
    };

    Ok(build_db_connections(&parse_dotenv(&text)))
}

/// Build the connection list from parsed `.env` pairs. Pure, so it's
/// unit-testable without disk I/O.
pub(crate) fn build_db_connections(pairs: &[(String, String)]) -> Vec<DbConnection> {
    use std::collections::BTreeMap;
    let map: BTreeMap<&str, &str> = pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    // Discover connection prefixes: "" (default) plus any "<PREFIX>_" that
    // appears before DB_HOST / DB_DATABASE. The prefix must be empty or end
    // in '_', so `SOMEDB_HOST` doesn't masquerade as a "SOME" connection.
    let mut prefixes: Vec<String> = Vec::new();
    for key in map.keys() {
        for field in ["DB_HOST", "DB_DATABASE"] {
            if let Some(pref) = key.strip_suffix(field) {
                if (pref.is_empty() || pref.ends_with('_'))
                    && !prefixes.iter().any(|p| p == pref)
                {
                    prefixes.push(pref.to_string());
                }
            }
        }
    }
    prefixes.sort(); // "" sorts first → Default leads, then alphabetical.

    let mut out = Vec::new();
    for pref in prefixes {
        let get = |field: &str| -> String {
            map.get(format!("{pref}{field}").as_str())
                .copied()
                .unwrap_or_default()
                .to_string()
        };
        let host = get("DB_HOST");
        let database = get("DB_DATABASE");
        if host.is_empty() && database.is_empty() {
            continue;
        }
        let driver = get("DB_CONNECTION");
        let port = get("DB_PORT");
        let username = get("DB_USERNAME");
        let password = get("DB_PASSWORD");
        let url = connection_url(&driver, &host, &port, &username, &password, &database);
        let name = if pref.is_empty() {
            "Default".to_string()
        } else {
            pref.trim_end_matches('_').to_string()
        };
        out.push(DbConnection {
            name,
            driver,
            host,
            port,
            database,
            username,
            password,
            url,
        });
    }
    out
}

/// Build a `scheme://user:pass@host:port/db` URL for a DB client to open.
/// `None` when there's no host (sqlite / hostless). Userinfo and path are
/// percent-encoded by the `url` crate, so passwords containing `@` or `:`
/// don't corrupt the URL.
fn connection_url(
    driver: &str,
    host: &str,
    port: &str,
    user: &str,
    pass: &str,
    database: &str,
) -> Option<String> {
    if host.is_empty() {
        return None;
    }
    let driver_lc = driver.to_ascii_lowercase();
    let scheme = match driver_lc.as_str() {
        "pgsql" | "postgres" | "postgresql" => "postgresql",
        "sqlsrv" => "sqlserver",
        "redis" => "redis",
        // Empty driver defaults to mysql — Laravel's historical default and
        // the dominant local-dev case.
        "" | "mysql" | "mariadb" => "mysql",
        other => other,
    };

    let mut url = Url::parse(&format!("{scheme}://{host}")).ok()?;
    if !port.is_empty() {
        if let Ok(p) = port.parse::<u16>() {
            let _ = url.set_port(Some(p));
        }
    }
    if !user.is_empty() {
        let _ = url.set_username(user);
    }
    if !pass.is_empty() {
        let _ = url.set_password(Some(pass));
    }
    if !database.is_empty() {
        url.set_path(&format!("/{database}"));
    }
    Some(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn parses_a_standard_laravel_connection() {
        let conns = build_db_connections(&pairs(&[
            ("APP_ENV", "local"),
            ("DB_CONNECTION", "mysql"),
            ("DB_HOST", "127.0.0.1"),
            ("DB_PORT", "3306"),
            ("DB_DATABASE", "myapp"),
            ("DB_USERNAME", "root"),
            ("DB_PASSWORD", "secret"),
        ]));
        assert_eq!(conns.len(), 1);
        let c = &conns[0];
        assert_eq!(c.name, "Default");
        assert_eq!(c.driver, "mysql");
        assert_eq!(c.database, "myapp");
        assert_eq!(c.url.as_deref(), Some("mysql://root:secret@127.0.0.1:3306/myapp"));
    }

    #[test]
    fn percent_encodes_special_chars_in_password() {
        let conns = build_db_connections(&pairs(&[
            ("DB_HOST", "localhost"),
            ("DB_USERNAME", "user"),
            ("DB_PASSWORD", "p@ss:w0rd"),
            ("DB_DATABASE", "app"),
        ]));
        // '@' and ':' in the password must not break the authority.
        assert_eq!(
            conns[0].url.as_deref(),
            Some("mysql://user:p%40ss%3Aw0rd@localhost/app")
        );
    }

    #[test]
    fn postgres_driver_maps_to_postgresql_scheme() {
        let conns = build_db_connections(&pairs(&[
            ("DB_CONNECTION", "pgsql"),
            ("DB_HOST", "db.local"),
            ("DB_PORT", "5432"),
            ("DB_DATABASE", "store"),
        ]));
        assert!(conns[0].url.as_deref().unwrap().starts_with("postgresql://db.local:5432/store"));
    }

    #[test]
    fn sqlite_has_no_openable_url() {
        let conns = build_db_connections(&pairs(&[
            ("DB_CONNECTION", "sqlite"),
            ("DB_DATABASE", "/abs/database.sqlite"),
        ]));
        assert_eq!(conns.len(), 1);
        assert_eq!(conns[0].name, "Default");
        assert!(conns[0].url.is_none(), "no host → nothing to open");
    }

    #[test]
    fn additional_prefixed_connection_is_detected() {
        let conns = build_db_connections(&pairs(&[
            ("DB_HOST", "127.0.0.1"),
            ("DB_DATABASE", "primary"),
            ("READ_DB_HOST", "replica.local"),
            ("READ_DB_DATABASE", "primary"),
        ]));
        assert_eq!(conns.len(), 2);
        // Default sorts first.
        assert_eq!(conns[0].name, "Default");
        assert_eq!(conns[1].name, "READ");
        assert_eq!(conns[1].host, "replica.local");
    }

    #[test]
    fn no_db_vars_yields_no_connections() {
        let conns = build_db_connections(&pairs(&[("APP_ENV", "local"), ("APP_KEY", "base64:x")]));
        assert!(conns.is_empty());
    }

    #[test]
    fn lookalike_key_does_not_create_a_connection() {
        // `SOMEDB_HOST` ends with "DB_HOST" but the prefix "SOME" doesn't end
        // in '_', so it must not become a connection.
        let conns = build_db_connections(&pairs(&[("SOMEDB_HOST", "x")]));
        assert!(conns.is_empty());
    }
}
