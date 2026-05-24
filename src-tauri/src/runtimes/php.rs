//! PHP runtime — adapter over the existing `crate::php` detection.
//!
//! Keeps the PHP-specific logic in `src/php/` (which the reconciler
//! already depends on for FPM lifecycle) and re-shapes its output
//! into the generic `RuntimeInstall` + `ConfigTab` surface the
//! `/languages` panel renders.
//!
//! The FPM and PHP tabs are **editable**: their values come from the saved
//! [`PhpVersionConfig`] for the version (falling back to defaults), and edits
//! flow back through [`PhpRuntime::apply_config`] into the registry. The
//! reconciler folds that config into the per-version FPM pool — the system
//! php.ini is never touched (overrides apply per-pool via `php_admin_value`).

use std::collections::BTreeMap;

use crate::php::lifecycle::fpm_process_id;
use crate::php::{self, PhpInstall, PhpSource};
use crate::registry::{FpmTuning, RuntimeSettings};
use crate::runtimes::{
    ApplyResult, ConfigTab, InstallSource, KvRow, LanguageRuntime, RuntimeInstall,
};

pub struct PhpRuntime;

/// php.ini keys the PHP tab surfaces as editable fields, in display order.
/// `(key, label)`. Keeping this list in one place keeps the tab renderer and
/// the apply path agreed on which overrides exist.
const INI_FIELDS: &[(&str, &str)] = &[
    ("memory_limit", "Memory limit"),
    ("upload_max_filesize", "Upload max filesize"),
    ("post_max_size", "Post max size"),
    ("max_execution_time", "Max execution time"),
    ("date.timezone", "Default timezone"),
];

impl LanguageRuntime for PhpRuntime {
    fn id(&self) -> &'static str {
        "php"
    }
    fn display_name(&self) -> &'static str {
        "PHP"
    }
    fn install_hint(&self) -> &'static str {
        "brew install php@8.3"
    }

    fn detect(&self) -> Vec<RuntimeInstall> {
        php::detect_all()
            .into_iter()
            .map(|p| RuntimeInstall {
                version: p.version.clone(),
                binary: p.php_bin.clone(),
                source: source_from(p.source),
                config_dir: p.additional_ini_dir.clone(),
            })
            .collect()
    }

    fn tabs(&self, install: &RuntimeInstall, settings: &RuntimeSettings) -> Vec<ConfigTab> {
        // Re-probe via the same helper the reconciler uses so the
        // detail tabs reflect the exact same view of the install.
        // Cheap (php --ini + php -m); only runs on panel-open.
        let php = php::detect_all()
            .into_iter()
            .find(|p| p.version == install.version)
            .unwrap_or_else(|| placeholder_install(install));

        let cfg = settings
            .php
            .get(&install.version)
            .cloned()
            .unwrap_or_default();

        vec![
            fpm_tab(&php, &cfg.fpm),
            php_tab(&php, &cfg.ini),
            extensions_tab(&php),
        ]
    }

    fn apply_config(
        &self,
        version: &str,
        tab_id: &str,
        patches: &BTreeMap<String, String>,
        settings: &mut RuntimeSettings,
    ) -> Result<ApplyResult, String> {
        let entry = settings.php.entry(version.to_string()).or_default();
        match tab_id {
            "fpm" => apply_fpm(&mut entry.fpm, patches)?,
            "php" => apply_ini(&mut entry.ini, patches)?,
            other => return Err(format!("PHP has no editable tab `{other}`")),
        }
        // The reconciler rewrites this version's pool config from the saved
        // settings every tick; restarting the FPM process picks it up now.
        Ok(ApplyResult {
            restart_processes: vec![fpm_process_id(version)],
        })
    }
}

