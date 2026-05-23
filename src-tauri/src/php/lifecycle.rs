//! PHP-FPM lifecycle helpers — generate one PC process entry per PHP
//! version that any registered project actually uses.
//!
//! We don't run FPM continuously for every installed version — only
//! for the versions the registry currently has projects for. The
//! reconcile loop calls [`fpm_process_id`] to derive the process id
//! that gets written into the generated process-compose YAML.

use crate::php::PhpInstall;

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

/// Render the minimum-viable FPM pool config for a version. One
/// `[www]` pool listening on the socket path. The user can drop
/// additional `.ini` files in the version's `extension_dir` to tune
/// extensions (Xdebug, OPcache, etc.) without touching this file.
pub fn render_pool_config(install: &PhpInstall, socket_path: &std::path::Path) -> String {
    format!(
        "; PortBay-managed FPM pool for PHP {ver}\n\
         [global]\n\
         daemonize = no\n\
         error_log = /tmp/portbay-php-fpm-{ver_safe}.log\n\
         \n\
         [www]\n\
         user = $USER\n\
         group = staff\n\
         listen = {sock}\n\
         listen.owner = $USER\n\
         listen.group = staff\n\
         listen.mode = 0660\n\
         pm = dynamic\n\
         pm.max_children = 8\n\
         pm.start_servers = 2\n\
         pm.min_spare_servers = 1\n\
         pm.max_spare_servers = 3\n\
         pm.max_requests = 500\n\
         clear_env = no\n",
        ver = install.version,
        ver_safe = install.version.replace('.', "-"),
        sock = socket_path.display(),
    )
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

    #[test]
    fn render_pool_config_includes_version_and_socket() {
        let install = PhpInstall {
            version: "8.3".into(),
            php_bin: "/opt/homebrew/opt/php@8.3/bin/php".into(),
            php_fpm_bin: Some("/opt/homebrew/opt/php@8.3/sbin/php-fpm".into()),
            php_ini: None,
            additional_ini_dir: None,
            extension_dir: None,
            loaded_extensions: vec![],
            source: crate::php::PhpSource::Homebrew,
        };
        let cfg = render_pool_config(&install, Path::new("/tmp/data/php/8.3/php-fpm.sock"));
        assert!(cfg.contains("PHP 8.3"));
        assert!(cfg.contains("/tmp/data/php/8.3/php-fpm.sock"));
        assert!(cfg.contains("[www]"));
    }
}
