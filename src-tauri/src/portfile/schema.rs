//! `.portbay.json` schema — the on-disk shape that crosses machine /
//! repo boundaries.
//!
//! Field naming is `camelCase` on the wire so the file reads
//! naturally for humans editing it by hand; serde rename attributes
//! keep the Rust struct idiomatic.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::registry::{MobileRunConfig, ProjectType, Readiness, WebServer};

/// Current schema version emitted by `export_project`. Bump when the
/// shape changes; older files with smaller version numbers continue
/// to deserialise (forward compatibility), newer files are rejected
/// (backward compatibility — the user is told to update PortBay).
pub const SCHEMA_VERSION: u32 = 1;

/// The conventional filename PortBay reads from a project root and
/// writes to with `export_project`.
pub const PORTBAY_FILE_NAME: &str = ".portbay.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortbayFile {
    pub version: u32,

    pub name: String,

    #[serde(rename = "type")]
    pub kind: ProjectType,

    pub hostname: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub php_version: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_server: Option<WebServer>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mobile_run: Option<MobileRunConfig>,

    pub https: bool,

    pub auto_start: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_command: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_root: Option<String>,

    /// Non-sensitive env vars baked into the file. Values may carry
    /// `${PROJECT_PATH}` and `${PROJECT_NAME}` placeholders that the
    /// importer expands.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env_template: BTreeMap<String, String>,

    /// Names of env vars the importer must prompt the user to fill
    /// in — passwords, API keys, anything not safe to commit. The
    /// file carries the *names* only, never the values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<String>,

    /// **Legacy.** Pre-dates the pre/post-start hooks below. Earlier docs
    /// described it as "commands the importer can offer to run after the
    /// project is registered". Export never populated it; it is kept only so
    /// older hand-authored files still parse, and on import its contents are
    /// folded into [`Self::pre_start`] when that's empty. New files use
    /// `pre_start` / `post_start`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_install: Vec<String>,

    /// Shell commands run before the dev server on each start (deps install,
    /// DB migrate). Mirrors `Project::pre_start`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_start: Vec<String>,

    /// Shell commands run after the dev server reports ready (health checks,
    /// warm-up). Mirrors `Project::post_start`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_start: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness: Option<Readiness>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}
