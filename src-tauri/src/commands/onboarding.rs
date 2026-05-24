//! First-run onboarding state + template scaffolding.
//!
//! Three surfaces:
//!
//! - `onboarding_status()` — `{ onboarded, registryEmpty }`. Drives the
//!   first-launch redirect: the frontend boots, calls this, and routes
//!   to `/onboarding` when both are true (or when the user has hit
//!   "Re-run setup" from Settings, which clears the marker).
//! - `mark_onboarded()` / `reset_onboarding()` — writes / deletes the
//!   marker file. Idempotent; safe to call from anywhere.
//! - `scaffold_template(kind, parentPath, name)` — spawns the upstream
//!   scaffolder for one of the five supported templates, streams its
//!   stdout/stderr via a `portbay://scaffold` event channel, and on
//!   success registers the project + returns its id.
//!
//! Templates are intentionally limited to five (Next.js, Laravel, Vite,
//! Astro, plain PHP). Adding more is a one-line enum addition + a new
//! `ScaffoldKind::command_for(...)` branch; the gallery card and
//! folder-picker UI live on the frontend.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

use crate::commands::dto::{AddProjectInput, ProjectView};
use crate::commands::projects::add_project;
use crate::error::{AppError, AppResult};
use crate::registry::ProjectType;
use crate::state::AppState;

/// Filename written into the PortBay app-data directory on completion.
/// Presence == the user has finished or skipped onboarding at least
/// once; absence == route to `/onboarding`.
const MARKER_FILENAME: &str = "onboarded";

/// Status snapshot used by the frontend on cold boot to decide whether
/// to route to `/onboarding`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingStatus {
    pub onboarded: bool,
    pub registry_empty: bool,
}

#[tauri::command]
pub async fn onboarding_status(state: State<'_, AppState>) -> AppResult<OnboardingStatus> {
    let marker = marker_path()?;
    let registry = crate::commands::projects::load_registry(&state)
        .map(|r| r.list_projects().is_empty())
        .unwrap_or(true);
    Ok(OnboardingStatus {
        onboarded: marker.exists(),
        registry_empty: registry,
    })
}

#[tauri::command]
pub async fn mark_onboarded() -> AppResult<()> {
    let marker = marker_path()?;
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Body is ignored; presence is the signal. We store the unix-epoch
    // millisecond timestamp so support requests can correlate
    // first-launch time without touching the registry.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    std::fs::write(&marker, now_ms.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn reset_onboarding() -> AppResult<()> {
    let marker = marker_path()?;
    if marker.exists() {
        std::fs::remove_file(&marker)?;
    }
    Ok(())
}

/// Templates exposed by the gallery. Adding a new template means adding
/// an enum variant and a `command_for(...)` branch.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScaffoldKind {
    Nextjs,
    Vite,
    Astro,
    Laravel,
    Php,
}

impl ScaffoldKind {
    fn project_type(self) -> ProjectType {
        match self {
            Self::Nextjs | Self::Vite | Self::Astro => ProjectType::Node,
            Self::Laravel | Self::Php => ProjectType::Php,
        }
    }

    /// Default start command for the scaffolded project. Frameworks
    /// have stable enough conventions that we don't need to detect.
    fn default_start_command(self) -> Option<&'static str> {
        match self {
            Self::Nextjs | Self::Vite | Self::Astro => Some("pnpm dev"),
            Self::Laravel => Some("php artisan serve"),
            Self::Php => None, // pure-Caddy-served; no PC process
        }
    }

    /// `(program, args)` for the upstream scaffolder. Run inside the
    /// chosen parent directory; `name` is the new folder it creates.
    fn command_for(self, name: &str) -> (&'static str, Vec<String>) {
        match self {
            Self::Nextjs => (
                "pnpm",
                vec![
                    "create".into(),
                    "next-app@latest".into(),
                    name.into(),
                    "--ts".into(),
                    "--app".into(),
                    "--src-dir".into(),
                    "--tailwind".into(),
                    "--eslint".into(),
                    "--import-alias".into(),
                    "@/*".into(),
                    "--use-pnpm".into(),
                    "--yes".into(),
                ],
            ),
            Self::Vite => (
                "pnpm",
                vec![
                    "create".into(),
                    "vite@latest".into(),
                    name.into(),
                    "--template".into(),
                    "vanilla-ts".into(),
                ],
            ),
            Self::Astro => (
                "pnpm",
                vec![
                    "create".into(),
                    "astro@latest".into(),
                    name.into(),
                    "--template".into(),
                    "minimal".into(),
                    "--install".into(),
                    "--no-git".into(),
                    "--yes".into(),
                ],
            ),
            Self::Laravel => (
                "composer",
                vec![
                    "create-project".into(),
                    "laravel/laravel".into(),
                    name.into(),
                    "--no-interaction".into(),
                ],
            ),
            Self::Php => (
                // No upstream scaffolder; we mkdir + drop an index.php.
                // Handled in `scaffold_template`'s php branch.
                "true",
                vec![],
            ),
        }
    }
}

