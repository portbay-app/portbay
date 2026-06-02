//! Reusable SSH identity management. An identity is a shared credential —
//! username + key/agent/password method — that many connections can borrow so
//! the same login isn't restated per host (see [`Registry::effective_ssh_connection`]).
//! Secret-free like connections; a password still lives in the OS keychain keyed
//! by the borrowing connection.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::projects::{load_registry, save_registry};
use crate::commands::ssh_tunnels::unique_connection_id;
use crate::error::{AppError, AppResult};
use crate::registry::{SshAuthKind, SshIdentity, SshIdentityId};
use crate::state::AppState;

/// An identity plus how many connections borrow it (delete is blocked at >0).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshIdentityView {
    #[serde(flatten)]
    pub identity: SshIdentity,
    pub connection_count: usize,
    pub in_use: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSshIdentityInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub ssh_user: String,
    #[serde(default)]
    pub auth_kind: SshAuthKind,
    #[serde(default)]
    pub key_path: Option<String>,
}

fn trimmed_opt(value: Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

/// List saved identities, each with the count of connections that borrow it.
#[tauri::command]
pub async fn ssh_identities_list(state: State<'_, AppState>) -> AppResult<Vec<SshIdentityView>> {
    let registry = load_registry(&state)?;
    let mut views: Vec<SshIdentityView> = registry
        .list_ssh_identities()
        .iter()
        .cloned()
        .map(|identity| {
            let connection_count = registry
                .list_ssh_connections()
                .iter()
                .filter(|c| c.identity_id.as_ref() == Some(&identity.id))
                .count();
            SshIdentityView {
                in_use: connection_count > 0,
                connection_count,
                identity,
            }
        })
        .collect();
    views.sort_by(|a, b| {
        a.identity
            .name
            .to_lowercase()
            .cmp(&b.identity.name.to_lowercase())
    });
    Ok(views)
}

/// Create or update a reusable identity.
#[tauri::command]
pub async fn ssh_identity_save(
    state: State<'_, AppState>,
    input: SaveSshIdentityInput,
) -> AppResult<SshIdentityView> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::BadInput("an identity name is required".into()));
    }

    let mut registry = load_registry(&state)?;
    // Reuse the connection id-uniqueness helper (same slug + collision logic).
    let id = match input.id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(existing) => SshIdentityId::new(existing),
        None => SshIdentityId::new(unique_connection_id(&registry, name)),
    };
    let exists = registry.get_ssh_identity(&id).is_some();

    let identity = SshIdentity {
        id: id.clone(),
        name: name.to_string(),
        ssh_user: input.ssh_user.trim().to_string(),
        auth_kind: input.auth_kind,
        key_path: trimmed_opt(input.key_path),
    };

    if exists {
        registry
            .update_ssh_identity(identity.clone())
            .map_err(AppError::Registry)?;
    } else {
        registry
            .add_ssh_identity(identity.clone())
            .map_err(AppError::Registry)?;
    }
    save_registry(&state, &registry)?;

    let connection_count = registry
        .list_ssh_connections()
        .iter()
        .filter(|c| c.identity_id.as_ref() == Some(&identity.id))
        .count();
    Ok(SshIdentityView {
        in_use: connection_count > 0,
        connection_count,
        identity,
    })
}

/// Delete an identity. Refuses while any connection still borrows it.
#[tauri::command]
pub async fn ssh_identity_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let id = SshIdentityId::new(id);
    let mut registry = load_registry(&state)?;
    if registry.ssh_identity_in_use(&id) {
        return Err(AppError::BadInput(
            "this identity is still used by one or more hosts — reassign those first".into(),
        ));
    }
    registry
        .remove_ssh_identity(&id)
        .map_err(AppError::Registry)?;
    save_registry(&state, &registry)?;
    Ok(())
}
