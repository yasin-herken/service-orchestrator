//! IPC mutation functions mapping daemon operation results onto desktop view
//! models.
//!
//! These are the write-oriented use cases behind the service-lifecycle Tauri
//! commands (`tasks/019.md` §10–§16). Like the read queries, they take an
//! [`OrchestratorClient`] — the same interface the TUI and MCP adapters use — so
//! starting, stopping, and restarting a service from the GUI runs through the
//! shared daemon and application core, never a second task engine or process
//! manager.
//!
//! Every function returns a normalized [`OperationOutcome`]: either a scheduled
//! task id, a scheduled workflow id (a bulk operation), or an explicit
//! "already in the desired state" signal. The frontend follows the real ids to
//! observe task-engine progress and never marks an operation successful on its
//! own (`tasks/019.md` §11, §24, §25).

use service_orchestrator_application::HealthCheckScope;
use service_orchestrator_client::OrchestratorClient;

use crate::dto::OperationOutcome;
use crate::error::CommandError;
use crate::validation::{parse_service_id, parse_service_ids, parse_workspace_id};

/// Starts one service.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected identifier, or the daemon's
/// structured error (including a permission denial) for anything else.
pub fn start_service(
    client: &dyn OrchestratorClient,
    service_id: &str,
) -> Result<OperationOutcome, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let result = client
        .start_service(&service_id)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Stops one service.
///
/// # Errors
///
/// Same as [`start_service`].
pub fn stop_service(
    client: &dyn OrchestratorClient,
    service_id: &str,
) -> Result<OperationOutcome, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let result = client
        .stop_service(&service_id)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Restarts one service through the application's coordinated restart.
///
/// The daemon owns the stop-then-start lifecycle; the GUI never issues an
/// independent stop and start pair (`tasks/019.md` §13).
///
/// # Errors
///
/// Same as [`start_service`].
pub fn restart_service(
    client: &dyn OrchestratorClient,
    service_id: &str,
) -> Result<OperationOutcome, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let result = client
        .restart_service(&service_id)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Starts the selected services of a workspace as one workflow.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected workspace or service identifier (or an
/// empty selection), or the daemon's structured error for anything else.
pub fn start_services(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
    service_ids: &[String],
) -> Result<OperationOutcome, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let service_ids = parse_service_ids(service_ids)?;
    let result = client
        .start_services(&workspace_id, &service_ids)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Stops the selected services of a workspace as one workflow.
///
/// # Errors
///
/// Same as [`start_services`].
pub fn stop_services(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
    service_ids: &[String],
) -> Result<OperationOutcome, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let service_ids = parse_service_ids(service_ids)?;
    let result = client
        .stop_services(&workspace_id, &service_ids)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Restarts the selected services of a workspace as one workflow.
///
/// # Errors
///
/// Same as [`start_services`].
pub fn restart_services(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
    service_ids: &[String],
) -> Result<OperationOutcome, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let service_ids = parse_service_ids(service_ids)?;
    let result = client
        .restart_services(&workspace_id, &service_ids)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Runs every configured health check for one service.
///
/// # Errors
///
/// Same as [`start_service`].
pub fn check_service_health(
    client: &dyn OrchestratorClient,
    service_id: &str,
) -> Result<OperationOutcome, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let result = client
        .check_service_health(&service_id, HealthCheckScope::All)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_identifiers_are_rejected_before_any_ipc_call() {
        // The domain value types reject these, so a client is never consulted.
        // We only assert the validation path here; the real client call is
        // exercised end-to-end in `tests/daemon_ipc.rs`.
        assert!(parse_service_id("  ").is_err());
        assert!(parse_workspace_id("").is_err());
        let error = parse_service_ids(&[]).expect_err("an empty selection is invalid");
        assert_eq!(error.code, "INVALID_INPUT");
    }
}
