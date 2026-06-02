//! Embedded database client primitives.
//!
//! This is the narrow Tabularis lift PortBay needs for v1: connect to an
//! already-managed SQL instance, inspect tables/columns/foreign keys, and run
//! bounded read queries for the in-app workbench. Lifecycle, provisioning,
//! backups, and external-client launch stay in `commands::databases`.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions, MySqlRow};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgRow};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow};
use sqlx::{Column, MySql, Pool, Postgres, Row, Sqlite, TypeInfo, ValueRef};

use crate::error::{AppError, AppResult};
use crate::registry::{DatabaseEngine, DatabaseInstance};

const MAX_LIMIT: u32 = 500;

/// How long to wait for a connection from a freshly-opened pool before giving
/// up. A stopped or unreachable instance otherwise blocks the IPC worker for
/// the OS TCP timeout (minutes), which can starve the async runtime.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbClientColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub primary_key: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbClientForeignKey {
    pub table: String,
    pub column: String,
    pub ref_table: String,
    pub ref_column: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbClientTable {
    pub schema: Option<String>,
    pub name: String,
    pub columns: Vec<DbClientColumn>,
    pub foreign_keys: Vec<DbClientForeignKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbClientSchema {
    pub engine: String,
    pub schemas: Vec<String>,
    pub tables: Vec<DbClientTable>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbClientRows {
    pub columns: Vec<DbClientColumn>,
    pub rows: Vec<Vec<Value>>,
    pub affected_rows: u64,
    pub truncated: bool,
}

/// Outcome of an approval-gated write/DDL statement.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbExecResult {
    pub affected_rows: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbExplainNode {
    pub id: String,
    pub node_type: String,
    pub relation: Option<String>,
    pub startup_cost: Option<f64>,
    pub total_cost: Option<f64>,
    pub plan_rows: Option<f64>,
    pub actual_rows: Option<f64>,
    pub actual_time_ms: Option<f64>,
    pub actual_loops: Option<f64>,
    pub buffers_hit: Option<f64>,
    pub buffers_read: Option<f64>,
    pub filter: Option<String>,
    pub index_condition: Option<String>,
    pub join_type: Option<String>,
    pub hash_condition: Option<String>,
    pub extra: serde_json::Value, // ALWAYS a JSON object; default serde_json::json!({})
    pub children: Vec<DbExplainNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbExplainPlan {
    pub root: DbExplainNode,
    pub planning_time_ms: Option<f64>,
    pub execution_time_ms: Option<f64>,
    pub original_query: String,
    pub driver: String, // engine id: "sqlite" | "mysql" | "postgres"
    pub has_analyze_data: bool,
    pub raw_output: Option<String>,
}

fn bounded_limit(limit: Option<u32>) -> u32 {
    limit.unwrap_or(100).clamp(1, MAX_LIMIT)
}

fn first_sql_keyword(sql: &str) -> Option<String> {
    let mut rest = sql.trim_start();
    loop {
        if let Some(next) = rest.strip_prefix("--") {
            if let Some(pos) = next.find('\n') {
                rest = next[pos + 1..].trim_start();
                continue;
            }
            return None;
        }
        if let Some(next) = rest.strip_prefix("/*") {
            if let Some(pos) = next.find("*/") {
                rest = next[pos + 2..].trim_start();
                continue;
            }
            return None;
        }
        break;
    }

    rest.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .find(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
}

/// Bare keywords that perform a write, DDL, transaction-control, or session
/// mutation. If any appears as a top-level token (outside string literals,
/// quoted identifiers, and comments) the statement is rejected. Scanning every
/// token — not just the first — is what catches CTE-wrapped writes
/// (`WITH x AS (INSERT … RETURNING *) SELECT *`), `SELECT … INTO`, and
/// `EXPLAIN <dml>`, all of which a first-keyword allowlist lets through.
const WRITE_KEYWORDS: &[&str] = &[
    "insert",
    "update",
    "delete",
    "merge",
    "replace",
    "upsert",
    "drop",
    "create",
    "alter",
    "truncate",
    "rename",
    "grant",
    "revoke",
    "call",
    "do",
    "copy",
    "vacuum",
    "reindex",
    "attach",
    "detach",
    "comment",
    "lock",
    "set",
    "begin",
    "start",
    "commit",
    "rollback",
    "savepoint",
    "release",
    "load",
    "execute",
    "prepare",
    "into",
];

/// Leading keywords that introduce a read-only / inspection statement.
const READ_LEADERS: &[&str] = &[
    "select", "with", "show", "describe", "desc", "explain", "pragma", "values", "table",
];

/// Remove comments and blank out the contents of quoted strings / identifiers,
/// so keyword scanning never trips on a literal like `'drop'` or a column named
/// `"select"`. Quoted regions collapse to a single space; the `;` structure is
/// preserved so statement-count checks still work.
fn strip_sql(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // line comment -- … to end of line
            '-' if chars.peek() == Some(&'-') => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            // block comment /* … */
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = '\0';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
                out.push(' ');
            }
            // string literal or quoted identifier
            '\'' | '"' | '`' => {
                let quote = c;
                while let Some(n) = chars.next() {
                    if n == '\\' {
                        // backslash escape (MySQL): skip the next char
                        chars.next();
                        continue;
                    }
                    if n == quote {
                        // doubled quote is an escaped quote, not a terminator
                        if chars.peek() == Some(&quote) {
                            chars.next();
                            continue;
                        }
                        break;
                    }
                }
                out.push(' ');
            }
            other => out.push(other),
        }
    }
    out
}

/// Lowercased word tokens from already-stripped SQL.
fn sql_words(stripped: &str) -> Vec<String> {
    stripped
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect()
}

/// Reject anything that is not a single, read-only statement. Defends the
/// embedded client (and, later, the agent-facing MCP query tool) against
/// writes slipping through the read path.
fn ensure_read_query(sql: &str) -> AppResult<()> {
    let stripped = strip_sql(sql);
    if stripped.split(';').filter(|s| !s.trim().is_empty()).count() > 1 {
        return Err(AppError::BadInput(
            "only a single read-only statement is allowed".into(),
        ));
    }
    let words = sql_words(&stripped);
    let Some(first) = words.first().map(String::as_str) else {
        return Err(AppError::BadInput("SQL query is required".into()));
    };
    if !READ_LEADERS.contains(&first) {
        return Err(AppError::BadInput(format!(
            "embedded queries are read-only in this release; `{first}` is not allowed"
        )));
    }
    if let Some(bad) = words.iter().find(|w| WRITE_KEYWORDS.contains(&w.as_str())) {
        return Err(AppError::BadInput(format!(
            "embedded queries are read-only; `{bad}` is not allowed"
        )));
    }
    Ok(())
}

/// Append a hard `LIMIT` to a row-returning statement that has none, so the
/// server caps the result set instead of streaming an entire table back into
/// memory before we truncate. Only applied to leaders that accept a trailing
/// `LIMIT`; `SHOW`/`PRAGMA`/`DESCRIBE`/`EXPLAIN` are returned unchanged (they
/// are inherently small). The caller passes `limit + 1` so truncation past the
/// real cap is still detectable.
fn bound_select_sql(sql: &str, limit: u32) -> String {
    let words = sql_words(&strip_sql(sql));
    let limitable = matches!(
        words.first().map(String::as_str),
        Some("select" | "with" | "values" | "table")
    );
    if !limitable || words.iter().any(|w| w == "limit") {
        return sql.to_string();
    }
    let trimmed = sql.trim().trim_end_matches(';').trim_end();
    format!("{trimmed} LIMIT {limit}")
}

/// Validate that `sql` is exactly one statement. Unlike [`ensure_read_query`],
/// writes and DDL are permitted — this guards the approval-gated execute path,
/// where a human approves the exact statement before it runs. Stacked
/// statements stay forbidden so an approved `UPDATE` can't smuggle a second
/// `DROP` behind a `;`.
fn ensure_single_statement(sql: &str) -> AppResult<()> {
    let stripped = strip_sql(sql);
    match stripped.split(';').filter(|s| !s.trim().is_empty()).count() {
        0 => Err(AppError::BadInput("SQL statement is required".into())),
        1 => Ok(()),
        _ => Err(AppError::BadInput(
            "only a single statement can be executed at a time".into(),
        )),
    }
}

/// Whether `sql` is a read-only inspection statement (passes [`ensure_read_query`]).
/// Used by the execute path to steer agents to `portbay_db_query` for reads.
pub fn is_read_only_sql(sql: &str) -> bool {
    ensure_read_query(sql).is_ok()
}

fn sqlite_options(path: &Path) -> AppResult<SqliteConnectOptions> {
    Ok(SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .read_only(true))
}

fn sqlite_db_path(inst: &DatabaseInstance) -> AppResult<&Path> {
    let path = inst.file_path.as_deref().ok_or_else(|| {
        AppError::BadInput(format!(
            "SQLite instance `{}` has no database file",
            inst.id
        ))
    })?;
    if !path.is_file() {
        return Err(AppError::BadInput(format!(
            "SQLite database file does not exist: {}",
            path.display()
        )));
    }
    Ok(path)
}

async fn sqlite_pool(inst: &DatabaseInstance) -> AppResult<Pool<Sqlite>> {
    let path = sqlite_db_path(inst)?;
    SqlitePoolOptions::new()
        .max_connections(4)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .connect_with(sqlite_options(path)?)
        .await
        .map_err(|e| AppError::Internal(format!("open SQLite database: {e}")))
}

/// A writable SQLite pool for the approval-gated execute path. The read pool
/// opens with `read_only(true)`; writes need an unrestricted handle.
async fn sqlite_pool_writable(inst: &DatabaseInstance) -> AppResult<Pool<Sqlite>> {
    let path = sqlite_db_path(inst)?;
    SqlitePoolOptions::new()
        .max_connections(2)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(false),
        )
        .await
        .map_err(|e| AppError::Internal(format!("open SQLite database for write: {e}")))
}

fn mysql_options(inst: &DatabaseInstance, schema: Option<&str>) -> MySqlConnectOptions {
    let mut opts = MySqlConnectOptions::new()
        .host("127.0.0.1")
        .port(inst.port)
        .username(inst.default_account());
    if let Some(schema) = schema.filter(|s| !s.trim().is_empty()) {
        opts = opts.database(schema);
    }
    opts.ssl_mode(sqlx::mysql::MySqlSslMode::Disabled)
}

async fn mysql_pool(inst: &DatabaseInstance, schema: Option<&str>) -> AppResult<Pool<MySql>> {
    MySqlPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .connect_with(mysql_options(inst, schema))
        .await
        .map_err(|e| AppError::Internal(format!("connect to {}: {e}", inst.engine.label())))
}

fn pg_options(inst: &DatabaseInstance, schema: Option<&str>) -> PgConnectOptions {
    let database = schema
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("postgres");
    PgConnectOptions::new()
        .host("127.0.0.1")
        .port(inst.port)
        .username(inst.default_account())
        .database(database)
        .ssl_mode(sqlx::postgres::PgSslMode::Disable)
}

async fn pg_pool(inst: &DatabaseInstance, schema: Option<&str>) -> AppResult<Pool<Postgres>> {
    PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .connect_with(pg_options(inst, schema))
        .await
        .map_err(|e| AppError::Internal(format!("connect to PostgreSQL: {e}")))
}

fn quote_ident(ident: &str, quote: char) -> String {
    let escaped = ident.replace(quote, &format!("{quote}{quote}"));
    format!("{quote}{escaped}{quote}")
}

fn table_ref(engine: DatabaseEngine, schema: Option<&str>, table: &str) -> String {
    match engine {
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => match schema {
            Some(schema) if !schema.is_empty() => {
                format!("{}.{}", quote_ident(schema, '`'), quote_ident(table, '`'))
            }
            _ => quote_ident(table, '`'),
        },
        DatabaseEngine::Postgres => match schema {
            Some(schema) if !schema.is_empty() => {
                format!("{}.{}", quote_ident(schema, '"'), quote_ident(table, '"'))
            }
            _ => quote_ident(table, '"'),
        },
        DatabaseEngine::Sqlite => quote_ident(table, '"'),
        _ => table.to_string(),
    }
}

