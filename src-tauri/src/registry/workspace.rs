//! Monorepo / workspace detection.
//!
//! Given a folder, decide whether it's a JS monorepo root and, if so,
//! enumerate the *runnable* apps inside it — packages that declare a `dev`
//! script. The add-project flow uses this to offer running just one app
//! (configured as a standalone project at its sub-directory) instead of a root
//! `pnpm dev` that fans out through `turbo run dev --parallel` and starts every
//! app in the repo.
//!
//! Pure filesystem + parse. Deliberately does **no** [`ProjectType`] inference
//! — that stays in the command layer (`commands::projects::detect_kind`), which
//! keeps this a low-level registry helper with no dependency back up on
//! `commands`.
//!
//! [`ProjectType`]: crate::registry::ProjectType

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::registry::types::WorkspaceTool;

/// A detected monorepo and the runnable apps inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceLayout {
    /// Best-guess tool to scope a single-app run, inferred from the lockfile.
    /// Turbo is never auto-selected (it pulls in the build pipeline); the user
    /// opts into it from the project detail panel if their dev script needs it.
    pub tool: WorkspaceTool,
    /// Runnable packages (those declaring a `dev` script), sorted by `rel_dir`.
    pub packages: Vec<WorkspacePackage>,
}

/// One package inside a monorepo that the picker can offer to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePackage {
    /// `name` from the package's `package.json` — the workspace filter token.
    pub name: String,
    /// Directory relative to the monorepo root (e.g. `apps/web`).
    pub rel_dir: String,
    /// Absolute directory (root joined with `rel_dir`).
    pub abs_dir: PathBuf,
}

/// Detect whether `root` is a monorepo and enumerate its runnable apps.
///
/// Returns `None` when `root` isn't a recognised monorepo, or is one with no
/// package declaring a `dev` script — in both cases the caller falls back to
/// treating the folder as a single standalone project.
pub fn detect(root: &Path) -> Option<WorkspaceLayout> {
    let globs = workspace_globs(root)?;
    let tool = detect_tool(root);

    let mut packages: Vec<WorkspacePackage> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for glob in &globs {
        for rel_dir in expand_glob(root, glob) {
            if !seen.insert(rel_dir.clone()) {
                continue;
            }
            if let Some(pkg) = read_runnable_package(root, &rel_dir) {
                packages.push(pkg);
            }
        }
    }

    if packages.is_empty() {
        return None;
    }
    packages.sort_by(|a, b| a.rel_dir.cmp(&b.rel_dir));
    Some(WorkspaceLayout { tool, packages })
}

/// The workspace package globs declared at `root`, or `None` if `root` isn't a
/// monorepo. Sources, in order: `pnpm-workspace.yaml`, the root `package.json`
/// `workspaces` field, `lerna.json` `packages`. When only a `turbo.json` /
/// `nx.json` marks the repo (no explicit globs), fall back to the conventional
/// `apps/*` + `packages/*` layout those tools assume.
fn workspace_globs(root: &Path) -> Option<Vec<String>> {
    // pnpm-workspace.yaml: `packages: [ "apps/*", ... ]`
    if let Ok(text) = std::fs::read_to_string(root.join("pnpm-workspace.yaml")) {
        if let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&text) {
            if let Some(list) = doc.get("packages").and_then(|v| v.as_sequence()) {
                let globs: Vec<String> = list
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect();
                if !globs.is_empty() {
                    return Some(globs);
                }
            }
        }
    }

    // package.json "workspaces": [ ... ] or { "packages": [ ... ] }
    if let Some(globs) = package_json_workspaces(root) {
        return Some(globs);
    }

    // lerna.json "packages": [ ... ] (defaults to ["packages/*"] when absent).
    if let Ok(text) = std::fs::read_to_string(root.join("lerna.json")) {
        if let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) {
            let globs = string_array(doc.get("packages"));
            return Some(if globs.is_empty() {
                vec!["packages/*".into()]
            } else {
                globs
            });
        }
    }

    // turbo.json / nx.json mark a monorepo but don't carry globs themselves;
    // assume the conventional layout they're almost always paired with.
    if root.join("turbo.json").exists() || root.join("nx.json").exists() {
        return Some(vec!["apps/*".into(), "packages/*".into()]);
    }

    None
}

