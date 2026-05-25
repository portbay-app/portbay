//! PHP-FPM lifecycle helpers — generate one PC process entry per PHP
//! version that any registered project actually uses.
//!
//! We don't run FPM continuously for every installed version — only
//! for the versions the registry currently has projects for. The
//! reconcile loop calls [`fpm_process_id`] to derive the process id
//! that gets written into the generated process-compose YAML.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::php::PhpInstall;
use crate::registry::FpmTuning;

/// Stable process-compose id for a given PHP version's FPM pool.
/// Used by both the YAML generator and (eventually) Caddy's reverse-
/// proxy upstream resolver.
pub fn fpm_process_id(version: &str) -> String {
    format!("php-fpm-{}", version.replace('.', "-"))
}

/// Per-version FPM pool config file. Lives under the PortBay data dir
/// so PortBay owns it (the Homebrew default `/usr/local/etc/php@8.3/
/// php-fpm.d/www.conf` is shared with other tools and is fragile to
/// touch). The reconciler writes one of these per used version.
pub fn fpm_pool_path(data_dir: &std::path::Path, version: &str) -> std::path::PathBuf {
    data_dir.join("php").join(version).join("php-fpm.conf")
}

/// Default UNIX-socket path FPM listens on for a given version.
/// Caddy uses this as its `fastcgi` upstream.
pub fn fpm_socket_path(data_dir: &std::path::Path, version: &str) -> std::path::PathBuf {
    data_dir.join("php").join(version).join("php-fpm.sock")
}

/// Default slow-log file for a PHP version's PortBay-owned pool.
pub fn fpm_slowlog_path(data_dir: &std::path::Path, version: &str) -> std::path::PathBuf {
    data_dir.join("php").join(version).join("php-fpm.slow.log")
}

/// Access-log file for a PHP version's PortBay-owned pool.
pub fn fpm_access_log_path(data_dir: &std::path::Path, version: &str) -> std::path::PathBuf {
    data_dir
        .join("php")
        .join(version)
        .join("php-fpm.access.log")
}

/// FPM listen address generated from the saved per-version tuning.
pub fn fpm_listen_address(tuning: &FpmTuning, socket_path: &std::path::Path) -> String {
    if tuning.listen == "tcp" {
        format!("127.0.0.1:{}", tuning.tcp_port)
    } else {
        socket_path.to_string_lossy().into_owned()
    }
}

/// Caddy's FastCGI upstream dial string for the same saved tuning.
pub fn fpm_fastcgi_dial(tuning: &FpmTuning, socket_path: &std::path::Path) -> String {
    if tuning.listen == "tcp" {
        format!("127.0.0.1:{}", tuning.tcp_port)
    } else {
        format!("unix/{}", socket_path.to_string_lossy())
    }
}