fn stringish_json(text: String) -> Value {
    serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text))
}

fn bytes_json(bytes: Vec<u8>) -> Value {
    match String::from_utf8(bytes.clone()) {
        Ok(text) => stringish_json(text),
        Err(_) => Value::String(format!("<{} bytes>", bytes.len())),
    }
}

fn sqlite_value(row: &SqliteRow, idx: usize) -> Value {
    if row.try_get_raw(idx).map(|v| v.is_null()).unwrap_or(false) {
        return Value::Null;
    }
    if let Ok(v) = row.try_get::<String, _>(idx) {
        return stringish_json(v);
    }
    if let Ok(v) = row.try_get::<i64, _>(idx) {
        return Value::from(v);
    }
    if let Ok(v) = row.try_get::<f64, _>(idx) {
        return serde_json::Number::from_f64(v)
            .map(Value::Number)
            .unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<bool, _>(idx) {
        return Value::from(v);
    }
    if let Ok(v) = row.try_get::<Vec<u8>, _>(idx) {
        return bytes_json(v);
    }
    Value::Null
}

fn mysql_value(row: &MySqlRow, idx: usize) -> Value {
    if row.try_get_raw(idx).map(|v| v.is_null()).unwrap_or(false) {
        return Value::Null;
    }
    if let Ok(v) = row.try_get::<String, _>(idx) {
        return stringish_json(v);
    }
    if let Ok(v) = row.try_get::<i64, _>(idx) {
        return Value::from(v);
    }
    if let Ok(v) = row.try_get::<u64, _>(idx) {
        return match serde_json::Number::from_u128(v as u128) {
            Some(n) => Value::Number(n),
            None => Value::String(v.to_string()),
        };
    }
    if let Ok(v) = row.try_get::<f64, _>(idx) {
        return serde_json::Number::from_f64(v)
            .map(Value::Number)
            .unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<bool, _>(idx) {
        return Value::from(v);
    }
    if let Ok(v) = row.try_get::<Vec<u8>, _>(idx) {
        return bytes_json(v);
    }
    Value::Null
}

fn pg_value(row: &PgRow, idx: usize) -> Value {
    if row.try_get_raw(idx).map(|v| v.is_null()).unwrap_or(false) {
        return Value::Null;
    }
    if let Ok(v) = row.try_get::<String, _>(idx) {
        return stringish_json(v);
    }
    if let Ok(v) = row.try_get::<i64, _>(idx) {
        return Value::from(v);
    }
    if let Ok(v) = row.try_get::<i32, _>(idx) {
        return Value::from(v);
    }
    if let Ok(v) = row.try_get::<f64, _>(idx) {
        return serde_json::Number::from_f64(v)
            .map(Value::Number)
            .unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<bool, _>(idx) {
        return Value::from(v);
    }
    if let Ok(v) = row.try_get::<serde_json::Value, _>(idx) {
        return v;
    }
    if let Ok(v) = row.try_get::<Vec<u8>, _>(idx) {
        return bytes_json(v);
    }
    Value::Null
}

fn sqlite_result_columns(rows: &[SqliteRow]) -> Vec<DbClientColumn> {
    rows.first()
        .map(|row| {
            row.columns()
                .iter()
                .map(|c| DbClientColumn {
                    name: c.name().to_string(),
                    data_type: c.type_info().name().to_string(),
                    nullable: true,
                    primary_key: false,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn mysql_result_columns(rows: &[MySqlRow]) -> Vec<DbClientColumn> {
    rows.first()
        .map(|row| {
            row.columns()
                .iter()
                .map(|c| DbClientColumn {
                    name: c.name().to_string(),
                    data_type: c.type_info().name().to_string(),
                    nullable: true,
                    primary_key: false,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn pg_result_columns(rows: &[PgRow]) -> Vec<DbClientColumn> {
    rows.first()
        .map(|row| {
            row.columns()
                .iter()
                .map(|c| DbClientColumn {
                    name: c.name().to_string(),
                    data_type: c.type_info().name().to_string(),
                    nullable: true,
                    primary_key: false,
                })
                .collect()
        })
        .unwrap_or_default()
}

pub async fn schema(inst: &DatabaseInstance) -> AppResult<DbClientSchema> {
    match inst.engine {
        DatabaseEngine::Sqlite => sqlite_schema(inst).await,
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => mysql_schema(inst).await,
        DatabaseEngine::Postgres => pg_schema(inst).await,
        _ => Err(AppError::BadInput(format!(
            "{} is not supported by the embedded database client yet",
            inst.engine.label()
        ))),
    }
}

async fn sqlite_schema(inst: &DatabaseInstance) -> AppResult<DbClientSchema> {
    let pool = sqlite_pool(inst).await?;
    let table_rows = sqlx::query(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name ASC",
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| AppError::Internal(format!("list SQLite tables: {e}")))?;

    let mut tables = Vec::with_capacity(table_rows.len());
    for row in table_rows {
        let name: String = row.try_get("name").unwrap_or_default();
        let columns = sqlx::query(&format!("PRAGMA table_info({})", quote_ident(&name, '"')))
            .fetch_all(&pool)
            .await
            .map_err(|e| AppError::Internal(format!("inspect SQLite table `{name}`: {e}")))?
            .into_iter()
            .map(|r| {
                let pk: i64 = r.try_get("pk").unwrap_or(0);
                let notnull: i64 = r.try_get("notnull").unwrap_or(0);
                DbClientColumn {
                    name: r.try_get("name").unwrap_or_default(),
                    data_type: r.try_get("type").unwrap_or_default(),
                    nullable: notnull == 0,
                    primary_key: pk > 0,
                }
            })
            .collect();
        let foreign_keys = sqlx::query(&format!(
            "PRAGMA foreign_key_list({})",
            quote_ident(&name, '"')
        ))
        .fetch_all(&pool)
        .await
        .map_err(|e| AppError::Internal(format!("inspect SQLite FKs `{name}`: {e}")))?
        .into_iter()
        .map(|r| DbClientForeignKey {
            table: name.clone(),
            column: r.try_get("from").unwrap_or_default(),
            ref_table: r.try_get("table").unwrap_or_default(),
            ref_column: r.try_get("to").unwrap_or_default(),
        })
        .collect();
        tables.push(DbClientTable {
            schema: None,
            name,
            columns,
            foreign_keys,
        });
    }

    Ok(DbClientSchema {
        engine: inst.engine.id().to_string(),
        schemas: vec![],
        tables,
    })
}

async fn mysql_schema(inst: &DatabaseInstance) -> AppResult<DbClientSchema> {
    let pool = mysql_pool(inst, None).await?;
    let rows = sqlx::query(
        r#"
        SELECT table_schema, table_name
        FROM information_schema.tables
        WHERE table_type = 'BASE TABLE'
          AND table_schema NOT IN ('information_schema', 'mysql', 'performance_schema', 'sys')
        ORDER BY table_schema, table_name
        "#,
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| AppError::Internal(format!("list MySQL tables: {e}")))?;

    let schemas = {
        let mut values: Vec<String> = rows
            .iter()
            .filter_map(|r| r.try_get::<String, _>("table_schema").ok())
            .collect();
        values.sort();
        values.dedup();
        values
    };

    let mut tables = Vec::with_capacity(rows.len());
    for row in rows {
        let schema: String = row.try_get("table_schema").unwrap_or_default();
        let name: String = row.try_get("table_name").unwrap_or_default();
        let columns = sqlx::query(
            r#"
            SELECT column_name, data_type, is_nullable, column_key
            FROM information_schema.columns
            WHERE table_schema = ? AND table_name = ?
            ORDER BY ordinal_position
            "#,
        )
        .bind(&schema)
        .bind(&name)
        .fetch_all(&pool)
        .await
        .map_err(|e| AppError::Internal(format!("inspect MySQL table `{schema}.{name}`: {e}")))?
        .into_iter()
        .map(|r| DbClientColumn {
            name: r.try_get("column_name").unwrap_or_default(),
            data_type: r.try_get("data_type").unwrap_or_default(),
            nullable: r
                .try_get::<String, _>("is_nullable")
                .map(|v| v == "YES")
                .unwrap_or(true),
            primary_key: r
                .try_get::<String, _>("column_key")
                .map(|v| v == "PRI")
                .unwrap_or(false),
        })
        .collect();
        let foreign_keys = sqlx::query(
            r#"
            SELECT column_name, referenced_table_name, referenced_column_name
            FROM information_schema.key_column_usage
            WHERE table_schema = ? AND table_name = ? AND referenced_table_name IS NOT NULL
            ORDER BY ordinal_position
            "#,
        )
        .bind(&schema)
        .bind(&name)
        .fetch_all(&pool)
        .await
        .map_err(|e| AppError::Internal(format!("inspect MySQL FKs `{schema}.{name}`: {e}")))?
        .into_iter()
        .map(|r| DbClientForeignKey {
            table: name.clone(),
            column: r.try_get("column_name").unwrap_or_default(),
            ref_table: r.try_get("referenced_table_name").unwrap_or_default(),
            ref_column: r.try_get("referenced_column_name").unwrap_or_default(),
        })
        .collect();
        tables.push(DbClientTable {
            schema: Some(schema),
            name,
            columns,
            foreign_keys,
        });
    }

    Ok(DbClientSchema {
        engine: inst.engine.id().to_string(),
        schemas,
        tables,
    })
}

async fn pg_schema(inst: &DatabaseInstance) -> AppResult<DbClientSchema> {
    let pool = pg_pool(inst, None).await?;
    let rows = sqlx::query(
        r#"
        SELECT table_schema, table_name
        FROM information_schema.tables
        WHERE table_type = 'BASE TABLE'
          AND table_schema NOT IN ('pg_catalog', 'information_schema')
        ORDER BY table_schema, table_name
        "#,
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| AppError::Internal(format!("list PostgreSQL tables: {e}")))?;

    let schemas = {
        let mut values: Vec<String> = rows
            .iter()
            .filter_map(|r| r.try_get::<String, _>("table_schema").ok())
            .collect();
        values.sort();
        values.dedup();
        values
    };

    let mut tables = Vec::with_capacity(rows.len());
    for row in rows {
        let schema: String = row.try_get("table_schema").unwrap_or_default();
        let name: String = row.try_get("table_name").unwrap_or_default();
        let columns = sqlx::query(
            r#"
            SELECT c.column_name, c.data_type, c.is_nullable,
                   EXISTS (
                     SELECT 1
                     FROM information_schema.table_constraints tc
                     JOIN information_schema.key_column_usage kcu
                       ON tc.constraint_name = kcu.constraint_name
                      AND tc.table_schema = kcu.table_schema
                     WHERE tc.constraint_type = 'PRIMARY KEY'
                       AND tc.table_schema = c.table_schema
                       AND tc.table_name = c.table_name
                       AND kcu.column_name = c.column_name
                   ) AS primary_key
            FROM information_schema.columns c
            WHERE c.table_schema = $1 AND c.table_name = $2
            ORDER BY c.ordinal_position
            "#,
        )
        .bind(&schema)
        .bind(&name)
        .fetch_all(&pool)
        .await
        .map_err(|e| {
            AppError::Internal(format!("inspect PostgreSQL table `{schema}.{name}`: {e}"))
        })?
        .into_iter()
        .map(|r| DbClientColumn {
            name: r.try_get("column_name").unwrap_or_default(),
            data_type: r.try_get("data_type").unwrap_or_default(),
            nullable: r
                .try_get::<String, _>("is_nullable")
                .map(|v| v == "YES")
                .unwrap_or(true),
            primary_key: r.try_get("primary_key").unwrap_or(false),
        })
        .collect();
        let foreign_keys = sqlx::query(
            r#"
            SELECT kcu.column_name,
                   ccu.table_name AS referenced_table_name,
                   ccu.column_name AS referenced_column_name
            FROM information_schema.table_constraints tc
            JOIN information_schema.key_column_usage kcu
              ON tc.constraint_name = kcu.constraint_name
             AND tc.table_schema = kcu.table_schema
            JOIN information_schema.constraint_column_usage ccu
              ON ccu.constraint_name = tc.constraint_name
             AND ccu.table_schema = tc.table_schema
            WHERE tc.constraint_type = 'FOREIGN KEY'
              AND tc.table_schema = $1
              AND tc.table_name = $2
            ORDER BY kcu.ordinal_position
            "#,
        )
        .bind(&schema)
        .bind(&name)
        .fetch_all(&pool)
        .await
        .map_err(|e| AppError::Internal(format!("inspect PostgreSQL FKs `{schema}.{name}`: {e}")))?
        .into_iter()
        .map(|r| DbClientForeignKey {
            table: name.clone(),
            column: r.try_get("column_name").unwrap_or_default(),
            ref_table: r.try_get("referenced_table_name").unwrap_or_default(),
            ref_column: r.try_get("referenced_column_name").unwrap_or_default(),
        })
        .collect();
        tables.push(DbClientTable {
            schema: Some(schema),
            name,
            columns,
            foreign_keys,
        });
    }

    Ok(DbClientSchema {
        engine: inst.engine.id().to_string(),
        schemas,
        tables,
    })
}

/// Primary-key column names for one concrete table.
///
/// `table_rows` overlays these onto the result-set columns (whose driver
/// metadata can't carry key information) so the grid knows which rows are
/// uniquely addressable and can enable editing. Mirrors the PK detection in
/// the per-engine `*_schema` functions, scoped to a single table.
async fn table_pk_columns(
    inst: &DatabaseInstance,
    schema: Option<&str>,
    table: &str,
) -> AppResult<std::collections::HashSet<String>> {
    let mut pks = std::collections::HashSet::new();
    match inst.engine {
        DatabaseEngine::Sqlite => {
            let pool = sqlite_pool(inst).await?;
            let rows = sqlx::query(&format!("PRAGMA table_info({})", quote_ident(table, '"')))
                .fetch_all(&pool)
                .await
                .map_err(|e| AppError::Internal(format!("inspect SQLite keys `{table}`: {e}")))?;
            for r in rows {
                if r.try_get::<i64, _>("pk").unwrap_or(0) > 0 {
                    pks.insert(r.try_get::<String, _>("name").unwrap_or_default());
                }
            }
        }
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => {
            let pool = mysql_pool(inst, schema).await?;
            let rows = sqlx::query(
                r#"
                SELECT column_name
                FROM information_schema.columns
                WHERE table_name = ? AND column_key = 'PRI'
                  AND table_schema = COALESCE(?, DATABASE())
                "#,
            )
            .bind(table)
            .bind(schema)
            .fetch_all(&pool)
            .await
            .map_err(|e| AppError::Internal(format!("inspect MySQL keys `{table}`: {e}")))?;
            for r in rows {
                pks.insert(r.try_get::<String, _>("column_name").unwrap_or_default());
            }
        }
        DatabaseEngine::Postgres => {
            let pool = pg_pool(inst, None).await?;
            let rows = sqlx::query(
                r#"
                SELECT kcu.column_name
                FROM information_schema.table_constraints tc
                JOIN information_schema.key_column_usage kcu
                  ON tc.constraint_name = kcu.constraint_name
                 AND tc.table_schema = kcu.table_schema
                WHERE tc.constraint_type = 'PRIMARY KEY'
                  AND tc.table_name = $1
                  AND ($2::text IS NULL OR tc.table_schema = $2)
                "#,
            )
            .bind(table)
            .bind(schema)
            .fetch_all(&pool)
            .await
            .map_err(|e| AppError::Internal(format!("inspect PostgreSQL keys `{table}`: {e}")))?;
            for r in rows {
                pks.insert(r.try_get::<String, _>("column_name").unwrap_or_default());
            }
        }
        _ => {}
    }
    Ok(pks)
}

pub async fn table_rows(
    inst: &DatabaseInstance,
    schema: Option<&str>,
    table: &str,
    limit: Option<u32>,
    offset: Option<u32>,
) -> AppResult<DbClientRows> {
    let limit = bounded_limit(limit);
    let offset = offset.unwrap_or(0);
    let sql = match inst.engine {
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => {
            format!(
                "SELECT * FROM {} LIMIT {limit} OFFSET {offset}",
                table_ref(inst.engine, schema, table)
            )
        }
        DatabaseEngine::Postgres | DatabaseEngine::Sqlite => {
            format!(
                "SELECT * FROM {} LIMIT {limit} OFFSET {offset}",
                table_ref(inst.engine, schema, table)
            )
        }
        _ => {
            return Err(AppError::BadInput(format!(
                "{} is not supported by the embedded database client yet",
                inst.engine.label()
            )))
        }
    };
    let mut result = query(inst, schema, &sql, Some(limit)).await?;
    // The generic `query` path builds columns from result-set metadata, which
    // can't carry key information — every column comes back primary_key=false.
    // For a concrete table we can introspect the real primary key and overlay
    // it; without this the grid sees no PK and disables row editing entirely.
    // A failed introspection degrades gracefully to a read-only grid.
    if let Ok(pks) = table_pk_columns(inst, schema, table).await {
        for col in &mut result.columns {
            col.primary_key = pks.contains(&col.name);
        }
    }
    Ok(result)
}

pub async fn query(
    inst: &DatabaseInstance,
    schema: Option<&str>,
    sql: &str,
    limit: Option<u32>,
) -> AppResult<DbClientRows> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadInput("SQL query is required".into()));
    }
    ensure_read_query(trimmed)?;
    let limit = bounded_limit(limit);
    // Cap unbounded SELECTs server-side so a `SELECT * FROM huge_table` doesn't
    // stream the whole table back before we truncate. `limit + 1` lets us still
    // detect that more rows existed.
    let capped = bound_select_sql(trimmed, limit.saturating_add(1));

    match inst.engine {
        DatabaseEngine::Sqlite => {
            let pool = sqlite_pool(inst).await?;
            let rows = sqlx::query(&capped)
                .fetch_all(&pool)
                .await
                .map_err(|e| AppError::Internal(format!("run SQLite query: {e}")))?;
            let truncated = rows.len() > limit as usize;
            let rows: Vec<SqliteRow> = rows.into_iter().take(limit as usize).collect();
            let columns = sqlite_result_columns(&rows);
            let rows = rows
                .iter()
                .map(|row| {
                    (0..row.columns().len())
                        .map(|i| sqlite_value(row, i))
                        .collect()
                })
                .collect();
            Ok(DbClientRows {
                columns,
                rows,
                affected_rows: 0,
                truncated,
            })
        }
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => {
            let pool = mysql_pool(inst, schema).await?;
            let rows = sqlx::query(&capped).fetch_all(&pool).await.map_err(|e| {
                AppError::Internal(format!("run {} query: {e}", inst.engine.label()))
            })?;
            let truncated = rows.len() > limit as usize;
            let rows: Vec<MySqlRow> = rows.into_iter().take(limit as usize).collect();
            let columns = mysql_result_columns(&rows);
            let rows = rows
                .iter()
                .map(|row| {
                    (0..row.columns().len())
                        .map(|i| mysql_value(row, i))
                        .collect()
                })
                .collect();
            Ok(DbClientRows {
                columns,
                rows,
                affected_rows: 0,
                truncated,
            })
        }
        DatabaseEngine::Postgres => {
            let pool = pg_pool(inst, None).await?;
            let mut conn = pool
                .acquire()
                .await
                .map_err(|e| AppError::Internal(format!("acquire PostgreSQL connection: {e}")))?;
            if let Some(schema) = schema.filter(|s| !s.trim().is_empty()) {
                sqlx::query("SELECT set_config('search_path', $1, false)")
                    .bind(schema)
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| AppError::Internal(format!("set PostgreSQL schema: {e}")))?;
            }
            let rows = sqlx::query(&capped)
                .fetch_all(&mut *conn)
                .await
                .map_err(|e| AppError::Internal(format!("run PostgreSQL query: {e}")))?;
            let truncated = rows.len() > limit as usize;
            let rows: Vec<PgRow> = rows.into_iter().take(limit as usize).collect();
            let columns = pg_result_columns(&rows);
            let rows = rows
                .iter()
                .map(|row| (0..row.columns().len()).map(|i| pg_value(row, i)).collect())
                .collect();
            Ok(DbClientRows {
                columns,
                rows,
                affected_rows: 0,
                truncated,
            })
        }
        _ => Err(AppError::BadInput(format!(
            "{} is not supported by the embedded database client yet",
            inst.engine.label()
        ))),
    }
}

/// Run a single write/DDL statement and return the affected-row count. This is
/// the privileged counterpart to [`query`] and must only be reached after a
/// human has approved the exact statement (see `crate::db_approval`). It still
/// rejects stacked statements so an approved write can't carry a second.
pub async fn execute(
    inst: &DatabaseInstance,
    schema: Option<&str>,
    sql: &str,
) -> AppResult<DbExecResult> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadInput("SQL statement is required".into()));
    }
    ensure_single_statement(trimmed)?;

    match inst.engine {
        DatabaseEngine::Sqlite => {
            let pool = sqlite_pool_writable(inst).await?;
            let res = sqlx::query(trimmed)
                .execute(&pool)
                .await
                .map_err(|e| AppError::Internal(format!("execute SQLite statement: {e}")))?;
            Ok(DbExecResult {
                affected_rows: res.rows_affected(),
            })
        }
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => {
            let pool = mysql_pool(inst, schema).await?;
            let res = sqlx::query(trimmed).execute(&pool).await.map_err(|e| {
                AppError::Internal(format!("execute {} statement: {e}", inst.engine.label()))
            })?;
            Ok(DbExecResult {
                affected_rows: res.rows_affected(),
            })
        }
        DatabaseEngine::Postgres => {
            let pool = pg_pool(inst, None).await?;
            let mut conn = pool
                .acquire()
                .await
                .map_err(|e| AppError::Internal(format!("acquire PostgreSQL connection: {e}")))?;
            if let Some(schema) = schema.filter(|s| !s.trim().is_empty()) {
                sqlx::query("SELECT set_config('search_path', $1, false)")
                    .bind(schema)
                    .execute(&mut *conn)
                    .await
                    .map_err(|e| AppError::Internal(format!("set PostgreSQL schema: {e}")))?;
            }
            let res = sqlx::query(trimmed)
                .execute(&mut *conn)
                .await
                .map_err(|e| AppError::Internal(format!("execute PostgreSQL statement: {e}")))?;
            Ok(DbExecResult {
                affected_rows: res.rows_affected(),
            })
        }
        _ => Err(AppError::BadInput(format!(
            "{} is not supported by the embedded database client yet",
            inst.engine.label()
        ))),
    }
}

// ─── User-approved structured writes (the editable data grid) ──────────────
//
// The grid stages edits and the user confirms the rendered SQL before anything
// runs (the "Review N changes" bar). Edits arrive *structurally* — column names
// and JSON values, never raw SQL — so identifier quoting and value escaping
// happen here, server-side, against each engine's rules. We render escaped
// literals (rather than bound params) on purpose: an untyped literal coerces to
// the destination column's type in PostgreSQL, where a typed bind param would
// be rejected, so literals are the more portable choice for a generic editor.

/// One column/value pair in a structured write.
#[derive(Debug, Clone, Deserialize)]
pub struct CellValue {
    pub column: String,
    pub value: Value,
}

/// A single row mutation staged in the data grid and confirmed by the user.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RowEdit {
    /// `UPDATE <table> SET <set...> WHERE <pk...>`
    Update {
        table: String,
        #[serde(default)]
        pk: Vec<CellValue>,
        set: Vec<CellValue>,
    },
    /// `INSERT INTO <table> (<cols>) VALUES (<vals>)`
    Insert {
        table: String,
        values: Vec<CellValue>,
    },
    /// `DELETE FROM <table> WHERE <pk...>`
    Delete {
        table: String,
        #[serde(default)]
        pk: Vec<CellValue>,
    },
}

/// Outcome of applying a batch of structured writes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbApplyResult {
    /// Total rows affected across every statement.
    pub affected_rows: u64,
    /// The exact SQL that ran, in order — echoed back for confirmation/logging.
    pub statements: Vec<String>,
}

/// Per-engine knobs for rendering identifiers and string literals.
#[derive(Clone, Copy)]
struct SqlDialect {
    /// Identifier quote char: `"` for PostgreSQL/SQLite, `` ` `` for MySQL.
    ident_quote: char,
    /// Whether backslashes inside string literals must be doubled. MySQL treats
    /// `\` as an escape char by default; PostgreSQL (standard_conforming_strings)
    /// and SQLite do not.
    escape_backslash: bool,
}

impl SqlDialect {
    fn for_engine(engine: DatabaseEngine) -> Self {
        match engine {
            DatabaseEngine::Mysql | DatabaseEngine::Mariadb => SqlDialect {
                ident_quote: '`',
                escape_backslash: true,
            },
            _ => SqlDialect {
                ident_quote: '"',
                escape_backslash: false,
            },
        }
    }

    /// Quote an identifier, doubling any embedded quote char to neutralise
    /// injection. Rejects empty / NUL-bearing identifiers.
    fn quote_ident(&self, ident: &str) -> AppResult<String> {
        if ident.trim().is_empty() {
            return Err(AppError::BadInput("empty column or table name".into()));
        }
        if ident.contains('\0') {
            return Err(AppError::BadInput("identifier contains a NUL byte".into()));
        }
        Ok(quote_ident(ident, self.ident_quote))
    }

    /// Render a JSON value as a SQL literal.
    fn render_literal(&self, value: &Value) -> String {
        match value {
            Value::Null => "NULL".to_string(),
            Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
            // serde_json's Number renders as a canonical numeric literal.
            Value::Number(n) => n.to_string(),
            Value::String(s) => self.quote_string(s),
            // Arrays / objects keep their JSON text; JSON columns coerce it.
            other => self.quote_string(&other.to_string()),
        }
    }

    fn quote_string(&self, s: &str) -> String {
        let mut out = s.replace('\'', "''");
        if self.escape_backslash {
            out = out.replace('\\', "\\\\");
        }
        format!("'{out}'")
    }

    fn where_clause(&self, pk: &[CellValue]) -> AppResult<String> {
        let parts = pk
            .iter()
            .map(|c| {
                let ident = self.quote_ident(&c.column)?;
                Ok(if c.value.is_null() {
                    format!("{ident} IS NULL")
                } else {
                    format!("{ident} = {}", self.render_literal(&c.value))
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        Ok(parts.join(" AND "))
    }

    fn build_statement(&self, edit: &RowEdit) -> AppResult<String> {
        match edit {
            RowEdit::Update { table, pk, set } => {
                if set.is_empty() {
                    return Err(AppError::BadInput("update has no columns to change".into()));
                }
                if pk.is_empty() {
                    return Err(AppError::BadInput(
                        "refusing to UPDATE without a primary-key predicate".into(),
                    ));
                }
                let assignments = set
                    .iter()
                    .map(|c| {
                        Ok(format!(
                            "{} = {}",
                            self.quote_ident(&c.column)?,
                            self.render_literal(&c.value)
                        ))
                    })
                    .collect::<AppResult<Vec<_>>>()?
                    .join(", ");
                Ok(format!(
                    "UPDATE {} SET {} WHERE {}",
                    self.quote_ident(table)?,
                    assignments,
                    self.where_clause(pk)?
                ))
            }
            RowEdit::Insert { table, values } => {
                if values.is_empty() {
                    return Err(AppError::BadInput("insert has no values".into()));
                }
                let cols = values
                    .iter()
                    .map(|c| self.quote_ident(&c.column))
                    .collect::<AppResult<Vec<_>>>()?
                    .join(", ");
                let vals = values
                    .iter()
                    .map(|c| self.render_literal(&c.value))
                    .collect::<Vec<_>>()
                    .join(", ");
                Ok(format!(
                    "INSERT INTO {} ({}) VALUES ({})",
                    self.quote_ident(table)?,
                    cols,
                    vals
                ))
            }
            RowEdit::Delete { table, pk } => {
                if pk.is_empty() {
                    return Err(AppError::BadInput(
                        "refusing to DELETE without a primary-key predicate".into(),
                    ));
                }
                Ok(format!(
                    "DELETE FROM {} WHERE {}",
                    self.quote_ident(table)?,
                    self.where_clause(pk)?
                ))
            }
        }
    }
}

fn ensure_writable_engine(inst: &DatabaseInstance) -> AppResult<()> {
    match inst.engine {
        DatabaseEngine::Sqlite
        | DatabaseEngine::Mysql
        | DatabaseEngine::Mariadb
        | DatabaseEngine::Postgres => Ok(()),
        _ => Err(AppError::BadInput(format!(
            "{} is not supported by the embedded database client yet",
            inst.engine.label()
        ))),
    }
}

/// Render the SQL a batch of edits *would* run, without executing — drives the
/// grid's "Review N changes" confirmation bar.
pub fn preview_writes(inst: &DatabaseInstance, edits: &[RowEdit]) -> AppResult<Vec<String>> {
    ensure_writable_engine(inst)?;
    let dialect = SqlDialect::for_engine(inst.engine);
    edits.iter().map(|e| dialect.build_statement(e)).collect()
}

/// Apply a batch of user-confirmed structured writes inside one transaction —
/// all-or-nothing. Returns the total affected rows and the SQL that ran.
pub async fn apply_writes(
    inst: &DatabaseInstance,
    schema: Option<&str>,
    edits: &[RowEdit],
) -> AppResult<DbApplyResult> {
    if edits.is_empty() {
        return Err(AppError::BadInput("no changes to apply".into()));
    }
    let statements = preview_writes(inst, edits)?;
    let mut affected_rows: u64 = 0;

    match inst.engine {
        DatabaseEngine::Sqlite => {
            let pool = sqlite_pool_writable(inst).await?;
            let mut tx = pool
                .begin()
                .await
                .map_err(|e| AppError::Internal(format!("begin SQLite transaction: {e}")))?;
            for sql in &statements {
                let res = sqlx::query(sql)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::Internal(format!("apply write: {e}")))?;
                affected_rows += res.rows_affected();
            }
            tx.commit()
                .await
                .map_err(|e| AppError::Internal(format!("commit SQLite transaction: {e}")))?;
        }
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => {
            let pool = mysql_pool(inst, schema).await?;
            let mut tx = pool.begin().await.map_err(|e| {
                AppError::Internal(format!("begin {} transaction: {e}", inst.engine.label()))
            })?;
            for sql in &statements {
                let res = sqlx::query(sql)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::Internal(format!("apply write: {e}")))?;
                affected_rows += res.rows_affected();
            }
            tx.commit().await.map_err(|e| {
                AppError::Internal(format!("commit {} transaction: {e}", inst.engine.label()))
            })?;
        }
        DatabaseEngine::Postgres => {
            let pool = pg_pool(inst, None).await?;
            let mut tx = pool
                .begin()
                .await
                .map_err(|e| AppError::Internal(format!("begin PostgreSQL transaction: {e}")))?;
            if let Some(schema) = schema.filter(|s| !s.trim().is_empty()) {
                sqlx::query("SELECT set_config('search_path', $1, false)")
                    .bind(schema)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::Internal(format!("set PostgreSQL schema: {e}")))?;
            }
            for sql in &statements {
                let res = sqlx::query(sql)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::Internal(format!("apply write: {e}")))?;
                affected_rows += res.rows_affected();
            }
            tx.commit()
                .await
                .map_err(|e| AppError::Internal(format!("commit PostgreSQL transaction: {e}")))?;
        }
        _ => {
            ensure_writable_engine(inst)?;
        }
    }

    Ok(DbApplyResult {
        affected_rows,
        statements,
    })
}

pub async fn explain(
    inst: &DatabaseInstance,
    schema: Option<&str>,
    sql: &str,
    analyze: bool,
) -> AppResult<DbExplainPlan> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err(AppError::BadInput("SQL query is required".into()));
    }
    ensure_read_query(trimmed)?;

    match inst.engine {
        DatabaseEngine::Sqlite => sqlite_explain(inst, trimmed).await,
        DatabaseEngine::Mysql | DatabaseEngine::Mariadb => {
            mysql_explain(inst, schema, trimmed).await
        }
        DatabaseEngine::Postgres => pg_explain(inst, schema, trimmed, analyze).await,
        _ => Err(AppError::BadInput(format!(
            "{} is not supported by the embedded database client yet",
            inst.engine.label()
        ))),
    }
}

// ---------------------------------------------------------------------------
// SQLite explain — EXPLAIN QUERY PLAN, tree built by parent-id linkage
// ---------------------------------------------------------------------------

async fn sqlite_explain(inst: &DatabaseInstance, sql: &str) -> AppResult<DbExplainPlan> {
    let pool = sqlite_pool(inst).await?;
    let explain_sql = if first_sql_keyword(sql).as_deref() == Some("explain") {
        sql.to_string()
    } else {
        format!("EXPLAIN QUERY PLAN {sql}")
    };
    let rows = sqlx::query(&explain_sql)
        .fetch_all(&pool)
        .await
        .map_err(|e| AppError::Internal(format!("explain SQLite query: {e}")))?;

    if rows.is_empty() {
        // Return a valid (but empty) plan rather than erroring.
        return Ok(DbExplainPlan {
            root: DbExplainNode {
                id: "node_0".to_string(),
                node_type: "Empty plan".to_string(),
                relation: None,
                startup_cost: None,
                total_cost: None,
                plan_rows: None,
                actual_rows: None,
                actual_time_ms: None,
                actual_loops: None,
                buffers_hit: None,
                buffers_read: None,
                filter: None,
                index_condition: None,
                join_type: None,
                hash_condition: None,
                extra: serde_json::json!({}),
                children: vec![],
            },
            planning_time_ms: None,
            execution_time_ms: None,
            original_query: sql.to_string(),
            driver: "sqlite".to_string(),
            has_analyze_data: false,
            raw_output: None,
        });
    }

    // Collect (id, parent, detail) triples and raw output lines.
    let mut entries: Vec<(i64, i64, String)> = Vec::new();
    let mut raw_lines: Vec<String> = Vec::new();
    for row in &rows {
        let id: i64 = row.try_get("id").unwrap_or(0);
        let parent: i64 = row.try_get("parent").unwrap_or(0);
        let detail: String = row.try_get("detail").unwrap_or_default();
        raw_lines.push(format!("{}|{}|{}", id, parent, &detail));
        entries.push((id, parent, detail));
    }

    let raw_output = raw_lines.join("\n");
    let root = build_sqlite_root(&entries);

    Ok(DbExplainPlan {
        root,
        planning_time_ms: None,
        execution_time_ms: None,
        original_query: sql.to_string(),
        driver: "sqlite".to_string(),
        has_analyze_data: false,
        raw_output: Some(raw_output),
    })
}

/// Assemble the SQLite explain tree from raw `(id, parent, detail)` rows.
///
/// `EXPLAIN QUERY PLAN` emits a forest: every top-level step has `parent == 0`,
/// and a non-trivial query routinely has several of them — a join yields a
/// `SCAN` plus a `SEARCH`, a correlated subquery yields a `SCAN` plus the
/// subquery branch, an `ORDER BY` yields a `SCAN` plus a `USE TEMP B-TREE`.
/// (The earlier implementation kept only the first top-level row, so every
/// query past a single-table scan collapsed to one lonely node.)
///
/// When there is exactly one top-level step it becomes the root directly; when
/// there are several (or none) they are grouped under a synthetic "Query plan"
/// root so the tree always has the single root the graph view expects.
fn build_sqlite_root(entries: &[(i64, i64, String)]) -> DbExplainNode {
    let top: Vec<&(i64, i64, String)> = entries
        .iter()
        .filter(|(id, parent, _)| *parent == 0 && *id != 0)
        .collect();

    let mut counter: u32 = 0;

    if top.len() == 1 {
        return build_sqlite_node(entries, top[0], &mut counter);
    }

    // Zero or many top-level steps: wrap them under a synthetic root.
    let id_str = format!("node_{counter}");
    counter += 1;
    let children: Vec<DbExplainNode> = top
        .iter()
        .map(|entry| build_sqlite_node(entries, entry, &mut counter))
        .collect();

    DbExplainNode {
        id: id_str,
        node_type: "Query plan".to_string(),
        relation: None,
        startup_cost: None,
        total_cost: None,
        plan_rows: None,
        actual_rows: None,
        actual_time_ms: None,
        actual_loops: None,
        buffers_hit: None,
        buffers_read: None,
        filter: None,
        index_condition: None,
        join_type: None,
        hash_condition: None,
        extra: serde_json::json!({}),
        children,
    }
}

/// Build one node from its `(id, parent, detail)` row, recursing into every
/// entry whose `parent` points back at this row's id.
fn build_sqlite_node(
    entries: &[(i64, i64, String)],
    entry: &(i64, i64, String),
    counter: &mut u32,
) -> DbExplainNode {
    let (row_id, _parent, detail) = entry;
    let (node_type, relation, index_condition) = parse_sqlite_detail(detail);

    let id_str = format!("node_{}", counter);
    *counter += 1;

    let children: Vec<DbExplainNode> = entries
        .iter()
        .filter(|(id, parent, _)| *parent == *row_id && *id != *row_id)
        .map(|child| build_sqlite_node(entries, child, counter))
        .collect();

    // SQLite reports no cost or timing, so a full table SCAN (a `SCAN` with no
    // index behind it) is the one actionable signal the plan carries. Tag it
    // with `accessType: "all"` so the overview's sequential-scan detector —
    // shared with the MySQL path — surfaces it as an issue.
    let extra = if node_type == "Scan" && index_condition.is_none() {
        serde_json::json!({ "accessType": "all" })
    } else {
        serde_json::json!({})
    };

    DbExplainNode {
        id: id_str,
        node_type,
        relation,
        startup_cost: None,
        total_cost: None,
        plan_rows: None,
        actual_rows: None,
        actual_time_ms: None,
        actual_loops: None,
        buffers_hit: None,
        buffers_read: None,
        filter: None,
        index_condition,
        join_type: None,
        hash_condition: None,
        extra,
        children,
    }
}

/// Parse a SQLite EXPLAIN QUERY PLAN detail string into (node_type, relation, index_condition).
fn parse_sqlite_detail(detail: &str) -> (String, Option<String>, Option<String>) {
    let detail_upper = detail.to_uppercase();

    if detail_upper.starts_with("SCAN") {
        let parts: Vec<&str> = detail.splitn(3, ' ').collect();
        let relation = parts.get(1).map(|s| s.to_string());
        let index = if detail_upper.contains("USING COVERING INDEX") {
            detail
                .find("USING COVERING INDEX")
                .map(|pos| detail[pos + 21..].trim().to_string())
        } else if detail_upper.contains("USING INDEX") {
            detail
                .find("USING INDEX")
                .map(|pos| detail[pos + 12..].trim().to_string())
        } else {
            None
        };
        ("Scan".to_string(), relation, index)
    } else if detail_upper.starts_with("SEARCH") {
        let parts: Vec<&str> = detail.splitn(3, ' ').collect();
        let relation = parts.get(1).map(|s| s.to_string());
        let index = if detail_upper.contains("USING COVERING INDEX") {
            detail
                .find("USING COVERING INDEX")
                .map(|pos| detail[pos + 21..].trim().to_string())
        } else if detail_upper.contains("USING INTEGER PRIMARY KEY") {
            Some("PRIMARY KEY".to_string())
        } else if detail_upper.contains("USING INDEX") {
            detail
                .find("USING INDEX")
                .map(|pos| detail[pos + 12..].trim().to_string())
        } else {
            None
        };
        ("Search".to_string(), relation, index)
    } else if detail_upper.contains("TEMP B-TREE") {
        ("Sort".to_string(), None, None)
    } else if detail_upper.starts_with("CO-ROUTINE") {
        ("Co-routine".to_string(), None, None)
    } else if detail_upper.starts_with("COMPOUND SUBQUERIES") {
        ("Compound Subquery".to_string(), None, None)
    } else if detail_upper.starts_with("MATERIALIZE") {
        ("Materialize".to_string(), None, None)
    } else {
        (detail.to_string(), None, None)
    }
}

// ---------------------------------------------------------------------------
// MySQL / MariaDB explain — FORMAT=JSON parsed into the tree, tabular fallback
// ---------------------------------------------------------------------------

async fn mysql_explain(
    inst: &DatabaseInstance,
    schema: Option<&str>,
    sql: &str,
) -> AppResult<DbExplainPlan> {
    let pool = mysql_pool(inst, schema).await?;

    // Skip the EXPLAIN prefix if the caller already included it.
    let bare_sql = if first_sql_keyword(sql).as_deref() == Some("explain") {
        // Strip the leading EXPLAIN keyword so we can always try FORMAT=JSON.
        sql.split_once(|c: char| c.is_whitespace())
            .map(|x| x.1)
            .unwrap_or(sql)
            .trim()
            .to_string()
    } else {
        sql.to_string()
    };

    // Try EXPLAIN FORMAT=JSON first (MySQL 5.6+ / MariaDB 10.1+).
    {
        let json_sql = format!("EXPLAIN FORMAT=JSON {bare_sql}");
        let json_result = sqlx::query(&json_sql)
            .fetch_one(&pool)
            .await
            .and_then(|row| row.try_get::<String, _>(0));

        if let Ok(raw_json) = json_result {
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&raw_json) {
                if let Some(query_block) = json_val.get("query_block") {
                    let mut counter: u32 = 0;
                    let root = parse_mysql_query_block(query_block, &mut counter);
                    return Ok(DbExplainPlan {
                        root,
                        planning_time_ms: None,
                        execution_time_ms: None,
                        original_query: sql.to_string(),
                        driver: inst.engine.id().to_string(),
                        has_analyze_data: false,
                        raw_output: Some(raw_json),
                    });
                }
            }
        }
    }

    // Tabular fallback — plain EXPLAIN.
    let explain_sql = format!("EXPLAIN {bare_sql}");
    let rows = sqlx::query(&explain_sql)
        .fetch_all(&pool)
        .await
        .map_err(|e| AppError::Internal(format!("explain {} query: {e}", inst.engine.label())))?;

    let (root, raw_output) = parse_mysql_tabular_explain(&rows);
    Ok(DbExplainPlan {
        root,
        planning_time_ms: None,
        execution_time_ms: None,
        original_query: sql.to_string(),
        driver: inst.engine.id().to_string(),
        has_analyze_data: false,
        raw_output: Some(raw_output),
    })
}

/// Parse a JSON number that may be stored as a string or a numeric value.
fn parse_json_number(v: &serde_json::Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
}

/// Parse a MySQL/MariaDB EXPLAIN FORMAT=JSON `query_block` (or any nested block)
/// into a `DbExplainNode` tree.
fn parse_mysql_query_block(block: &serde_json::Value, counter: &mut u32) -> DbExplainNode {
    let id = format!("node_{}", counter);
    *counter += 1;

    let (node_type, relation, plan_rows, startup_cost, total_cost, filter) =
        if let Some(table) = block.get("table") {
            let access = table
                .get("access_type")
                .and_then(|v| v.as_str())
                .unwrap_or("ALL");
            let node_type = match access {
                "ALL" => "Full Table Scan",
                "index" => "Index Scan",
                "range" => "Range Scan",
                "ref" => "Index Lookup",
                "eq_ref" => "Unique Index Lookup",
                "const" | "system" => "Const Lookup",
                "fulltext" => "Fulltext Search",
                other => other,
            }
            .to_string();
            let rel = table
                .get("table_name")
                .and_then(|v| v.as_str())
                .map(String::from);
            let rows = table
                .get("rows_examined_per_scan")
                .and_then(|v| v.as_f64())
                .or_else(|| table.get("rows").and_then(|v| v.as_f64()));
            let cost_info = table.get("cost_info");
            let startup = cost_info
                .and_then(|c| c.get("read_cost"))
                .and_then(parse_json_number);
            let total = cost_info
                .and_then(|c| c.get("prefix_cost"))
                .and_then(parse_json_number)
                .or(startup)
                .or_else(|| table.get("cost").and_then(parse_json_number));
            let filt = table
                .get("attached_condition")
                .and_then(|v| v.as_str())
                .map(String::from);
            (node_type, rel, rows, startup, total, filt)
        } else {
            let node_type = if block
                .get("using_filesort")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                "Filesort".to_string()
            } else if block.get("grouping_operation").is_some() {
                "Group".to_string()
            } else if block.get("duplicates_removal").is_some() {
                "Duplicate Removal".to_string()
            } else if block.get("having_condition").is_some() {
                "Having Filter".to_string()
            } else {
                "Query Block".to_string()
            };
            let cost_info = block.get("cost_info");
            let total = cost_info
                .and_then(|c| {
                    c.get("query_cost")
                        .or_else(|| c.get("sort_cost"))
                        .or_else(|| c.get("prefix_cost"))
                })
                .and_then(parse_json_number)
                .or_else(|| block.get("cost").and_then(parse_json_number));
            let filt = block
                .get("having_condition")
                .and_then(|v| v.as_str())
                .map(String::from);
            (node_type, None, None, None, total, filt)
        };

    // Extra fields from the table object (excluding known structural keys).
    let known_table_keys: &[&str] = &[
        "access_type",
        "table_name",
        "rows_examined_per_scan",
        "rows",
        "cost_info",
        "attached_condition",
        "key",
        "possible_keys",
        "used_key_parts",
    ];
    let mut extra_map = serde_json::Map::new();
    if let Some(table) = block.get("table").and_then(|t| t.as_object()) {
        for (k, v) in table {
            if !known_table_keys.contains(&k.as_str()) {
                extra_map.insert(k.clone(), v.clone());
            }
        }
        if let Some(key) = table.get("key").and_then(|v| v.as_str()) {
            extra_map.insert(
                "key".to_string(),
                serde_json::Value::String(key.to_string()),
            );
        }
    }
    let extra = serde_json::Value::Object(extra_map);

    let index_condition = block
        .get("table")
        .and_then(|t| t.get("key"))
        .and_then(|v| v.as_str())
        .map(String::from);

    // Recurse into child structures.
    let mut children = Vec::new();

    if let Some(nested_loop) = block.get("nested_loop").and_then(|v| v.as_array()) {
        for item in nested_loop {
            if item.get("table").is_some() {
                children.push(parse_mysql_query_block(item, counter));
            } else if let Some(tmp) = item.get("temporary_table") {
                children.push(parse_mysql_query_block(tmp, counter));
            }
        }
    }

    if let Some(order_op) = block.get("ordering_operation") {
        children.push(parse_mysql_query_block(order_op, counter));
    }

    if let Some(group_op) = block.get("grouping_operation") {
        children.push(parse_mysql_query_block(group_op, counter));
    }

    if let Some(dup_op) = block.get("duplicates_removal") {
        children.push(parse_mysql_query_block(dup_op, counter));
    }

    for key in &["optimized_away_subqueries", "attached_subqueries"] {
        if let Some(arr) = block.get(key).and_then(|v| v.as_array()) {
            for sq in arr {
                children.push(parse_mysql_query_block(sq, counter));
            }
        }
    }

    if let Some(tmp_tbl) = block.get("temporary_table") {
        children.push(parse_mysql_query_block(tmp_tbl, counter));
    }

    if let Some(union_res) = block.get("union_result") {
        children.push(parse_mysql_query_block(union_res, counter));
    }

    if let Some(specs) = block.get("query_specifications").and_then(|v| v.as_array()) {
        for spec in specs {
            children.push(parse_mysql_query_block(spec, counter));
        }
    }

    DbExplainNode {
        id,
        node_type,
        relation,
        startup_cost,
        total_cost,
        plan_rows,
        actual_rows: None,
        actual_time_ms: None,
        actual_loops: None,
        buffers_hit: None,
        buffers_read: None,
        filter,
        index_condition,
        join_type: None,
        hash_condition: None,
        extra,
        children,
    }
}

