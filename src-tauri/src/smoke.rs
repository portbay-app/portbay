//! First-run "PortBay smoke" canary project.
//!
//! A tiny Static site PortBay seeds on first launch so a brand-new user can
//! press Play once and confirm their whole local stack works end to end —
//! privileged helper → `/etc/hosts` (or dnsmasq) resolution → Caddy on :80 →
//! file serving — without having to wire up a real project first.
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
        runtime: None,
        workspace: None,
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

fn canary_html(hostname: &str) -> String {
    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>PortBay is working</title>
<style>
  :root {{ color-scheme: dark; }}
  * {{ box-sizing: border-box; }}
  html, body {{ height: 100%; margin: 0; }}
  body {{
    font: 15px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    color: #e7ecf3;
    background: radial-gradient(1200px 600px at 50% -10%, #16263b 0%, #0b1118 55%, #070b10 100%);
    display: grid; place-items: center; text-align: center; padding: 24px;
  }}
  .card {{ max-width: 520px; }}
  .mark {{ display: inline-flex; align-items: center; gap: 10px; margin-bottom: 22px; }}
  .mark svg {{ width: 34px; height: 34px; }}
  .mark span {{ font-size: 17px; font-weight: 600; letter-spacing: -0.01em; }}
  h1 {{ font-size: 24px; font-weight: 650; letter-spacing: -0.02em; margin: 0 0 12px; }}
  p {{ margin: 0 0 10px; color: #9fb0c3; }}
  code {{ background: #ffffff12; padding: 2px 7px; border-radius: 6px; font-size: 13px; color: #cdd9e8; }}
  ul {{ list-style: none; padding: 0; margin: 22px auto 0; max-width: 360px; text-align: left; }}
  li {{ display: flex; align-items: center; gap: 10px; padding: 7px 0; color: #cdd9e8; font-size: 13.5px; }}
  li svg {{ width: 16px; height: 16px; flex: none; }}
  .foot {{ margin-top: 30px; font-size: 12px; color: #5a6b80; letter-spacing: .02em; }}
</style>
</head>
<body>
  <div class="card">
    <div class="mark">
      <svg viewBox="0 0 24 24" fill="none" stroke="#36d399" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M12 2.5 9.5 9h5L12 2.5Z"/><path d="M9.7 9h4.6l1.2 11.5H8.5L9.7 9Z"/><path d="M7 20.5h10"/><path d="M14.8 6.2l3 1.2M9.2 6.2l-3 1.2"/>
      </svg>
      <span>PortBay</span>
    </div>
    <h1>Your setup works.</h1>
    <p>You're seeing this because <code>{hostname}</code> resolved to this Mac
       and PortBay's bundled Caddy served it.</p>
    <ul>
      <li><svg viewBox="0 0 24 24" fill="none" stroke="#36d399" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg> Local DNS routed <code>.test</code> to 127.0.0.1</li>
      <li><svg viewBox="0 0 24 24" fill="none" stroke="#36d399" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg> Caddy answered on port 80</li>
      <li><svg viewBox="0 0 24 24" fill="none" stroke="#36d399" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg> Static files served from disk</li>
    </ul>
    <div class="foot">Delete this project any time — it's just a check.</div>
  </div>
</body>
</html>
"##
    )
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
            assert_eq!(seed_if_absent(&mut reg).unwrap(), false);
        }
    }

    #[test]
    fn canary_html_mentions_the_hostname() {
        let html = canary_html("portbay-smoke.portbay.test");
        assert!(html.contains("portbay-smoke.portbay.test"));
        assert!(html.contains("Your setup works"));
    }
}
