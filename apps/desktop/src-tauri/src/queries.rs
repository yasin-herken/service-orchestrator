//! IPC query functions mapping daemon responses onto desktop view models.
//!
//! These are the read-oriented use cases behind the Tauri commands
//! (`tasks/018.md` §7, §11; `tasks/019.md` §18). They take an
//! [`OrchestratorClient`] — the same interface the TUI and MCP adapters use —
//! so the GUI provably runs through the shared application core rather than
//! speaking IPC on its own.

use service_orchestrator_client::OrchestratorClient;

use crate::dto::{ServiceDetail, ServiceSummary, WorkspaceStatus, WorkspaceSummary};
use crate::error::CommandError;
use crate::validation::{parse_service_id, parse_workspace_id};

/// Lists every configured workspace as selector/dashboard summaries.
///
/// # Errors
///
/// Returns the daemon's structured error when it is unreachable or the
/// response cannot be decoded.
pub fn list_workspaces(
    client: &dyn OrchestratorClient,
) -> Result<Vec<WorkspaceSummary>, CommandError> {
    let workspaces = client.list_workspaces().map_err(CommandError::from)?;
    Ok(workspaces.into_iter().map(WorkspaceSummary::from).collect())
}

/// Returns one workspace with the observed status of each of its services.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for an identifier the domain rejects, or the
/// daemon's structured error for anything else.
pub fn workspace_status(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
) -> Result<WorkspaceStatus, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let status = client
        .get_workspace_status(&workspace_id)
        .map_err(CommandError::from)?;
    Ok(WorkspaceStatus::from(status))
}

/// Lists a workspace's services as table rows.
///
/// # Errors
///
/// Same as [`workspace_status`].
pub fn list_services(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
) -> Result<Vec<ServiceSummary>, CommandError> {
    Ok(workspace_status(client, workspace_id)?.services)
}

/// Returns one service's status, validated against its workspace.
///
/// The workspace check keeps a stale selection from silently showing a
/// service from a different workspace (`tasks/018.md` §7; `tasks/019.md` §30).
///
/// # Errors
///
/// Returns `INVALID_INPUT` for an identifier the domain rejects,
/// `NOT_FOUND` when the service belongs to a different workspace, or the
/// daemon's structured error (including an unknown service) for anything else.
pub fn service_status(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
    service_id: &str,
) -> Result<ServiceSummary, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let service_id = parse_service_id(service_id)?;
    let status = client
        .get_service_status(&service_id)
        .map_err(CommandError::from)?;
    if status.workspace_id != workspace_id {
        return Err(CommandError::new(
            "NOT_FOUND",
            format!("service '{service_id}' does not belong to workspace '{workspace_id}'"),
        ));
    }
    Ok(ServiceSummary::from(status))
}

/// Returns a detailed view of one service, validated against its workspace.
///
/// The detail combines three existing, side-effect-free queries — the service
/// configuration (`get_service`), its observed runtime status
/// (`get_service_status`), and its structured diagnostics
/// (`get_service_diagnostics`) — so the panel renders real configuration and
/// real observed facts without re-deriving anything in TypeScript
/// (`tasks/019.md` §18).
///
/// # Errors
///
/// Returns `INVALID_INPUT` for an identifier the domain rejects,
/// `NOT_FOUND` when the service belongs to a different workspace, or the
/// daemon's structured error for anything else.
pub fn service_detail(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
    service_id: &str,
) -> Result<ServiceDetail, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let service_id = parse_service_id(service_id)?;
    let service = client
        .get_service(&service_id)
        .map_err(CommandError::from)?;
    let status = client
        .get_service_status(&service_id)
        .map_err(CommandError::from)?;
    if status.workspace_id != workspace_id {
        return Err(CommandError::new(
            "NOT_FOUND",
            format!("service '{service_id}' does not belong to workspace '{workspace_id}'"),
        ));
    }
    let diagnostics = client
        .get_service_diagnostics(&service_id)
        .map_err(CommandError::from)?;
    Ok(ServiceDetail::new(service, status, diagnostics))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_identifiers_are_rejected_before_any_ipc_call() {
        // The value type rejects the identifier, so no client is needed.
        assert!(parse_workspace_id("  ").is_err());
        assert!(parse_service_id("").is_err());
    }
}
