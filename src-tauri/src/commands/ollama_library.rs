//! Live Ollama model catalog, scraped from ollama.com/library.
//!
//! The model picker on the AI page used to ship a hardcoded catalog that went
//! stale the moment Ollama published a new model (phi4-reasoning never showed
//! up). ollama.com has no JSON API for its library, but the library and tags
//! pages carry stable `x-test-*` test-hook attributes on every field we need,
//! so this scrapes those. Results are cached on disk so the page renders
//! instantly and keeps working offline; a failed refresh serves the stale
//! cache (flagged `stale: true`) instead of erroring.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// How long a cached page stays fresh. New models land on a scale of weeks,
/// so a day-old list is "latest" for all practical purposes.
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Bump whenever the parsed shape changes (new fields, different variant
/// naming) — cache files written by older builds then self-invalidate instead
/// of serving field-less rows until their TTL runs out.
const CACHE_SCHEMA: u32 = 3;

const LIBRARY_URL: &str = "https://ollama.com/library?sort=popular";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryModel {
    pub name: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub sizes: Vec<String>,
    pub pull_count: Option<String>,
    pub updated: Option<String>,
    /// Carries ollama.com's "cloud" badge: inference runs on Ollama's cloud,
    /// so there is nothing to download and prompts leave the machine.
    #[serde(default)]
    pub cloud: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryCatalog {
    /// Cache-file invalidation marker; defaults to 0 on pre-schema files.
    #[serde(default)]
    pub schema: u32,
    pub fetched_at: String,
    pub models: Vec<LibraryModel>,
    #[serde(default)]
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryTag {
    /// Full pullable name, e.g. `phi4-reasoning:14b`.
    pub name: String,
    /// Download size as ollama.com prints it, e.g. `11GB`.
    pub size: Option<String>,
    /// Context window, e.g. `32K`.
    pub context: Option<String>,
    /// Input modalities, e.g. `Text`.
    pub input: Option<String>,
    /// Whether this tag is what `:latest` points at.
    pub latest: bool,
    /// 12-char manifest digest prefix as printed on the tags page. The local
    /// `/api/tags` digest starts with exactly this when the install is
    /// current — a mismatch means an update is available.
    #[serde(default)]
    pub digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryTagsResult {
    /// Cache-file invalidation marker; defaults to 0 on pre-schema files.
    #[serde(default)]
    pub schema: u32,
    pub model: String,
    pub fetched_at: String,
    pub tags: Vec<LibraryTag>,
    #[serde(default)]
    pub stale: bool,
}

#[tauri::command]
pub async fn ollama_library(
    state: State<'_, AppState>,
    refresh: Option<bool>,
) -> AppResult<LibraryCatalog> {
    let path = cache_dir(&state).join("catalog.json");
    if !refresh.unwrap_or(false) {
        if let Some(cached) =
            read_fresh::<LibraryCatalog>(&path).filter(|c| c.schema == CACHE_SCHEMA)
        {
            return Ok(cached);
        }
    }
    match fetch_library().await {
        Ok(models) => {
            let catalog = LibraryCatalog {
                schema: CACHE_SCHEMA,
                fetched_at: chrono::Utc::now().to_rfc3339(),
                models,
                stale: false,
            };
            write_cache(&path, &catalog);
            Ok(catalog)
        }
        Err(err) => read_any::<LibraryCatalog>(&path)
            .filter(|c| c.schema == CACHE_SCHEMA)
            .map(|mut cached| {
                cached.stale = true;
                cached
            })
            .ok_or(err),
    }
}

#[tauri::command]
pub async fn ollama_library_tags(
    state: State<'_, AppState>,
    model: String,
    refresh: Option<bool>,
) -> AppResult<LibraryTagsResult> {
    let model = model.trim().to_string();
    if model.is_empty() || !model.chars().all(is_model_name_char) {
        return Err(AppError::BadInput("Invalid library model name.".into()));
    }
    let path = cache_dir(&state).join(format!("tags-{model}.json"));
    if !refresh.unwrap_or(false) {
        if let Some(cached) =
            read_fresh::<LibraryTagsResult>(&path).filter(|c| c.schema == CACHE_SCHEMA)
        {
            return Ok(cached);
        }
    }
    match fetch_tags(&model).await {
        Ok(tags) => {
            let result = LibraryTagsResult {
                schema: CACHE_SCHEMA,
                model,
                fetched_at: chrono::Utc::now().to_rfc3339(),
                tags,
                stale: false,
            };
            write_cache(&path, &result);
            Ok(result)
        }
        Err(err) => read_any::<LibraryTagsResult>(&path)
            .filter(|c| c.schema == CACHE_SCHEMA)
            .map(|mut cached| {
                cached.stale = true;
                cached
            })
            .ok_or(err),
    }
}

/// Library model names are short DNS-ish slugs (`qwen2.5-coder`, `phi4-mini`);
/// anything else is rejected before it can reach a URL or a cache filename.
fn is_model_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')
}

async fn fetch_library() -> AppResult<Vec<LibraryModel>> {
    let html = fetch_page(LIBRARY_URL).await?;
    let models = parse_library_html(&html);
    if models.is_empty() {
        // A 200 with zero models means ollama.com changed its markup — surface
        // that rather than caching an empty catalog over a good one.
        return Err(AppError::Internal(
            "ollama.com library page had no recognizable models (site layout changed?)".into(),
        ));
    }
    Ok(models)
}

async fn fetch_tags(model: &str) -> AppResult<Vec<LibraryTag>> {
    let html = fetch_page(&format!("https://ollama.com/library/{model}/tags")).await?;
    let tags = parse_tags_html(&html);
    if tags.is_empty() {
        return Err(AppError::Internal(format!(
            "ollama.com tags page for {model} had no recognizable tags"
        )));
    }
    Ok(tags)
}

async fn fetch_page(url: &str) -> AppResult<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("PortBay (+https://portbay.app)")
        .build()
        .map_err(|e| AppError::Internal(format!("failed to build HTTP client: {e}")))?;
    let res = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("ollama.com request failed: {e}")))?;
    if !res.status().is_success() {
        return Err(AppError::Internal(format!(
            "ollama.com returned HTTP {}",
            res.status()
        )));
    }
    res.text()
        .await
        .map_err(|e| AppError::Internal(format!("ollama.com response was unreadable: {e}")))
}

