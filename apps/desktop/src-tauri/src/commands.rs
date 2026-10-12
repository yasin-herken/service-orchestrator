//! The read-only Tauri commands exposed to the webview.
//!
//! Every command is a thin adapter: validate input, delegate to a query over
//! the shared [`OrchestratorClient`], and funnel failures through the
//! connection tracker (`tasks/018.md` §7, §11). There is no start/stop or
//! other mutating command until task 019.
//!
//! The command set:
//!
//! - `get_daemon_status` — daemon process facts for the dashboard;
//! - `list_workspaces` — workspace summaries for the selector;
//! - `list_services` — service rows for a workspace;
//! - `get_workspace_status` — one workspace with all service statuses;
//! - `get_service_status` — one service, validated against its workspace;
//! - `get_connection_state` — the adapter's current connection snapshot.

use std::sync::Arc;

use service_orchestrator_client::{DaemonStatus, RemoteClient};
use tauri::State;

use crate::connection::ConnectionSnapshot;
use crate::dto::{ServiceSummary, WorkspaceStatus, WorkspaceSummary};
use crate::error::CommandError;
use crate::queries;
use crate::state::AppState;

/// Runs one blocking query against the shared client.
///
/// On success the connection state is confirmed (`Connected`), which lets an
/// explicit Retry clear the banner as soon as the daemon answers. On failure
/// connection-shaped errors update the pushed connection state, while input
/// and not-found errors stay local to the command.
async fn query<T, F>(state: &AppState, operation: F) -> Result<T, CommandError>
where
    F: FnOnce(Arc<RemoteClient>) -> Result<T, CommandError> + Send + 'static,
    T: Send + 'static,
{
    let client = state.ensure_client().await?;
    let outcome = tauri::async_runtime::spawn_blocking(move || operation(client))
        .await
        .map_err(|error| CommandError::internal(format!("query task failed: {error}")))?;
    match outcome {
        Ok(value) => {
            state.push(&crate::connection::ProbeOutcome::Connected);
            Ok(value)
        }
        Err(error) => {
            state.observe(&error);
            Err(error)
        }
    }
}

/// Returns the daemon's diagnostic status for the dashboard.
///
/// # Errors
///
/// A structured `{ code, message }` error when the daemon cannot be reached.
#[tauri::command]
pub async fn get_daemon_status(state: State<'_, AppState>) -> Result<DaemonStatus, CommandError> {
    query(state.inner(), |client| {
        client.daemon_status().map_err(CommandError::from)
    })
    .await
}

/// Lists every configured workspace as selector and dashboard summaries.
///
/// # Errors
///
/// A structured `{ code, message }` error when the daemon cannot be reached.
#[tauri::command]
pub async fn list_workspaces(
    state: State<'_, AppState>,
) -> Result<Vec<WorkspaceSummary>, CommandError> {
    query(state.inner(), |client| {
        queries::list_workspaces(client.as_ref())
    })
    .await
}

/// Lists a workspace's services as table rows.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected workspace id, otherwise a structured
/// `{ code, message }` error.
#[tauri::command]
pub async fn list_services(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<Vec<ServiceSummary>, CommandError> {
    query(state.inner(), move |client| {
        queries::list_services(client.as_ref(), &workspace_id)
    })
    .await
}

/// Returns one workspace with the observed status of each of its services.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected workspace id, otherwise a structured
/// `{ code, message }` error.
#[tauri::command]
pub async fn get_workspace_status(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<WorkspaceStatus, CommandError> {
    query(state.inner(), move |client| {
        queries::workspace_status(client.as_ref(), &workspace_id)
    })
    .await
}

/// Returns one service's status, validated against its workspace.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected identifier, `NOT_FOUND` when the service
/// belongs to a different workspace, otherwise a structured
/// `{ code, message }` error.
#[tauri::command]
pub async fn get_service_status(
    state: State<'_, AppState>,
    workspace_id: String,
    service_id: String,
) -> Result<ServiceSummary, CommandError> {
    query(state.inner(), move |client| {
        queries::service_status(client.as_ref(), &workspace_id, &service_id)
    })
    .await
}

/// Returns the adapter's current connection snapshot.
///
/// The frontend pulls this once on startup and then follows the pushed
/// `daemon://connection` events; the pull is only a bootstrap.
#[tauri::command]
pub fn get_connection_state(state: State<'_, AppState>) -> ConnectionSnapshot {
    state.connection_snapshot()
}
