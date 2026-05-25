//! Project-specific runtime detection and marker-file management.
//!
//! PortBay stores a project runtime pin in the registry and also mirrors it to
//! conventional version-manager files when possible. That gives both PortBay's
//! Play button and the user's terminal the same project-local runtime choice.

use std::path::{Path, PathBuf};

use crate::registry::Runtime;

pub fn detect(path: &Path) -> Option<Runtime> {
    detect_from_tool_versions(path)
        .or_else(|| detect_single_file(path, ".nvmrc", "node"))
        .or_else(|| detect_single_file(path, ".node-version", "node"))
        .or_else(|| detect_single_file(path, ".php-version", "php"))
        .or_else(|| detect_single_file(path, ".python-version", "python"))
        .or_else(|| detect_single_file(path, ".ruby-version", "ruby"))
        .or_else(|| detect_single_file(path, ".go-version", "go"))
        .or_else(|| detect_single_file(path, ".flutter-version", "flutter"))
        .or_else(|| detect_single_file(path, ".fvmrc", "flutter"))
        .or_else(|| detect_from_fvm_config(path))
        .or_else(|| detect_from_mise(path))
        .or_else(|| detect_from_package_json(path))
        .or_else(|| detect_from_composer(path))
        .or_else(|| detect_from_pyproject(path))
}

pub fn ensure_marker_files(path: &Path, runtime: &Runtime) -> Result<(), String> {
    if !path.is_dir() {
        return Ok(());
    }
    ensure_tool_versions(path, runtime)?;
    match runtime.lang.as_str() {
        "node" => {
            ensure_single(path.join(".node-version"), &runtime.version)?;
            ensure_single(path.join(".nvmrc"), &runtime.version)?;
        }
        "php" => ensure_single(path.join(".php-version"), &runtime.version)?,
        "python" => ensure_single(path.join(".python-version"), &runtime.version)?,
        "ruby" => ensure_single(path.join(".ruby-version"), &runtime.version)?,
        "go" => ensure_single(path.join(".go-version"), &runtime.version)?,
        "flutter" => {
            ensure_single(path.join(".flutter-version"), &runtime.version)?;
            ensure_single(path.join(".fvmrc"), &runtime.version)?;
        }
        _ => {}
    }
    Ok(())
}

fn detect_single_file(path: &Path, name: &str, lang: &str) -> Option<Runtime> {
    let raw = std::fs::read_to_string(path.join(name)).ok()?;
    let version = clean_version(raw.lines().next()?.trim())?;
    Some(Runtime {
        lang: lang.into(),
        version,
    })
}

fn detect_from_tool_versions(path: &Path) -> Option<Runtime> {
    let raw = std::fs::read_to_string(path.join(".tool-versions")).ok()?;
    for line in raw.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let lang = normalise_lang(parts.next()?);
        let version = clean_version(parts.next()?)?;
        if managed_lang(lang).is_some() {
            return Some(Runtime {
                lang: lang.into(),
                version,
            });
        }
    }
    None
}

fn detect_from_mise(path: &Path) -> Option<Runtime> {
    let raw = std::fs::read_to_string(path.join("mise.toml"))
        .or_else(|_| std::fs::read_to_string(path.join(".mise.toml")))
        .ok()?;
    for line in raw.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let lang = normalise_lang(key.trim().trim_matches('"'));
        if managed_lang(lang).is_none() {
            continue;
        }
        let version = clean_version(value.trim().trim_matches(['"', '\'']))?;
        return Some(Runtime {
            lang: lang.into(),
            version,
        });
    }
    None
}

