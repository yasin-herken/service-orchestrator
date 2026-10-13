//! Health-monitoring use cases behind the desktop commands.
//!
//! These are thin adapters over the shared [`OrchestratorClient`]: the
//! dashboard joins the observed service status (runtime state) with the Health
//! Manager's last observed index, and the detail view adds the structured
//! diagnostics (`tasks/021.md` §17–§19). Nothing here probes a process, opens
//! a socket, or sends an HTTP request — checks run as daemon tasks through
//! `check_service_health` / `check_service_readiness` (`tasks/021.md` §20).

use service_orchestrator_application::{ServiceHealth, ServiceStatus};
use service_orchestrator_client::OrchestratorClient;

use crate::dto::{DiagnosticFailureView, HealthDetailView, HealthOutcomeView, HealthRowView};
use crate::error::CommandError;
use crate::validation::parse_service_id;

/// Lists a workspace's services with their observed runtime and health state.
///
/// Runtime state and health state are reported as separate fields: a running
/// process may be unhealthy, and health is the Health Manager's last
/// observation, never a probe performed by this query (`tasks/021.md` §17,
/// §18). Services that have never been checked get an explicit `unknown`
/// health state (`tasks/021.md` §22).
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected workspace id, or the daemon's
/// structured error for anything else.
pub fn list_health(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
) -> Result<Vec<HealthRowView>, CommandError> {
    let workspace_id = crate::validation::parse_workspace_id(workspace_id)?;
    let status = client
        .get_workspace_status(&workspace_id)
        .map_err(CommandError::from)?;
    let observed = client
        .list_service_health(&workspace_id)
        .map_err(CommandError::from)?;

    Ok(status
        .services
        .iter()
        .map(|service| {
            let health = observed
                .iter()
                .find(|entry| entry.service_id == service.service_id);
            HealthRowView::new(service, health.unwrap_or(&unknown_health(service)))
        })
        .collect())
}

/// Returns one service's detailed health view.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected service id, `NOT_FOUND` when the
/// service belongs to a different workspace than the one being viewed, or the
/// daemon's structured error for anything else.
pub fn get_health(
    client: &dyn OrchestratorClient,
    service_id: &str,
    workspace_id: &str,
) -> Result<HealthDetailView, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let workspace_id = crate::validation::parse_workspace_id(workspace_id)?;
    let status = client
        .get_service_status(&service_id)
        .map_err(CommandError::from)?;
    if status.workspace_id != workspace_id {
        return Err(CommandError::new(
            "NOT_FOUND",
            format!("service '{service_id}' does not belong to workspace '{workspace_id}'"),
        ));
    }
    let health = client
        .get_service_health(&service_id)
        .map_err(CommandError::from)?;
    let diagnostics = client
        .get_service_diagnostics(&service_id)
        .map_err(CommandError::from)?;

    let row = HealthRowView::new(&status, &health);
    let checks = diagnostics
        .health_checks
        .iter()
        .map(HealthOutcomeView::from)
        .collect();
    let last_failure = diagnostics
        .last_failure
        .as_ref()
        .map(|failure| DiagnosticFailureView {
            source: failure.source.clone(),
            code: failure.code.clone(),
            message: failure.message.clone(),
            at: failure.at.map(crate::dto::timestamp_seconds),
        });
    Ok(HealthDetailView {
        row,
        checks,
        managed: diagnostics.managed,
        pid: diagnostics.pid,
        ports: diagnostics.ports.iter().map(|port| port.value()).collect(),
        last_failure,
        recent_errors: diagnostics.recent_errors.clone(),
    })
}

/// Builds an explicit "never observed" health record for a service.
fn unknown_health(service: &ServiceStatus) -> ServiceHealth {
    ServiceHealth::unknown(service.workspace_id.clone(), service.service_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_validated_before_any_ipc_call() {
        assert_eq!(
            parse_service_id("").expect_err("empty").code,
            "INVALID_INPUT"
        );
        assert_eq!(
            crate::validation::parse_workspace_id("two words")
                .expect_err("spaces are invalid")
                .code,
            "INVALID_INPUT"
        );
    }
}
