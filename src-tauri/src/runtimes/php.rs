//! PHP runtime — adapter over the existing `crate::php` detection.
//!
//! Keeps the PHP-specific logic in `src/php/` (which the reconciler
//! already depends on for FPM lifecycle) and re-shapes its output
//! into the generic `RuntimeInstall` + `ConfigTab` surface the
//! `/languages` panel renders.

use crate::php::{self, PhpInstall, PhpSource};
use crate::runtimes::{
    ConfigTab, InstallSource, KvRow, LanguageRuntime, RuntimeInstall,
};

pub struct PhpRuntime;

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

    fn tabs(&self, install: &RuntimeInstall) -> Vec<ConfigTab> {
        // Re-probe via the same helper the reconciler uses so the
        // detail tabs reflect the exact same view of the install.
        // Cheap (php --ini + php -m); only runs on panel-open.
        let php = php::detect_all()
            .into_iter()
            .find(|p| p.version == install.version)
            .unwrap_or_else(|| placeholder_install(install));

        vec![
            fpm_tab(&php),
            php_tab(&php),
            extensions_tab(&php),
        ]
    }
}

fn source_from(s: PhpSource) -> InstallSource {
    match s {
        PhpSource::Homebrew => InstallSource::Homebrew,
        PhpSource::System => InstallSource::System,
    }
}

fn fpm_tab(p: &PhpInstall) -> ConfigTab {
    let rows = vec![
        KvRow {
            label: "FPM binary".into(),
            value: p
                .php_fpm_bin
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Not installed".into()),
            hint: if p.php_fpm_bin.is_none() {
                Some("Without php-fpm, PortBay can't serve PHP sites with this version.".into())
            } else {
                None
            },
            is_path: p.php_fpm_bin.is_some(),
        },
        KvRow {
            label: "Listen mode".into(),
            value: "Unix socket".into(),
            hint: Some(
                "PortBay launches FPM on a per-version unix socket and Caddy fastcgi_proxies to it.".into(),
            ),
            is_path: false,
        },
        KvRow {
            label: "Process manager".into(),
            value: "dynamic".into(),
            hint: Some(
                "ServBay-compatible default; tunable per-version in a follow-up.".into(),
            ),
            is_path: false,
        },
    ];
    ConfigTab {
        id: "fpm".into(),
        label: "FPM".into(),
        rows,
    }
}

fn php_tab(p: &PhpInstall) -> ConfigTab {
    let rows = vec![
        KvRow {
            label: "Version".into(),
            value: p.version.clone(),
            hint: None,
            is_path: false,
        },
        KvRow {
            label: "PHP binary".into(),
            value: p.php_bin.to_string_lossy().into_owned(),
            hint: None,
            is_path: true,
        },
        KvRow {
            label: "Loaded php.ini".into(),
            value: p
                .php_ini
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "(none)".into()),
            hint: None,
            is_path: p.php_ini.is_some(),
        },
        KvRow {
            label: "Additional .ini dir".into(),
            value: p
                .additional_ini_dir
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "(none)".into()),
            hint: Some(
                "Drop additional .ini files here to layer settings without editing the main php.ini."
                    .into(),
            ),
            is_path: p.additional_ini_dir.is_some(),
        },
        KvRow {
            label: "Extension dir".into(),
            value: p
                .extension_dir
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "(none)".into()),
            hint: None,
            is_path: p.extension_dir.is_some(),
        },
    ];
    ConfigTab {
        id: "php".into(),
        label: "PHP".into(),
        rows,
    }
}

fn extensions_tab(p: &PhpInstall) -> ConfigTab {
    let rows = p
        .loaded_extensions
        .iter()
        .map(|name| KvRow {
            label: name.clone(),
            value: "Loaded".into(),
            hint: None,
            is_path: false,
        })
        .collect();
    ConfigTab {
        id: "extensions".into(),
        label: format!("Extensions ({})", p.loaded_extensions.len()),
        rows,
    }
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