fn detect_from_package_json(path: &Path) -> Option<Runtime> {
    let raw = std::fs::read_to_string(path.join("package.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let node = json
        .get("engines")
        .and_then(|e| e.get("node"))
        .and_then(|v| v.as_str())?;
    Some(Runtime {
        lang: "node".into(),
        version: extract_version(node)?,
    })
}

fn detect_from_composer(path: &Path) -> Option<Runtime> {
    let raw = std::fs::read_to_string(path.join("composer.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let php = json
        .get("config")
        .and_then(|c| c.get("platform"))
        .and_then(|p| p.get("php"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            json.get("require")
                .and_then(|r| r.get("php"))
                .and_then(|v| v.as_str())
        })?;
    Some(Runtime {
        lang: "php".into(),
        version: extract_major_minor(php)?,
    })
}

fn detect_from_pyproject(path: &Path) -> Option<Runtime> {
    let raw = std::fs::read_to_string(path.join("pyproject.toml")).ok()?;
    for line in raw.lines() {
        let line = line.trim();
        if !line.starts_with("requires-python") {
            continue;
        }
        let (_, value) = line.split_once('=')?;
        return Some(Runtime {
            lang: "python".into(),
            version: extract_major_minor(value.trim().trim_matches(['"', '\'']))?,
        });
    }
    None
}

fn detect_from_fvm_config(path: &Path) -> Option<Runtime> {
    let raw = std::fs::read_to_string(path.join(".fvm").join("fvm_config.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let version = json.get("flutterSdkVersion").and_then(|v| v.as_str())?;
    Some(Runtime {
        lang: "flutter".into(),
        version: clean_version(version)?,
    })
}

fn ensure_single(path: PathBuf, version: &str) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    std::fs::write(&path, format!("{version}\n"))
        .map_err(|e| format!("write {}: {e}", path.display()))
}

fn ensure_tool_versions(path: &Path, runtime: &Runtime) -> Result<(), String> {
    let tool = tool_versions_lang(&runtime.lang);
    let file = path.join(".tool-versions");
    let mut lines = std::fs::read_to_string(&file)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut replaced = false;
    for line in &mut lines {
        let trimmed = line.split('#').next().unwrap_or("").trim();
        let Some(first) = trimmed.split_whitespace().next() else {
            continue;
        };
        if normalise_lang(first) == runtime.lang {
            *line = format!("{tool} {}", runtime.version);
            replaced = true;
        }
    }
    if !replaced {
        lines.push(format!("{tool} {}", runtime.version));
    }
    let mut body = lines.join("\n");
    body.push('\n');
    std::fs::write(&file, body).map_err(|e| format!("write {}: {e}", file.display()))
}

fn tool_versions_lang(lang: &str) -> &str {
    match lang {
        "node" => "nodejs",
        other => other,
    }
}

fn normalise_lang(lang: &str) -> &str {
    match lang {
        "nodejs" => "node",
        "python3" => "python",
        other => other,
    }
}

fn managed_lang(lang: &str) -> Option<&str> {
    matches!(
        lang,
        "node" | "php" | "python" | "ruby" | "go" | "bun" | "flutter"
    )
    .then_some(lang)
}

fn clean_version(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches('v');
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("system") {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn extract_version(raw: &str) -> Option<String> {
    let cleaned = raw
        .trim()
        .trim_start_matches(['^', '~', '>', '=', '<', ' '])
        .trim_start_matches('v');
    let version = cleaned.split_whitespace().next()?.trim_matches(',');
    clean_version(version)
}

fn extract_major_minor(raw: &str) -> Option<String> {
    let version = extract_version(raw)?;
    let mut parts = version.split('.');
    let major = parts.next()?;
    let Some(minor) = parts.next() else {
        return Some(major.to_string());
    };
    Some(format!("{major}.{minor}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_node_from_nvmrc() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".nvmrc"), "v20.11.1\n").unwrap();
        let rt = detect(dir.path()).unwrap();
        assert_eq!(rt.lang, "node");
        assert_eq!(rt.version, "20.11.1");
    }

    #[test]
    fn detects_php_from_composer_constraint_as_major_minor() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("composer.json"),
            r#"{ "require": { "php": "^8.3" } }"#,
        )
        .unwrap();
        let rt = detect(dir.path()).unwrap();
        assert_eq!(rt.lang, "php");
        assert_eq!(rt.version, "8.3");
    }

    #[test]
    fn writes_marker_files_without_clobbering_existing_language_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".nvmrc"), "20\n").unwrap();
        ensure_marker_files(
            dir.path(),
            &Runtime {
                lang: "node".into(),
                version: "22.1.0".into(),
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".nvmrc")).unwrap(),
            "20\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".node-version")).unwrap(),
            "22.1.0\n"
        );
        assert!(std::fs::read_to_string(dir.path().join(".tool-versions"))
            .unwrap()
            .contains("nodejs 22.1.0"));
    }

    #[test]
    fn detects_flutter_from_fvm_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".fvm")).unwrap();
        std::fs::write(
            dir.path().join(".fvm").join("fvm_config.json"),
            r#"{ "flutterSdkVersion": "3.24.5" }"#,
        )
        .unwrap();
        let rt = detect(dir.path()).unwrap();
        assert_eq!(rt.lang, "flutter");
        assert_eq!(rt.version, "3.24.5");
    }
}
