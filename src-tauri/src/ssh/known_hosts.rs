//! Minimal `~/.ssh/known_hosts` editing for the "reset host trust" action.
//!
//! The connect/probe paths use `russh::keys` for trust-on-first-use; this module
//! adds the one mutating operation they don't: removing a host's recorded key
//! (the GUI equivalent of `ssh-keygen -R host`), so a user who sees a "key
//! changed" warning — or just wants to forget a host — can clear the stale entry
//! and re-establish trust on the next connect.
//!
//! Matches all three line forms OpenSSH writes: a plain hostname, a
//! comma-separated host list, and the hashed `|1|salt|hash` form (via HMAC-SHA1,
//! exactly as OpenSSH computes it). Non-default ports are keyed `[host]:port`.

use std::path::PathBuf;

use base64::Engine;
use hmac::{Hmac, Mac};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

/// Path to the user's `known_hosts`, or `None` if HOME can't be resolved.
fn known_hosts_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".ssh").join("known_hosts"))
}

/// The name OpenSSH stores for a host: bare for port 22, `[host]:port` otherwise.
pub(crate) fn host_entry_name(host: &str, port: u16) -> String {
    host_key(host, port)
}

fn host_key(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    }
}

/// Does a hashed host field (`|1|b64salt|b64hash`) match `name`?
fn hashed_matches(field: &str, name: &str) -> bool {
    let parts: Vec<&str> = field.split('|').collect();
    // Empty first segment from the leading '|': ["", "1", salt, hash].
    if parts.len() != 4 || parts[1] != "1" {
        return false;
    }
    let engine = base64::engine::general_purpose::STANDARD;
    let (Ok(salt), Ok(expected)) = (engine.decode(parts[2]), engine.decode(parts[3])) else {
        return false;
    };
    let Ok(mut mac) = HmacSha1::new_from_slice(&salt) else {
        return false;
    };
    mac.update(name.as_bytes());
    mac.verify_slice(&expected).is_ok()
}

/// Does one known_hosts line refer to `name`? Handles `@cert-authority`/`@revoked`
/// markers, comma host-lists, and hashed entries.
fn line_matches(line: &str, name: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return false;
    }
    let mut fields = trimmed.split_whitespace();
    let mut first = match fields.next() {
        Some(f) => f,
        None => return false,
    };
    // A marker line (`@cert-authority host …`) puts the host in the 2nd field.
    if first.starts_with('@') {
        first = match fields.next() {
            Some(f) => f,
            None => return false,
        };
    }
    if first.starts_with("|1|") {
        return hashed_matches(first, name);
    }
    first.split(',').any(|pat| pat == name)
}

/// The SHA256 fingerprint (`SHA256:…`) of the key recorded for `host`/`port`
/// whose algorithm matches `key_type` (e.g. `ssh-ed25519`), if any. Used to show
/// the previously-trusted key next to a changed one in the accept dialog.
/// Best-effort: returns `None` when the file, a matching entry, or the stored
/// key can't be read or parsed — the UI then just omits the comparison.
pub fn stored_fingerprint(host: &str, port: u16, key_type: &str) -> Option<String> {
    let path = known_hosts_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    let name = host_key(host, port);
    for line in content.lines() {
        if !line_matches(line, &name) {
            continue;
        }
        // Fields are `[@marker] host(s) keytype base64 [comment]`; pull the key
        // type + its base64 blob, skipping a leading `@cert-authority`/`@revoked`
        // marker (which shifts the host into the second field).
        let mut fields = line.split_whitespace();
        let first = fields.next()?;
        if first.starts_with('@') {
            let _host = fields.next()?;
        }
        let kt = fields.next()?;
        let b64 = fields.next()?;
        if kt != key_type {
            continue;
        }
        if let Ok(key) = russh::keys::parse_public_key_base64(b64) {
            return Some(key.fingerprint(russh::keys::HashAlg::Sha256).to_string());
        }
    }
    None
}

