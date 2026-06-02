//! Parse `~/.ssh/config` into importable SSH connection candidates.
//!
//! This is a deliberately small, presentation-only parser: it models simple
//! `Host` stanzas (the directives PortBay's connection model actually stores —
//! `HostName`, `Port`, `User`, `IdentityFile`, `ProxyJump`) and nothing else.
//! It does **not** implement OpenSSH's full matching semantics: directives in a
//! wildcard `Host *` defaults block are not inherited into other hosts, and
//! `Match` blocks are ignored. Wildcard/negation `Host` patterns are *flagged*
//! (not silently dropped) so the import UI can show why they weren't imported.
//!
//! The candidates are never written directly — the user picks from them and the
//! command layer saves the picks through the normal `ssh_connection_save` path,
//! which assigns a fresh, collision-free id. Import therefore can never
//! overwrite an existing connection.

/// One importable host parsed from an OpenSSH `config` file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfigCandidate {
    /// The `Host` alias — the first pattern on the `Host` line. Used as the
    /// suggested connection name.
    pub host_alias: String,
    /// `HostName`, falling back to the alias when the stanza omits it.
    pub ssh_host: String,
    /// `Port`, defaulting to 22.
    pub ssh_port: u16,
    /// `User`, empty when the stanza omits it.
    pub ssh_user: String,
    /// `IdentityFile` (raw — a leading `~` is preserved and expanded at auth
    /// time, matching how saved connections store key paths).
    pub key_path: Option<String>,
    /// `ProxyJump` target, when set.
    pub proxy_jump: Option<String>,
    /// The `Host` line is a wildcard/negation pattern (`*`, `?`, `!`) — an
    /// OpenSSH defaults block, not a concrete host. Flagged, not importable.
    pub wildcard: bool,
    /// A saved connection already uses this alias's id. Importing still creates
    /// a new, suffixed connection (never an overwrite); the UI warns instead.
    /// Set by the command layer against the live registry — the pure parser
    /// always leaves this `false`.
    pub already_exists: bool,
}

/// Parse an OpenSSH `config` file body into host candidates, in file order.
pub fn parse_ssh_config(input: &str) -> Vec<SshConfigCandidate> {
    let mut candidates = Vec::new();
    let mut current: Option<Builder> = None;

    for raw_line in input.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        let Some((keyword, value)) = split_directive(line) else {
            continue;
        };
        match keyword.to_ascii_lowercase().as_str() {
            "host" => {
                if let Some(builder) = current.take() {
                    candidates.push(builder.finish());
                }
                current = Some(Builder::new(value));
            }
            // A `Match` block ends the current host stanza; its directives are
            // conditional and not modelled here, so close out and ignore them.
            "match" => {
                if let Some(builder) = current.take() {
                    candidates.push(builder.finish());
                }
            }
            other => {
                // Directives before the first `Host` (global defaults) have no
                // stanza to attach to — skip them.
                let Some(builder) = current.as_mut() else {
                    continue;
                };
                match other {
                    "hostname" => builder.ssh_host = Some(unquote(value)),
                    "port" => {
                        if let Ok(port) = unquote(value).parse::<u16>() {
                            builder.ssh_port = port;
                        }
                    }
                    "user" => builder.ssh_user = Some(unquote(value)),
                    // OpenSSH uses the *first* IdentityFile; keep that behaviour
                    // (a later one falls through the guard to the no-op arm).
                    "identityfile" if builder.key_path.is_none() => {
                        builder.key_path = Some(unquote(value));
                    }
                    "proxyjump" => builder.proxy_jump = Some(unquote(value)),
                    _ => {}
                }
            }
        }
    }
    if let Some(builder) = current.take() {
        candidates.push(builder.finish());
    }
    candidates
}

/// Accumulates one `Host` stanza's directives before it's frozen into a
/// [`SshConfigCandidate`].
struct Builder {
    alias_line: String,
    ssh_host: Option<String>,
    ssh_port: u16,
    ssh_user: Option<String>,
    key_path: Option<String>,
    proxy_jump: Option<String>,
}

impl Builder {
    fn new(alias_line: &str) -> Self {
        Self {
            alias_line: alias_line.to_string(),
            ssh_host: None,
            ssh_port: 22,
            ssh_user: None,
            key_path: None,
            proxy_jump: None,
        }
    }

    fn finish(self) -> SshConfigCandidate {
        let patterns: Vec<&str> = self.alias_line.split_whitespace().collect();
        let wildcard = patterns.iter().any(|p| is_wildcard_pattern(p));
        let alias = patterns.first().copied().unwrap_or_default().to_string();
        let ssh_host = self
            .ssh_host
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| alias.clone());
        SshConfigCandidate {
            host_alias: alias,
            ssh_host,
            ssh_port: self.ssh_port,
            ssh_user: self.ssh_user.unwrap_or_default(),
            key_path: self.key_path,
            proxy_jump: self.proxy_jump,
            wildcard,
            already_exists: false,
        }
    }
}

