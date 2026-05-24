//! Local developer-tool integrations surfaced through the GUI.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_shell::ShellExt;

use crate::commands::projects::load_registry;
use crate::error::{AppError, AppResult};
use crate::registry::ProjectId;
use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolKind {
    Editor,
    Agent,
    Terminal,
}

/// The custom URL schemes PortBay knows how to launch. A typed enum (rather
/// than a `&str`) makes `deep_link_url` and `scheme_is_available` exhaustive —
/// adding a variant is a compile error until both are updated, so a deep-link
/// can never reach an `unreachable!`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeepLinkScheme {
    ClaudeCli,
    Claude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchMode {
    Cli {
        cli: &'static str,
        fallback: Option<MacApp>,
    },
    MacApp(MacApp),
    DeepLink {
        scheme: DeepLinkScheme,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MacApp {
    app_names: &'static [&'static str],
    bundle_ids: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ToolDefinition {
    id: &'static str,
    label: &'static str,
    kind: ToolKind,
    launch: LaunchMode,
}

const TOOL_DEFINITIONS: &[ToolDefinition] = &[
    ToolDefinition {
        id: "vscode",
        label: "VS Code",
        kind: ToolKind::Editor,
        launch: LaunchMode::Cli {
            cli: "code",
            fallback: Some(MacApp {
                app_names: &["Visual Studio Code"],
                bundle_ids: &["com.microsoft.VSCode"],
            }),
        },
    },
    ToolDefinition {
        id: "cursor",
        label: "Cursor",
        kind: ToolKind::Editor,
        launch: LaunchMode::Cli {
            cli: "cursor",
            fallback: Some(MacApp {
                app_names: &["Cursor"],
                bundle_ids: &["com.todesktop.230313mzl4w4u92"],
            }),
        },
    },
    ToolDefinition {
        id: "phpstorm",
        label: "PHPStorm",
        kind: ToolKind::Editor,
        launch: LaunchMode::Cli {
            cli: "phpstorm",
            fallback: Some(MacApp {
                app_names: &["PhpStorm", "PHPStorm"],
                bundle_ids: &["com.jetbrains.PhpStorm"],
            }),
        },
    },
    ToolDefinition {
        id: "sublime",
        label: "Sublime Text",
        kind: ToolKind::Editor,
        launch: LaunchMode::Cli {
            cli: "subl",
            fallback: Some(MacApp {
                app_names: &["Sublime Text"],
                bundle_ids: &["com.sublimetext.4"],
            }),
        },
    },
    ToolDefinition {
        id: "zed",
        label: "Zed",
        kind: ToolKind::Editor,
        launch: LaunchMode::Cli {
            cli: "zed",
            fallback: Some(MacApp {
                app_names: &["Zed"],
                bundle_ids: &["dev.zed.Zed"],
            }),
        },
    },
    ToolDefinition {
        id: "claude-code",
        label: "Claude Code",
        kind: ToolKind::Agent,
        launch: LaunchMode::DeepLink {
            scheme: DeepLinkScheme::ClaudeCli,
        },
    },
    ToolDefinition {
        id: "claude-desktop",
        label: "Claude Desktop",
        kind: ToolKind::Agent,
        launch: LaunchMode::DeepLink {
            scheme: DeepLinkScheme::Claude,
        },
    },
    ToolDefinition {
        id: "codex",
        label: "Codex",
        kind: ToolKind::Agent,
        launch: LaunchMode::Cli {
            cli: "codex",
            fallback: Some(MacApp {
                app_names: &["Codex"],
                bundle_ids: &["com.openai.codex"],
            }),
        },
    },
    ToolDefinition {
        id: "antigravity",
        label: "Antigravity",
        kind: ToolKind::Agent,
        launch: LaunchMode::MacApp(MacApp {
            app_names: &["Antigravity", "Antigravity IDE"],
            bundle_ids: &["com.google.antigravity"],
        }),
    },
    ToolDefinition {
        id: "warp",
        label: "Warp",
        kind: ToolKind::Terminal,
        launch: LaunchMode::MacApp(MacApp {
            app_names: &["Warp"],
            bundle_ids: &["dev.warp.Warp-Stable"],
        }),
    },
    ToolDefinition {
        id: "iterm",
        label: "iTerm",
        kind: ToolKind::Terminal,
        launch: LaunchMode::MacApp(MacApp {
            app_names: &["iTerm"],
            bundle_ids: &["com.googlecode.iterm2"],
        }),
    },
    // macOS Terminal.app — ships with the OS, always available. Listed
    // last so user-installed terminals take precedence when both are
    // present, but it's still surfaced as the system default when
    // nothing else is detected.
    ToolDefinition {
        id: "terminal",
        label: "Terminal",
        kind: ToolKind::Terminal,
        launch: LaunchMode::MacApp(MacApp {
            app_names: &["Terminal"],
            bundle_ids: &["com.apple.Terminal"],
        }),
    },
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DevToolInfo {
    pub id: String,
    pub label: String,
    pub kind: String,
}

/// `installed_dev_tools()` — ordered list of supported local tools.
#[tauri::command]
pub async fn installed_dev_tools() -> AppResult<Vec<DevToolInfo>> {
    Ok(detect_installed_dev_tools())
}

/// `open_in_ide(id, ide)` — open a project folder in an editor, agent, or terminal.
#[tauri::command]
pub async fn open_in_ide(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    ide: String,
) -> AppResult<()> {
    let registry = load_registry(&state)?;
    let project = registry
        .get_project(&ProjectId::new(id.clone()))
        .ok_or_else(|| AppError::NotFound(id))?;

    let definition = tool_definition(&ide)
        .ok_or_else(|| AppError::BadInput(format!("unsupported tool: {ide}")))?;

    match definition.launch {
        LaunchMode::Cli { cli, fallback } => {
            if which::which(cli).is_ok() {
                app.shell()
                    .command(cli)
                    .arg(&project.path)
                    .spawn()
                    .map_err(|e| {
                        AppError::Internal(format!("failed to open {}: {e}", definition.label))
                    })?;
            } else if let Some(mac_app) = fallback {
                open_mac_app(&app, mac_app, &project.path, definition.label)?;
            } else {
                return Err(AppError::BadInput(format!(
                    "{} CLI not found on PATH",
                    definition.label
                )));
            }
        }
        LaunchMode::MacApp(mac_app) => {
            open_mac_app(&app, mac_app, &project.path, definition.label)?;
        }
        LaunchMode::DeepLink { scheme } => {
            let url = deep_link_url(scheme, &project.path.to_string_lossy());
            app.shell().command("open").arg(url).spawn().map_err(|e| {
                AppError::Internal(format!("failed to open {}: {e}", definition.label))
            })?;
        }
    }

    Ok(())
}

fn detect_installed_dev_tools() -> Vec<DevToolInfo> {
    detect_installed_dev_tools_with(
        |cli| which::which(cli).is_ok(),
        |app| resolve_mac_app(app).is_some(),
        scheme_is_available,
    )
}

fn detect_installed_dev_tools_with(
    mut cli_exists: impl FnMut(&str) -> bool,
    mut app_exists: impl FnMut(MacApp) -> bool,
    mut scheme_exists: impl FnMut(DeepLinkScheme) -> bool,
) -> Vec<DevToolInfo> {
    TOOL_DEFINITIONS
        .iter()
        .filter(|definition| match definition.launch {
            LaunchMode::Cli { cli, fallback } => {
                cli_exists(cli) || fallback.map(&mut app_exists).unwrap_or(false)
            }
            LaunchMode::MacApp(mac_app) => app_exists(mac_app),
            LaunchMode::DeepLink { scheme } => scheme_exists(scheme),
        })
        .map(|definition| DevToolInfo {
            id: definition.id.to_string(),
            label: definition.label.to_string(),
            kind: match definition.kind {
                ToolKind::Editor => "editor",
                ToolKind::Agent => "agent",
                ToolKind::Terminal => "terminal",
            }
            .to_string(),
        })
        .collect()
}

fn tool_definition(id: &str) -> Option<ToolDefinition> {
    TOOL_DEFINITIONS
        .iter()
        .copied()
        .find(|definition| definition.id == id)
}

fn scheme_is_available(scheme: DeepLinkScheme) -> bool {
    match scheme {
        DeepLinkScheme::ClaudeCli => {
            which::which("claude").is_ok()
                || dirs::home_dir()
                    .map(|home| {
                        home.join("Applications/Claude Code URL Handler.app")
                            .exists()
                    })
                    .unwrap_or(false)
        }
        DeepLinkScheme::Claude => resolve_mac_app(MacApp {
            app_names: &["Claude"],
            bundle_ids: &["com.anthropic.claudefordesktop"],
        })
        .is_some(),
    }
}

fn open_mac_app(app: &AppHandle, mac_app: MacApp, path: &Path, label: &str) -> AppResult<()> {
    let mut command = app.shell().command("open");
    if let Some(bundle_path) = resolve_mac_app(mac_app) {
        command = command.arg("-a").arg(bundle_path);
    } else {
        command = command.args(["-a", mac_app.app_names[0]]);
    }
    command
        .arg(path)
        .spawn()
        .map_err(|e| AppError::Internal(format!("failed to open {label}: {e}")))?;
    Ok(())
}

fn resolve_mac_app(app: MacApp) -> Option<PathBuf> {
    find_app_by_bundle_id(app.bundle_ids).or_else(|| find_app_by_name(app.app_names))
}

fn find_app_by_bundle_id(bundle_ids: &[&str]) -> Option<PathBuf> {
    bundle_ids.iter().find_map(|bundle_id| {
        let output = std::process::Command::new("mdfind")
            .arg(format!("kMDItemCFBundleIdentifier == '{bundle_id}'"))
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(PathBuf::from)
            .find(|path| path.extension().is_some_and(|ext| ext == "app"))
    })
}

fn find_app_by_name(app_names: &[&str]) -> Option<PathBuf> {
    standard_app_dirs().into_iter().find_map(|dir| {
        app_names
            .iter()
            .map(|name| dir.join(format!("{name}.app")))
            .find(|path| path.exists())
    })
}

fn standard_app_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/Applications/Utilities"),
        PathBuf::from("/System/Applications"),
        // Terminal.app lives here on modern macOS — needed for the
        // built-in macOS Terminal fallback in TOOL_DEFINITIONS.
        PathBuf::from("/System/Applications/Utilities"),
    ];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join("Applications"));
    }
    if let Ok(extra) = std::env::var("PORTBAY_EXTRA_APP_DIR") {
        dirs.push(PathBuf::from(extra));
    }
    dirs
}

