//! Generated PHP web-server configs.
//!
//! Caddy remains PortBay's public edge (host routing, HTTPS, placeholder
//! errors). When a PHP project selects Nginx or Apache, PortBay supervises a
//! private loopback web server and has Caddy reverse-proxy to it.

use std::path::{Path, PathBuf};

use crate::process_compose::config::WebServerSpec;
use crate::registry::{FpmTuning, Project, ProjectType, Registry, WebServer};

pub fn specs_for(reg: &Registry, app_data: &Path, logs_dir: &Path) -> Vec<WebServerSpec> {
    let mut specs = Vec::new();
    for project in reg.list_projects() {
        if project.kind != ProjectType::Php
            || project.start_command.is_some()
            || matches!(project.web_server_effective(), WebServer::Caddy)
        {
            continue;
        }

        let Some(port) = project.port else {
            tracing::warn!(
                target: "reconciler",
                "PHP project `{}` selected {} but has no loopback port; web-server entry skipped.",
                project.id,
                project.web_server_effective().id()
            );
            continue;
        };
        let Some(version) = project.php_version_effective() else {
            tracing::warn!(
                target: "reconciler",
                "PHP project `{}` selected {} but has no PHP version; web-server entry skipped.",
                project.id,
                project.web_server_effective().id()
            );
            continue;
        };

        let server = project.web_server_effective();
        let conf_dir = app_data
            .join("webservers")
            .join(server.id())
            .join(project.id.as_str());
        if let Err(e) = std::fs::create_dir_all(&conf_dir) {
            tracing::warn!(
                target: "reconciler",
                "couldn't create web-server config dir {}: {e}",
                conf_dir.display()
            );
            continue;
        }

        let socket_path = crate::php::lifecycle::fpm_socket_path(app_data, version);
        // Same FPM tuning the Caddy reverse-proxy and the FPM pool reconciler
        // use, so the web server dials PHP-FPM exactly where it's listening
        // (unix socket vs. TCP) instead of always assuming a socket.
        let tuning = reg
            .runtimes
            .php
            .get(version)
            .map(|cfg| cfg.fpm.clone())
            .unwrap_or_default();
        let log_dir = logs_dir.join("webservers").join(project.id.as_str());
        if let Err(e) = std::fs::create_dir_all(&log_dir) {
            tracing::warn!(
                target: "reconciler",
                "couldn't create web-server log dir {}: {e}",
                log_dir.display()
            );
            continue;
        }

        match server {
            WebServer::Caddy => {}
            WebServer::Nginx => {
                let Some(bin) = managed_web_server_binary(reg, "nginx").or_else(nginx_binary)
                else {
                    warn_missing(project, "nginx");
                    continue;
                };
                let conf_path = conf_dir.join("nginx.conf");
                let body = render_nginx_config(project, port, &socket_path, &tuning, &log_dir);
                if let Err(e) = std::fs::write(&conf_path, body) {
                    tracing::warn!(
                        target: "reconciler",
                        "couldn't write {}: {e}",
                        conf_path.display()
                    );
                    continue;
                }
                specs.push(WebServerSpec {
                    process_id: format!("web-nginx-{}", project.id),
                    description: format!("Nginx - {}", project.name),
                    command: format!(
                        "{bin} -p {prefix} -c {conf} -g 'daemon off;'",
                        bin = shell_quote(&bin.to_string_lossy()),
                        prefix = shell_quote(&conf_dir.to_string_lossy()),
                        conf = shell_quote(&conf_path.to_string_lossy()),
                    ),
                    working_dir: conf_dir,
                    port,
                    auto_start: project.auto_start,
                });
            }
            WebServer::Apache => {
                let Some(bin) = managed_web_server_binary(reg, "apache").or_else(apache_binary)
                else {
                    warn_missing(project, "httpd");
                    continue;
                };
                let conf_path = conf_dir.join("httpd.conf");
                let body = render_apache_config(
                    project,
                    port,
                    &socket_path,
                    &tuning,
                    &conf_dir,
                    &log_dir,
                    &bin,
                );
                if let Err(e) = std::fs::write(&conf_path, body) {
                    tracing::warn!(
                        target: "reconciler",
                        "couldn't write {}: {e}",
                        conf_path.display()
                    );
                    continue;
                }
                specs.push(WebServerSpec {
                    process_id: format!("web-apache-{}", project.id),
                    description: format!("Apache - {}", project.name),
                    command: format!(
                        "{bin} -f {conf} -DFOREGROUND",
                        bin = shell_quote(&bin.to_string_lossy()),
                        conf = shell_quote(&conf_path.to_string_lossy()),
                    ),
                    working_dir: conf_dir,
                    port,
                    auto_start: project.auto_start,
                });
            }
        }
    }
    specs.sort_by(|a, b| a.process_id.cmp(&b.process_id));
    specs
}