/// Render the FPM pool config for a version. One `[www]` pool listening on
/// the socket path, with process-manager tuning from `tuning` and any php.ini
/// overrides from `ini` layered in as `php_admin_value` directives.
///
/// The `pm.*` lines are gated by the manager mode: FPM rejects
/// `pm.start_servers` / `pm.*_spare_servers` under `static`/`ondemand`, so we
/// only emit the directives the selected mode actually accepts. `ini`
/// overrides apply per-pool — the system php.ini is never edited. The user can
/// still drop additional `.ini` files in the version's `extension_dir` for
/// settings PortBay doesn't surface.
pub fn render_pool_config(
    install: &PhpInstall,
    socket_path: &std::path::Path,
    tuning: &FpmTuning,
    ini: &BTreeMap<String, String>,
) -> String {
    let listen = fpm_listen_address(tuning, socket_path);
    let pool_dir = socket_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/tmp"));
    let slowlog_path = if tuning.slowlog.trim().is_empty() {
        pool_dir.join("php-fpm.slow.log")
    } else {
        std::path::PathBuf::from(tuning.slowlog.trim())
    };
    let access_log_path = pool_dir.join("php-fpm.access.log");
    let mut out = format!(
        "; PortBay-managed FPM pool for PHP {ver}\n\
         [global]\n\
         daemonize = no\n\
         error_log = /tmp/portbay-php-fpm-{ver_safe}.log\n\
         \n\
         [www]\n\
         user = $USER\n\
         group = staff\n\
         listen = {listen}\n\
         pm = {pm}\n\
         pm.max_children = {max_children}\n",
        ver = install.version,
        ver_safe = install.version.replace('.', "-"),
        listen = listen,
        pm = pm_mode(&tuning.pm),
        max_children = tuning.max_children.max(1),
    );
    if tuning.listen != "tcp" {
        out.push_str(
            "listen.owner = $USER\n\
             listen.group = staff\n\
             listen.mode = 0660\n",
        );
    }

    // start/spare servers are dynamic-only; ondemand uses an idle timeout.
    match pm_mode(&tuning.pm) {
        "dynamic" => {
            let _ = write!(
                out,
                "pm.start_servers = {start}\n\
                 pm.min_spare_servers = {min}\n\
                 pm.max_spare_servers = {max}\n",
                start = tuning.start_servers,
                min = tuning.min_spare_servers,
                max = tuning.max_spare_servers,
            );
        }
        "ondemand" => {
            out.push_str("pm.process_idle_timeout = 10s\n");
        }
        _ => {} // static: max_children only.
    }

    let _ = writeln!(out, "pm.max_requests = {}", tuning.max_requests);
    out.push_str("clear_env = no\n");
    let slow_timeout = tuning.request_slowlog_timeout.trim();
    if !slow_timeout.is_empty() && slow_timeout != "0" && slow_timeout != "0s" {
        let _ = writeln!(out, "request_slowlog_timeout = {slow_timeout}");
        let _ = writeln!(out, "slowlog = {}", slowlog_path.display());
    }
    let _ = writeln!(
        out,
        "catch_workers_output = {}",
        if tuning.catch_workers_output {
            "yes"
        } else {
            "no"
        }
    );
    let _ = writeln!(
        out,
        "decorate_workers_output = {}",
        if tuning.decorate_workers_output {
            "yes"
        } else {
            "no"
        }
    );
    if tuning.access_log {
        let _ = writeln!(out, "access.log = {}", access_log_path.display());
    }

    // php.ini overrides, applied per-pool. `php_admin_value` is read by FPM
    // and not overridable from userland, which is what we want for a managed
    // dev environment. Keys are sorted (BTreeMap) for a stable, diff-friendly
    // file the reconciler can rewrite idempotently.
    for (key, value) in ini {
        if key.trim().is_empty() {
            continue;
        }
        let _ = writeln!(out, "php_admin_value[{key}] = {value}");
    }
    if !tuning.raw_params.trim().is_empty() {
        out.push_str("\n; User-managed raw FPM directives\n");
        out.push_str(tuning.raw_params.trim_end());
        out.push('\n');
    }

    out
}

