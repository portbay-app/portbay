//! Sandboxed project runner support.
//!
//! Logs, readiness, stop/restart, and Caddy routing remain the same because PC
//! still owns the supervised process.

use std::fs;
use std::path::{Path, PathBuf};

use crate::registry::{Project, SandboxConfig, SandboxNetworkPolicy};

/// Legacy marker used by the first sandbox build. Kept as read compatibility
/// so existing registries don't silently run unrestricted after upgrade.
pub const SANDBOX_TAG: &str = "portbay:sandbox";

pub fn is_enabled(project: &Project) -> bool {
    project
        .sandbox
        .as_ref()
        .map(|cfg| cfg.enabled)
        .unwrap_or_else(|| project.tags.iter().any(|tag| tag == SANDBOX_TAG))
}

pub fn config(project: &Project) -> SandboxConfig {
    project.sandbox.clone().unwrap_or_else(|| SandboxConfig {
        enabled: project.tags.iter().any(|tag| tag == SANDBOX_TAG),
        ..SandboxConfig::default()
    })
}

pub fn enable(project: &mut Project, network: SandboxNetworkPolicy, ephemeral: bool) {
    project.sandbox = Some(SandboxConfig::enabled(network, ephemeral));
    project.tags.retain(|tag| tag != SANDBOX_TAG);
}

pub fn disable(project: &mut Project) {
    project.tags.retain(|tag| tag != SANDBOX_TAG);
    if let Some(cfg) = &mut project.sandbox {
        cfg.enabled = false;
    }
}

pub fn network_policy_key(policy: SandboxNetworkPolicy) -> &'static str {
    match policy {
        SandboxNetworkPolicy::LoopbackOnly => "loopback_only",
        SandboxNetworkPolicy::Outbound => "outbound",
        SandboxNetworkPolicy::Full => "full",
        SandboxNetworkPolicy::Blocked => "blocked",
    }
}

/// Inverse of [`network_policy_key`]: parse a policy key (the snake_case wire
/// value, plus a couple of friendly aliases) back into a policy. Shared by the
/// CLI and the MCP server so both accept the same spellings. `None` for an
/// unrecognised value.
pub fn parse_network_policy(value: &str) -> Option<SandboxNetworkPolicy> {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "loopback_only" | "loopback" => Some(SandboxNetworkPolicy::LoopbackOnly),
        "outbound" => Some(SandboxNetworkPolicy::Outbound),
        "full" => Some(SandboxNetworkPolicy::Full),
        "blocked" | "none" => Some(SandboxNetworkPolicy::Blocked),
        _ => None,
    }
}

pub fn profile_path(data_dir: &Path, project: &Project) -> PathBuf {
    data_dir
        .join("sandbox")
        .join(format!("{}.sb", project.id.as_str()))
}

/// Per-project ephemeral scratch dir. Wiped before each sandboxed start (see
/// [`reset_ephemeral_state`]) and pointed at by `TMPDIR` + package-manager cache
/// vars (see `ephemeral_env_prefix`), so transient writes never persist or leak
/// between runs. Kept inside the writable set by [`profile`].
pub fn ephemeral_dir(data_dir: &Path, project: &Project) -> PathBuf {
    data_dir
        .join("sandbox")
        .join(project.id.as_str())
        .join("ephemeral")
}

#[cfg(target_os = "macos")]
pub fn ensure_profile(data_dir: &Path, project: &Project) -> std::io::Result<PathBuf> {
    let path = profile_path(data_dir, project);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // SBPL can't expand `~`, so the credential deny-list is baked from the
    // real home dir at generation time.
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/var/empty"));
    fs::write(&path, profile(project, data_dir, &home))?;
    Ok(path)
}

/// Wrap a project's launch command so it runs under the macOS Seatbelt sandbox.
/// The profile is regenerated each call (so an edited policy always takes
/// effect) and, when ephemeral mode is on, the toolchain's temp + caches are
/// steered into the per-run scratch dir.
#[cfg(target_os = "macos")]
pub fn wrap_command(data_dir: &Path, project: &Project, command: &str) -> String {
    match ensure_profile(data_dir, project) {
        Ok(profile) => format!(
            "{env}sandbox-exec -f {profile} /bin/zsh -lc {cmd}",
            env = ephemeral_env_prefix(data_dir, project),
            profile = shell_quote(&profile.to_string_lossy()),
            cmd = shell_quote(command),
        ),
        Err(err) => format!(
            "printf %s {} >&2; exit 126",
            shell_quote(&format!("PortBay sandbox profile error: {err}\n"))
        ),
    }
}