/// Append a trusted-key line for `host`/`port` in the standard OpenSSH format,
/// creating `~/.ssh/known_hosts` (mode 0600) if it doesn't exist yet. Used by
/// the system-ssh trust pre-flight's Trust & Save so both OpenSSH and russh
/// read the entry back on the next connect.
pub fn append_host(host: &str, port: u16, key_type: &str, key_base64: &str) -> std::io::Result<()> {
    let Some(path) = known_hosts_path() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "cannot resolve HOME to locate known_hosts",
        ));
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&path)?;
    writeln!(file, "{} {} {}", host_key(host, port), key_type, key_base64)
}

/// Remove every `known_hosts` entry for `host`/`port`. Returns the number of
/// lines removed (0 if the host wasn't present or the file doesn't exist).
pub fn remove_host(host: &str, port: u16) -> std::io::Result<usize> {
    let Some(path) = known_hosts_path() else {
        return Ok(0);
    };
    remove_host_at(&path, &host_key(host, port))
}

/// The rewrite behind [`remove_host`], split out so the atomic-write behaviour
/// is unit-testable against a temp file. Writes a sibling tempfile and renames
/// over the original — a crash mid-write can never truncate `known_hosts` —
/// and sets the rewritten file to 0600 (matching [`append_host`]).
fn remove_host_at(path: &std::path::Path, name: &str) -> std::io::Result<usize> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };

    let mut removed = 0usize;
    let kept: Vec<&str> = content
        .lines()
        .filter(|line| {
            let hit = line_matches(line, name);
            if hit {
                removed += 1;
            }
            !hit
        })
        .collect();

    if removed > 0 {
        let mut out = kept.join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        let tmp = path.with_extension("portbay-tmp");
        std::fs::write(&tmp, out)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, path)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_plain_and_bracketed() {
        assert!(line_matches("example.com ssh-ed25519 AAAA", "example.com"));
        assert!(line_matches(
            "[example.com]:2222 ssh-rsa AAAA",
            "[example.com]:2222"
        ));
        assert!(!line_matches("other.com ssh-rsa AAAA", "example.com"));
    }

    #[test]
    fn matches_comma_list_and_markers() {
        assert!(line_matches("a.com,b.com ssh-rsa AAAA", "b.com"));
        assert!(line_matches(
            "@cert-authority *.example.com ssh-rsa AAAA",
            "*.example.com"
        ));
        assert!(!line_matches("# a comment", "a.com"));
    }

    /// Pins the P2-2 fix from the 2026-06-10 assessment: removing a host
    /// rewrites via tmp→rename (never a bare truncating write) and leaves the
    /// file owner-only, like `append_host` creates it.
    #[test]
    fn remove_rewrites_atomically_with_owner_only_perms() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        std::fs::write(
            &path,
            "example.com ssh-ed25519 AAAA\nkeep.com ssh-ed25519 BBBB\n",
        )
        .unwrap();

        let removed = remove_host_at(&path, "example.com").unwrap();
        assert_eq!(removed, 1);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "keep.com ssh-ed25519 BBBB\n"
        );
        // No leftover tempfile.
        assert!(!path.with_extension("portbay-tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        // Missing file and missing host are both clean no-ops.
        assert_eq!(remove_host_at(&path, "absent.com").unwrap(), 0);
        assert_eq!(
            remove_host_at(&dir.path().join("nope"), "example.com").unwrap(),
            0
        );
    }

    #[test]
    fn matches_hashed_entry() {
        // A hashed line OpenSSH would write for "example.com": compute it the
        // same way and confirm the matcher accepts it.
        use base64::engine::general_purpose::STANDARD;
        let salt = b"0123456789abcdef0123"; // 20-byte SHA1 block-ish salt
        let mut mac = HmacSha1::new_from_slice(salt).unwrap();
        mac.update(b"example.com");
        let hash = mac.finalize().into_bytes();
        let field = format!("|1|{}|{}", STANDARD.encode(salt), STANDARD.encode(hash));
        assert!(hashed_matches(&field, "example.com"));
        assert!(!hashed_matches(&field, "evil.com"));
    }
}
