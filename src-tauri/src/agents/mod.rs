//! Public agent-kind registry — shared between the board engine, the CLI, and
//! any future module that needs to name an AI coding agent without pulling in
//! the per-project board machinery.
//!
//! This module deliberately has **no dependency on `crate::context`**. It is the
//! source of truth for the `AgentKind` enum and its wire-format representation.
//! Board-specific behaviour (e.g. which adapter file an agent loads, spawning an
//! agent process for a task) lives in `crate::context::launchers` and
//! `crate::context::config`, which may import from here.

use serde::{Deserialize, Serialize};

/// Which external AI coding agent PortBay can dispatch to.
///
/// Serde wire format: `#[serde(rename_all = "lowercase")]` — variant names are
/// all lowercase on the wire. **Do not rename variants or change the rename
/// rule** — this value is persisted in `config.json`, `preferences.json`, and
/// IPC responses; changing it would silently break existing configs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum AgentKind {
    #[default]
    Claude,
    Codex,
    Cursor,
    Gemini,
    Aider,
    OpenCode,
    Amp,
    Qwen,
    Copilot,
    Antigravity,
    Custom,
}

impl AgentKind {
    /// Every dispatchable agent, in display order.
    pub const ALL: [AgentKind; 11] = [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::Cursor,
        AgentKind::Gemini,
        AgentKind::Aider,
        AgentKind::OpenCode,
        AgentKind::Amp,
        AgentKind::Qwen,
        AgentKind::Copilot,
        AgentKind::Antigravity,
        AgentKind::Custom,
    ];

    /// The stable, lowercase, wire-format id for this agent.
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Cursor => "cursor",
            AgentKind::Gemini => "gemini",
            AgentKind::Aider => "aider",
            AgentKind::OpenCode => "opencode",
            AgentKind::Amp => "amp",
            AgentKind::Qwen => "qwen",
            AgentKind::Copilot => "copilot",
            AgentKind::Antigravity => "antigravity",
            AgentKind::Custom => "custom",
        }
    }

    /// Human-facing label for the agent picker.
    pub fn label(&self) -> &'static str {
        match self {
            AgentKind::Claude => "Claude Code",
            AgentKind::Codex => "Codex",
            AgentKind::Cursor => "Cursor",
            AgentKind::Gemini => "Gemini",
            AgentKind::Aider => "Aider",
            AgentKind::OpenCode => "OpenCode",
            AgentKind::Amp => "Amp",
            AgentKind::Qwen => "Qwen Code",
            AgentKind::Copilot => "Copilot CLI",
            AgentKind::Antigravity => "Antigravity",
            AgentKind::Custom => "Custom",
        }
    }

    /// Parse a case-insensitive agent id string into an `AgentKind`.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "claude" => AgentKind::Claude,
            "codex" => AgentKind::Codex,
            "cursor" => AgentKind::Cursor,
            "gemini" => AgentKind::Gemini,
            "aider" => AgentKind::Aider,
            "opencode" => AgentKind::OpenCode,
            "amp" => AgentKind::Amp,
            "qwen" => AgentKind::Qwen,
            "copilot" => AgentKind::Copilot,
            "antigravity" | "gravity" => AgentKind::Antigravity,
            "custom" => AgentKind::Custom,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_has_correct_count() {
        assert_eq!(AgentKind::ALL.len(), 11);
    }

    #[test]
    fn as_str_round_trips_through_parse() {
        for kind in AgentKind::ALL {
            assert_eq!(AgentKind::parse(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn parse_is_case_insensitive() {
        assert_eq!(AgentKind::parse("CLAUDE"), Some(AgentKind::Claude));
        assert_eq!(AgentKind::parse("Gemini"), Some(AgentKind::Gemini));
    }

    #[test]
    fn parse_antigravity_alias() {
        assert_eq!(AgentKind::parse("gravity"), Some(AgentKind::Antigravity));
        assert_eq!(
            AgentKind::parse("antigravity"),
            Some(AgentKind::Antigravity)
        );
    }

    #[test]
    fn parse_unknown_returns_none() {
        assert!(AgentKind::parse("nonexistent").is_none());
    }

    #[test]
    fn serde_wire_format_is_lowercase() {
        // Wire format must not change — configs persisted to disk rely on this.
        let s = serde_json::to_string(&AgentKind::Claude).unwrap();
        assert_eq!(s, r#""claude""#);
        let s = serde_json::to_string(&AgentKind::OpenCode).unwrap();
        assert_eq!(s, r#""opencode""#);
        let s = serde_json::to_string(&AgentKind::Antigravity).unwrap();
        assert_eq!(s, r#""antigravity""#);
    }

    #[test]
    fn default_is_claude() {
        assert_eq!(AgentKind::default(), AgentKind::Claude);
    }
}