/// Off macOS there is no Seatbelt backend. Rather than run the project
/// unconfined (silently dropping the safety the user asked for) or emit a
/// `sandbox-exec` call that doesn't exist, refuse loudly. Sandboxed Run is gated
/// to macOS at the command layer too; this is the defense-in-depth path for a
/// synced or hand-edited registry that carries `sandbox.enabled` on another OS.
#[cfg(not(target_os = "macos"))]
pub fn wrap_command(_data_dir: &Path, _project: &Project, command: &str) -> String {
    format!(
        "printf %s {} >&2; exit 126",
        shell_quote(&format!(
            "PortBay: Sandboxed Run is only supported on macOS; refusing to run '{command}' unconfined.\n"
        ))
    )
}

/// Whether the macOS Seatbelt frontend is present. `sandbox-exec` is deprecated
/// by Apple but still shipped; if a future macOS removes it we must fail closed
/// (never silently run a "sandboxed" project unconfined), so callers check this
/// before promising confinement.
#[cfg(target_os = "macos")]
pub fn is_available() -> bool {
    Path::new("/usr/bin/sandbox-exec").exists()
}

#[cfg(not(target_os = "macos"))]
pub fn is_available() -> bool {
    false
}

/// Prove macOS actually accepts the generated profile *before* we rely on it.
///
/// A malformed profile makes `sandbox-exec` refuse to launch the command, which
/// would otherwise surface only as a cryptic project-start failure. Running the
/// profile against `/usr/bin/true` here means a bad profile is caught at the
/// moment the user enables Sandboxed Run, with a clear message — and guarantees
/// there is no path where the project starts but the confinement silently
/// didn't apply. On success the profile file exists and is known-good.
#[cfg(target_os = "macos")]
pub fn preflight(data_dir: &Path, project: &Project) -> Result<(), String> {
    if !is_available() {
        return Err(
            "macOS sandbox-exec is unavailable on this system; refusing to run unconfined.".into(),
        );
    }
    let profile = ensure_profile(data_dir, project)
        .map_err(|e| format!("could not write the sandbox profile: {e}"))?;
    let output = std::process::Command::new("/usr/bin/sandbox-exec")
        .arg("-f")
        .arg(&profile)
        .arg("/usr/bin/true")
        .output()
        .map_err(|e| format!("sandbox preflight could not run: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "macOS rejected the sandbox profile: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

pub fn reset_ephemeral_state(data_dir: &Path, project: &Project) -> std::io::Result<()> {
    let cfg = config(project);
    if !cfg.enabled || !cfg.ephemeral {
        return Ok(());
    }
    let root = ephemeral_dir(data_dir, project);
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    fs::create_dir_all(&root)
}

pub fn violation_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("deny(")
                || lower.contains("sandbox")
                    && (lower.contains("deny") || lower.contains("operation not permitted"))
        })
        .cloned()
        .collect()
}