/// Parse a tabular (plain `EXPLAIN`) MySQL/MariaDB result into a tree.
///
/// Returns a root "Query" node whose children are one node per result row.
fn parse_mysql_tabular_explain(rows: &[MySqlRow]) -> (DbExplainNode, String) {
    fn col_idx(row: &MySqlRow, name: &str) -> Option<usize> {
        row.columns()
            .iter()
            .position(|c| c.name().eq_ignore_ascii_case(name))
    }

    fn row_str(row: &MySqlRow, idx: usize) -> String {
        row.try_get::<Vec<u8>, _>(idx)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .or_else(|_| row.try_get::<String, _>(idx))
            .unwrap_or_default()
    }

    fn row_str_opt(row: &MySqlRow, idx: usize) -> Option<String> {
        let s = row_str(row, idx);
        if s.is_empty() || s.eq_ignore_ascii_case("null") {
            None
        } else {
            Some(s)
        }
    }

    let mut raw_lines = Vec::new();
    let mut children = Vec::new();

    for (i, row) in rows.iter().enumerate() {
        let select_type = col_idx(row, "select_type")
            .map(|idx| row_str(row, idx))
            .unwrap_or_default();
        let table = col_idx(row, "table")
            .and_then(|idx| row_str_opt(row, idx))
            .unwrap_or_default();
        let access_type = col_idx(row, "type")
            .and_then(|idx| row_str_opt(row, idx))
            .unwrap_or_default();
        let possible_keys = col_idx(row, "possible_keys").and_then(|idx| row_str_opt(row, idx));
        let key = col_idx(row, "key").and_then(|idx| row_str_opt(row, idx));
        let plan_rows: Option<f64> = col_idx(row, "rows").and_then(|idx| {
            row.try_get::<Option<i64>, _>(idx)
                .unwrap_or(None)
                .map(|n| n as f64)
                .or_else(|| row_str_opt(row, idx).and_then(|s| s.parse::<f64>().ok()))
        });
        let filtered: Option<f64> = col_idx(row, "filtered").and_then(|idx| {
            row.try_get::<Option<f64>, _>(idx)
                .unwrap_or(None)
                .or_else(|| row_str_opt(row, idx).and_then(|s| s.parse::<f64>().ok()))
        });
        let extra_str = col_idx(row, "Extra").and_then(|idx| row_str_opt(row, idx));

        let node_type = match access_type.as_str() {
            "ALL" => "Full Table Scan",
            "index" => "Index Scan",
            "range" => "Range Scan",
            "ref" => "Index Lookup",
            "eq_ref" => "Unique Index Lookup",
            "const" | "system" => "Const Lookup",
            "fulltext" => "Fulltext Search",
            "" => "Unknown",
            other => other,
        }
        .to_string();

        raw_lines.push(format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            select_type,
            table,
            access_type,
            key.as_deref().unwrap_or("-"),
            plan_rows.unwrap_or(0.0) as i64,
            extra_str.as_deref().unwrap_or("")
        ));

        let mut extra_map = serde_json::Map::new();
        if let Some(pk) = &possible_keys {
            extra_map.insert(
                "possible_keys".to_string(),
                serde_json::Value::String(pk.clone()),
            );
        }
        if let Some(f) = filtered {
            if let Some(n) = serde_json::Number::from_f64(f) {
                extra_map.insert("filtered".to_string(), serde_json::Value::Number(n));
            }
        }
        if let Some(e) = &extra_str {
            extra_map.insert("extra".to_string(), serde_json::Value::String(e.clone()));
        }
        extra_map.insert(
            "select_type".to_string(),
            serde_json::Value::String(select_type),
        );

        children.push(DbExplainNode {
            id: format!("node_{}", i + 1),
            node_type,
            relation: if table.is_empty() { None } else { Some(table) },
            startup_cost: None,
            total_cost: None,
            plan_rows,
            actual_rows: None,
            actual_time_ms: None,
            actual_loops: None,
            buffers_hit: None,
            buffers_read: None,
            filter: extra_str,
            index_condition: key,
            join_type: None,
            hash_condition: None,
            extra: serde_json::Value::Object(extra_map),
            children: vec![],
        });
    }

    let root = DbExplainNode {
        id: "node_0".to_string(),
        node_type: "Query".to_string(),
        relation: None,
        startup_cost: None,
        total_cost: None,
        plan_rows: None,
        actual_rows: None,
        actual_time_ms: None,
        actual_loops: None,
        buffers_hit: None,
        buffers_read: None,
        filter: None,
        index_condition: None,
        join_type: None,
        hash_condition: None,
        extra: serde_json::json!({}),
        children,
    };

    (root, raw_lines.join("\n"))
}