/// Normalise an FPM process-manager mode to a value FPM accepts, falling back
/// to `dynamic` for anything unrecognised (defends against a hand-edited
/// registry).
fn pm_mode(pm: &str) -> &'static str {
    match pm {
        "static" => "static",
        "ondemand" => "ondemand",
        _ => "dynamic",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn process_id_is_stable_and_safe() {
        assert_eq!(fpm_process_id("8.3"), "php-fpm-8-3");
        assert_eq!(fpm_process_id("7.4"), "php-fpm-7-4");
    }

    #[test]
    fn pool_path_lives_under_data_dir() {
        let p = fpm_pool_path(Path::new("/tmp/data"), "8.3");
        assert_eq!(p, Path::new("/tmp/data/php/8.3/php-fpm.conf"));
    }

    #[test]
    fn socket_path_lives_alongside_pool() {
        let s = fpm_socket_path(Path::new("/tmp/data"), "8.3");
        assert_eq!(s, Path::new("/tmp/data/php/8.3/php-fpm.sock"));
    }

    fn sample_install() -> PhpInstall {
        PhpInstall {
            version: "8.3".into(),
            php_bin: "/opt/homebrew/opt/php@8.3/bin/php".into(),
            php_fpm_bin: Some("/opt/homebrew/opt/php@8.3/sbin/php-fpm".into()),
            php_ini: None,
            additional_ini_dir: None,
            extension_dir: None,
            loaded_extensions: vec![],
            source: crate::php::PhpSource::Homebrew,
        }
    }

    #[test]
    fn render_pool_config_includes_version_and_socket() {
        let cfg = render_pool_config(
            &sample_install(),
            Path::new("/tmp/data/php/8.3/php-fpm.sock"),
            &FpmTuning::default(),
            &BTreeMap::new(),
        );
        assert!(cfg.contains("PHP 8.3"));
        assert!(cfg.contains("/tmp/data/php/8.3/php-fpm.sock"));
        assert!(cfg.contains("[www]"));
    }

    #[test]
    fn default_tuning_renders_the_historical_dynamic_pool() {
        let cfg = render_pool_config(
            &sample_install(),
            Path::new("/tmp/s.sock"),
            &FpmTuning::default(),
            &BTreeMap::new(),
        );
        assert!(cfg.contains("pm = dynamic"));
        assert!(cfg.contains("pm.max_children = 8"));
        assert!(cfg.contains("pm.start_servers = 2"));
        assert!(cfg.contains("pm.min_spare_servers = 1"));
        assert!(cfg.contains("pm.max_spare_servers = 3"));
        assert!(cfg.contains("pm.max_requests = 500"));
    }

    #[test]
    fn static_mode_omits_spare_server_directives() {
        let tuning = FpmTuning {
            pm: "static".into(),
            max_children: 4,
            ..FpmTuning::default()
        };
        let cfg = render_pool_config(
            &sample_install(),
            Path::new("/tmp/s.sock"),
            &tuning,
            &BTreeMap::new(),
        );
        assert!(cfg.contains("pm = static"));
        assert!(cfg.contains("pm.max_children = 4"));
        // FPM rejects these under static — they must not be emitted.
        assert!(!cfg.contains("pm.start_servers"));
        assert!(!cfg.contains("pm.min_spare_servers"));
        assert!(!cfg.contains("pm.max_spare_servers"));
        // max_requests stays valid for every mode.
        assert!(cfg.contains("pm.max_requests"));
    }

    #[test]
    fn ondemand_mode_uses_idle_timeout_not_spare_servers() {
        let tuning = FpmTuning {
            pm: "ondemand".into(),
            ..FpmTuning::default()
        };
        let cfg = render_pool_config(
            &sample_install(),
            Path::new("/tmp/s.sock"),
            &tuning,
            &BTreeMap::new(),
        );
        assert!(cfg.contains("pm = ondemand"));
        assert!(cfg.contains("pm.process_idle_timeout"));
        assert!(!cfg.contains("pm.start_servers"));
    }

    #[test]
    fn ini_overrides_render_as_sorted_php_admin_values() {
        let mut ini = BTreeMap::new();
        ini.insert("upload_max_filesize".to_string(), "64M".to_string());
        ini.insert("memory_limit".to_string(), "256M".to_string());
        ini.insert("".to_string(), "ignored".to_string()); // blank key skipped
        let cfg = render_pool_config(
            &sample_install(),
            Path::new("/tmp/s.sock"),
            &FpmTuning::default(),
            &ini,
        );
        assert!(cfg.contains("php_admin_value[memory_limit] = 256M"));
        assert!(cfg.contains("php_admin_value[upload_max_filesize] = 64M"));
        assert!(!cfg.contains("php_admin_value[] ="));
        // BTreeMap iteration is sorted: memory_limit precedes upload_max_filesize.
        let mem = cfg.find("memory_limit").unwrap();
        let upload = cfg.find("upload_max_filesize").unwrap();
        assert!(mem < upload);
    }

    #[test]
    fn unknown_pm_mode_falls_back_to_dynamic() {
        let tuning = FpmTuning {
            pm: "bogus".into(),
            ..FpmTuning::default()
        };
        let cfg = render_pool_config(
            &sample_install(),
            Path::new("/tmp/s.sock"),
            &tuning,
            &BTreeMap::new(),
        );
        assert!(cfg.contains("pm = dynamic"));
    }
}
