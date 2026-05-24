//! Go runtime detector.
//!
//! Discovery via `runtimes::env` — no hardcoded paths or versions.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use crate::registry::RuntimeSettings;
use crate::runtimes::env;
use crate::runtimes::{
    version_from, ApplyResult, ConfigTab, InstallSource, KvRow, LanguageRuntime, RuntimeInstall,
};

pub struct GoRuntime;

impl LanguageRuntime for GoRuntime {
    fn id(&self) -> &'static str {
        "go"
    }
    fn display_name(&self) -> &'static str {
        "Go"
    }
    fn install_hint(&self) -> &'static str {
        "brew install go"
    }
    fn probe_version(&self, binary: &std::path::Path) -> Option<String> {
        // `go --version` is not valid; Go reports via `go version`.
        version_from(binary, "version")
    }

    fn tabs(&self, _install: &RuntimeInstall, _settings: &RuntimeSettings) -> Vec<ConfigTab> {
        // Go config is system-owned: `go env -w` persists to a global `env`
        // file (shared across every Go version). We read/write it directly.
        let env_vars = read_go_env();
        let path_hint = go_env_path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "(unknown)".into());

        let tab = ConfigTab::editable(
            "env",
            "Environment",
            vec![
                KvRow::text(
                    "GOPROXY",
                    "GOPROXY",
                    env_vars.get("GOPROXY").cloned().unwrap_or_default(),
                )
                .with_hint(
                    "Module proxy. Blank uses Go's default \
                     (https://proxy.golang.org,direct). Accepts a comma list, \
                     `direct`, or `off`.",
                ),
                KvRow::text(
                    "GOPATH",
                    "GOPATH",
                    env_vars.get("GOPATH").cloned().unwrap_or_default(),
                )
                .with_hint("Workspace root. Blank uses Go's default (~/go)."),
                KvRow::path("Env file", path_hint),
            ],
        );
        vec![tab]
    }

    fn apply_config(
        &self,
        _version: &str,
        tab_id: &str,
        patches: &BTreeMap<String, String>,
        _settings: &mut RuntimeSettings,
    ) -> Result<ApplyResult, String> {
        let updates = validate_go_env_patch(tab_id, patches)?;
        let refs: Vec<(&str, Option<String>)> = updates
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect();
        write_go_env(&refs)?;
        Ok(ApplyResult::default()) // Go has no daemon.
    }

    fn detect(&self) -> Vec<RuntimeInstall> {
        let mut out: Vec<RuntimeInstall> = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();

        for (_, dir) in env::brew_formulae_matching("go") {
            push(
                &mut out,
                &mut seen,
                dir.join("bin").join("go"),
                InstallSource::Homebrew,
            );
        }

        if let Some(asdf) = env::asdf_root() {
            // asdf-golang installs land under <root>/installs/golang/<ver>/go/bin/go
            let golang = asdf.join("installs").join("golang");
            if let Ok(entries) = std::fs::read_dir(&golang) {
                for entry in entries.flatten() {
                    push(
                        &mut out,
                        &mut seen,
                        entry.path().join("go").join("bin").join("go"),
                        InstallSource::Asdf,
                    );
                }
            }
        }
        if let Some(mise) = env::mise_installs_root() {
            let go_dir = mise.join("go");
            if let Ok(entries) = std::fs::read_dir(&go_dir) {
                for entry in entries.flatten() {
                    push(
                        &mut out,
                        &mut seen,
                        entry.path().join("bin").join("go"),
                        InstallSource::Mise,
                    );
                }
            }
        }

        if let Ok(p) = which::which("go") {
            push(&mut out, &mut seen, p, InstallSource::System);
        }
        out
    }
}

fn push(
    out: &mut Vec<RuntimeInstall>,
    seen: &mut HashSet<PathBuf>,
    bin: PathBuf,
    source: InstallSource,
) {
    if !bin.exists() {
        return;
    }
    let canonical = bin.canonicalize().unwrap_or_else(|_| bin.clone());
    if !seen.insert(canonical) {
        return;
    }
    let Some(version) = version_from(&bin, "version") else {
        return;
    };
    out.push(RuntimeInstall {
        version,
        binary: bin,
        source,
        config_dir: None,
    });
}

// ---------------------------------------------------------------------------
// Go env file — the target of `go env -w`. A flat `KEY=value` file under the
// user config dir (or $GOENV), shared across every Go version. We edit it
// directly with the shared surgical writer rather than shelling out.
// ---------------------------------------------------------------------------

fn go_env_path() -> Option<PathBuf> {
    match std::env::var("GOENV") {
        Ok(p) if !p.is_empty() && p != "off" => Some(PathBuf::from(p)),
        _ => dirs::config_dir().map(|c| c.join("go").join("env")),
    }
}

fn read_go_env() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(path) = go_env_path() else {
        return out;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return out;
    };
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            out.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out
}

fn validate_go_env_patch(
    tab_id: &str,
    patches: &BTreeMap<String, String>,
) -> Result<Vec<(String, Option<String>)>, String> {
    if tab_id != "env" {
        return Err(format!("Go has no editable tab `{tab_id}`"));
    }
    let mut out = Vec::new();
    for (key, raw) in patches {
        if !matches!(key.as_str(), "GOPROXY" | "GOPATH") {
            return Err(format!("unknown Go setting `{key}`"));
        }
        let val = raw.trim();
        if val.is_empty() {
            out.push((key.clone(), None));
            continue;
        }
        if val.contains(['\n', '\r']) {
            return Err(format!("`{key}` contains an illegal character"));
        }
        out.push((key.clone(), Some(val.to_string())));
    }
    Ok(out)
}

fn write_go_env(updates: &[(&str, Option<String>)]) -> Result<(), String> {
    let path = go_env_path().ok_or("could not resolve Go env file path")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("couldn't create {}: {e}", parent.display()))?;
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let body = crate::runtimes::apply_flat_config(&existing, '=', "=", updates);
    std::fs::write(&path, body).map_err(|e| format!("couldn't write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn validate_accepts_known_keys_and_clears_blanks() {
        let updates = validate_go_env_patch(
            "env",
            &patch(&[
                ("GOPROXY", "https://proxy.example/,direct"),
                ("GOPATH", "  "),
            ]),
        )
        .unwrap();
        assert!(updates.contains(&(
            "GOPROXY".to_string(),
            Some("https://proxy.example/,direct".to_string())
        )));
        assert!(updates.contains(&("GOPATH".to_string(), None)));
    }

    #[test]
    fn validate_rejects_unknown_tab_and_key() {
        assert!(validate_go_env_patch("nope", &patch(&[("GOPROXY", "x")])).is_err());
        assert!(validate_go_env_patch("env", &patch(&[("GOROOT", "/x")])).is_err());
    }
}