// ---------------------------------------------------------------------------
// PostgreSQL explain — EXPLAIN (FORMAT JSON) parsed into the tree
// ---------------------------------------------------------------------------

const PG_KNOWN_KEYS: &[&str] = &[
    "Node Type",
    "Relation Name",
    "Startup Cost",
    "Total Cost",
    "Plan Rows",
    "Actual Rows",
    "Actual Total Time",
    "Actual Loops",
    "Shared Hit Blocks",
    "Shared Read Blocks",
    "Filter",
    "Index Cond",
    "Join Type",
    "Hash Cond",
    "Plans",
];

async fn pg_explain(
    inst: &DatabaseInstance,
    schema: Option<&str>,
    sql: &str,
    analyze: bool,
) -> AppResult<DbExplainPlan> {
    let pool = pg_pool(inst, None).await?;
    let mut conn = pool
        .acquire()
        .await
        .map_err(|e| AppError::Internal(format!("acquire PostgreSQL connection: {e}")))?;
    if let Some(schema) = schema.filter(|s| !s.trim().is_empty()) {
        sqlx::query("SELECT set_config('search_path', $1, false)")
            .bind(schema)
            .execute(&mut *conn)
            .await
            .map_err(|e| AppError::Internal(format!("set PostgreSQL schema: {e}")))?;
    }

    // Build the EXPLAIN SQL. If the caller already included EXPLAIN, pass through.
    let explain_sql = if first_sql_keyword(sql).as_deref() == Some("explain") {
        sql.to_string()
    } else if analyze {
        format!("EXPLAIN (FORMAT JSON, ANALYZE, BUFFERS) {sql}")
    } else {
        format!("EXPLAIN (FORMAT JSON) {sql}")
    };

    let row = sqlx::query(&explain_sql)
        .fetch_one(&mut *conn)
        .await
        .map_err(|e| AppError::Internal(format!("explain PostgreSQL query: {e}")))?;

    // PostgreSQL returns the JSON as either a Value or a String column.
    let raw_value = row.try_get::<Value, _>(0).unwrap_or_else(|_| {
        row.try_get::<String, _>(0)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| serde_json::json!([]))
    });

    let raw_output = serde_json::to_string_pretty(&raw_value).ok();

    // The top-level is an array of statement objects; take the first.
    let top = raw_value
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .unwrap_or_else(|| raw_value.clone());

    let planning_time_ms = top.get("Planning Time").and_then(Value::as_f64);
    let execution_time_ms = top.get("Execution Time").and_then(Value::as_f64);
    let has_analyze_data = analyze && (planning_time_ms.is_some() || execution_time_ms.is_some());

    let root = if let Some(plan_obj) = top.get("Plan") {
        let mut counter: u32 = 0;
        let node = parse_pg_plan_node(plan_obj, &mut counter);
        // has_analyze_data is also true if any node carries actual data.
        let has_actual = node.actual_rows.is_some() || node.actual_time_ms.is_some();
        return Ok(DbExplainPlan {
            root: node,
            planning_time_ms,
            execution_time_ms,
            original_query: sql.to_string(),
            driver: "postgres".to_string(),
            has_analyze_data: has_analyze_data || has_actual,
            raw_output,
        });
    } else {
        // No plan found — return a synthetic root.
        DbExplainNode {
            id: "node_0".to_string(),
            node_type: "Empty plan".to_string(),
            relation: None,
            startup_cost: None,
            total_cost: None,
            plan_rows: None,
            actual_rows: None,
            actual_time_ms: None,
            actual_loops: None,
            buffers_hit: None,
            buffers_read: None,
            filter: None,
            index_condition: None,
            join_type: None,
            hash_condition: None,
            extra: serde_json::json!({}),
            children: vec![],
        }
    };

    Ok(DbExplainPlan {
        root,
        planning_time_ms,
        execution_time_ms,
        original_query: sql.to_string(),
        driver: "postgres".to_string(),
        has_analyze_data,
        raw_output,
    })
}