/// Validate + apply FPM pool patches onto `tuning`. Unknown keys are rejected
/// so a buggy frontend can't silently no-op.
fn apply_fpm(tuning: &mut FpmTuning, patches: &BTreeMap<String, String>) -> Result<(), String> {
    for (key, raw) in patches {
        let val = raw.trim();
        match key.as_str() {
            "pm" => {
                if !matches!(val, "dynamic" | "static" | "ondemand") {
                    return Err(format!(
                        "process manager must be dynamic, static, or ondemand (got `{val}`)"
                    ));
                }
                tuning.pm = val.to_string();
            }
            "max_children" => tuning.max_children = parse_count(key, val, 1)?,
            "start_servers" => tuning.start_servers = parse_count(key, val, 0)?,
            "min_spare_servers" => tuning.min_spare_servers = parse_count(key, val, 0)?,
            "max_spare_servers" => tuning.max_spare_servers = parse_count(key, val, 0)?,
            "max_requests" => tuning.max_requests = parse_count(key, val, 0)?,
            other => return Err(format!("unknown FPM setting `{other}`")),
        }
    }
    // Keep the spare-server window coherent so FPM doesn't reject the pool.
    if tuning.pm == "dynamic" && tuning.min_spare_servers > tuning.max_spare_servers {
        return Err(format!(
            "min spare servers ({}) can't exceed max spare servers ({})",
            tuning.min_spare_servers, tuning.max_spare_servers
        ));
    }
    Ok(())
}

/// Validate + apply php.ini override patches. A blank value clears the
/// override (so the system default applies). Keys outside [`INI_FIELDS`] are
/// rejected, and values are checked for the characters that would let a patch
/// inject extra pool directives.
fn apply_ini(
    ini: &mut BTreeMap<String, String>,
    patches: &BTreeMap<String, String>,
) -> Result<(), String> {
    for (key, raw) in patches {
        if !INI_FIELDS.iter().any(|(k, _)| k == key) {
            return Err(format!("unknown php.ini setting `{key}`"));
        }
        let val = raw.trim();
        if val.is_empty() {
            ini.remove(key);
            continue;
        }
        if val.contains(['\n', '\r', '[', ']']) {
            return Err(format!("`{key}` contains an illegal character"));
        }
        ini.insert(key.clone(), val.to_string());
    }
    Ok(())
}

fn parse_count(key: &str, val: &str, min: u32) -> Result<u32, String> {
    let n: u32 = val
        .parse()
        .map_err(|_| format!("`{key}` must be a whole number (got `{val}`)"))?;
    if n < min {
        return Err(format!("`{key}` must be at least {min}"));
    }
    Ok(n)
}

fn source_from(s: PhpSource) -> InstallSource {
    match s {
        PhpSource::Homebrew => InstallSource::Homebrew,
        PhpSource::System => InstallSource::System,
    }
}

fn fpm_tab(p: &PhpInstall, t: &FpmTuning) -> ConfigTab {
    let dynamic = t.pm == "dynamic";
    let mut rows = vec![
        KvRow::select(
            "pm",
            "Process manager",
            t.pm.clone(),
            vec!["dynamic".into(), "static".into(), "ondemand".into()],
        )
        .with_hint("dynamic keeps spare workers ready; static fixes the pool size; ondemand spawns on demand."),
        KvRow::number("max_children", "Max children", t.max_children, Some(1), Some(512)),
    ];
    // The spare-server window only applies to the dynamic manager — surface it
    // as read-only context under static/ondemand rather than letting the user
    // edit values FPM would ignore.
    if dynamic {
        rows.push(KvRow::number(
            "start_servers",
            "Start servers",
            t.start_servers,
            Some(0),
            Some(512),
        ));
        rows.push(KvRow::number(
            "min_spare_servers",
            "Min spare servers",
            t.min_spare_servers,
            Some(0),
            Some(512),
        ));
        rows.push(KvRow::number(
            "max_spare_servers",
            "Max spare servers",
            t.max_spare_servers,
            Some(0),
            Some(512),
        ));
    }
    rows.push(
        KvRow::number(
            "max_requests",
            "Max requests",
            t.max_requests,
            Some(0),
            Some(100_000),
        )
        .with_hint("Requests a worker handles before respawning (0 = never)."),
    );

    // Read-only context: where FPM lives and how PortBay wires it.
    rows.push(
        KvRow::info(
            "FPM binary",
            p.php_fpm_bin
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Not installed".into()),
        )
        .with_hint(if p.php_fpm_bin.is_none() {
            "Without php-fpm, PortBay can't serve PHP sites with this version."
        } else {
            "PortBay launches FPM on a per-version unix socket; Caddy fastcgi-proxies to it."
        }),
    );

    ConfigTab::editable("fpm", "FPM", rows)
}