/// Read `workspaces` out of the root `package.json`, accepting both the array
/// form and the Yarn `{ "packages": [...] }` object form.
fn package_json_workspaces(root: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(root.join("package.json")).ok()?;
    let doc: serde_json::Value = serde_json::from_str(&text).ok()?;
    let ws = doc.get("workspaces")?;
    let globs = if ws.is_array() {
        string_array(Some(ws))
    } else {
        string_array(ws.get("packages"))
    };
    if globs.is_empty() {
        None
    } else {
        Some(globs)
    }
}

/// Collect a JSON value into a `Vec<String>`, dropping non-string entries.
/// `None`/non-array yields an empty vec.
fn string_array(v: Option<&serde_json::Value>) -> Vec<String> {
    v.and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|e| e.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Infer the scoping tool from the lockfile present at `root`. Defaults to
/// pnpm — matching `detect_kind`'s `pnpm dev` bias — when none is found.
fn detect_tool(root: &Path) -> WorkspaceTool {
    if root.join("pnpm-lock.yaml").exists() {
        WorkspaceTool::Pnpm
    } else if root.join("yarn.lock").exists() {
        WorkspaceTool::Yarn
    } else if root.join("package-lock.json").exists() {
        WorkspaceTool::Npm
    } else {
        WorkspaceTool::Pnpm
    }
}

/// Expand one workspace glob into relative package directories. Handles the two
/// shapes real monorepos use: a one-level `prefix/*` (and `prefix/**`, treated
/// the same — one level, a documented limitation) and an exact path. Any other
/// glob shape is skipped rather than mis-expanded.
fn expand_glob(root: &Path, pattern: &str) -> Vec<String> {
    let pattern = pattern.trim_start_matches("./").trim_end_matches('/');
    if pattern.is_empty() {
        return Vec::new();
    }

    let one_level_prefix = pattern
        .strip_suffix("/**")
        .or_else(|| pattern.strip_suffix("/*"));
    if let Some(prefix) = one_level_prefix {
        // `prefix` itself must be a literal directory (no nested globs).
        if prefix.contains('*') {
            return Vec::new();
        }
        let base = root.join(prefix);
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&base) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        // Skip dotfiles like `.turbo`.
                        if !name.starts_with('.') {
                            out.push(format!("{prefix}/{name}"));
                        }
                    }
                }
            }
        }
        out
    } else if pattern.contains('*') {
        // Unsupported complex glob (e.g. `apps/*/web`); skip it.
        Vec::new()
    } else if root.join(pattern).is_dir() {
        vec![pattern.to_string()]
    } else {
        Vec::new()
    }
}