// Discovery is restricted to neutral locations — Homebrew, the macOS system
// binary, and the user's PATH. PortBay never runs a *competitor* dev-env app's
// nginx/httpd (ServBay/Herd/MAMP/XAMPP/FlyEnv): borrowing their binaries couples
// us to their layout and is the wrong thing for a tool not associated with them.
// `is_competitor_managed` also canonicalises, so a competitor binary symlinked
// onto PATH (e.g. `/usr/local/bin/php` → XAMPP) is still rejected.
/// A PortBay-managed nginx/apache build for the current arch, when one has been
/// downloaded. Preferred over neutral host discovery so PortBay runs **its own**
/// web server (the same managed-runtime model as PHP-FPM); falls through to
/// Homebrew/system when absent. `lang` is `"nginx"` or `"apache"`.
fn managed_web_server_binary(reg: &Registry, lang: &str) -> Option<PathBuf> {
    let arch = crate::runtimes::download::manifest::current_arch();
    reg.runtimes
        .managed
        .iter()
        .find(|m| m.lang == lang && m.arch == arch)
        .map(|m| m.binary.clone())
        .filter(|p| p.exists())
}

pub fn nginx_binary() -> Option<PathBuf> {
    first_existing(&[
        "/opt/homebrew/opt/nginx/bin/nginx",
        "/usr/local/opt/nginx/bin/nginx",
        "/opt/homebrew/sbin/nginx",
        "/opt/homebrew/bin/nginx",
        "/usr/local/sbin/nginx",
        "/usr/local/bin/nginx",
    ])
    .or_else(|| which::which("nginx").ok())
    .filter(|p| !crate::runtimes::env::is_competitor_managed(p))
}

pub fn apache_binary() -> Option<PathBuf> {
    first_existing(&[
        "/opt/homebrew/opt/httpd/bin/httpd",
        "/usr/local/opt/httpd/bin/httpd",
        "/opt/homebrew/bin/httpd",
        "/usr/local/bin/httpd",
        "/usr/sbin/httpd",
    ])
    .or_else(|| which::which("httpd").ok())
    .filter(|p| !crate::runtimes::env::is_competitor_managed(p))
}

fn first_existing(paths: &[&str]) -> Option<PathBuf> {
    paths.iter().map(PathBuf::from).find(|p| p.exists())
}

fn warn_missing(project: &Project, binary: &str) {
    tracing::warn!(
        target: "reconciler",
        "PHP project `{}` selected {} but `{binary}` was not found; web-server entry skipped.",
        project.id,
        project.web_server_effective().id(),
    );
}