// --- Cache ---------------------------------------------------------------

fn cache_dir(state: &AppState) -> PathBuf {
    state
        .logs_dir
        .parent()
        .unwrap_or(&state.logs_dir)
        .join("ollama-library")
}

/// Cached value if the file exists and is younger than [`CACHE_TTL`].
fn read_fresh<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let age = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())?;
    if age > CACHE_TTL {
        return None;
    }
    read_any(path)
}

/// Cached value regardless of age — the offline / fetch-failure fallback.
fn read_any<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_cache<T: Serialize>(path: &Path, value: &T) {
    let Ok(bytes) = serde_json::to_vec(value) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Best effort: a failed cache write only costs a refetch next launch.
    let _ = std::fs::write(path, bytes);
}

// --- Parsing -------------------------------------------------------------
//
// No HTML-parser crate in the tree, and these two pages are simple enough not
// to warrant one: every field sits behind a stable `x-test-*` attribute that
// ollama.com's own test suite depends on.

/// Substring of `hay` between the end of `start` and the next `end`.
fn between<'a>(hay: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = hay.find(start)? + start.len();
    let to = hay[from..].find(end)? + from;
    Some(&hay[from..to])
}

/// Text content of every element flagged with `marker` (an `x-test-*`
/// attribute): the text between the tag's closing `>` and the next `<`.
fn marker_values(block: &str, marker: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = block;
    while let Some(pos) = rest.find(marker) {
        rest = &rest[pos + marker.len()..];
        let Some(gt) = rest.find('>') else { break };
        rest = &rest[gt + 1..];
        let Some(lt) = rest.find('<') else { break };
        let value = decode_entities(rest[..lt].trim());
        if !value.is_empty() {
            out.push(value);
        }
    }
    out
}

fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn parse_library_html(html: &str) -> Vec<LibraryModel> {
    // Each model is an `<li x-test-model class=…>`. The trailing space keeps
    // the split from also matching `x-test-model-title`.
    html.split("x-test-model ")
        .skip(1)
        .filter_map(|block| {
            let name = between(block, "href=\"/library/", "\"")?;
            if name.is_empty() || !name.chars().all(is_model_name_char) {
                return None;
            }
            let description = between(block, "break-words", "</p>")
                .and_then(|p| p.split_once('>'))
                .map(|(_, text)| decode_entities(text.trim()))
                .unwrap_or_default();
            Some(LibraryModel {
                name: name.to_string(),
                description,
                capabilities: marker_values(block, "x-test-capability"),
                sizes: marker_values(block, "x-test-size"),
                pull_count: marker_values(block, "x-test-pull-count").into_iter().next(),
                updated: marker_values(block, "x-test-updated").into_iter().next(),
                // The cloud badge has no x-test hook; `>` anchors the match to
                // an element's full text ("480b-cloud" sizes don't trip it).
                cloud: block.contains(">cloud</span>"),
            })
        })
        .collect()
}

