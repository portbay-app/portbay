//! Shared `doctor` core — the single source of truth behind the CLI
//! `portbay doctor` command and the MCP `portbay_doctor` tool.
//!
//! Like [`crate::sidecar_probe`], this module exists so the two surfaces can't
//! drift: both call [`report`] and get the same structured [`DoctorReport`].
//! The CLI renders it as a colored, grouped table; MCP serializes it to JSON
//! for an agent. Presentation (badges, colors) lives in the caller — this
//! module is data only, and intentionally pulls in no terminal/UI crates.
//!
//! Bundled sidecars (Caddy, mkcert, dnsmasq, Mailpit, cloudflared) are reported
//! via [`crate::sidecar_probe`], **never resolved from `$PATH`** — a foreign
//! install must never be mistaken for PortBay's own toolchain.

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;

use crate::registry::Registry;

/// A check's outcome. Serializes as `"ok"` / `"warn"` / `"fail"` (the wire
/// shape the MCP tool has always used).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Ok,
    Warn,
    Fail,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Ok => "ok",
            Verdict::Warn => "warn",
            Verdict::Fail => "fail",
        }
    }

    /// The more severe of two verdicts (Fail > Warn > Ok).
    fn worse(self, other: Verdict) -> Verdict {
        match (self, other) {
            (Verdict::Fail, _) | (_, Verdict::Fail) => Verdict::Fail,
            (Verdict::Warn, _) | (_, Verdict::Warn) => Verdict::Warn,
            _ => Verdict::Ok,
        }
    }
}

/// One environment-health check (a single row under a category).
#[derive(Debug, Clone, Serialize)]
pub struct DoctorCheck {
    pub check: String,
    pub verdict: Verdict,
    pub detail: String,
}

impl DoctorCheck {
    pub fn ok(check: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            check: check.into(),
            verdict: Verdict::Ok,
            detail: detail.into(),
        }
    }
    pub fn warn(check: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            check: check.into(),
            verdict: Verdict::Warn,
            detail: detail.into(),
        }
    }
    pub fn fail(check: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            check: check.into(),
            verdict: Verdict::Fail,
            detail: detail.into(),
        }
    }
}

/// A titled group of checks. `verdict` is the worst of its rows.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorCategory {
    pub title: String,
    pub verdict: Verdict,
    pub checks: Vec<DoctorCheck>,
}

impl DoctorCategory {
    fn new(title: impl Into<String>, checks: Vec<DoctorCheck>) -> Self {
        let verdict = checks
            .iter()
            .fold(Verdict::Ok, |acc, c| acc.worse(c.verdict));
        Self {
            title: title.into(),
            verdict,
            checks,
        }
    }
}

/// The full report: grouped categories plus a top-level `ok` (no fatal check).
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub ok: bool,
    pub categories: Vec<DoctorCategory>,
}

impl DoctorReport {
    /// `(warnings, fatals)` across every check — for the CLI summary line.
    pub fn counts(&self) -> (usize, usize) {
        let mut warns = 0;
        let mut fails = 0;
        for c in self.categories.iter().flat_map(|c| &c.checks) {
            match c.verdict {
                Verdict::Warn => warns += 1,
                Verdict::Fail => fails += 1,
                Verdict::Ok => {}
            }
        }
        (warns, fails)
    }
}