/// Generate the Seatbelt (SBPL) profile for a sandboxed project.
///
/// Posture: `(deny default)` whitelist. Reads are broadly allowed so dev
/// toolchains keep working, then secrets are **denied back** — SBPL is
/// last-match-wins, so a later deny overrides the broad allow. The denies cover
/// credential stores, browser + messaging data, shell history, and every
/// `.env*` file anywhere (including *other* projects'); the project's own tree
/// is then re-allowed so it still reads its own `.env`. This closes the primary
/// supply-chain risk: an untrusted dependency reading your SSH/cloud/registry
/// secrets, keychains, browser cookies, or a sibling project's `.env` and
/// exfiltrating them.
///
/// It is a curated deny-list, not a default-deny read jail — dev toolchains
/// read so widely that a strict read-allowlist breaks real builds. It covers
/// the realistic exfiltration targets; it is not a proof that no secret in an
/// unusual location is readable (see [`secret_read_denies`]). Writes are
/// confined to the project, the per-run ephemeral scratch dir, and OS temp
/// dirs. `mach-lookup` is an allowlist (the main Seatbelt escape surface)
/// rather than wide open.
#[cfg(target_os = "macos")]
fn profile(project: &Project, data_dir: &Path, home: &Path) -> String {
    // CRITICAL: Seatbelt matches the *canonical* (symlink-resolved) path, not
    // the path as typed. `/tmp` → `/private/tmp`, a symlinked home or work dir,
    // etc. would otherwise make every `subpath` rule silently miss — the secret
    // denies wouldn't fire and the project's own writes/reads wouldn't be
    // re-allowed. Bake resolved paths so the rules actually apply.
    let project_path_buf = canonical(&project.path);
    let project_path = project_path_buf.to_string_lossy();
    let home = canonical(home);
    let cfg = config(project);
    let network = match cfg.network {
        SandboxNetworkPolicy::Blocked => "",
        SandboxNetworkPolicy::LoopbackOnly => {
            r#"; Loopback-only networking: local dev-server bind/connect.
(allow network* (local ip "localhost:*"))
(allow network* (remote ip "localhost:*"))
"#
        }
        SandboxNetworkPolicy::Outbound => {
            r#"; Outbound package-manager access plus local dev-server bind.
(allow network-outbound)
(allow network* (local ip "localhost:*"))
"#
        }
        SandboxNetworkPolicy::Full => "(allow network*)\n",
    };

    // Per-run scratch dir, writable only when ephemeral mode is on.
    let ephemeral_rule = if cfg.ephemeral {
        format!(
            "  (subpath {})\n",
            sbpl_string(&ephemeral_dir(data_dir, project).to_string_lossy())
        )
    } else {
        String::new()
    };

    format!(
        r#"(version 1)
(deny default)

; Launch shells, package managers, interpreters, and their children. Children
; inherit this sandbox, so allowing exec does NOT let them escape it.
(allow process*)
(allow signal (target same-sandbox))
(allow sysctl-read)

; Mach services are the classic Seatbelt escape surface, so this is an
; allowlist of the globals a dev server legitimately needs (logging,
; notifications, user/host lookups, DNS, network reachability) instead of a
; blanket `(allow mach-lookup)`.
(allow mach-lookup
  (global-name "com.apple.system.logger")
  (global-name "com.apple.system.notification_center")
  (global-name "com.apple.system.opendirectoryd.libinfo")
  (global-name "com.apple.system.opendirectoryd.membership")
  (global-name "com.apple.SystemConfiguration.configd")
  (global-name "com.apple.SystemConfiguration.SCNetworkReachability")
  (global-name "com.apple.dnssd.service")
  (global-name "com.apple.mDNSResponder")
  (global-name "com.apple.coreservices.launchservicesd")
  (global-name "com.apple.CoreServices.coreservicesd"))

; Runtimes read toolchains, frameworks, lockfiles, and package caches all over
; the disk, so reads are broadly allowed …
(allow file-read*)

; … then secrets are clawed back (last match wins): credential stores, browser
; + app data, shell history, and every `.env*` file anywhere on disk.
{secret_denies}
; Finally the project's own tree is re-allowed, so its `.env` / config stay
; readable — the denies above protect everything *outside* the code we run,
; including other projects' `.env` files.
(allow file-read* (subpath {project_path_q}))

; Writes are constrained to the project, the ephemeral scratch dir, and the OS
; temp/cache locations package managers and dev servers expect.
(allow file-write*
  (subpath {project_path_q})
{ephemeral_rule}  (subpath "/tmp")
  (subpath "/private/tmp")
  (literal "/dev/null")
  (regex #"^/private/var/folders/"))

{network}
"#,
        secret_denies = secret_read_denies(&home),
        project_path_q = sbpl_string(&project_path),
        ephemeral_rule = ephemeral_rule,
        network = network,
    )
}

/// The secrets denied to a sandboxed process even though reads are otherwise
/// broad: credential stores, browser + messaging data, shell history, and (via
/// a regex below) every `.env*` file outside the project. Home-relative because
/// SBPL doesn't expand `~`. A denied `~/.npmrc` means private-registry tokens
/// can't be read inside the sandbox — that's the point; a build that genuinely
/// needs them shouldn't be sandboxed.
///
/// This is a curated deny-list, not a default-deny read jail: dev toolchains
/// read so widely across the disk that a strict read-allowlist breaks real
/// builds. It covers the realistic exfiltration targets a malicious dependency
/// goes after; it is *not* a guarantee that no secret in an unusual location is
/// readable. The `.env` regex + the explicit stores below are the high-value set.
#[cfg(target_os = "macos")]
fn secret_read_denies(home: &Path) -> String {
    // Directories holding keys / credentials / private user data.
    const SECRET_DIRS: &[&str] = &[
        // Keys & cloud / infra credentials.
        ".ssh",
        ".aws",
        ".gnupg",
        ".kube",
        ".docker",
        ".azure",
        ".config/gh",
        ".config/gcloud",
        ".config/fly",
        ".config/doctl",
        ".password-store",
        ".1password",
        ".terraform.d",
        // Browser profiles (cookies, saved logins, history, tokens).
        "Library/Application Support/Google/Chrome",
        "Library/Application Support/Chromium",
        "Library/Application Support/BraveSoftware",
        "Library/Application Support/Microsoft Edge",
        "Library/Application Support/Firefox",
        "Library/Safari",
        "Library/Cookies",
        // Messaging / mail stores.
        "Library/Messages",
        "Library/Mail",
        // Keychains.
        "Library/Keychains",
    ];
    // Individual credential / history files (registry tokens, git/HTTP creds,
    // shell history that often contains pasted secrets, …).
    const SECRET_FILES: &[&str] = &[
        ".npmrc",
        ".netrc",
        ".git-credentials",
        ".pgpass",
        ".composer/auth.json",
        ".cargo/credentials",
        ".cargo/credentials.toml",
        ".vault-token",
        ".config/hub",
        ".zsh_history",
        ".bash_history",
        ".python_history",
        ".node_repl_history",
        ".psql_history",
        ".mysql_history",
    ];

    // `canonical` per entry: dotfile managers often symlink these (e.g.
    // `~/.ssh` → `~/dotfiles/ssh`), and the kernel resolves the symlink before
    // Seatbelt checks the path — so the deny must name the *resolved* path.
    let mut out = String::new();
    for rel in SECRET_DIRS {
        out.push_str(&format!(
            "(deny file-read* (subpath {}))\n",
            sbpl_string(&canonical(&home.join(rel)).to_string_lossy())
        ));
    }
    for rel in SECRET_FILES {
        out.push_str(&format!(
            "(deny file-read* (literal {}))\n",
            sbpl_string(&canonical(&home.join(rel)).to_string_lossy())
        ));
    }
    // Every `.env*` file anywhere on disk — the most common place app secrets
    // live, and the gap a credential-CLI-only blocklist would miss (a malicious
    // dep reading a *sibling* project's `.env`). The project's own tree is
    // re-allowed after this in `profile()`, so it still reads its own `.env`.
    out.push_str("(deny file-read* (regex #\"/\\.env\"))\n");
    // System-wide keychains, independent of home.
    out.push_str("(deny file-read* (subpath \"/Library/Keychains\"))\n");
    out.push_str("(deny file-read* (subpath \"/private/var/db/Keychains\"))\n");
    out
}

/// Environment assignments that steer transient writes + package-manager caches
/// into the per-run ephemeral scratch dir (wiped by [`reset_ephemeral_state`]).
/// Empty when ephemeral mode is off. Emitted before the command so they export
/// into the child's environment.
#[cfg(target_os = "macos")]
fn ephemeral_env_prefix(data_dir: &Path, project: &Project) -> String {
    if !config(project).ephemeral {
        return String::new();
    }
    let dir = ephemeral_dir(data_dir, project);
    // Guarantee the TMPDIR target exists on every sandboxed start, not just the
    // explicit "Run in Sandbox" path that calls `reset_ephemeral_state` — an
    // `auto_start` sandboxed project is launched by the reconciler, which never
    // resets. Idempotent and cheap; the explicit reset still wipes first.
    let _ = fs::create_dir_all(&dir);
    let root = shell_quote(&dir.to_string_lossy());
    format!("TMPDIR={root} npm_config_cache={root}/npm YARN_CACHE_FOLDER={root}/yarn ")
}

/// Resolve symlinks so SBPL `subpath`/`literal` rules match the canonical path
/// macOS actually evaluates against (e.g. `/tmp` → `/private/tmp`). Falls back
/// to the input when canonicalization fails — e.g. a secret path that doesn't
/// exist on this machine, which is fine because there's nothing to read there.
#[cfg(target_os = "macos")]
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Quote a string as an SBPL (TinyScheme) string literal: wrap in double quotes
/// and backslash-escape `\` and `"`. Replaces the old reliance on Rust's `{:?}`
/// Debug formatting, whose escaping only *coincidentally* matched SBPL's.
#[cfg(target_os = "macos")]
fn sbpl_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{ProjectId, ProjectType};
    use std::collections::BTreeMap;

    fn project() -> Project {
        Project {
            id: ProjectId::new("demo"),
            name: "Demo".into(),
            path: PathBuf::from("/tmp/demo"),
            kind: ProjectType::Node,
            start_command: Some("pnpm dev".into()),
            port: Some(3000),
            extra_ports: vec![],
            hostname: "demo.test".into(),
            https: false,
            services: vec![],
            env: BTreeMap::new(),
            readiness: None,
            auto_start: false,
            tags: vec![],
            document_root: None,
            php_version: None,
            web_server: None,
            mobile_run: None,
            runtime: None,
            workspace: None,
            cors: None,
            sandbox: None,
            domain: None,
        }
    }

    #[test]
    fn config_helpers_are_idempotent() {
        let mut p = project();
        enable(&mut p, SandboxNetworkPolicy::Outbound, false);
        enable(&mut p, SandboxNetworkPolicy::Outbound, false);
        assert!(is_enabled(&p));
        assert_eq!(config(&p).network, SandboxNetworkPolicy::Outbound);
        disable(&mut p);
        assert!(!is_enabled(&p));
    }

    #[test]
    fn network_policy_key_matches_api_wire_values() {
        assert_eq!(
            network_policy_key(SandboxNetworkPolicy::LoopbackOnly),
            "loopback_only"
        );
        assert_eq!(
            network_policy_key(SandboxNetworkPolicy::Outbound),
            "outbound"
        );
        assert_eq!(network_policy_key(SandboxNetworkPolicy::Full), "full");
        assert_eq!(network_policy_key(SandboxNetworkPolicy::Blocked), "blocked");
    }

    #[test]
    fn parse_network_policy_round_trips_keys_and_aliases() {
        for policy in [
            SandboxNetworkPolicy::LoopbackOnly,
            SandboxNetworkPolicy::Outbound,
            SandboxNetworkPolicy::Full,
            SandboxNetworkPolicy::Blocked,
        ] {
            assert_eq!(
                parse_network_policy(network_policy_key(policy)),
                Some(policy)
            );
        }
        // CLI-friendly aliases + spelling tolerance.
        assert_eq!(
            parse_network_policy("loopback-only"),
            Some(SandboxNetworkPolicy::LoopbackOnly)
        );
        assert_eq!(
            parse_network_policy("LOOPBACK"),
            Some(SandboxNetworkPolicy::LoopbackOnly)
        );
        assert_eq!(
            parse_network_policy("none"),
            Some(SandboxNetworkPolicy::Blocked)
        );
        assert_eq!(parse_network_policy("bogus"), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn wrapper_uses_profile_and_original_command() {
        let p = project();
        let cmd = wrap_command(Path::new("/tmp/portbay"), &p, "pnpm dev");
        assert!(cmd.contains("sandbox-exec -f"));
        assert!(cmd.contains("/bin/zsh -lc 'pnpm dev'"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn profile_allows_broad_read_but_denies_credential_stores() {
        let p = project();
        let prof = profile(&p, Path::new("/tmp/portbay"), Path::new("/Users/demo"));
        assert!(prof.contains("(deny default)"));
        assert!(prof.contains("(allow file-read*)"));
        // The high-value secret locations are clawed back after the broad allow.
        assert!(prof.contains(r#"(deny file-read* (subpath "/Users/demo/.ssh"))"#));
        assert!(prof.contains(r#"(deny file-read* (subpath "/Users/demo/.aws"))"#));
        assert!(prof.contains(r#"(deny file-read* (literal "/Users/demo/.npmrc"))"#));
        assert!(prof.contains(r#"(deny file-read* (subpath "/Library/Keychains"))"#));
        // Browser data and shell history are denied too, not just credential CLIs.
        assert!(
            prof.contains(r#"(subpath "/Users/demo/Library/Application Support/Google/Chrome")"#)
        );
        assert!(prof.contains(r#"(deny file-read* (literal "/Users/demo/.zsh_history"))"#));
        // Every `.env*` outside the project is denied via regex …
        assert!(prof.contains(r#"(deny file-read* (regex #"/\.env"))"#));
        // … but the project's own tree is re-allowed *after* that deny, so the
        // project still reads its own `.env`. Order matters (last match wins).
        let env_deny_at = prof.find(r#"(regex #"/\.env"))"#).unwrap();
        let project_reallow_at = prof
            .find(r#"(allow file-read* (subpath "/tmp/demo"))"#)
            .unwrap();
        assert!(
            project_reallow_at > env_deny_at,
            "project re-allow must follow the .env deny so the project's own .env stays readable"
        );

        // The deny lines must come *after* the broad allow, or they're inert.
        let allow_at = prof.find("(allow file-read*)").unwrap();
        let deny_at = prof.find(".ssh").unwrap();
        assert!(
            deny_at > allow_at,
            "deny must follow allow (last match wins)"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn profile_filters_mach_lookup_instead_of_allowing_all() {
        let p = project();
        let prof = profile(&p, Path::new("/tmp/portbay"), Path::new("/Users/demo"));
        assert!(prof.contains(r#"(global-name "com.apple.mDNSResponder")"#));
        // Not the wide-open form.
        assert!(!prof.contains("(allow mach-lookup)\n"));
    }

    // Regression guard for the symlink/canonicalization bug found under real
    // sandbox-exec: Seatbelt matches the resolved path, so a project at a
    // symlinked path must appear in the profile as its canonical path or every
    // `subpath` rule silently misses (secrets readable, project writes denied).
    #[cfg(target_os = "macos")]
    #[test]
    fn profile_canonicalizes_symlinked_project_path() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real_proj");
        std::fs::create_dir_all(&real).unwrap();
        let link = tmp.path().join("link_proj");
        symlink(&real, &link).unwrap();

        let mut p = project();
        p.path = link.clone();
        let prof = profile(&p, Path::new("/tmp/portbay"), Path::new("/Users/demo"));

        let canon_q = sbpl_string(&std::fs::canonicalize(&real).unwrap().to_string_lossy());
        // Both the read re-allow and the writable rule must name the resolved path.
        assert!(
            prof.contains(&format!("(allow file-read* (subpath {canon_q}))")),
            "read re-allow must use the canonical project path"
        );
        assert!(
            prof.contains(&format!("(subpath {canon_q})")),
            "writable rule must use the canonical project path"
        );
        // …and the symlink path itself must not leak in.
        let link_q = sbpl_string(&link.to_string_lossy());
        assert!(
            !prof.contains(&format!("(subpath {link_q})")),
            "the un-resolved symlink path must not appear"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ephemeral_on_makes_scratch_writable_and_redirects_temp() {
        let mut p = project();
        enable(&mut p, SandboxNetworkPolicy::LoopbackOnly, true);
        let data = Path::new("/tmp/portbay");
        let prof = profile(&p, data, Path::new("/Users/demo"));
        let eph = sbpl_string(&ephemeral_dir(data, &p).to_string_lossy());
        assert!(
            prof.contains(&format!("(subpath {eph})")),
            "scratch dir writable"
        );

        let cmd = wrap_command(data, &p, "pnpm dev");
        assert!(cmd.contains("TMPDIR="));
        assert!(cmd.contains("npm_config_cache="));
        assert!(cmd.contains("YARN_CACHE_FOLDER="));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ephemeral_off_does_not_redirect_temp() {
        let mut p = project();
        enable(&mut p, SandboxNetworkPolicy::LoopbackOnly, false);
        let cmd = wrap_command(Path::new("/tmp/portbay"), &p, "pnpm dev");
        assert!(!cmd.contains("TMPDIR="));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn sbpl_string_escapes_quotes_and_backslashes() {
        assert_eq!(sbpl_string("/a/b"), r#""/a/b""#);
        assert_eq!(sbpl_string(r#"/a"b"#), r#""/a\"b""#);
        assert_eq!(sbpl_string(r"/a\b"), r#""/a\\b""#);
    }
}