/// Progress events streamed to the frontend via the `Channel<T>` arg.
/// `done` carries the registered project id on success; on failure the
/// command itself returns `Err(AppError)`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ScaffoldEvent {
    Log { line: String },
    Done { project_id: String },
}

/// Spawn the upstream scaffolder, wait for it, register the result.
///
/// `parent_path` is the directory the user picked; the scaffolder
/// creates `<parent_path>/<name>/`. After the child exits the new
/// folder is handed to `add_project` so the registry, hosts, and
/// reconcile loop pick it up automatically.
#[tauri::command]
pub async fn scaffold_template(
    app: AppHandle,
    state: State<'_, AppState>,
    kind: ScaffoldKind,
    parent_path: String,
    name: String,
    on_event: Channel<ScaffoldEvent>,
) -> AppResult<ProjectView> {
    if name.trim().is_empty() {
        return Err(AppError::BadInput("project name cannot be empty".into()));
    }
    let parent = PathBuf::from(&parent_path);
    if !parent.exists() || !parent.is_dir() {
        return Err(AppError::BadInput(format!(
            "parent path is not a directory: {parent_path}"
        )));
    }
    let target = parent.join(&name);
    if target.exists() {
        return Err(AppError::BadInput(format!(
            "target folder already exists: {}",
            target.display()
        )));
    }

    // The pure-PHP template has no upstream scaffolder. Materialise a
    // minimal index.php in a new folder and short-circuit registration.
    if matches!(kind, ScaffoldKind::Php) {
        std::fs::create_dir_all(&target)?;
        std::fs::write(
            target.join("index.php"),
            "<?php\necho \"Hello from PortBay!\";\n",
        )?;
        let _ = on_event.send(ScaffoldEvent::Log {
            line: format!("Created {}", target.display()),
        });
        return finalize(&app, &state, kind, &target, &name, on_event).await;
    }

    let (program, args) = kind.command_for(&name);
    let _ = on_event.send(ScaffoldEvent::Log {
        line: format!("$ {program} {}", args.join(" ")),
    });

    let cmd = app.shell().command(program).args(args).current_dir(&parent);
    let (mut rx, _child) = cmd
        .spawn()
        .map_err(|e| AppError::Internal(format!("spawn failed: {e}")))?;

    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => {
                let line = String::from_utf8_lossy(&bytes).trim_end().to_string();
                if !line.is_empty() {
                    let _ = on_event.send(ScaffoldEvent::Log { line });
                }
            }
            CommandEvent::Terminated(payload) if payload.code != Some(0) => {
                return Err(AppError::Internal(format!(
                    "{program} exited with code {:?}",
                    payload.code
                )));
            }
            _ => {}
        }
    }

    if !target.exists() {
        return Err(AppError::Internal(format!(
            "{program} reported success but target folder is missing: {}",
            target.display()
        )));
    }

    finalize(&app, &state, kind, &target, &name, on_event).await
}

/// Register the scaffolded folder via the existing `add_project` flow,
/// mark onboarding complete, and announce the project id to the
/// frontend.
async fn finalize(
    _app: &AppHandle,
    state: &State<'_, AppState>,
    kind: ScaffoldKind,
    target: &Path,
    name: &str,
    on_event: Channel<ScaffoldEvent>,
) -> AppResult<ProjectView> {
    let input = AddProjectInput {
        path: target.to_string_lossy().to_string(),
        id: None,
        name: Some(name.to_string()),
        hostname: None,
        kind: kind.project_type(),
        port: None,
        start_command: kind.default_start_command().map(str::to_string),
        https: true,
        auto_start: false,
        workspace: None,
    };

    let view = add_project(state.clone(), input).await?;
    // Marker write is best-effort — failing to write the marker should
    // not undo a successful scaffold.
    let _ = mark_onboarded().await;
    let _ = on_event.send(ScaffoldEvent::Done {
        project_id: view.id.clone(),
    });
    Ok(view)
}

fn marker_path() -> std::io::Result<PathBuf> {
    let mut dir = dirs::data_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no data dir"))?;
    dir.push("PortBay");
    Ok(dir.join(MARKER_FILENAME))
}