/// An OpenSSH `Host` pattern that matches many hosts rather than naming one:
/// glob wildcards (`*`, `?`) or a negation (`!`).
fn is_wildcard_pattern(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.starts_with('!')
}

/// OpenSSH treats only whole lines whose first non-blank character is `#` as
/// comments; a `#` inside a value is literal (paths can contain one).
fn strip_comment(line: &str) -> &str {
    if line.trim_start().starts_with('#') {
        ""
    } else {
        line
    }
}

/// Split a config line into `(keyword, value)`. OpenSSH accepts both
/// `Keyword value` and `Keyword=value` (with optional spaces around `=`).
/// Returns `None` for a keyword with no value.
fn split_directive(line: &str) -> Option<(&str, &str)> {
    let end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    if end == 0 {
        return None;
    }
    let keyword = &line[..end];
    let value = line[end..]
        .trim_start_matches(|c: char| c.is_whitespace() || c == '=')
        .trim();
    if value.is_empty() {
        None
    } else {
        Some((keyword, value))
    }
}

/// Strip one layer of surrounding double quotes from a value, if present.
fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Multiple hosts, an alias with HostName, an IdentityFile, a ProxyJump,
    // a `Port=` form, a quoted path, and a wildcard defaults block.
    const FIXTURE: &str = r#"
# Personal hosts
Host bastion
    HostName bastion.example.com
    User deploy
    Port 2222
    IdentityFile ~/.ssh/id_ed25519

Host db
    HostName 10.0.0.5
    User postgres
    ProxyJump bastion

Host vps
    HostName=vps.example.net

Host quoted
    HostName quoted.example.com
    IdentityFile "~/.ssh/my key"

Host *.internal *
    User ubuntu
    IdentityFile ~/.ssh/internal_key
"#;

    #[test]
    fn parses_each_host_stanza() {
        let candidates = parse_ssh_config(FIXTURE);
        assert_eq!(candidates.len(), 5);
        let aliases: Vec<&str> = candidates.iter().map(|c| c.host_alias.as_str()).collect();
        assert_eq!(
            aliases,
            vec!["bastion", "db", "vps", "quoted", "*.internal"]
        );
    }

    #[test]
    fn parses_hostname_port_user_and_identityfile() {
        let bastion = &parse_ssh_config(FIXTURE)[0];
        assert_eq!(bastion.host_alias, "bastion");
        assert_eq!(bastion.ssh_host, "bastion.example.com");
        assert_eq!(bastion.ssh_user, "deploy");
        assert_eq!(bastion.ssh_port, 2222);
        assert_eq!(bastion.key_path.as_deref(), Some("~/.ssh/id_ed25519"));
        assert_eq!(bastion.proxy_jump, None);
        assert!(!bastion.wildcard);
    }

    #[test]
    fn parses_proxyjump_and_defaults_port_to_22() {
        let db = &parse_ssh_config(FIXTURE)[1];
        assert_eq!(db.ssh_host, "10.0.0.5");
        assert_eq!(db.ssh_user, "postgres");
        assert_eq!(db.ssh_port, 22);
        assert_eq!(db.proxy_jump.as_deref(), Some("bastion"));
    }

    #[test]
    fn accepts_equals_form_for_directives() {
        let vps = &parse_ssh_config(FIXTURE)[2];
        assert_eq!(vps.ssh_host, "vps.example.net");
        assert_eq!(vps.ssh_user, "");
        assert_eq!(vps.ssh_port, 22);
    }

    #[test]
    fn unquotes_identityfile_paths() {
        let quoted = &parse_ssh_config(FIXTURE)[3];
        assert_eq!(quoted.key_path.as_deref(), Some("~/.ssh/my key"));
    }

    #[test]
    fn flags_wildcard_host_patterns() {
        let wild = &parse_ssh_config(FIXTURE)[4];
        assert!(wild.wildcard, "a `*` pattern must be flagged, not imported");
        assert_eq!(wild.host_alias, "*.internal");
    }

    #[test]
    fn hostname_falls_back_to_alias_when_omitted() {
        let candidates = parse_ssh_config("Host plain-alias\n    User root\n");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].ssh_host, "plain-alias");
        assert_eq!(candidates[0].ssh_user, "root");
    }

    #[test]
    fn ignores_match_blocks_and_global_defaults() {
        let input =
            "User globaluser\nHost a\n  HostName a.example.com\nMatch host b\n  User someone\n";
        let candidates = parse_ssh_config(input);
        // Only the single `Host a` stanza becomes a candidate; the leading
        // global `User` and the `Match` block contribute nothing.
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].host_alias, "a");
        assert_eq!(candidates[0].ssh_host, "a.example.com");
        assert_eq!(candidates[0].ssh_user, "");
    }

    #[test]
    fn keeps_only_the_first_identityfile() {
        let input = "Host multi\n  IdentityFile ~/.ssh/first\n  IdentityFile ~/.ssh/second\n";
        let candidates = parse_ssh_config(input);
        assert_eq!(candidates[0].key_path.as_deref(), Some("~/.ssh/first"));
    }
}
