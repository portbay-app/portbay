//! `/etc/hosts` management — the registry's hostname column needs to
//! resolve before Caddy can serve it.
//!
//! Two-part scope for Phase 1:
//!
//! 1. **This module** owns the parsing and atomic rewrite of `/etc/hosts`
//!    inside a clearly delimited PortBay block. Pure logic, fully tested
//!    against tempfiles.
//!
//! 2. **A privileged helper installed via `SMAppService`** is the planned
//!    way to call into this module without prompting for sudo on every
//!    project change. That helper is deferred to a follow-up card because
//!    the SMAppService dance is genuinely macOS-specific polish work.
//!
//! In the meantime the CLI's hosts commands gracefully degrade: if the
//! current process can't write to `/etc/hosts` (i.e. not running as root),
//! they print a friendly hint to re-run with `sudo`. This keeps the v1
//! UX honest and unblocks every other Phase 1 deliverable.

use std::collections::BTreeMap;
use std::io::Write;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

/// The marker lines that wrap PortBay-managed entries inside `/etc/hosts`.
/// These exact strings are part of the contract — never paraphrase.
pub const BEGIN_MARKER: &str = "# BEGIN PortBay";
pub const END_MARKER: &str = "# END PortBay";

#[derive(thiserror::Error, Debug)]
pub enum HostsError {
    #[error("permission denied writing to {path} — re-run with sudo, or install the PortBay privileged helper")]
    PermissionDenied { path: PathBuf },

    #[error("invalid hostname `{0}`: contains whitespace or comment chars")]
    InvalidHostname(String),

    #[error("hosts file at {path} is malformed: {detail}")]
    Malformed { path: PathBuf, detail: String },

    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl HostsError {
    fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        let path = path.into();
        if source.kind() == std::io::ErrorKind::PermissionDenied {
            Self::PermissionDenied { path }
        } else {
            Self::Io { path, source }
        }
    }
}

pub type Result<T> = std::result::Result<T, HostsError>;

/// A single hosts-file entry within the PortBay-managed block.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct HostsEntry {
    pub ip: Ipv4Addr,
    pub hostname: String,
}

/// Reads / mutates / writes `/etc/hosts` while leaving lines outside the
/// PortBay-managed block strictly untouched.
///
/// Cheap to construct; opens the file only inside method calls.
#[derive(Debug, Clone)]
pub struct HostsManager {
    path: PathBuf,
}