fn parse_tags_html(html: &str) -> Vec<LibraryTag> {
    let mut tags: Vec<LibraryTag> = Vec::new();
    // One `group px-4 py-3` block per tag row (it contains both the mobile and
    // desktop layouts; dedup below covers a future split into two blocks).
    for block in html.split("class=\"group px-4 py-3\"").skip(1) {
        let Some(full) = between(block, "href=\"/library/", "\"") else {
            continue;
        };
        // Tag links are `/library/<model>:<tag>`; anything without a colon is
        // a stray link back to the model page.
        if !full.contains(':') || !full.chars().all(|c| is_model_name_char(c) || c == ':') {
            continue;
        }
        // The `:latest` alias row duplicates the canonical tag (which carries
        // a "latest" pill); the picker builds `model:size` names, so the alias
        // row is noise.
        if full.ends_with(":latest") {
            continue;
        }
        // Desktop layout: `col-span-2` cells — download size, context window,
        // input modalities. Cloud tags have NO size cell (nothing to
        // download), so cells are classified by shape rather than position.
        let mut cells = Vec::new();
        let mut rest = block;
        while let Some(pos) = rest.find("col-span-2") {
            rest = &rest[pos + "col-span-2".len()..];
            let Some(gt) = rest.find('>') else { break };
            rest = &rest[gt + 1..];
            let Some(lt) = rest.find('<') else { break };
            cells.push(rest[..lt].trim().to_string());
        }
        let (size, context, input) = classify_cells(cells);
        let tag = LibraryTag {
            name: full.to_string(),
            size,
            context,
            input,
            latest: block.contains(">latest</span>"),
            digest: parse_digest(block),
        };
        match tags.iter_mut().find(|t| t.name == tag.name) {
            Some(existing) => {
                // Same tag seen again (layout duplication): keep whichever
                // copy actually carried the data cells.
                if existing.size.is_none() {
                    existing.size = tag.size;
                    existing.context = tag.context;
                    existing.input = tag.input;
                }
                if existing.digest.is_none() {
                    existing.digest = tag.digest;
                }
                existing.latest |= tag.latest;
            }
            None => tags.push(tag),
        }
    }
    tags
}

