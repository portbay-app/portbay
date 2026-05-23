//! Small crate-wide helpers shared across modules.

/// Turn an arbitrary display string into a URL/ID-safe slug.
///
/// Lowercases ASCII alphanumerics, collapses every run of other characters
/// into a single `-`, and trims leading/trailing dashes. Non-ASCII letters
/// are dropped (we don't transliterate). This is the single source of truth
/// for ID generation — the CLI, the project commands, and the group commands
/// all call it, so they can't drift apart.
pub fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = true; // suppresses a leading dash
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

/// Deterministic 64-bit FNV-1a hash.
///
/// Used for reconciler cache keys ("has the generated config changed since
/// we last applied it?"). Unlike `std::collections::hash_map::DefaultHasher`,
/// the result is stable across Rust toolchain versions and platforms, so a
/// compiler bump can't silently invalidate every cache and trigger spurious
/// Caddy/dnsmasq restarts on the next launch.
pub fn stable_hash(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_lowercases_and_hyphenates() {
        assert_eq!(slugify("My Cool App"), "my-cool-app");
        assert_eq!(slugify("  Trim --- Me  "), "trim-me");
        assert_eq!(slugify("Already-Slugged"), "already-slugged");
    }

    #[test]
    fn slugify_collapses_runs_and_drops_non_ascii() {
        assert_eq!(slugify("a___b...c"), "a-b-c");
        assert_eq!(slugify("café résumé"), "caf-r-sum");
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn stable_hash_is_deterministic_and_distinguishes_input() {
        assert_eq!(stable_hash(b"portbay"), stable_hash(b"portbay"));
        assert_ne!(stable_hash(b"a"), stable_hash(b"b"));
        // Known FNV-1a anchor: empty input is the offset basis.
        assert_eq!(stable_hash(b""), 0xcbf2_9ce4_8422_2325);
    }
}
