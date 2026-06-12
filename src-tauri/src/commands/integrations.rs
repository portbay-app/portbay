//! Local developer-tool integrations surfaced through the GUI.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
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
    FileManager,
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
        id: "xcode",
        label: "Xcode",
        kind: ToolKind::Editor,
        launch: LaunchMode::Cli {
            cli: "xed",
            fallback: Some(MacApp {
                app_names: &["Xcode"],
                bundle_ids: &["com.apple.dt.Xcode"],
            }),
        },
    },
    ToolDefinition {
        id: "android-studio",
        label: "Android Studio",
        kind: ToolKind::Editor,
        launch: LaunchMode::Cli {
            cli: "studio",
            fallback: Some(MacApp {
                app_names: &["Android Studio"],
                bundle_ids: &["com.google.android.studio"],
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
        id: "ghostty",
        label: "Ghostty",
        kind: ToolKind::Terminal,
        launch: LaunchMode::MacApp(MacApp {
            app_names: &["Ghostty"],
            bundle_ids: &["com.mitchellh.ghostty"],
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
    // Finder is always present on macOS. Listed as a file manager so
    // it stays in its own section rather than cluttering the editor or
    // terminal lists.
    ToolDefinition {
        id: "finder",
        label: "Finder",
        kind: ToolKind::FileManager,
        launch: LaunchMode::MacApp(MacApp {
            app_names: &["Finder"],
            bundle_ids: &["com.apple.finder"],
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
    // Detection shells out to `mdfind` (Spotlight, 200ms–1s) per bundle-id
    // probe across ~14 tool definitions — blocking work that would starve
    // the shared async workers if run inline.
    tokio::task::spawn_blocking(detect_installed_dev_tools)
        .await
        .map_err(|e| AppError::Internal(format!("dev-tool detection task failed: {e}")))
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
                ToolKind::FileManager => "file-manager",
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

/// Resolve a terminal tool id (`warp` / `iterm` / `ghostty` / `terminal`) to its
/// installed `.app` bundle path. Used by the agent dispatcher to host an
/// interactive run in the user's preferred terminal. Returns `None` for an
/// unknown id, a non-terminal id, or a terminal that isn't installed.
/// Only the tasks-gated agent dispatcher (`crate::context`) calls this.
#[cfg(feature = "tasks")]
pub(crate) fn resolve_terminal_app(id: &str) -> Option<PathBuf> {
    let def = tool_definition(id)?;
    if def.kind != ToolKind::Terminal {
        return None;
    }
    match def.launch {
        LaunchMode::MacApp(app) => resolve_mac_app(app),
        _ => None,
    }
}

/// The first detected terminal id, in `TOOL_DEFINITIONS` order (user-installed
/// terminals before the always-present macOS Terminal.app). Used as the default
/// when the user hasn't picked a preferred terminal yet.
/// Only the tasks-gated agent dispatcher (`crate::context`) calls this.
#[cfg(feature = "tasks")]
pub(crate) fn first_detected_terminal() -> Option<String> {
    TOOL_DEFINITIONS
        .iter()
        .filter(|d| d.kind == ToolKind::Terminal)
        .find(|d| match d.launch {
            LaunchMode::MacApp(app) => resolve_mac_app(app).is_some(),
            _ => false,
        })
        .map(|d| d.id.to_string())
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

/// `resolve_mcp_binary_path` — locate the `portbay-mcp` sidecar so the frontend
/// can surface it in copy-paste snippets for MCP clients (Claude Code, Cursor, etc.).
///
/// Resolution order (first that exists wins):
/// 1. Next to the running executable — the production location once the .app is
///    built; sidecars land in `Contents/MacOS/` with the target-triple stripped.
/// 2. `<resource_dir>/binaries/portbay-mcp-<target-triple>` — dev / bundle layout,
///    same triple detection used by `resolve_mkcert_binary`.
/// 3. `which::which("portbay-mcp")` — PATH fallback (e.g. Homebrew install).
///
/// Returns `None` when none of the three resolve. The frontend falls back to the
/// conventional production path string so the user still gets a usable snippet.
#[tauri::command]
pub async fn resolve_mcp_binary_path(app: AppHandle) -> Option<String> {
    use std::env::consts::{ARCH, OS};

    // Production path: sidecar lives beside the main executable (triple stripped).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("portbay-mcp");
            if candidate.exists() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }

    // Dev / bundle path: resource_dir/binaries/portbay-mcp-<triple>.
    let triple = match (OS, ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        _ => None,
    };
    if let Some(triple) = triple {
        if let Ok(resource_dir) = app.path().resource_dir() {
            let candidate = resource_dir.join(format!("binaries/portbay-mcp-{triple}"));
            if candidate.exists() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }

    // PATH fallback.
    which::which("portbay-mcp")
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// `open_privacy_settings(kind)` — open the relevant macOS Privacy pane in System Settings.
#[tauri::command]
pub async fn open_privacy_settings(app: AppHandle, kind: String) -> AppResult<()> {
    let url = match kind.as_str() {
        // PortBay's privileged hosts/DNS helper shows here as a background item.
        "login-items" => "x-apple.systempreferences:com.apple.LoginItems-Settings.extension",
        "accessibility" => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        "screen-recording" => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
        "full-disk-access" => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"
        }
        "microphone" => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        }
        "camera" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Camera",
        // Not a privacy pane, but the same guided-dialog flow: the user
        // unticks macOS's own ⌘⇧3/4/5 screenshot keys so PortBay's capture
        // hotkeys can take over. System Settings has no deep link to the
        // Shortcuts sheet itself — the Keyboard pane is as close as it gets.
        "keyboard-shortcuts" => {
            "x-apple.systempreferences:com.apple.Keyboard-Settings.extension"
        }
        _ => {
            return Err(AppError::BadInput(format!(
                "unknown permission kind: {kind}"
            )))
        }
    };
    app.shell()
        .command("open")
        .arg(url)
        .spawn()
        .map_err(|e| AppError::Internal(format!("failed to open privacy settings: {e}")))?;
    Ok(())
}

/// What the frontend needs to start a native drag of PortBay itself: the path
/// the OS should drag (the `.app` bundle in a packaged build, the executable in
/// dev) and a small icon to show under the cursor. Dropping the bundle into the
/// System Settings Accessibility list is the macOS gesture that grants the
/// permission, so the drag sheet hands these to `tauri-plugin-drag`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionDragPayload {
    /// Absolute path to drag (the `.app` bundle when packaged, else the exe).
    pub bundle_path: String,
    /// Absolute path to a PNG drag image (PortBay's app icon, written to temp).
    pub icon_path: String,
}

/// Resolve the drag target + icon for the permission sheet's drag-to-grant
/// gesture. The `.app` bundle is found by walking up from the running
/// executable; the drag image is PortBay's bundled icon, materialised to a
/// temp PNG once so `tauri-plugin-drag` has a real filesystem path.
/// Resolve a real `.app` bundle to drag into the privacy list. macOS only
/// accepts an app bundle there (not a bare binary), so we try, in order:
///   1. The `.app` enclosing the running executable — the packaged/installed
///      case, where dropping it grants *this* process directly.
///   2. The `tauri build` bundle that sits next to the dev binary at
///      `target/<profile>/bundle/macos/PortBay.app` — so the drop still
///      completes during `tauri dev` (build the app once to populate it).
///   3. An installed copy in /Applications or ~/Applications.
///   4. The executable itself, as a last resort.
fn resolve_app_bundle(exe: &Path) -> PathBuf {
    if let Some(app) = exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
    {
        return app.to_path_buf();
    }
    if let Some(dir) = exe.parent() {
        let dev_bundle = dir.join("bundle/macos/PortBay.app");
        if dev_bundle.exists() {
            return dev_bundle;
        }
    }
    let mut installed = vec![PathBuf::from("/Applications/PortBay.app")];
    if let Some(home) = dirs::home_dir() {
        installed.push(home.join("Applications/PortBay.app"));
    }
    if let Some(found) = installed.into_iter().find(|p| p.exists()) {
        return found;
    }
    exe.to_path_buf()
}

#[tauri::command]
pub async fn permission_drag_payload() -> AppResult<PermissionDragPayload> {
    let exe = std::env::current_exe()
        .map_err(|e| AppError::Internal(format!("cannot resolve current exe: {e}")))?;

    // The macOS Accessibility list only accepts a real `.app` bundle drop — a
    // bare executable is rejected, so we must resolve an actual bundle.
    let bundle = resolve_app_bundle(&exe);

    // Materialise the bundled icon to a stable temp path (idempotent: only
    // written when missing). 128px is plenty for a drag cursor image.
    const ICON_PNG: &[u8] = include_bytes!("../../icons/128x128.png");
    let icon_path = std::env::temp_dir().join("portbay-drag-icon.png");
    if !icon_path.exists() {
        std::fs::write(&icon_path, ICON_PNG)
            .map_err(|e| AppError::Internal(format!("cannot write drag icon: {e}")))?;
    }

    Ok(PermissionDragPayload {
        bundle_path: bundle.to_string_lossy().into_owned(),
        icon_path: icon_path.to_string_lossy().into_owned(),
    })
}

/// Relaunch PortBay. macOS caches a process's Accessibility trust at launch, so
/// a permission granted while the app is running only takes effect after a
/// restart — the permission sheet calls this once the user has added PortBay to
/// the Accessibility list. `restart()` replaces the process and never returns.
#[tauri::command]
pub async fn relaunch_app(app: AppHandle) {
    app.restart();
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