/// The tag row's manifest digest: the first `font-mono` span whose text is a
/// 12-char hex string (the row repeats it across its mobile/desktop layouts).
fn parse_digest(block: &str) -> Option<String> {
    marker_values(block, "class=\"font-mono\"")
        .into_iter()
        .find(|v| v.len() == 12 && v.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Sort a tag row's detail cells into (size, context, input) by their shape:
/// download sizes look like `11GB`, context windows like `256K`, and whatever
/// is left is the input modalities ("Text", "Text, Image"). Cloud tags simply
/// have no size-shaped cell.
fn classify_cells(cells: Vec<String>) -> (Option<String>, Option<String>, Option<String>) {
    let mut size = None;
    let mut context = None;
    let mut input = None;
    for cell in cells {
        if cell.is_empty() {
            continue;
        }
        if size.is_none() && is_byte_size(&cell) {
            size = Some(cell);
        } else if context.is_none() && is_context_window(&cell) {
            context = Some(cell);
        } else if input.is_none() && !is_byte_size(&cell) && !is_context_window(&cell) {
            input = Some(cell);
        }
    }
    (size, context, input)
}

fn is_byte_size(value: &str) -> bool {
    ["MB", "GB", "TB"].iter().any(|unit| {
        value
            .strip_suffix(unit)
            .is_some_and(|n| !n.is_empty() && n.parse::<f64>().is_ok())
    })
}

fn is_context_window(value: &str) -> bool {
    ["K", "M"].iter().any(|unit| {
        value
            .strip_suffix(unit)
            .is_some_and(|n| !n.is_empty() && n.parse::<f64>().is_ok())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed verbatim from https://ollama.com/library?sort=popular.
    const LIBRARY_HTML: &str = r#"
    <li x-test-model class="flex items-baseline border-b border-neutral-200 py-6">
      <a href="/library/deepseek-r1" class="group w-full space-y-5">
        <div x-test-model-title title="deepseek-r1" class="flex flex-col">
          <h2><div><span class="group-hover:underline truncate">deepseek-r1</span></div></h2>
          <p class="max-w-lg break-words text-neutral-800 text-md">DeepSeek-R1 is a family of open reasoning models with performance approaching that of leading models, such as O3 &amp; Gemini 2.5 Pro.</p>
        </div>
        <div>
          <span x-test-capability class="inline-flex">tools</span>
          <span x-test-capability class="inline-flex">thinking</span>
          <span x-test-size class="inline-flex">1.5b</span>
          <span x-test-size class="inline-flex">7b</span>
          <span x-test-size class="inline-flex">14b</span>
          <p><span class="flex items-center"><svg></svg><span x-test-pull-count>63.9M</span><span>&nbsp;Pulls</span></span>
          <span x-test-updated>5 months ago</span></p>
        </div>
      </a>
    </li>
    <li x-test-model class="flex items-baseline border-b border-neutral-200 py-6">
      <a href="/library/nomic-embed-text" class="group w-full space-y-5">
        <div x-test-model-title title="nomic-embed-text" class="flex flex-col">
          <p class="max-w-lg break-words text-neutral-800 text-md">A high-performing open embedding model with a large token context window.</p>
        </div>
        <div>
          <span x-test-capability class="inline-flex">embedding</span>
          <p><span x-test-pull-count>73.3M</span></p>
        </div>
      </a>
    </li>
    <li x-test-model class="flex items-baseline border-b border-neutral-200 py-6">
      <a href="/library/kimi-k2.6" class="group w-full space-y-5">
        <div x-test-model-title title="kimi-k2.6" class="flex flex-col">
          <p class="max-w-lg break-words text-neutral-800 text-md">Kimi K2.6 is an open-source, native multimodal agentic model.</p>
        </div>
        <div>
          <span x-test-capability class="inline-flex">vision</span>
          <span x-test-capability class="inline-flex">tools</span>
          <span class="inline-flex items-center rounded-md bg-cyan-50 px-2 py-0.5 text-xs font-medium text-cyan-500 sm:text-[13px]">cloud</span>
          <p><span x-test-pull-count>287.3K</span></p>
        </div>
      </a>
    </li>
    "#;

    // Trimmed verbatim from https://ollama.com/library/phi4-reasoning/tags —
    // one group block per tag, each holding both layout variants.
    const TAGS_HTML: &str = r#"
    <div class="group px-4 py-3">
      <div class="flex flex-col">
        <span class="group-hover:underline">phi4-reasoning:latest</span>
      </div>
      <div class="grid grid-cols-12 items-center">
        <span class="flex items-center font-medium col-span-6 group text-sm">
          <a href="/library/phi4-reasoning:latest" class="group-hover:underline">phi4-reasoning:latest</a>
          <input class="command hidden" value="phi4-reasoning:latest" />
        </span>
        <p class="col-span-2 text-neutral-500 text-[13px]">11GB</p>
        <p class="col-span-2 text-neutral-500 text-[13px]">32K</p>
        <div class="col-span-2 text-neutral-500 text-[13px] ">
          Text
        </div>
      </div>
    </div>
    <div class="group px-4 py-3">
      <div class="md:hidden flex flex-col text-neutral-500 text-[13px]">
        <span>
          <span class="font-mono">
            6f96fb1c1edd</span> • 11GB • 32K context window  •
          <span>Text</span>
        </span>
      </div>
      <div class="grid grid-cols-12 items-center">
        <span class="flex items-center font-medium col-span-6 group text-sm">
          <a href="/library/phi4-reasoning:14b" class="group-hover:underline">phi4-reasoning:14b</a>
          <span class="ml-2 inline-flex items-center rounded-full px-2 py-px text-xs font-medium border border-blue-500 text-blue-600">latest</span>
          <input class="command hidden" value="phi4-reasoning:14b" />
        </span>
        <p class="col-span-2 text-neutral-500 text-[13px]">11GB</p>
        <p class="col-span-2 text-neutral-500 text-[13px]">32K</p>
        <div class="col-span-2 text-neutral-500 text-[13px] ">
          Text
        </div>
      </div>
    </div>
    <div class="group px-4 py-3">
      <div class="grid grid-cols-12 items-center">
        <span class="flex items-center font-medium col-span-6 group text-sm">
          <a href="/library/phi4-reasoning:14b-q8_0" class="group-hover:underline">phi4-reasoning:14b-q8_0</a>
          <input class="command hidden" value="phi4-reasoning:14b-q8_0" />
        </span>
        <p class="col-span-2 text-neutral-500 text-[13px]">16GB</p>
        <p class="col-span-2 text-neutral-500 text-[13px]">32K</p>
        <div class="col-span-2 text-neutral-500 text-[13px] ">
          Text
        </div>
      </div>
    </div>
    "#;

    // Cloud tag row (https://ollama.com/library/kimi-k2.6/tags): no download
    // size cell at all — only context and input.
    const CLOUD_TAGS_HTML: &str = r#"
    <div class="group px-4 py-3">
      <div class="grid grid-cols-12 items-center">
        <span class="flex items-center font-medium col-span-6 group text-sm">
          <a href="/library/kimi-k2.6:cloud" class="group-hover:underline">kimi-k2.6:cloud</a>
          <span class="ml-2 inline-flex items-center rounded-full px-2 py-px text-xs font-medium border border-blue-500 text-blue-600">latest</span>
          <input class="command hidden" value="kimi-k2.6:cloud" />
        </span>
        <p class="col-span-2 text-neutral-500 text-[13px]">256K</p>
        <div class="col-span-2 text-neutral-500 text-[13px] ">
          Text, Image
        </div>
      </div>
    </div>
    "#;

    #[test]
    fn parses_library_models_with_all_fields() {
        let models = parse_library_html(LIBRARY_HTML);
        assert_eq!(models.len(), 3);
        let r1 = &models[0];
        assert_eq!(r1.name, "deepseek-r1");
        assert!(r1.description.starts_with("DeepSeek-R1 is a family"));
        assert!(r1.description.contains("O3 & Gemini")); // entity decoded
        assert_eq!(r1.capabilities, vec!["tools", "thinking"]);
        assert_eq!(r1.sizes, vec!["1.5b", "7b", "14b"]);
        assert_eq!(r1.pull_count.as_deref(), Some("63.9M"));
        assert_eq!(r1.updated.as_deref(), Some("5 months ago"));
        assert!(!r1.cloud);
        let embed = &models[1];
        assert_eq!(embed.name, "nomic-embed-text");
        assert_eq!(embed.capabilities, vec!["embedding"]);
        assert!(embed.sizes.is_empty());
        assert!(!embed.cloud);
        let kimi = &models[2];
        assert_eq!(kimi.name, "kimi-k2.6");
        assert!(kimi.cloud);
        assert!(kimi.sizes.is_empty());
    }

    #[test]
    fn cloud_tags_have_no_size_and_context_stays_context() {
        let tags = parse_tags_html(CLOUD_TAGS_HTML);
        assert_eq!(tags.len(), 1);
        let tag = &tags[0];
        assert_eq!(tag.name, "kimi-k2.6:cloud");
        assert_eq!(tag.size, None); // 256K must NOT be misread as a download size
        assert_eq!(tag.context.as_deref(), Some("256K"));
        assert_eq!(tag.input.as_deref(), Some("Text, Image"));
        assert!(tag.latest);
    }

    #[test]
    fn parses_tags_with_sizes_and_latest_marker() {
        let tags = parse_tags_html(TAGS_HTML);
        // The `:latest` alias row is dropped; the canonical tags survive.
        assert_eq!(
            tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            vec!["phi4-reasoning:14b", "phi4-reasoning:14b-q8_0"]
        );
        let canonical = &tags[0];
        assert_eq!(canonical.size.as_deref(), Some("11GB"));
        assert_eq!(canonical.context.as_deref(), Some("32K"));
        assert_eq!(canonical.input.as_deref(), Some("Text"));
        assert!(canonical.latest);
        assert_eq!(canonical.digest.as_deref(), Some("6f96fb1c1edd"));
        assert!(!tags[1].latest);
        assert_eq!(tags[1].size.as_deref(), Some("16GB"));
        assert_eq!(tags[1].digest, None); // row without a digest span
    }

    #[test]
    fn rejects_hostile_model_names() {
        assert!("qwen2.5-coder".chars().all(is_model_name_char));
        assert!(!"../etc/passwd".chars().all(is_model_name_char));
        assert!(!"a b".chars().all(is_model_name_char));
        assert!(!"x/y".chars().all(is_model_name_char));
    }
}