fn php_tab(p: &PhpInstall, ini: &BTreeMap<String, String>) -> ConfigTab {
    let mut rows: Vec<KvRow> = INI_FIELDS
        .iter()
        .map(|(key, label)| {
            let value = ini.get(*key).cloned().unwrap_or_default();
            KvRow::text(*key, *label, value)
        })
        .collect();
    rows.push(
        KvRow::path(
            "Loaded php.ini",
            p.php_ini
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "(none)".into()),
        )
        .with_hint(
            "Overrides above apply per-pool via php_admin_value — this system ini is never edited.",
        ),
    );
    rows.push(KvRow::path(
        "Extension dir",
        p.extension_dir
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "(none)".into()),
    ));
    ConfigTab::editable("php", "PHP", rows)
}

fn extensions_tab(p: &PhpInstall) -> ConfigTab {
    let rows = p
        .loaded_extensions
        .iter()
        .map(|name| KvRow::info(name.clone(), "Loaded"))
        .collect();
    ConfigTab::readonly(
        "extensions",
        format!("Extensions ({})", p.loaded_extensions.len()),
        rows,
    )
}

/// Fallback when re-probing fails (rare — the binary moved between
/// the initial detect and the tab open). Keeps the surface alive
/// with the data we already have.
fn placeholder_install(install: &RuntimeInstall) -> PhpInstall {
    PhpInstall {
        version: install.version.clone(),
        php_bin: install.binary.clone(),
        php_fpm_bin: None,
        php_ini: None,
        additional_ini_dir: install.config_dir.clone(),
        extension_dir: None,
        loaded_extensions: Vec::new(),
        source: PhpSource::System,
    }
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
    fn fpm_patch_updates_tuning_and_requests_restart() {
        let mut settings = RuntimeSettings::default();
        let res = PhpRuntime
            .apply_config(
                "8.3",
                "fpm",
                &patch(&[("pm", "static"), ("max_children", "16")]),
                &mut settings,
            )
            .unwrap();
        let saved = &settings.php["8.3"].fpm;
        assert_eq!(saved.pm, "static");
        assert_eq!(saved.max_children, 16);
        assert_eq!(res.restart_processes, vec!["php-fpm-8-3".to_string()]);
    }

    #[test]
    fn fpm_patch_rejects_invalid_pm_and_nonnumeric_counts() {
        let mut settings = RuntimeSettings::default();
        assert!(PhpRuntime
            .apply_config("8.3", "fpm", &patch(&[("pm", "turbo")]), &mut settings)
            .is_err());
        assert!(PhpRuntime
            .apply_config(
                "8.3",
                "fpm",
                &patch(&[("max_children", "lots")]),
                &mut settings
            )
            .is_err());
        // zero children is nonsensical for a pool.
        assert!(PhpRuntime
            .apply_config(
                "8.3",
                "fpm",
                &patch(&[("max_children", "0")]),
                &mut settings
            )
            .is_err());
    }

    #[test]
    fn fpm_patch_rejects_inverted_spare_window() {
        let mut settings = RuntimeSettings::default();
        let err = PhpRuntime
            .apply_config(
                "8.3",
                "fpm",
                &patch(&[("min_spare_servers", "9"), ("max_spare_servers", "3")]),
                &mut settings,
            )
            .unwrap_err();
        assert!(err.contains("spare"));
    }

    #[test]
    fn ini_patch_sets_and_clears_overrides() {
        let mut settings = RuntimeSettings::default();
        PhpRuntime
            .apply_config(
                "8.3",
                "php",
                &patch(&[("memory_limit", "256M")]),
                &mut settings,
            )
            .unwrap();
        assert_eq!(settings.php["8.3"].ini["memory_limit"], "256M");
        // A blank value clears the override.
        PhpRuntime
            .apply_config("8.3", "php", &patch(&[("memory_limit", "")]), &mut settings)
            .unwrap();
        assert!(!settings.php["8.3"].ini.contains_key("memory_limit"));
    }

    #[test]
    fn ini_patch_rejects_unknown_key_and_injection() {
        let mut settings = RuntimeSettings::default();
        assert!(PhpRuntime
            .apply_config("8.3", "php", &patch(&[("evil", "1")]), &mut settings)
            .is_err());
        assert!(PhpRuntime
            .apply_config(
                "8.3",
                "php",
                &patch(&[("memory_limit", "256M]\nphp_admin_value[disable_functions")]),
                &mut settings,
            )
            .is_err());
    }

    #[test]
    fn unknown_tab_is_rejected() {
        let mut settings = RuntimeSettings::default();
        assert!(PhpRuntime
            .apply_config("8.3", "extensions", &patch(&[("x", "y")]), &mut settings)
            .is_err());
    }
}