fn deep_link_url(scheme: DeepLinkScheme, path: &str) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    match scheme {
        DeepLinkScheme::ClaudeCli => {
            query.append_pair("cwd", path);
            format!("claude-cli://open?{}", query.finish())
        }
        DeepLinkScheme::Claude => {
            query.append_pair("folder", path);
            format!("claude://code/new?{}", query.finish())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_tools_in_preferred_order() {
        let found = detect_installed_dev_tools_with(
            |cli| matches!(cli, "cursor" | "code" | "subl"),
            |app| app.app_names.contains(&"Warp"),
            |scheme| scheme == DeepLinkScheme::ClaudeCli,
        );
        let ids: Vec<&str> = found.iter().map(|tool| tool.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["vscode", "cursor", "sublime", "claude-code", "warp"]
        );
    }

    #[test]
    fn detects_no_tools_when_no_launcher_is_present() {
        let found = detect_installed_dev_tools_with(|_| false, |_| false, |_| false);
        assert!(found.is_empty());
    }

    #[test]
    fn cli_tools_can_be_detected_from_mac_app_fallbacks() {
        let found = detect_installed_dev_tools_with(
            |_| false,
            |app| {
                app.app_names
                    .iter()
                    .any(|name| matches!(*name, "Visual Studio Code" | "Cursor" | "Codex"))
            },
            |_| false,
        );
        let ids: Vec<&str> = found.iter().map(|tool| tool.id.as_str()).collect();
        assert_eq!(ids, vec!["vscode", "cursor", "codex"]);
    }

    #[test]
    fn standard_app_dirs_include_user_applications() {
        let dirs = standard_app_dirs();
        assert!(dirs
            .iter()
            .any(|dir| dir == &PathBuf::from("/Applications")));
        assert!(dirs.iter().any(|dir| dir.ends_with("Applications")));
    }

    #[test]
    fn rejects_unknown_tool_ids() {
        assert!(tool_definition("vim").is_none());
    }

    #[test]
    fn builds_claude_code_deep_link_with_encoded_cwd() {
        let url = deep_link_url(DeepLinkScheme::ClaudeCli, "/Users/me/My Project");
        assert_eq!(url, "claude-cli://open?cwd=%2FUsers%2Fme%2FMy+Project");
    }

    #[test]
    fn builds_claude_desktop_code_link_with_encoded_folder() {
        let url = deep_link_url(DeepLinkScheme::Claude, "/Users/me/My Project");
        assert_eq!(url, "claude://code/new?folder=%2FUsers%2Fme%2FMy+Project");
    }
}
