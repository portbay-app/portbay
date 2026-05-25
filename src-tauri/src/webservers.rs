//! Generated PHP web-server configs.
//!
//! Caddy remains PortBay's public edge (host routing, HTTPS, placeholder
//! errors). When a PHP project selects Nginx or Apache, PortBay supervises a
//! private loopback web server and has Caddy reverse-proxy to it.

use std::path::{Path, PathBuf};

use crate::process_compose::config::WebServerSpec;
use crate::registry::{Project, ProjectType, Registry, WebServer};

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
                let Some(bin) = nginx_binary() else {
                    warn_missing(project, "nginx");
                    continue;
                };
                let conf_path = conf_dir.join("nginx.conf");
                let body = render_nginx_config(project, port, &socket_path, &log_dir);
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
                let Some(bin) = apache_binary() else {
                    warn_missing(project, "httpd");
                    continue;
                };
                let conf_path = conf_dir.join("httpd.conf");
                let body =
                    render_apache_config(project, port, &socket_path, &conf_dir, &log_dir, &bin);
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

pub fn nginx_binary() -> Option<PathBuf> {
    first_existing(&[
        "/Applications/ServBay/script/alias/nginx",
        "/opt/homebrew/opt/nginx/bin/nginx",
        "/usr/local/opt/nginx/bin/nginx",
        "/opt/homebrew/bin/nginx",
        "/usr/local/bin/nginx",
    ])
    .or_else(|| which::which("nginx").ok())
}

pub fn apache_binary() -> Option<PathBuf> {
    first_existing(&[
        "/Applications/ServBay/script/alias/httpd",
        "/Applications/ServBay/script/alias/apachectl",
        "/opt/homebrew/opt/httpd/bin/httpd",
        "/usr/local/opt/httpd/bin/httpd",
        "/opt/homebrew/bin/httpd",
        "/usr/local/bin/httpd",
        "/usr/sbin/httpd",
    ])
    .or_else(|| which::which("httpd").ok())
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

fn doc_root(project: &Project) -> PathBuf {
    project
        .document_root
        .as_deref()
        .map(|d| project.path.join(d))
        .unwrap_or_else(|| project.path.clone())
}

fn render_nginx_config(project: &Project, port: u16, socket_path: &Path, log_dir: &Path) -> String {
    let root = doc_root(project);
    let access_log = log_dir.join("nginx-access.log");
    let error_log = log_dir.join("nginx-error.log");
    format!(
        r#"worker_processes 1;
error_log {error_log} warn;
pid nginx.pid;

events {{
    worker_connections 256;
}}

http {{
    default_type application/octet-stream;
    access_log {access_log};
    sendfile on;

    server {{
        listen 127.0.0.1:{port};
        server_name {host};
        root {root};
        index index.php index.html index.htm;

        location / {{
            try_files $uri $uri/ /router.php /index.php?$query_string;
        }}

        location ~ \.php(?:/|$) {{
            fastcgi_pass unix:{socket};
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
        socket = socket_path.display(),
    )
}

fn render_apache_config(
    project: &Project,
    port: u16,
    socket_path: &Path,
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

    format!(
        r#"ServerRoot "{server_root}"
PidFile "{server_root}/httpd.pid"
Listen 127.0.0.1:{port}
ServerName {host}
DocumentRoot "{root}"
ErrorLog "{error_log}"
CustomLog "{access_log}" common

{mpm}{authz_core}{authz_host}{dir}{mime}{rewrite}{proxy}{proxy_fcgi}{unixd}
TypesConfig /etc/apache2/mime.types
DirectoryIndex index.php index.html index.htm

<Directory "{root}">
    Options FollowSymLinks
    AllowOverride All
    Require all granted
</Directory>

<FilesMatch "\.php$">
    SetHandler "proxy:unix:{socket}|fcgi://localhost/"
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
        socket = socket_path.display(),
        mpm = load_module("mpm_event", "mod_mpm_event.so"),
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

fn apache_module_dir(httpd_bin: &Path) -> Option<PathBuf> {
    let bin = httpd_bin.to_string_lossy();
    if bin.starts_with("/opt/homebrew/") {
        Some(PathBuf::from("/opt/homebrew/opt/httpd/lib/httpd/modules"))
    } else if bin.starts_with("/usr/local/") {
        Some(PathBuf::from("/usr/local/opt/httpd/lib/httpd/modules"))
    } else if bin.starts_with("/usr/sbin/") {
        Some(PathBuf::from("/usr/libexec/apache2"))
    } else {
        None
    }
    .filter(|p| p.exists())
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