/// Read a package's `package.json` and return it only if it's a runnable app:
/// it has a `name` (needed as the filter token) and a `scripts.dev` entry.
fn read_runnable_package(root: &Path, rel_dir: &str) -> Option<WorkspacePackage> {
    let abs_dir = root.join(rel_dir);
    let text = std::fs::read_to_string(abs_dir.join("package.json")).ok()?;
    let doc: serde_json::Value = serde_json::from_str(&text).ok()?;

    let name = doc.get("name").and_then(|v| v.as_str())?.to_string();
    let has_dev_script = doc
        .get("scripts")
        .and_then(|s| s.get("dev"))
        .and_then(|d| d.as_str())
        .is_some_and(|s| !s.trim().is_empty());
    if !has_dev_script {
        return None;
    }

    Some(WorkspacePackage {
        name,
        rel_dir: rel_dir.to_string(),
        abs_dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Write a package.json with the given name and optional dev script.
    fn write_pkg(dir: &Path, name: &str, dev: Option<&str>) {
        fs::create_dir_all(dir).unwrap();
        let scripts = match dev {
            Some(cmd) => format!(r#""scripts": {{ "dev": "{cmd}" }}"#),
            None => r#""scripts": { "build": "tsc" }"#.to_string(),
        };
        fs::write(
            dir.join("package.json"),
            format!(r#"{{ "name": "{name}", {scripts} }}"#),
        )
        .unwrap();
    }

    #[test]
    fn detects_pnpm_workspace_and_lists_only_runnable_apps() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("pnpm-workspace.yaml"),
            "packages:\n  - \"apps/*\"\n  - \"packages/*\"\n",
        )
        .unwrap();
        fs::write(root.join("pnpm-lock.yaml"), "").unwrap();
        fs::write(root.join("turbo.json"), "{}").unwrap();

        write_pkg(&root.join("apps/web"), "@acme/web", Some("next dev"));
        write_pkg(
            &root.join("apps/realtime"),
            "@acme/realtime",
            Some("node server.js"),
        );
        // A library package with no dev script — must be excluded.
        write_pkg(&root.join("packages/ui"), "@acme/ui", None);

        let layout = detect(root).expect("monorepo should be detected");
        assert_eq!(layout.tool, WorkspaceTool::Pnpm);
        let names: Vec<&str> = layout.packages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["@acme/realtime", "@acme/web"]); // sorted by rel_dir
        assert!(layout
            .packages
            .iter()
            .all(|p| p.rel_dir.starts_with("apps/")));
        let web = layout
            .packages
            .iter()
            .find(|p| p.name == "@acme/web")
            .unwrap();
        assert_eq!(web.rel_dir, "apps/web");
        assert_eq!(web.abs_dir, root.join("apps/web"));
    }

    #[test]
    fn detects_package_json_workspaces_array_and_yarn_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("package.json"),
            r#"{ "name": "root", "private": true, "workspaces": ["apps/*"] }"#,
        )
        .unwrap();
        fs::write(root.join("yarn.lock"), "").unwrap();
        write_pkg(&root.join("apps/site"), "site", Some("vite"));

        let layout = detect(root).expect("workspaces array should detect");
        assert_eq!(layout.tool, WorkspaceTool::Yarn);
        assert_eq!(layout.packages.len(), 1);
        assert_eq!(layout.packages[0].name, "site");
    }

    #[test]
    fn detects_yarn_object_workspaces_form() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("package.json"),
            r#"{ "name": "root", "workspaces": { "packages": ["packages/*"] } }"#,
        )
        .unwrap();
        write_pkg(&root.join("packages/app"), "app", Some("dev"));

        let layout = detect(root).expect("object workspaces form should detect");
        assert_eq!(layout.packages.len(), 1);
        assert_eq!(layout.packages[0].rel_dir, "packages/app");
    }

    #[test]
    fn turbo_only_falls_back_to_conventional_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // turbo.json but no pnpm-workspace.yaml and no package.json workspaces.
        fs::write(root.join("turbo.json"), r#"{ "pipeline": {} }"#).unwrap();
        write_pkg(&root.join("apps/dash"), "dash", Some("next dev"));

        let layout = detect(root).expect("turbo.json should mark a monorepo");
        assert_eq!(layout.packages.len(), 1);
        assert_eq!(layout.packages[0].name, "dash");
    }

    #[test]
    fn plain_folder_is_not_a_monorepo() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write_pkg(root, "single-app", Some("next dev"));
        assert!(detect(root).is_none());
    }

    #[test]
    fn monorepo_with_no_runnable_apps_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("pnpm-workspace.yaml"),
            "packages:\n  - \"packages/*\"\n",
        )
        .unwrap();
        write_pkg(&root.join("packages/lib"), "lib", None); // no dev script
        assert!(detect(root).is_none());
    }

    #[test]
    fn expand_glob_handles_one_level_exact_and_skips_complex() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("apps/web")).unwrap();
        fs::create_dir_all(root.join("apps/api")).unwrap();
        fs::create_dir_all(root.join("apps/.turbo")).unwrap(); // dotdir, skipped
        fs::create_dir_all(root.join("standalone")).unwrap();

        let mut star = expand_glob(root, "apps/*");
        star.sort();
        assert_eq!(star, vec!["apps/api".to_string(), "apps/web".to_string()]);

        assert_eq!(expand_glob(root, "standalone"), vec!["standalone"]);
        assert!(expand_glob(root, "missing").is_empty());
        assert!(expand_glob(root, "apps/*/web").is_empty()); // complex glob skipped
    }
}
