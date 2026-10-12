//! IPC query functions mapping daemon responses onto desktop view models.
//!
//! These are the read-only use cases behind the Tauri commands
//! (`tasks/018.md` §7, §11). They take an [`OrchestratorClient`] — the same
//! interface the TUI and MCP adapters use — so the GUI provably runs through
//! the shared application core rather than speaking IPC on its own.

use service_orchestrator_client::OrchestratorClient;
use service_orchestrator_domain::{ServiceId, WorkspaceId};

use crate::dto::{ServiceSummary, WorkspaceStatus, WorkspaceSummary};
use crate::error::CommandError;

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
/// service from a different workspace (`tasks/018.md` §7).
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

/// Validates a workspace identifier through the domain.
fn parse_workspace_id(raw: &str) -> Result<WorkspaceId, CommandError> {
    WorkspaceId::new(raw)
        .map_err(|error| CommandError::invalid_input(format!("invalid workspace id: {error}")))
}

/// Validates a service identifier through the domain.
fn parse_service_id(raw: &str) -> Result<ServiceId, CommandError> {
    ServiceId::new(raw)
        .map_err(|error| CommandError::invalid_input(format!("invalid service id: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_identifiers_are_rejected_by_the_domain() {
        let error = parse_workspace_id("  ").expect_err("empty is invalid");
        assert_eq!(error.code, "INVALID_INPUT");
        assert!(!error.message.is_empty());

        let error = parse_service_id("").expect_err("empty is invalid");
        assert_eq!(error.code, "INVALID_INPUT");
    }

    #[test]
    fn whitespace_identifiers_are_rejected_by_the_domain() {
        let error = parse_service_id("two words").expect_err("spaces are invalid");
        assert_eq!(error.code, "INVALID_INPUT");
    }

    #[test]
    fn valid_identifiers_parse() {
        assert_eq!(
            parse_workspace_id("demo").expect("valid").to_string(),
            "demo"
        );
        assert_eq!(
            parse_service_id("orders-api").expect("valid").to_string(),
            "orders-api"
        );
    }
}