impl HostsManager {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Default constructor pointing at `/etc/hosts`.
    ///
    /// Honours the `PORTBAY_HOSTS_PATH` override so tests (and any sandboxed
    /// run) can redirect writes off the real system file. `/etc/hosts` is a
    /// global path that the tempdir-based test harness can't otherwise
    /// isolate — without this, every `add_project` test would append a real
    /// line to the developer's machine.
    pub fn system() -> Self {
        match std::env::var_os("PORTBAY_HOSTS_PATH") {
            Some(path) => Self::new(path),
            None => Self::new("/etc/hosts"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read the current managed entries (lines inside `BEGIN…END PortBay`).
    /// Returns an empty list if the file or block is missing — both are
    /// expected first-run states.
    pub fn list_managed(&self) -> Result<Vec<HostsEntry>> {
        let contents = match std::fs::read_to_string(&self.path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(HostsError::io(&self.path, e)),
        };
        let block = extract_block(&contents);
        Ok(parse_entries(block))
    }

    /// Idempotent add. Inserting an existing `hostname` is a no-op (we
    /// don't update the IP — different hostnames sharing one IP is the
    /// normal case, but the same hostname pointing at two IPs is not).
    pub fn add(&self, hostname: &str, ip: Ipv4Addr) -> Result<()> {
        validate_hostname(hostname)?;
        let mut entries = self.list_managed()?;
        if entries.iter().any(|e| e.hostname == hostname) {
            return Ok(());
        }
        entries.push(HostsEntry {
            ip,
            hostname: hostname.to_owned(),
        });
        self.write_block(&entries)
    }

    /// Idempotent remove. Missing entries are silently fine.
    pub fn remove(&self, hostname: &str) -> Result<()> {
        validate_hostname(hostname)?;
        let mut entries = self.list_managed()?;
        let before = entries.len();
        entries.retain(|e| e.hostname != hostname);
        if entries.len() == before {
            return Ok(());
        }
        self.write_block(&entries)
    }

    /// Remove the entire managed block. Idempotent.
    pub fn clear(&self) -> Result<()> {
        self.write_block(&[])
    }

    /// Replace whatever's currently in the managed block with this exact
    /// set. Useful for full-registry reconciliation.
    pub fn replace_all<I, S>(&self, entries: I) -> Result<()>
    where
        I: IntoIterator<Item = (S, Ipv4Addr)>,
        S: Into<String>,
    {
        let mut deduped: BTreeMap<String, Ipv4Addr> = BTreeMap::new();
        for (host, ip) in entries {
            let host: String = host.into();
            validate_hostname(&host)?;
            deduped.insert(host, ip);
        }
        let entries: Vec<HostsEntry> = deduped
            .into_iter()
            .map(|(hostname, ip)| HostsEntry { ip, hostname })
            .collect();
        self.write_block(&entries)
    }

    // -- internals ----------------------------------------------------------

    fn write_block(&self, entries: &[HostsEntry]) -> Result<()> {
        let existing = match std::fs::read_to_string(&self.path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(HostsError::io(&self.path, e)),
        };
        let next = render(&existing, entries);

        // Atomic-ish write: rewrite to a sibling tempfile, fsync, rename.
        // `/etc/hosts` lives on a system volume; rename is atomic *within
        // the same filesystem*, so the tempfile must live in the same dir.
        let tmp = sibling_tmp(&self.path);
        {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp)
                .map_err(|e| HostsError::io(&tmp, e))?;
            f.write_all(next.as_bytes())
                .map_err(|e| HostsError::io(&tmp, e))?;
            f.sync_all().map_err(|e| HostsError::io(&tmp, e))?;
        }
        std::fs::rename(&tmp, &self.path).map_err(|e| HostsError::io(&self.path, e))?;
        Ok(())
    }
}

/// Hostname validation — keep it conservative. We refuse anything with
/// whitespace, comment marks, or chars that could be confused with the
/// hosts file's own field separators.
fn validate_hostname(host: &str) -> Result<()> {
    if host.is_empty() {
        return Err(HostsError::InvalidHostname(host.into()));
    }
    for c in host.chars() {
        if c.is_whitespace() || c == '#' {
            return Err(HostsError::InvalidHostname(host.into()));
        }
    }
    Ok(())
}

/// Build a sibling `<file>.portbay.tmp` for atomic rename.
fn sibling_tmp(path: &Path) -> PathBuf {
    let mut p = path.to_path_buf();
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("hosts"));
    name.push(".portbay.tmp");
    p.set_file_name(name);
    p
}

/// Return the lines strictly between BEGIN…END, exclusive. Returns "" if
/// the block isn't present.
fn extract_block(contents: &str) -> &str {
    let begin_idx = match contents.find(BEGIN_MARKER) {
        Some(i) => i,
        None => return "",
    };
    let after_begin = match contents[begin_idx..].find('\n') {
        Some(rel) => begin_idx + rel + 1,
        None => return "",
    };
    let end_idx = match contents[after_begin..].find(END_MARKER) {
        Some(rel) => after_begin + rel,
        None => return "",
    };
    // Trim a single trailing newline if present so parse_entries doesn't
    // see an empty final line.
    let block = &contents[after_begin..end_idx];
    block.trim_end_matches('\n')
}

fn parse_entries(block: &str) -> Vec<HostsEntry> {
    let mut out = Vec::new();
    for raw in block.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let ip_str = match parts.next() {
            Some(s) => s,
            None => continue,
        };
        let host = match parts.next() {
            Some(s) => s,
            None => continue,
        };
        if let Ok(ip) = ip_str.parse::<Ipv4Addr>() {
            out.push(HostsEntry {
                ip,
                hostname: host.to_owned(),
            });
        }
    }
    out
}

