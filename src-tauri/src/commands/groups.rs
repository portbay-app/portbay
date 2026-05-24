//! Group commands — CRUD over `Registry.groups` + batch lifecycle fanout.
//!
//! A group is just a named cluster of `ProjectId`s, so the lifecycle
//! actions ride the existing per-project start/stop/restart paths and
//! return a per-member report (mirroring `stop_all`'s pattern). The
//! group itself has no runtime state — "running" status is derived from
//! its members.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::projects::{load_registry, save_registry};
use crate::error::{AppError, AppResult};
use crate::registry::{Group, ProjectId};
use crate::state::AppState;

/// Wire shape returned by `list_groups`. Adds computed counts so the
/// sidebar can render member totals + "any running?" without a second
/// roundtrip.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    pub id: String,
    pub name: String,
    pub project_ids: Vec<String>,
    /// Subset of `project_ids` that exist in the registry. Drift between
    /// the group and the registry (e.g. a member was removed) is
    /// surfaced by `project_ids.len() != known_ids.len()`.
    pub known_ids: Vec<String>,
    pub member_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupInput {
    /// Group id. When `None`, derived from `name`.
    pub id: Option<String>,
    pub name: String,
    pub project_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupPatch {
    pub name: Option<String>,
    pub project_ids: Option<Vec<String>>,
}

/// One row of the batch lifecycle report. Same shape as `StopAllReport`
/// entries — different name to keep the frontend types separate.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupOpResult {
    pub project_id: String,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupOpReport {
    pub group_id: String,
    pub succeeded: usize,
    pub failed: usize,
    pub results: Vec<GroupOpResult>,
}

#[tauri::command]
pub async fn list_groups(state: State<'_, AppState>) -> AppResult<Vec<GroupView>> {
    let registry = load_registry(&state)?;
    let known: std::collections::HashSet<&str> = registry
        .list_projects()
        .iter()
        .map(|p| p.id.as_str())
        .collect();

    Ok(registry
        .list_groups()
        .iter()
        .map(|g| {
            let project_ids: Vec<String> = g
                .projects
                .iter()
                .map(|id| id.as_str().to_string())
                .collect();
            let known_ids: Vec<String> = project_ids
                .iter()
                .filter(|id| known.contains(id.as_str()))
                .cloned()
                .collect();
            GroupView {
                id: g.id.clone(),
                name: g.name.clone(),
                member_count: project_ids.len(),
                project_ids,
                known_ids,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn add_group(state: State<'_, AppState>, input: GroupInput) -> AppResult<GroupView> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadInput("group name cannot be empty".into()));
    }
    let id = input
        .id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| slugify(&name));
    if id.is_empty() {
        return Err(AppError::BadInput(
            "group id couldn't be derived from name".into(),
        ));
    }

    let mut registry = load_registry(&state)?;
    let project_ids: Vec<ProjectId> = input.project_ids.into_iter().map(ProjectId::new).collect();
    let group = Group {
        id: id.clone(),
        name,
        projects: project_ids,
    };
    registry.add_group(group.clone())?;
    save_registry(&state, &registry)?;

    Ok(view_from(&registry, &group))
}

#[tauri::command]
pub async fn update_group(
    state: State<'_, AppState>,
    id: String,
    patch: GroupPatch,
) -> AppResult<GroupView> {
    let mut registry = load_registry(&state)?;
    let current = registry
        .get_group(&id)
        .ok_or_else(|| AppError::NotFound(format!("group:{id}")))?
        .clone();

    let next = Group {
        id: current.id.clone(),
        name: patch
            .name
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or(current.name),
        projects: patch
            .project_ids
            .map(|ids| ids.into_iter().map(ProjectId::new).collect())
            .unwrap_or(current.projects),
    };
    registry.update_group(next.clone())?;
    save_registry(&state, &registry)?;

    Ok(view_from(&registry, &next))
}

#[tauri::command]
pub async fn remove_group(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let mut registry = load_registry(&state)?;
    registry.remove_group(&id)?;
    save_registry(&state, &registry)?;
    Ok(())
}

#[tauri::command]
pub async fn start_group(state: State<'_, AppState>, id: String) -> AppResult<GroupOpReport> {
    fanout(&state, &id, GroupOp::Start).await
}

#[tauri::command]
pub async fn stop_group(state: State<'_, AppState>, id: String) -> AppResult<GroupOpReport> {
    fanout(&state, &id, GroupOp::Stop).await
}

#[tauri::command]
pub async fn restart_group(state: State<'_, AppState>, id: String) -> AppResult<GroupOpReport> {
    fanout(&state, &id, GroupOp::Restart).await
}

#[derive(Clone, Copy)]
enum GroupOp {
    Start,
    Stop,
    Restart,
}

async fn fanout(
    state: &State<'_, AppState>,
    group_id: &str,
    op: GroupOp,
) -> AppResult<GroupOpReport> {
    let registry = load_registry(state)?;
    let group = registry
        .get_group(group_id)
        .ok_or_else(|| AppError::NotFound(format!("group:{group_id}")))?
        .clone();

    let known: std::collections::HashSet<&str> = registry
        .list_projects()
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    let client = state.pc_client()?;

    let mut report = GroupOpReport {
        group_id: group_id.to_string(),
        succeeded: 0,
        failed: 0,
        results: Vec::with_capacity(group.projects.len()),
    };

    for pid in &group.projects {
        let id_str = pid.as_str().to_string();
        if !known.contains(id_str.as_str()) {
            // Stale member — count as failed so the user sees the drift.
            report.failed += 1;
            report.results.push(GroupOpResult {
                project_id: id_str,
                ok: false,
                error: Some("project not in registry".into()),
            });
            continue;
        }
        // Mark stop intent so wrapper-translated SIGTERM exits don't
        // surface as crashes in the dashboard for batched ops.
        if matches!(op, GroupOp::Stop | GroupOp::Restart) {
            state.mark_stop_requested(&id_str);
        }
        let res = match op {
            GroupOp::Start => client.start(&id_str).await,
            GroupOp::Stop => client.stop(&id_str).await,
            GroupOp::Restart => client.restart(&id_str).await,
        };
        match res {
            Ok(_) => {
                report.succeeded += 1;
                report.results.push(GroupOpResult {
                    project_id: id_str,
                    ok: true,
                    error: None,
                });
            }
            Err(e) => {
                report.failed += 1;
                report.results.push(GroupOpResult {
                    project_id: id_str,
                    ok: false,
                    error: Some(e.to_string()),
                });
            }
        }
    }

    Ok(report)
}

fn view_from(registry: &crate::registry::Registry, group: &Group) -> GroupView {
    let known: std::collections::HashSet<&str> = registry
        .list_projects()
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    let project_ids: Vec<String> = group
        .projects
        .iter()
        .map(|id| id.as_str().to_string())
        .collect();
    let known_ids: Vec<String> = project_ids
        .iter()
        .filter(|id| known.contains(id.as_str()))
        .cloned()
        .collect();
    GroupView {
        id: group.id.clone(),
        name: group.name.clone(),
        member_count: project_ids.len(),
        project_ids,
        known_ids,
    }
}

/// Group ids are derived from the display name with the shared slugifier —
/// the same one the project commands and CLI use, so ids never diverge.
use crate::util::slugify;