/// A user-facing reason a PHP project's selected web server can't serve, derived
/// from the project plus current binary availability — `None` when it's fine
/// (Caddy, a dev-command project, a non-PHP project, or the chosen server is
/// installed). Surfaced on `ProjectView` so the dashboard explains *why* a
/// project only renders PortBay's placeholder: a missing nginx/apache is logged
/// (`warn_missing`) and its supervised entry skipped, but Caddy still
/// reverse-proxies to a dead loopback port — without this the user sees the
/// placeholder with no clue what to fix.
///
/// Recomputed wherever the DTO is built, so it tracks the live state: install
/// the binary (or switch to Caddy) and the next fetch clears it.
pub fn web_server_issue(project: &Project) -> Option<String> {
    // Only PHP projects that delegate to a managed nginx/apache (i.e. no dev
    // command of their own) can hit the missing-binary skip in `specs_for`.
    if project.kind != ProjectType::Php || project.start_command.is_some() {
        return None;
    }
    match project.web_server_effective() {
        WebServer::Caddy => None,
        WebServer::Nginx => nginx_binary()
            .is_none()
            .then(|| missing_binary_msg("nginx", "nginx")),
        WebServer::Apache => apache_binary()
            .is_none()
            .then(|| missing_binary_msg("Apache", "httpd")),
    }
}

/// Actionable message for an absent web-server binary: what broke + the two ways
/// out (install the neutral binary, or switch the project to Caddy).
fn missing_binary_msg(server_label: &str, brew_formula: &str) -> String {
    format!(
        "{server_label} isn't installed, so this project falls back to PortBay's \
         placeholder instead of serving PHP. Install it with `brew install \
         {brew_formula}`, or switch this project's web server to Caddy in its \
         detail panel."
    )
}

fn doc_root(project: &Project) -> PathBuf {
    project
        .document_root
        .as_deref()
        .map(|d| project.path.join(d))
        .unwrap_or_else(|| project.path.clone())
}

fn render_nginx_config(
    project: &Project,
    port: u16,
    socket_path: &Path,
    tuning: &FpmTuning,
    log_dir: &Path,
) -> String {
    let root = doc_root(project);
    let access_log = log_dir.join("nginx-access.log");
    let error_log = log_dir.join("nginx-error.log");
    // Dial PHP-FPM where it actually listens. The whole `unix:<path>` token is
    // double-quoted so a socket path containing a space (every macOS install —
    // "Application Support") parses; nginx rejects `unix:"<path>"` and any
    // unquoted path with a space, so quoting the entire token is the only form
    // that works. All file paths below are quoted for the same reason.
    let fastcgi_pass = if tuning.listen == "tcp" {
        format!("fastcgi_pass 127.0.0.1:{};", tuning.tcp_port)
    } else {
        format!("fastcgi_pass \"unix:{}\";", socket_path.display())
    };
    format!(
        r#"worker_processes 1;
error_log "{error_log}" warn;
pid nginx.pid;

events {{
    worker_connections 256;
}}

http {{
    default_type application/octet-stream;
    access_log "{access_log}";
    sendfile on;

    server {{
        listen 127.0.0.1:{port};
        server_name {host};
        root "{root}";
        index index.php index.html index.htm;

        location / {{
            try_files $uri $uri/ /router.php /index.php?$query_string;
        }}

        location ~ \.php(?:/|$) {{
            {fastcgi_pass}
            fastcgi_index index.php;
            fastcgi_split_path_info ^(.+\.php)(/.+)$;
            fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;
            fastcgi_param PATH_INFO $fastcgi_path_info;
            fastcgi_param QUERY_STRING $query_string;
            fastcgi_param REQUEST_METHOD $request_method;
            fastcgi_param CONTENT_TYPE $content_type;
            fastcgi_param CONTENT_LENGTH $content_length;
            fastcgi_param REQUEST_URI $request_uri;
            fastcgi_param DOCUMENT_URI $document_uri;
            fastcgi_param DOCUMENT_ROOT $document_root;
            fastcgi_param SERVER_PROTOCOL $server_protocol;
            fastcgi_param REQUEST_SCHEME $scheme;
            fastcgi_param HTTPS $https if_not_empty;
            fastcgi_param GATEWAY_INTERFACE CGI/1.1;
            fastcgi_param SERVER_SOFTWARE nginx;
            fastcgi_param REMOTE_ADDR $remote_addr;
            fastcgi_param REMOTE_PORT $remote_port;
            fastcgi_param SERVER_ADDR $server_addr;
            fastcgi_param SERVER_PORT $server_port;
            fastcgi_param SERVER_NAME $server_name;
        }}
    }}
}}
"#,
        access_log = access_log.display(),
        error_log = error_log.display(),
        host = project.hostname,
        root = root.display(),
    )
}