fn render(existing: &str, entries: &[HostsEntry]) -> String {
    // Strip any existing PortBay block.
    let stripped = strip_block(existing);
    let mut out = stripped;

    // Trim trailing whitespace so the appended block lands cleanly.
    while out.ends_with('\n') {
        out.pop();
    }

    // No PortBay entries → just leave the file without a block.
    if entries.is_empty() {
        // Restore the final newline if there's any content.
        if !out.is_empty() {
            out.push('\n');
        }
        return out;
    }

    if !out.is_empty() {
        out.push('\n');
        out.push('\n');
    }
    out.push_str(BEGIN_MARKER);
    out.push('\n');
    for e in entries {
        out.push_str(&format!("{}\t{}\n", e.ip, e.hostname));
    }
    out.push_str(END_MARKER);
    out.push('\n');
    out
}

/// Remove the PortBay block from a hosts-file string, returning the rest
/// of the file untouched. If the block doesn't exist, the input is
/// returned as-is.
fn strip_block(contents: &str) -> String {
    let Some(begin_idx) = contents.find(BEGIN_MARKER) else {
        return contents.to_owned();
    };
    // Find the start of the line containing BEGIN_MARKER (so leading
    // whitespace / blank lines preceding the block don't get orphaned).
    let line_start = contents[..begin_idx]
        .rfind('\n')
        .map(|i| i + 1)
        .unwrap_or(0);

    let after_begin = begin_idx;
    let Some(end_rel) = contents[after_begin..].find(END_MARKER) else {
        // Malformed: BEGIN without END. Conservatively keep everything.
        return contents.to_owned();
    };
    let end_idx = after_begin + end_rel + END_MARKER.len();
    // Eat the newline immediately after END_MARKER, if any.
    let mut tail_start = end_idx;
    if contents[tail_start..].starts_with('\n') {
        tail_start += 1;
    }
    let mut out = String::with_capacity(contents.len());
    out.push_str(&contents[..line_start]);
    out.push_str(&contents[tail_start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loopback() -> Ipv4Addr {
        Ipv4Addr::new(127, 0, 0, 1)
    }

    fn tmpfile(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts");
        std::fs::write(&path, contents).unwrap();
        (dir, path)
    }

    #[test]
    fn list_managed_returns_empty_when_no_block() {
        let (_d, path) = tmpfile("127.0.0.1 localhost\n");
        let m = HostsManager::new(&path);
        assert!(m.list_managed().unwrap().is_empty());
    }

    #[test]
    fn list_managed_returns_empty_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope");
        let m = HostsManager::new(path);
        assert!(m.list_managed().unwrap().is_empty());
    }

    #[test]
    fn add_creates_block_when_missing() {
        let (_d, path) = tmpfile("127.0.0.1 localhost\n");
        let m = HostsManager::new(&path);
        m.add("marketing-site.test", loopback()).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains(BEGIN_MARKER));
        assert!(body.contains(END_MARKER));
        assert!(body.contains("marketing-site.test"));
        // Outside lines preserved verbatim.
        assert!(body.starts_with("127.0.0.1 localhost"));
    }

    #[test]
    fn add_is_idempotent() {
        let (_d, path) = tmpfile("");
        let m = HostsManager::new(&path);
        m.add("a.test", loopback()).unwrap();
        m.add("a.test", loopback()).unwrap();
        let list = m.list_managed().unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn remove_strips_one_entry_leaving_others() {
        let (_d, path) = tmpfile("");
        let m = HostsManager::new(&path);
        m.add("a.test", loopback()).unwrap();
        m.add("b.test", loopback()).unwrap();
        m.remove("a.test").unwrap();
        let list = m.list_managed().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].hostname, "b.test");
    }

    #[test]
    fn remove_missing_is_no_op() {
        let (_d, path) = tmpfile("127.0.0.1 localhost\n");
        let m = HostsManager::new(&path);
        m.remove("ghost.test").unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(!body.contains(BEGIN_MARKER));
    }

    #[test]
    fn clear_removes_block_keeps_other_lines() {
        let (_d, path) = tmpfile("127.0.0.1 localhost\n# user comment\n");
        let m = HostsManager::new(&path);
        m.add("a.test", loopback()).unwrap();
        m.clear().unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(!body.contains(BEGIN_MARKER));
        assert!(!body.contains(END_MARKER));
        assert!(body.contains("127.0.0.1 localhost"));
        assert!(body.contains("# user comment"));
    }

    #[test]
    fn external_edits_outside_block_are_preserved_through_a_write() {
        let initial = "127.0.0.1 localhost\n255.255.255.255 broadcasthost\n::1 localhost\n";
        let (_d, path) = tmpfile(initial);
        let m = HostsManager::new(&path);
        m.add("p.test", loopback()).unwrap();
        m.add("q.test", loopback()).unwrap();
        m.remove("p.test").unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        // All original lines still there in original order.
        for line in initial.lines() {
            assert!(body.contains(line), "missing: {line}\nbody: {body}");
        }
        // q.test present, p.test absent.
        assert!(body.contains("q.test"));
        assert!(!body.contains("p.test"));
    }

    #[test]
    fn replace_all_dedupes_and_sorts() {
        let (_d, path) = tmpfile("");
        let m = HostsManager::new(&path);
        m.replace_all(vec![
            ("b.test", loopback()),
            ("a.test", loopback()),
            ("a.test", loopback()), // dup
        ])
        .unwrap();
        let list = m.list_managed().unwrap();
        assert_eq!(list.len(), 2);
        // BTreeMap order → alphabetical by hostname.
        assert_eq!(list[0].hostname, "a.test");
        assert_eq!(list[1].hostname, "b.test");
    }

    #[test]
    fn invalid_hostnames_are_rejected() {
        let (_d, path) = tmpfile("");
        let m = HostsManager::new(&path);
        assert!(matches!(
            m.add("has space.test", loopback()),
            Err(HostsError::InvalidHostname(_))
        ));
        assert!(matches!(
            m.add("has#comment.test", loopback()),
            Err(HostsError::InvalidHostname(_))
        ));
        assert!(matches!(
            m.add("", loopback()),
            Err(HostsError::InvalidHostname(_))
        ));
    }

    #[test]
    fn permission_denied_surfaces_explicitly() {
        // /etc/hosts requires root. This test self-skips when running as
        // root (CI sometimes does) — in that case write will succeed.
        let m = HostsManager::system();
        match m.add("portbay-test-only.test", loopback()) {
            Err(HostsError::PermissionDenied { .. }) => { /* expected for non-root */ }
            Ok(()) => {
                // Running as root — clean up.
                let _ = m.remove("portbay-test-only.test");
            }
            Err(other) => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn render_keeps_single_trailing_newline() {
        let out = render("a\nb\n", &[]);
        assert_eq!(out, "a\nb\n");
        let out2 = render(
            "a\nb\n",
            &[HostsEntry {
                ip: loopback(),
                hostname: "x.test".into(),
            }],
        );
        // Should end in a single newline after the END marker.
        assert!(out2.ends_with("# END PortBay\n"));
        assert!(!out2.ends_with("\n\n"));
    }

    #[test]
    fn strip_block_handles_block_at_start_of_file() {
        let input = "# BEGIN PortBay\n127.0.0.1 a.test\n# END PortBay\n127.0.0.1 localhost\n";
        let stripped = strip_block(input);
        assert!(!stripped.contains(BEGIN_MARKER));
        assert!(stripped.contains("127.0.0.1 localhost"));
    }
}