/// Build the environment report. `registry` is the caller's already-loaded
/// registry (or an error string if it failed to load); `pc_port` is the
/// Process Compose admin port; `data_dir` is PortBay's data directory (parent
/// of `registry.json`), used for the certs directory and tunnel state.
pub async fn report(
    registry: Result<&Registry, &str>,
    pc_port: u16,
    data_dir: &Path,
) -> DoctorReport {
    use crate::php::{self, PhpSource};
    use crate::registry::DatabaseEngine;
    use crate::sidecar_probe::{self, ProbeState};
    use crate::{databases, entitlements, tunnel};

    let suffix = registry
        .map(|r| r.domain_suffix.clone())
        .unwrap_or_else(|_| "test".into());

    // Probe app-owned sidecars (process-compose, dnsmasq, caddy, mkcert, mailpit)
    // from outside the daemon — never off $PATH.
    let probes = sidecar_probe::probe(pc_port, &suffix).await;
    let probe = |name: &str| probes.iter().find(|p| p.name == name);
    let probe_check = |label: &str, p: Option<&sidecar_probe::SidecarProbe>| -> DoctorCheck {
        match p {
            Some(p) => DoctorCheck {
                check: label.into(),
                // App-owned sidecars whose live state isn't visible from here
                // read as Ok (informational), not a warning.
                verdict: match p.state {
                    ProbeState::Running | ProbeState::Unknown => Verdict::Ok,
                    ProbeState::Stopped => Verdict::Warn,
                },
                detail: p.detail.clone(),
            },
            None => DoctorCheck::warn(label, "not probed"),
        }
    };

    let mut cats: Vec<DoctorCategory> = Vec::new();

    // ---- Core -------------------------------------------------------------
    let mut core = Vec::new();
    match registry {
        Ok(r) => core.push(DoctorCheck::ok(
            "registry",
            format!(
                "{} project(s) · v{} schema · .{}",
                r.list_projects().len(),
                r.version,
                r.domain_suffix
            ),
        )),
        Err(e) => core.push(DoctorCheck::fail("registry", e.to_string())),
    }
    core.push(probe_check("process-compose", probe("process-compose")));
    match crate::hosts::HostsManager::system().list_managed() {
        Ok(entries) => {
            let expected: HashSet<String> = registry
                .map(|r| {
                    r.list_projects()
                        .iter()
                        .map(|p| p.hostname.clone())
                        .collect()
                })
                .unwrap_or_default();
            let present: HashSet<String> = entries.iter().map(|e| e.hostname.clone()).collect();
            let missing = expected.difference(&present).count();
            let orphan = present.difference(&expected).count();
            if missing == 0 && orphan == 0 {
                core.push(DoctorCheck::ok(
                    "/etc/hosts",
                    format!("{} entries, all match registry", entries.len()),
                ));
            } else {
                core.push(DoctorCheck::warn(
                    "/etc/hosts",
                    format!(
                        "{} entries (missing {missing}, orphan {orphan}) — `sudo portbay hosts reconcile`",
                        entries.len()
                    ),
                ));
            }
        }
        Err(e) => core.push(DoctorCheck::warn("/etc/hosts", e.to_string())),
    }
    cats.push(DoctorCategory::new("Core", core));

    // ---- Web routing & TLS ------------------------------------------------
    let mut web = Vec::new();
    web.push(probe_check("caddy", probe("caddy")));
    web.push(probe_check("mkcert", probe("mkcert")));
    let certs = data_dir.join("certs");
    if certs.exists() {
        let count = std::fs::read_dir(&certs).map(|d| d.count()).unwrap_or(0);
        web.push(DoctorCheck::ok(
            "certs",
            format!("{count} cert(s) in {}", certs.display()),
        ));
    } else {
        web.push(DoctorCheck::warn(
            "certs",
            format!(
                "{} not created yet (issued on first HTTPS site)",
                certs.display()
            ),
        ));
    }
    cats.push(DoctorCategory::new("Web routing & TLS", web));

    // ---- PHP runtimes -----------------------------------------------------
    let mut phpc = Vec::new();
    let installs = php::detect_all();
    if installs.is_empty() {
        phpc.push(DoctorCheck::warn(
            "php",
            "none found — install via Homebrew or `portbay runtime install`",
        ));
    } else {
        for i in &installs {
            let src = match i.source {
                PhpSource::PortBay => "portbay-managed",
                PhpSource::Homebrew => "homebrew",
                PhpSource::ServBay => "servbay",
                PhpSource::FlyEnv => "flyenv",
                PhpSource::System => "system",
            };
            if i.php_fpm_bin.is_some() {
                phpc.push(DoctorCheck::ok(
                    format!("php {}", i.version),
                    format!("{} · {src}", i.php_bin.display()),
                ));
            } else {
                phpc.push(DoctorCheck::warn(
                    format!("php {}", i.version),
                    format!(
                        "{} · {src} — no php-fpm, can't serve sites",
                        i.php_bin.display()
                    ),
                ));
            }
        }
    }
    cats.push(DoctorCategory::new("PHP runtimes", phpc));

    // ---- Services ---------------------------------------------------------
    let mut svc = Vec::new();
    svc.push(probe_check("dnsmasq", probe("dnsmasq")));
    svc.push(probe_check("mailpit", probe("mailpit")));
    let engines = [
        (DatabaseEngine::Mysql, "mysql"),
        (DatabaseEngine::Mariadb, "mariadb"),
        (DatabaseEngine::Postgres, "postgres"),
        (DatabaseEngine::Redis, "redis"),
        (DatabaseEngine::Mongo, "mongo"),
        (DatabaseEngine::Memcached, "memcached"),
    ];
    let available: Vec<&str> = engines
        .iter()
        .filter(|(e, _)| databases::daemon_binary(*e).is_some())
        .map(|(_, n)| *n)
        .collect();
    if available.is_empty() {
        svc.push(DoctorCheck::warn(
            "databases",
            "no engines found — `brew install mysql` / `postgresql` etc.",
        ));
    } else {
        svc.push(DoctorCheck::ok(
            "databases",
            format!("{} available", available.join(", ")),
        ));
    }
    cats.push(DoctorCategory::new("Services", svc));

    // ---- Account & sharing ------------------------------------------------
    let mut acct = Vec::new();
    let eff = entitlements::current();
    let cap = eff
        .entitlements
        .max_projects
        .map(|n| n.to_string())
        .unwrap_or_else(|| "∞".into());
    match &eff.account {
        Some(a) => acct.push(DoctorCheck::ok(
            "account",
            format!("signed in as {} · {} ({cap} projects)", a.login, eff.tier),
        )),
        None => acct.push(DoctorCheck::ok(
            "account",
            format!(
                "not signed in · {} tier ({cap} projects) — `portbay login`",
                eff.tier
            ),
        )),
    }
    let active = tunnel::read_state(data_dir)
        .into_iter()
        .filter(|t| t.running)
        .count();
    acct.push(DoctorCheck::ok(
        "tunnels",
        format!("{active} active · cloudflared bundled with PortBay.app"),
    ));
    cats.push(DoctorCategory::new("Account & sharing", acct));

    let ok = !cats.iter().any(|c| matches!(c.verdict, Verdict::Fail));
    DoctorReport {
        ok,
        categories: cats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_verdict_is_worst_of_rows() {
        let cat = DoctorCategory::new(
            "x",
            vec![DoctorCheck::ok("a", "fine"), DoctorCheck::warn("b", "meh")],
        );
        assert!(matches!(cat.verdict, Verdict::Warn));

        let cat = DoctorCategory::new(
            "y",
            vec![
                DoctorCheck::ok("a", "fine"),
                DoctorCheck::fail("b", "broken"),
            ],
        );
        assert!(matches!(cat.verdict, Verdict::Fail));

        let cat = DoctorCategory::new("z", vec![DoctorCheck::ok("a", "fine")]);
        assert!(matches!(cat.verdict, Verdict::Ok));
    }

    #[test]
    fn counts_warnings_and_fatals() {
        let report = DoctorReport {
            ok: false,
            categories: vec![
                DoctorCategory::new(
                    "a",
                    vec![DoctorCheck::ok("x", ""), DoctorCheck::warn("y", "")],
                ),
                DoctorCategory::new(
                    "b",
                    vec![DoctorCheck::fail("z", ""), DoctorCheck::warn("w", "")],
                ),
            ],
        };
        assert_eq!(report.counts(), (2, 1));
    }

    #[test]
    fn verdict_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Verdict::Ok).unwrap(), "\"ok\"");
        assert_eq!(serde_json::to_string(&Verdict::Warn).unwrap(), "\"warn\"");
        assert_eq!(serde_json::to_string(&Verdict::Fail).unwrap(), "\"fail\"");
    }
}