fn render_apache_config(
    project: &Project,
    port: u16,
    socket_path: &Path,
    tuning: &FpmTuning,
    conf_dir: &Path,
    log_dir: &Path,
    httpd_bin: &Path,
) -> String {
    let root = doc_root(project);
    let access_log = log_dir.join("apache-access.log");
    let error_log = log_dir.join("apache-error.log");
    let modules = apache_module_dir(httpd_bin);
    let load_module = |name: &str, file: &str| -> String {
        modules
            .as_ref()
            .map(|dir| format!("LoadModule {name}_module \"{}/{}\"\n", dir.display(), file))
            .unwrap_or_default()
    };
    // FastCGI backend, matching where PHP-FPM actually listens. The whole value
    // is double-quoted by the SetHandler template, so a socket path with a space
    // is already safe here (unlike nginx's fastcgi_pass).
    let fcgi_backend = if tuning.listen == "tcp" {
        format!("proxy:fcgi://127.0.0.1:{}", tuning.tcp_port)
    } else {
        format!("proxy:unix:{}|fcgi://localhost/", socket_path.display())
    };

    // LoadModule lines come FIRST: Apache parses top-down, so a directive like
    // `CustomLog` (mod_log_config) or `SetHandler proxy:` (mod_proxy*) fails if
    // its module hasn't been loaded yet. `mod_log_config` was previously absent
    // entirely *and* the block sat below `CustomLog` — either alone is fatal.
    format!(
        r#"ServerRoot "{server_root}"
PidFile "{server_root}/httpd.pid"
Listen 127.0.0.1:{port}
ServerName {host}

{mpm}{log_config}{authz_core}{authz_host}{dir}{mime}{rewrite}{proxy}{proxy_fcgi}{unixd}
DocumentRoot "{root}"
ErrorLog "{error_log}"
CustomLog "{access_log}" common
TypesConfig /etc/apache2/mime.types
DirectoryIndex index.php index.html index.htm

<Directory "{root}">
    Options FollowSymLinks
    AllowOverride All
    Require all granted
</Directory>

<FilesMatch "\.php$">
    SetHandler "{fcgi_backend}"
</FilesMatch>

RewriteEngine On
RewriteCond "%{{REQUEST_FILENAME}}" !-f
RewriteCond "%{{REQUEST_FILENAME}}" !-d
RewriteRule "^" "/router.php" [L]
"#,
        server_root = conf_dir.display(),
        host = project.hostname,
        root = root.display(),
        error_log = error_log.display(),
        access_log = access_log.display(),
        mpm = load_module("mpm_event", "mod_mpm_event.so"),
        log_config = load_module("log_config", "mod_log_config.so"),
        authz_core = load_module("authz_core", "mod_authz_core.so"),
        authz_host = load_module("authz_host", "mod_authz_host.so"),
        dir = load_module("dir", "mod_dir.so"),
        mime = load_module("mime", "mod_mime.so"),
        rewrite = load_module("rewrite", "mod_rewrite.so"),
        proxy = load_module("proxy", "mod_proxy.so"),
        proxy_fcgi = load_module("proxy_fcgi", "mod_proxy_fcgi.so"),
        unixd = load_module("unixd", "mod_unixd.so"),
    )
}

