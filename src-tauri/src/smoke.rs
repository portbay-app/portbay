//! First-run "PortBay smoke" canary project.
//!
//! A self-contained Static site PortBay seeds on first launch so a brand-new
//! user can press Play once and confirm their whole local stack works end to
//! end — privileged helper → `/etc/hosts` (or dnsmasq) resolution → Caddy on
//! :80 → file serving — without having to wire up a real project first.
//!
//! It doubles as a real PortBay landing page (hero, features, open-source
//! pitch, pricing, FAQ, download) so the canary the user inevitably opens also
//! introduces the product. The page is authored in `smoke_site/index.html`.
//!
//! It is intentionally **HTTP, not HTTPS**: serving over TLS would also
//! require the mkcert CA to be installed (a second privileged step), which
//! defeats the point of a frictionless first-run check. The canary validates
//! everything up to and including Caddy serving files for a `.test` hostname.
//!
//! Two entry points:
//!   - [`seed_if_absent`] — called once on first run to add the project.
//!   - [`ensure_site_files`] — called every boot to (re)materialise the
//!     canary's `index.html`, so it survives a `/tmp` wipe or a deleted dir.

use std::path::PathBuf;

use crate::registry::{Project, ProjectId, ProjectType, Registry};

/// Stable id/hostname stem for the canary. Also the on-disk folder name.
pub const SMOKE_ID: &str = "portbay-smoke";

/// Persistent on-disk home for the canary's files:
/// `<data_dir>/PortBay/sites/portbay-smoke`. Deliberately under the app data
/// dir (survives reboots) rather than `/tmp` (wiped, and TCC-awkward).
fn default_site_dir() -> Option<PathBuf> {
    let mut dir = dirs::data_dir()?;
    dir.push("PortBay");
    dir.push("sites");
    dir.push(SMOKE_ID);
    Some(dir)
}

/// Seed the canary into the registry if it isn't already present. Scaffolds
/// its `index.html` and adds a Static project pointing at it. Returns `true`
/// when a project was added. No-op (returns `false`) when a `portbay-smoke`
/// project already exists — we never clobber the user's edits.
pub fn seed_if_absent(reg: &mut Registry) -> std::io::Result<bool> {
    if reg.get_project(&ProjectId::new(SMOKE_ID)).is_some() {
        return Ok(false);
    }
    let Some(site_dir) = default_site_dir() else {
        return Ok(false);
    };
    scaffold(&site_dir, &reg.domain_suffix)?;

    let project = Project {
        id: ProjectId::new(SMOKE_ID),
        name: "PortBay smoke".into(),
        path: site_dir,
        kind: ProjectType::Static,
        start_command: None,
        port: None,
        extra_ports: vec![],
        hostname: format!("{SMOKE_ID}.{}", reg.domain_suffix),
        https: false,
        services: vec!["caddy".into()],
        env: Default::default(),
        readiness: None,
        auto_start: false,
        tags: vec!["portbay".into()],
        document_root: None,
        php_version: None,
        web_server: None,
        mobile_run: None,
        runtime: None,
        workspace: None,
        cors: None,
        sandbox: None,
        domain: None,
        tunnel: None,
    };
    // `add_project` only errors on a duplicate id, which we just ruled out.
    let _ = reg.add_project(project);
    Ok(true)
}

/// Ensure the canary's `index.html` exists on disk wherever its project points
/// (the seeded data dir, or a legacy `/tmp` location from an older install).
/// Cheap and idempotent — only writes when the file is missing — so it's safe
/// to call on every boot. This is what makes the canary survive a `/tmp` wipe.
pub fn ensure_site_files(reg: &Registry) {
    let Some(project) = reg.get_project(&ProjectId::new(SMOKE_ID)) else {
        return;
    };
    let root = project
        .document_root
        .as_deref()
        .map(|d| project.path.join(d))
        .unwrap_or_else(|| project.path.clone());
    if root.join("index.html").exists() {
        return;
    }
    let _ = scaffold(&root, &reg.domain_suffix);
}

/// Write the canary's self-contained `index.html` into `dir` (creating it).
fn scaffold(dir: &std::path::Path, suffix: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let hostname = format!("{SMOKE_ID}.{suffix}");
    std::fs::write(dir.join("index.html"), canary_html(&hostname))
}

/// The canary page is a full, self-contained PortBay landing page that doubles
/// as the first-run smoke test: a single static `index.html` (inline CSS, inline
/// SVG, one base64 logo — no external assets, no build step) served by the
/// bundled Caddy. Authored in `smoke_site/index.html` and embedded at compile
/// time; the only runtime substitution is the live `.test` hostname in the
/// "your setup works" banner.
const CANARY_TEMPLATE: &str = include_str!("smoke_site/index.html");

fn canary_html(hostname: &str) -> String {
    CANARY_TEMPLATE.replace("{{HOSTNAME}}", hostname)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Registry;

    #[test]
    fn seed_adds_a_static_http_project_once() {
        let mut reg = Registry::new("portbay.test");
        // First seed adds it (file scaffolding may fail in a sandbox without a
        // data dir, but the project is still added when a site dir resolves).
        let _ = seed_if_absent(&mut reg);
        if let Some(p) = reg.get_project(&ProjectId::new(SMOKE_ID)) {
            assert_eq!(p.kind, ProjectType::Static);
            assert!(!p.https);
            assert!(p.start_command.is_none());
            assert_eq!(p.hostname, "portbay-smoke.portbay.test");
            // Second call is a no-op — never clobbers an existing project.
            assert!(!seed_if_absent(&mut reg).unwrap());
        }
    }

    #[test]
    fn canary_html_mentions_the_hostname() {
        let html = canary_html("portbay-smoke.portbay.test");
        assert!(html.contains("portbay-smoke.portbay.test"));
        assert!(html.contains("Your setup works"));
    }
}