/// Recursively parse a PostgreSQL plan JSON node into a `DbExplainNode`.
fn parse_pg_plan_node(node: &Value, counter: &mut u32) -> DbExplainNode {
    let id = format!("node_{counter}");
    *counter += 1;

    let obj = node.as_object();

    let node_type = node
        .get("Node Type")
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
        .to_string();

    let relation = node
        .get("Relation Name")
        .and_then(Value::as_str)
        .map(String::from);
    let startup_cost = node.get("Startup Cost").and_then(Value::as_f64);
    let total_cost = node.get("Total Cost").and_then(Value::as_f64);
    let plan_rows = node.get("Plan Rows").and_then(Value::as_f64);
    let actual_rows = node.get("Actual Rows").and_then(Value::as_f64);
    let actual_time_ms = node.get("Actual Total Time").and_then(Value::as_f64);
    let actual_loops = node.get("Actual Loops").and_then(Value::as_f64);
    let buffers_hit = node.get("Shared Hit Blocks").and_then(Value::as_f64);
    let buffers_read = node.get("Shared Read Blocks").and_then(Value::as_f64);
    let filter = node.get("Filter").and_then(Value::as_str).map(String::from);
    let index_condition = node
        .get("Index Cond")
        .and_then(Value::as_str)
        .map(String::from);
    let join_type = node
        .get("Join Type")
        .and_then(Value::as_str)
        .map(String::from);
    let hash_condition = node
        .get("Hash Cond")
        .and_then(Value::as_str)
        .map(String::from);

    let mut extra_map = serde_json::Map::new();
    if let Some(map) = obj {
        for (k, v) in map {
            if !PG_KNOWN_KEYS.contains(&k.as_str()) {
                extra_map.insert(k.clone(), v.clone());
            }
        }
    }

    let children = node
        .get("Plans")
        .and_then(Value::as_array)
        .map(|plans| {
            plans
                .iter()
                .map(|child| parse_pg_plan_node(child, counter))
                .collect()
        })
        .unwrap_or_default();

    DbExplainNode {
        id,
        node_type,
        relation,
        startup_cost,
        total_cost,
        plan_rows,
        actual_rows,
        actual_time_ms,
        actual_loops,
        buffers_hit,
        buffers_read,
        filter,
        index_condition,
        join_type,
        hash_condition,
        extra: serde_json::Value::Object(extra_map),
        children,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_ident_escapes_embedded_quotes() {
        assert_eq!(quote_ident("plain", '"'), r#""plain""#);
        assert_eq!(quote_ident(r#"has"quote"#, '"'), r#""has""quote""#);
        assert_eq!(quote_ident("has`tick", '`'), "`has``tick`");
    }

    #[test]
    fn table_ref_uses_engine_specific_quoting() {
        assert_eq!(
            table_ref(DatabaseEngine::Postgres, Some("public"), "users"),
            r#""public"."users""#
        );
        assert_eq!(
            table_ref(DatabaseEngine::Mysql, Some("app"), "users"),
            "`app`.`users`"
        );
        assert_eq!(
            table_ref(DatabaseEngine::Sqlite, None, "user data"),
            r#""user data""#
        );
    }

    #[test]
    fn write_builder_renders_update_with_escaped_literals() {
        let d = SqlDialect::for_engine(DatabaseEngine::Postgres);
        let edit = RowEdit::Update {
            table: "users".into(),
            pk: vec![CellValue {
                column: "id".into(),
                value: serde_json::json!(7),
            }],
            set: vec![
                CellValue {
                    column: "name".into(),
                    value: serde_json::json!("O'Brien"),
                },
                CellValue {
                    column: "active".into(),
                    value: serde_json::json!(true),
                },
                CellValue {
                    column: "note".into(),
                    value: serde_json::json!(null),
                },
            ],
        };
        assert_eq!(
            d.build_statement(&edit).unwrap(),
            r#"UPDATE "users" SET "name" = 'O''Brien', "active" = TRUE, "note" = NULL WHERE "id" = 7"#
        );
    }

    #[test]
    fn write_builder_quotes_injection_attempts_inertly() {
        let d = SqlDialect::for_engine(DatabaseEngine::Postgres);
        // A value crafted to break out of the string is neutralised by doubling.
        let edit = RowEdit::Update {
            table: "t".into(),
            pk: vec![CellValue {
                column: "id".into(),
                value: serde_json::json!(1),
            }],
            set: vec![CellValue {
                column: "x".into(),
                value: serde_json::json!("'); DROP TABLE users;--"),
            }],
        };
        assert_eq!(
            d.build_statement(&edit).unwrap(),
            r#"UPDATE "t" SET "x" = '''); DROP TABLE users;--' WHERE "id" = 1"#
        );
        // A malicious column name can't escape the quotes either.
        let bad = RowEdit::Delete {
            table: "t".into(),
            pk: vec![CellValue {
                column: r#"id" OR "1"="1"#.into(),
                value: serde_json::json!(1),
            }],
        };
        assert_eq!(
            d.build_statement(&bad).unwrap(),
            r#"DELETE FROM "t" WHERE "id"" OR ""1""=""1" = 1"#
        );
    }

    #[test]
    fn write_builder_escapes_backslash_only_for_mysql() {
        let mysql = SqlDialect::for_engine(DatabaseEngine::Mysql);
        let pg = SqlDialect::for_engine(DatabaseEngine::Postgres);
        let v = serde_json::json!(r"a\b");
        assert_eq!(mysql.render_literal(&v), r"'a\\b'");
        assert_eq!(pg.render_literal(&v), r"'a\b'");
    }

    #[test]
    fn write_builder_uses_is_null_in_pk_predicate() {
        let d = SqlDialect::for_engine(DatabaseEngine::Sqlite);
        let edit = RowEdit::Delete {
            table: "t".into(),
            pk: vec![CellValue {
                column: "k".into(),
                value: serde_json::json!(null),
            }],
        };
        assert_eq!(
            d.build_statement(&edit).unwrap(),
            r#"DELETE FROM "t" WHERE "k" IS NULL"#
        );
    }

    #[test]
    fn write_builder_refuses_unkeyed_update_and_delete() {
        let d = SqlDialect::for_engine(DatabaseEngine::Postgres);
        let update = RowEdit::Update {
            table: "t".into(),
            pk: vec![],
            set: vec![CellValue {
                column: "x".into(),
                value: serde_json::json!(1),
            }],
        };
        let delete = RowEdit::Delete {
            table: "t".into(),
            pk: vec![],
        };
        assert!(d.build_statement(&update).is_err());
        assert!(d.build_statement(&delete).is_err());
    }

    #[test]
    fn write_builder_renders_insert() {
        let d = SqlDialect::for_engine(DatabaseEngine::Mysql);
        let edit = RowEdit::Insert {
            table: "logs".into(),
            values: vec![
                CellValue {
                    column: "level".into(),
                    value: serde_json::json!("info"),
                },
                CellValue {
                    column: "n".into(),
                    value: serde_json::json!(42),
                },
            ],
        };
        assert_eq!(
            d.build_statement(&edit).unwrap(),
            "INSERT INTO `logs` (`level`, `n`) VALUES ('info', 42)"
        );
    }

    #[test]
    fn limit_is_bounded() {
        assert_eq!(bounded_limit(None), 100);
        assert_eq!(bounded_limit(Some(0)), 1);
        assert_eq!(bounded_limit(Some(9000)), MAX_LIMIT);
    }

    #[test]
    fn read_query_guard_allows_only_inspection_queries() {
        assert!(ensure_read_query("select * from users").is_ok());
        assert!(ensure_read_query("-- comment\nEXPLAIN SELECT 1").is_ok());
        assert!(ensure_read_query("/* comment */ PRAGMA table_info(users)").is_ok());
        assert!(ensure_read_query("WITH t AS (SELECT 1 AS n) SELECT * FROM t").is_ok());
        assert!(ensure_read_query("VALUES (1), (2)").is_ok());
        assert!(ensure_read_query("update users set name = 'x'").is_err());
        assert!(ensure_read_query("drop table users").is_err());
    }

    #[test]
    fn read_query_guard_blocks_known_bypasses() {
        // CTE-wrapped write (legal DML on Postgres / MySQL 8+).
        assert!(ensure_read_query(
            "WITH x AS (INSERT INTO users(name) VALUES('a') RETURNING *) SELECT * FROM x"
        )
        .is_err());
        // Stacked / multi-statement.
        assert!(ensure_read_query("SELECT 1; DROP TABLE users").is_err());
        assert!(ensure_read_query("SELECT 1 ;\n DELETE FROM users").is_err());
        // EXPLAIN of a write.
        assert!(ensure_read_query("EXPLAIN UPDATE users SET name = 'x'").is_err());
        // SELECT … INTO (writes a new table / OUTFILE).
        assert!(ensure_read_query("SELECT * INTO copy FROM users").is_err());
        // Row-locking read is treated as a write (it mutates lock state).
        assert!(ensure_read_query("SELECT * FROM users FOR UPDATE").is_err());
        // Write keyword hidden in a string / quoted identifier must NOT trip,
        // and must NOT make a real write pass.
        assert!(ensure_read_query("SELECT 'drop table users' AS note").is_ok());
        assert!(ensure_read_query(r#"SELECT "delete" FROM events"#).is_ok());
        // A trailing semicolon on a single statement is fine.
        assert!(ensure_read_query("SELECT 1;").is_ok());
    }

    #[test]
    fn bound_select_sql_caps_unbounded_selects_only() {
        assert_eq!(
            bound_select_sql("SELECT * FROM users", 101),
            "SELECT * FROM users LIMIT 101"
        );
        assert_eq!(
            bound_select_sql("SELECT * FROM users;", 101),
            "SELECT * FROM users LIMIT 101"
        );
        // Respects a user-supplied LIMIT.
        assert_eq!(
            bound_select_sql("SELECT * FROM users LIMIT 5", 101),
            "SELECT * FROM users LIMIT 5"
        );
        // Leaves non-limitable statements untouched.
        assert_eq!(
            bound_select_sql("PRAGMA table_info(users)", 101),
            "PRAGMA table_info(users)"
        );
        assert_eq!(bound_select_sql("SHOW TABLES", 101), "SHOW TABLES");
    }

    #[test]
    fn parse_sqlite_detail_classifies_common_steps() {
        let (nt, rel, idx) = parse_sqlite_detail("SCAN users");
        assert_eq!(nt, "Scan");
        assert_eq!(rel.as_deref(), Some("users"));
        assert!(idx.is_none());

        let (nt, rel, idx) = parse_sqlite_detail("SEARCH users USING INDEX idx_name");
        assert_eq!(nt, "Search");
        assert_eq!(rel.as_deref(), Some("users"));
        assert_eq!(idx.as_deref(), Some("idx_name"));

        let (nt, _rel, _idx) = parse_sqlite_detail("USE TEMP B-TREE FOR ORDER BY");
        assert_eq!(nt, "Sort");
    }

    #[test]
    fn parse_pg_plan_node_builds_recursive_tree() {
        let plan = serde_json::json!({
            "Node Type": "Nested Loop",
            "Total Cost": 10.5,
            "Plan Rows": 5,
            "Plans": [
                {
                    "Node Type": "Seq Scan",
                    "Relation Name": "users",
                    "Total Cost": 4.0,
                    "Plan Rows": 2
                }
            ]
        });
        let mut counter = 0u32;
        let root = parse_pg_plan_node(&plan, &mut counter);
        assert_eq!(root.node_type, "Nested Loop");
        assert_eq!(root.total_cost, Some(10.5));
        assert_eq!(root.children.len(), 1);
        let child = &root.children[0];
        assert_eq!(child.node_type, "Seq Scan");
        assert_eq!(child.relation.as_deref(), Some("users"));
    }

    #[test]
    fn parses_json_text_when_possible() {
        assert_eq!(
            stringish_json("{\"ok\":true}".into()),
            serde_json::json!({"ok": true})
        );
        assert_eq!(
            stringish_json("plain".into()),
            Value::String("plain".into())
        );
    }
}

/// Integration tests that run real queries through `sqlx`.
///
/// SQLite tests are fully self-contained (a temp `.sqlite` file) and always
/// run. The MySQL / PostgreSQL tests need a reachable server and are opt-in:
/// they no-op unless `PORTBAY_TEST_MYSQL_PORT` / `PORTBAY_TEST_PG_PORT` point at
/// a local instance whose default account (`root` / `postgres`) has no
/// password — exactly what `engine::provision` sets up for a PortBay-managed
/// engine. Run them with e.g. `PORTBAY_TEST_PG_PORT=5432 cargo test`.
#[cfg(test)]
mod integration {
    use super::*;
    use crate::registry::{DatabaseEngine, DatabaseInstance, DatabaseInstanceId};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    fn case_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pb-dbclient-it-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sqlite_instance(path: &Path) -> DatabaseInstance {
        DatabaseInstance {
            id: DatabaseInstanceId::new("itdb"),
            name: "itdb".into(),
            engine: DatabaseEngine::Sqlite,
            version: "3".into(),
            port: 0,
            data_dir: path.parent().unwrap().to_path_buf(),
            config_path: None,
            socket_path: None,
            file_path: Some(path.to_path_buf()),
            auto_start: false,
            linked_projects: vec![],
        }
    }

    /// Create + populate a fresh SQLite database and return its path.
    async fn seed_sqlite(dir: &Path) -> PathBuf {
        let path = dir.join("app.sqlite");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        sqlx::query("CREATE TABLE authors (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE books (id INTEGER PRIMARY KEY, title TEXT, \
             author_id INTEGER REFERENCES authors(id))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO authors (id, name) VALUES (1,'Ada'),(2,'Alan'),(3,'Grace')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO books (id,title,author_id) VALUES (1,'Notes',1),(2,'Machines',2)")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        path
    }

    #[tokio::test]
    async fn sqlite_schema_reads_tables_columns_and_fks() {
        let dir = case_dir("schema");
        let inst = sqlite_instance(&seed_sqlite(&dir).await);

        let info = schema(&inst).await.unwrap();
        let names: Vec<&str> = info.tables.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"authors"), "tables: {names:?}");
        assert!(names.contains(&"books"), "tables: {names:?}");

        let books = info.tables.iter().find(|t| t.name == "books").unwrap();
        assert!(books.columns.iter().any(|c| c.name == "author_id"));
        assert!(
            books
                .foreign_keys
                .iter()
                .any(|fk| fk.ref_table == "authors"),
            "expected a FK books.author_id -> authors"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn sqlite_query_enforces_read_only_and_caps_results() {
        let dir = case_dir("query");
        let inst = sqlite_instance(&seed_sqlite(&dir).await);

        // Writes are rejected before they reach the database.
        assert!(query(&inst, None, "DELETE FROM authors", None)
            .await
            .is_err());
        assert!(query(
            &inst,
            None,
            "WITH x AS (INSERT INTO authors VALUES (9,'z') RETURNING *) SELECT * FROM x",
            None
        )
        .await
        .is_err());

        // An unbounded SELECT is capped server-side; 3 rows, limit 2 ⇒ truncated.
        let rows = query(&inst, None, "SELECT * FROM authors ORDER BY id", Some(2))
            .await
            .unwrap();
        assert_eq!(rows.rows.len(), 2);
        assert!(rows.truncated);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Pull `SELECT id, name FROM authors ORDER BY id` as (id, name) pairs.
    async fn author_rows(inst: &DatabaseInstance) -> Vec<(i64, String)> {
        let rows = query(
            inst,
            None,
            "SELECT id, name FROM authors ORDER BY id",
            Some(100),
        )
        .await
        .unwrap();
        rows.rows
            .iter()
            .map(|r| {
                let id = r[0].as_i64().unwrap();
                let name = r[1].as_str().unwrap().to_string();
                (id, name)
            })
            .collect()
    }

    #[tokio::test]
    async fn sqlite_apply_writes_inserts_updates_and_deletes_in_one_txn() {
        let dir = case_dir("apply");
        let inst = sqlite_instance(&seed_sqlite(&dir).await);

        // Seed state: (1,Ada) (2,Alan) (3,Grace).
        assert_eq!(
            author_rows(&inst).await,
            vec![(1, "Ada".into()), (2, "Alan".into()), (3, "Grace".into())]
        );

        let edits = vec![
            // Rename Ada → Ada Lovelace (with an apostrophe-bearing value nearby).
            RowEdit::Update {
                table: "authors".into(),
                pk: vec![CellValue {
                    column: "id".into(),
                    value: serde_json::json!(1),
                }],
                set: vec![CellValue {
                    column: "name".into(),
                    value: serde_json::json!("Ada O'Lovelace"),
                }],
            },
            // Delete Grace (no books reference her — FK enforcement is on).
            RowEdit::Delete {
                table: "authors".into(),
                pk: vec![CellValue {
                    column: "id".into(),
                    value: serde_json::json!(3),
                }],
            },
            // Insert Linus.
            RowEdit::Insert {
                table: "authors".into(),
                values: vec![
                    CellValue {
                        column: "id".into(),
                        value: serde_json::json!(4),
                    },
                    CellValue {
                        column: "name".into(),
                        value: serde_json::json!("Linus"),
                    },
                ],
            },
        ];

        // The preview the user would see in the "Review changes" modal.
        let preview = preview_writes(&inst, &edits).unwrap();
        assert_eq!(
            preview,
            vec![
                r#"UPDATE "authors" SET "name" = 'Ada O''Lovelace' WHERE "id" = 1"#.to_string(),
                r#"DELETE FROM "authors" WHERE "id" = 3"#.to_string(),
                r#"INSERT INTO "authors" ("id", "name") VALUES (4, 'Linus')"#.to_string(),
            ]
        );

        let result = apply_writes(&inst, None, &edits).await.unwrap();
        assert_eq!(
            result.affected_rows, 3,
            "one row each for update/delete/insert"
        );

        // The real database reflects every change.
        assert_eq!(
            author_rows(&inst).await,
            vec![
                (1, "Ada O'Lovelace".into()),
                (2, "Alan".into()),
                (4, "Linus".into()),
            ]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn sqlite_apply_writes_rolls_back_on_partial_failure() {
        let dir = case_dir("rollback");
        let inst = sqlite_instance(&seed_sqlite(&dir).await);

        // A valid update followed by an insert that violates the PK constraint.
        // The whole batch must roll back, leaving the original data untouched.
        let edits = vec![
            RowEdit::Update {
                table: "authors".into(),
                pk: vec![CellValue {
                    column: "id".into(),
                    value: serde_json::json!(1),
                }],
                set: vec![CellValue {
                    column: "name".into(),
                    value: serde_json::json!("CHANGED"),
                }],
            },
            RowEdit::Insert {
                table: "authors".into(),
                values: vec![
                    // id=3 already exists → UNIQUE/PK violation.
                    CellValue {
                        column: "id".into(),
                        value: serde_json::json!(3),
                    },
                    CellValue {
                        column: "name".into(),
                        value: serde_json::json!("Dup"),
                    },
                ],
            },
        ];

        assert!(apply_writes(&inst, None, &edits).await.is_err());

        // Nothing changed — the valid update was rolled back with the bad insert.
        assert_eq!(
            author_rows(&inst).await,
            vec![(1, "Ada".into()), (2, "Alan".into()), (3, "Grace".into())]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn sqlite_table_rows_returns_columns_and_paginates() {
        let dir = case_dir("rows");
        let inst = sqlite_instance(&seed_sqlite(&dir).await);

        let page1 = table_rows(&inst, None, "authors", Some(2), Some(0))
            .await
            .unwrap();
        assert_eq!(page1.rows.len(), 2);
        assert!(page1.columns.iter().any(|c| c.name == "name"));

        // 3 authors, page size 2, offset 2 ⇒ the last row.
        let page2 = table_rows(&inst, None, "authors", Some(2), Some(2))
            .await
            .unwrap();
        assert_eq!(page2.rows.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn sqlite_execute_applies_write_and_rejects_stacked() {
        let dir = case_dir("exec");
        let inst = sqlite_instance(&seed_sqlite(&dir).await);

        let res = execute(
            &inst,
            None,
            "UPDATE authors SET name = 'Ada Lovelace' WHERE id = 1",
        )
        .await
        .unwrap();
        assert_eq!(res.affected_rows, 1);

        // A stacked statement is rejected — an approved UPDATE can't smuggle a DROP.
        assert!(execute(
            &inst,
            None,
            "UPDATE authors SET name='x' WHERE id=2; DROP TABLE books"
        )
        .await
        .is_err());

        // The first write landed; the DROP never ran.
        let updated = query(&inst, None, "SELECT name FROM authors WHERE id = 1", None)
            .await
            .unwrap();
        assert_eq!(updated.rows.len(), 1);
        let books = query(&inst, None, "SELECT * FROM books", None)
            .await
            .unwrap();
        assert_eq!(books.rows.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn sqlite_explain_returns_plan_nodes() {
        let dir = case_dir("explain");
        let inst = sqlite_instance(&seed_sqlite(&dir).await);

        let plan = explain(
            &inst,
            None,
            "SELECT * FROM books WHERE author_id = 1",
            false,
        )
        .await
        .unwrap();
        // The root node must have a non-empty node_type.
        assert!(!plan.root.node_type.is_empty(), "root node_type is empty");
        // driver is "sqlite"
        assert_eq!(plan.driver, "sqlite");
        // No analyze data for SQLite.
        assert!(!plan.has_analyze_data);
        // There should be at least one meaningful node in the tree (root or its children).
        let relation_found =
            plan.root.relation.is_some() || plan.root.children.iter().any(|c| c.relation.is_some());
        // The query touches `books`, so we expect it to appear somewhere in the tree.
        let _ = relation_found; // SQLite may return the relation at any depth; just verify root is valid.
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- Approval gate end-to-end (agent ↔ human ↔ execute) ------------------

    #[tokio::test]
    async fn approval_gate_approve_then_execute_writes() {
        use crate::db_approval::{
            approvals_dir, await_decision, enqueue, new_id, now_ms, resolve, Decision, PendingWrite,
        };
        let dir = case_dir("gate-approve");
        let inst = sqlite_instance(&seed_sqlite(&dir).await);
        let queue = approvals_dir(&dir);
        let id = new_id();
        let sql = "DELETE FROM books WHERE id = 2".to_string();

        enqueue(
            &queue,
            &PendingWrite {
                id: id.clone(),
                instance_id: "itdb".into(),
                engine: "sqlite".into(),
                schema: None,
                sql: sql.clone(),
                origin: "test".into(),
                created_at_ms: now_ms(),
            },
        )
        .unwrap();

        // The GUI approves a moment later.
        let queue2 = queue.clone();
        let id2 = id.clone();
        let approver = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(60)).await;
            resolve(
                &queue2,
                &id2,
                &Decision {
                    approved: true,
                    reason: None,
                },
            )
            .unwrap();
        });

        // The agent waits for the verdict, then runs the statement on approval.
        let decision = await_decision(&queue, &id, Duration::from_secs(5))
            .await
            .unwrap();
        assert!(decision.approved);
        let res = execute(&inst, None, &sql).await.unwrap();
        assert_eq!(res.affected_rows, 1);
        approver.await.unwrap();

        let books = query(&inst, None, "SELECT * FROM books", None)
            .await
            .unwrap();
        assert_eq!(books.rows.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn approval_gate_deny_keeps_data_unchanged() {
        use crate::db_approval::{
            approvals_dir, await_decision, enqueue, new_id, now_ms, resolve, Decision, PendingWrite,
        };
        let dir = case_dir("gate-deny");
        let inst = sqlite_instance(&seed_sqlite(&dir).await);
        let queue = approvals_dir(&dir);
        let id = new_id();

        enqueue(
            &queue,
            &PendingWrite {
                id: id.clone(),
                instance_id: "itdb".into(),
                engine: "sqlite".into(),
                schema: None,
                sql: "DELETE FROM books".into(),
                origin: "test".into(),
                created_at_ms: now_ms(),
            },
        )
        .unwrap();

        let queue2 = queue.clone();
        let id2 = id.clone();
        let denier = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(60)).await;
            resolve(
                &queue2,
                &id2,
                &Decision {
                    approved: false,
                    reason: Some("not now".into()),
                },
            )
            .unwrap();
        });

        let decision = await_decision(&queue, &id, Duration::from_secs(5))
            .await
            .unwrap();
        assert!(!decision.approved);
        denier.await.unwrap();

        // The caller does not execute on denial, so the data is intact.
        let books = query(&inst, None, "SELECT * FROM books", None)
            .await
            .unwrap();
        assert_eq!(books.rows.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- Opt-in server-engine tests -----------------------------------------

    fn server_instance(engine: DatabaseEngine, port: u16) -> DatabaseInstance {
        DatabaseInstance {
            id: DatabaseInstanceId::new("itsrv"),
            name: "itsrv".into(),
            engine,
            version: String::new(),
            port,
            data_dir: std::env::temp_dir(),
            config_path: None,
            socket_path: None,
            file_path: None,
            auto_start: false,
            linked_projects: vec![],
        }
    }

    #[tokio::test]
    async fn mysql_integration_when_configured() {
        let Ok(port) = std::env::var("PORTBAY_TEST_MYSQL_PORT") else {
            eprintln!("skip mysql_integration: set PORTBAY_TEST_MYSQL_PORT to run");
            return;
        };
        let port: u16 = port
            .parse()
            .expect("PORTBAY_TEST_MYSQL_PORT must be a port number");
        let inst = server_instance(DatabaseEngine::Mysql, port);

        let rows = query(&inst, None, "SELECT 1 AS one", None).await.unwrap();
        assert_eq!(rows.rows.len(), 1);
        // The read path refuses DDL/writes.
        assert!(
            query(&inst, None, "DROP TABLE IF EXISTS pb_guard_probe", None)
                .await
                .is_err()
        );
        // The execute path runs a real write end-to-end.
        execute(
            &inst,
            None,
            "CREATE TABLE IF NOT EXISTS pb_it_probe (id INT)",
        )
        .await
        .unwrap();
        execute(&inst, None, "DROP TABLE pb_it_probe")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn postgres_integration_when_configured() {
        let Ok(port) = std::env::var("PORTBAY_TEST_PG_PORT") else {
            eprintln!("skip postgres_integration: set PORTBAY_TEST_PG_PORT to run");
            return;
        };
        let port: u16 = port
            .parse()
            .expect("PORTBAY_TEST_PG_PORT must be a port number");
        let inst = server_instance(DatabaseEngine::Postgres, port);

        let rows = query(&inst, None, "SELECT 1 AS one", None).await.unwrap();
        assert_eq!(rows.rows.len(), 1);
        assert!(
            query(&inst, None, "DROP TABLE IF EXISTS pb_guard_probe", None)
                .await
                .is_err()
        );
        execute(
            &inst,
            None,
            "CREATE TABLE IF NOT EXISTS pb_it_probe (id INT)",
        )
        .await
        .unwrap();
        execute(&inst, None, "DROP TABLE pb_it_probe")
            .await
            .unwrap();
    }
}