/// Resolve the directory holding this httpd's `mod_*.so` files. Rather than map
/// hardcoded binary prefixes (which gets `/usr/sbin/httpd` → `/usr/libexec/apache2`
/// right but breaks for every non-standard layout, and a pure `httpd -V` HTTPD_ROOT
/// approach gets macOS *wrong* — it reports `/usr` while the modules live in
/// `/usr/libexec/apache2`), we collect candidate dirs from the binary's install
/// prefix, the binary's own reported `HTTPD_ROOT`, and known fallbacks, then pick
/// the first that actually contains `mod_mpm_event.so` (a module Apache can't run
/// without). Self-verifying, so a wrong guess never produces a config that fails
/// `No MPM loaded`.
fn apache_module_dir(httpd_bin: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(prefix) = httpd_bin.parent().and_then(Path::parent) {
        for sub in [
            "modules",
            "lib/httpd/modules",
            "libexec/apache2",
            "lib/apache2/modules",
        ] {
            candidates.push(prefix.join(sub));
        }
    }
    if let Some(root) = httpd_root(httpd_bin) {
        for sub in ["modules", "libexec/apache2", "lib/httpd/modules"] {
            candidates.push(root.join(sub));
        }
    }
    candidates.push(PathBuf::from("/usr/libexec/apache2"));
    candidates.push(PathBuf::from("/opt/homebrew/opt/httpd/lib/httpd/modules"));
    candidates.push(PathBuf::from("/usr/local/opt/httpd/lib/httpd/modules"));

    candidates
        .into_iter()
        .find(|dir| dir.join("mod_mpm_event.so").exists())
}

/// Parse `HTTPD_ROOT` out of `httpd -V`. Best-effort; `None` if the binary
/// can't be run or the line isn't present.
fn httpd_root(httpd_bin: &Path) -> Option<PathBuf> {
    let out = std::process::Command::new(httpd_bin)
        .arg("-V")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("-D HTTPD_ROOT=\"") {
            if let Some(val) = rest.strip_suffix('"') {
                return Some(PathBuf::from(val));
            }
        }
    }
    None
}

fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | ':'))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ProjectId;
    use std::path::PathBuf;

    /// A PHP project at a path with a space (the exact macOS hazard), serving
    /// from `public/`, on the given web server.
    fn php_project(web_server: WebServer) -> Project {
        Project {
            id: ProjectId::new("tribal-house-cms"),
            name: "Tribal House CMS".into(),
            path: PathBuf::from("/Volumes/DevSSD/projects/Clients/Tribal House/tribal-house-cms"),
            kind: ProjectType::Php,
            start_command: None,
            port: Some(8090),
            extra_ports: vec![],
            hostname: "tribal-house-cms.portbay.test".into(),
            https: false,
            services: vec!["caddy".into(), "php-fpm".into()],
            env: Default::default(),
            readiness: None,
            auto_start: false,
            tags: vec![],
            document_root: Some("public".into()),
            php_version: Some("8.3".into()),
            web_server: Some(web_server),
            mobile_run: None,
            runtime: None,
            workspace: None,
            cors: None,
            sandbox: None,
            domain: None,
            tunnel: None,
        }
    }

    /// Socket path carrying the unavoidable macOS "Application Support" space.
    fn spaced_socket() -> PathBuf {
        PathBuf::from("/Users/me/Library/Application Support/PortBay/php/8.3/php-fpm.sock")
    }

    #[test]
    fn managed_web_server_binary_is_preferred_over_host() {
        use crate::registry::{ManagedRuntime, Registry};
        // A PortBay-managed nginx must win over any host nginx: its binary path
        // shows up in the generated PC command regardless of what's on the box.
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("sbin").join("nginx");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, b"#!/bin/sh\n").unwrap();

        let mut reg = Registry::new("portbay.test");
        let mut p = php_project(WebServer::Nginx);
        p.start_command = None;
        reg.add_project(p).unwrap();
        reg.runtimes.managed.push(ManagedRuntime {
            lang: "nginx".into(),
            version: "1.27.0".into(),
            binary: bin.clone(),
            arch: crate::runtimes::download::manifest::current_arch().into(),
        });

        let data = tempfile::tempdir().unwrap();
        let specs = specs_for(&reg, data.path(), data.path());
        let nginx = specs
            .iter()
            .find(|s| s.process_id.starts_with("web-nginx-"))
            .expect("a managed nginx should produce a web-server spec");
        assert!(
            nginx.command.contains(&*bin.to_string_lossy()),
            "managed nginx binary should be used, got: {}",
            nginx.command
        );
    }

    #[test]
    fn nginx_config_quotes_every_path_and_the_socket() {
        let p = php_project(WebServer::Nginx);
        let cfg = render_nginx_config(
            &p,
            8090,
            &spaced_socket(),
            &FpmTuning::default(),
            Path::new("/tmp/logs"),
        );
        assert!(
            cfg.contains(
                "root \"/Volumes/DevSSD/projects/Clients/Tribal House/tribal-house-cms/public\";"
            ),
            "root must be quoted:\n{cfg}"
        );
        assert!(
            cfg.contains(
                "fastcgi_pass \"unix:/Users/me/Library/Application Support/PortBay/php/8.3/php-fpm.sock\";"
            ),
            "fastcgi_pass must wrap the whole unix: token in quotes:\n{cfg}"
        );
        assert!(cfg.contains("error_log \""), "error_log must be quoted");
        assert!(cfg.contains("access_log \""), "access_log must be quoted");
    }

    #[test]
    fn nginx_tcp_mode_dials_tcp_not_socket() {
        let p = php_project(WebServer::Nginx);
        let tuning = FpmTuning {
            listen: "tcp".into(),
            tcp_port: 9001,
            ..FpmTuning::default()
        };
        let cfg = render_nginx_config(&p, 8090, &spaced_socket(), &tuning, Path::new("/tmp/logs"));
        assert!(
            cfg.contains("fastcgi_pass 127.0.0.1:9001;"),
            "tcp dial expected:\n{cfg}"
        );
        assert!(!cfg.contains("unix:"), "must not dial a socket in tcp mode");
    }

    #[test]
    fn apache_loads_log_config_before_customlog() {
        let p = php_project(WebServer::Apache);
        let cfg = render_apache_config(
            &p,
            8090,
            &spaced_socket(),
            &FpmTuning::default(),
            Path::new("/tmp/conf"),
            Path::new("/tmp/logs"),
            Path::new("/usr/sbin/httpd"),
        );
        // The doc-root and socket are inside double-quoted directives, so spaces
        // are safe. The load-order invariant is the real fix here.
        if let (Some(m), Some(c)) = (cfg.find("mod_log_config.so"), cfg.find("CustomLog")) {
            assert!(m < c, "mod_log_config must load before CustomLog:\n{cfg}");
        }
        assert!(
            cfg.contains("SetHandler \"proxy:unix:"),
            "socket-mode SetHandler expected:\n{cfg}"
        );
    }

    #[test]
    fn apache_tcp_mode_uses_fcgi_tcp_backend() {
        let p = php_project(WebServer::Apache);
        let tuning = FpmTuning {
            listen: "tcp".into(),
            tcp_port: 9001,
            ..FpmTuning::default()
        };
        let cfg = render_apache_config(
            &p,
            8090,
            &spaced_socket(),
            &tuning,
            Path::new("/tmp/conf"),
            Path::new("/tmp/logs"),
            Path::new("/usr/sbin/httpd"),
        );
        assert!(
            cfg.contains("SetHandler \"proxy:fcgi://127.0.0.1:9001\""),
            "tcp fcgi backend expected:\n{cfg}"
        );
    }

    #[test]
    fn nginx_project_warns_iff_binary_absent() {
        // The warning is derived from live binary availability, so assert it
        // agrees with `nginx_binary()` on this machine rather than hardcoding a
        // present/absent expectation. When present (a CI/dev box with Homebrew
        // nginx) → None; when absent → an actionable message.
        let mut p = php_project(WebServer::Nginx);
        p.start_command = None;
        let issue = web_server_issue(&p);
        assert_eq!(issue.is_some(), nginx_binary().is_none());
        if let Some(msg) = issue {
            assert!(
                msg.contains("brew install nginx"),
                "needs an install hint: {msg}"
            );
            assert!(
                msg.to_lowercase().contains("caddy"),
                "needs the switch-to-Caddy escape hatch: {msg}"
            );
        }
    }

    #[test]
    fn apache_project_warns_iff_binary_absent() {
        let mut p = php_project(WebServer::Apache);
        p.start_command = None;
        let issue = web_server_issue(&p);
        assert_eq!(issue.is_some(), apache_binary().is_none());
        if let Some(msg) = issue {
            assert!(
                msg.contains("brew install httpd"),
                "needs an install hint: {msg}"
            );
        }
    }

    #[test]
    fn caddy_dev_command_and_non_php_have_no_web_server_issue() {
        // Caddy projects serve via the edge directly — never a missing-binary case.
        let mut caddy = php_project(WebServer::Caddy);
        caddy.start_command = None;
        assert!(web_server_issue(&caddy).is_none());

        // A PHP project with its own dev command runs that command (and Caddy
        // reverse-proxies to it), so the managed nginx/apache path is irrelevant.
        let mut dev = php_project(WebServer::Nginx);
        dev.start_command = Some("php -S 127.0.0.1:8000 router.php".into());
        assert!(
            web_server_issue(&dev).is_none(),
            "a dev-command project serves itself"
        );

        // Non-PHP projects never delegate to a managed web server.
        let mut node = php_project(WebServer::Nginx);
        node.kind = ProjectType::Next;
        node.start_command = None;
        assert!(
            web_server_issue(&node).is_none(),
            "non-PHP never uses a managed web server"
        );
    }

    /// Validate the *actual* generated config against the real binary when a
    /// neutral one is installed. Skips cleanly otherwise — competitor binaries
    /// are filtered out, so this only runs where Homebrew/system nginx exists.
    #[test]
    fn generated_nginx_config_passes_nginx_t() {
        let Some(bin) = nginx_binary() else {
            return;
        };
        // Use the system temp root explicitly: a `TMPDIR` on an external volume
        // can be unreadable by a sandboxed web-server binary ("Operation not
        // permitted"), which would mask the real syntax check we're after.
        let dir = tempfile::Builder::new()
            .prefix("portbay-ws")
            .tempdir_in("/tmp")
            .unwrap();
        let root = dir.path().join("My Doc Root");
        std::fs::create_dir_all(&root).unwrap();
        let mut p = php_project(WebServer::Nginx);
        p.path = root;
        p.document_root = None;
        let sock = dir.path().join("php-fpm.sock");
        let cfg = render_nginx_config(&p, 8190, &sock, &FpmTuning::default(), dir.path());
        let conf = dir.path().join("nginx.conf");
        std::fs::write(&conf, cfg).unwrap();
        let out = std::process::Command::new(&bin)
            .args(["-t", "-c"])
            .arg(&conf)
            .arg("-p")
            .arg(dir.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "nginx -t rejected the generated config:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn generated_apache_config_passes_httpd_t() {
        let Some(bin) = apache_binary() else {
            return;
        };
        // System temp root explicitly — see the nginx test for why.
        let dir = tempfile::Builder::new()
            .prefix("portbay-ws")
            .tempdir_in("/tmp")
            .unwrap();
        let root = dir.path().join("My Doc Root");
        std::fs::create_dir_all(&root).unwrap();
        let mut p = php_project(WebServer::Apache);
        p.path = root;
        p.document_root = None;
        let sock = dir.path().join("php-fpm.sock");
        let cfg = render_apache_config(
            &p,
            8190,
            &sock,
            &FpmTuning::default(),
            dir.path(),
            dir.path(),
            &bin,
        );
        let conf = dir.path().join("httpd.conf");
        std::fs::write(&conf, cfg).unwrap();
        let out = std::process::Command::new(&bin)
            .args(["-t", "-f"])
            .arg(&conf)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "httpd -t rejected the generated config:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
