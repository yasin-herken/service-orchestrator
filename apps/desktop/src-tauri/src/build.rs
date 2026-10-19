//! Build read and write use cases behind the desktop commands.
//!
//! The GUI is build-system agnostic: this module never names Maven or npm. It
//! asks the application core for each service's configured build system and the
//! operations that system's provider supports, and it submits builds through the
//! existing use cases (`tasks/020.md` §12, §13, §14, §16).
//!
//! Capability resolution is de-duplicated by build system: the provider's
//! support set depends only on the system, so at most one capability query runs
//! per configured system rather than one per service.

use std::collections::BTreeMap;

use service_orchestrator_application::{BuildCapabilities, PrepareEnvironmentRequest};
use service_orchestrator_client::OrchestratorClient;
use service_orchestrator_domain::{ServiceCapability, ServiceId};

use crate::dto::{BuildTargetSummary, OperationOutcome};
use crate::error::CommandError;
use crate::validation::{parse_service_id, parse_service_ids, parse_workspace_id};

/// Lists a workspace's build targets with their provider capabilities.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected workspace id, or the daemon's
/// structured error for anything else.
pub fn list_build_targets(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
) -> Result<Vec<BuildTargetSummary>, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let services = client
        .list_services(&workspace_id)
        .map_err(CommandError::from)?;
    let workspace = workspace_id.as_str();

    // Resolve each distinct build system at most once.
    let mut cache: BTreeMap<String, Result<BuildCapabilities, String>> = BTreeMap::new();
    for service in &services {
        let Some(system) = service.build_system else {
            continue;
        };
        let label = system.to_string();
        if cache.contains_key(&label) {
            continue;
        }
        let resolved = client
            .get_build_capabilities(&service.id)
            .map_err(|error| error.to_string());
        cache.insert(label, resolved);
    }

    let targets = services
        .iter()
        .filter(|service| {
            service.build_system.is_some() || service.supports(ServiceCapability::Build)
        })
        .map(|service| {
            let (capability, capability_error) = match service.build_system {
                Some(system) => match cache.get(&system.to_string()) {
                    Some(Ok(capabilities)) => (Some(capabilities), None),
                    Some(Err(message)) => (None, Some(message.clone())),
                    None => (
                        None,
                        Some("build capabilities were not resolved".to_owned()),
                    ),
                },
                None => (
                    None,
                    Some("no build system is configured for this service".to_owned()),
                ),
            };
            BuildTargetSummary::new(workspace, service, capability, capability_error)
        })
        .collect();
    Ok(targets)
}

/// Returns the build operations one service's build system supports.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected service id, `ServiceNotFound` for an
/// unknown service, or a `Build` error when the service has no build system or
/// no provider is registered for it.
pub fn get_build_capabilities(
    client: &dyn OrchestratorClient,
    service_id: &str,
) -> Result<crate::dto::BuildCapabilitiesView, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let capabilities = client
        .get_build_capabilities(&service_id)
        .map_err(CommandError::from)?;
    Ok(crate::dto::BuildCapabilitiesView::from(&capabilities))
}

/// Builds one service.
///
/// # Errors
///
/// Same as [`get_build_capabilities`], plus the daemon's structured error when
/// the build is rejected (for example a provider that does not support build).
pub fn build_service(
    client: &dyn OrchestratorClient,
    service_id: &str,
) -> Result<OperationOutcome, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let result = client
        .build_service(&service_id)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Builds the selected services as one workflow.
///
/// The daemon owns concurrency and dependency ordering: this submits a single
/// environment-preparation workflow restricted to the explicit selection, with
/// only the build phase enabled, rather than one request per service
/// (`tasks/020.md` §16).
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected workspace or service identifier (or an empty
/// selection), otherwise the daemon's structured error.
pub fn build_services(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
    service_ids: &[String],
) -> Result<OperationOutcome, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let service_ids = parse_service_ids(service_ids)?;
    let request = build_environment_request(workspace_id, service_ids);
    let result = client
        .prepare_environment(request)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Builds a request that runs only the build phase for an explicit selection.
fn build_environment_request(
    workspace_id: service_orchestrator_domain::WorkspaceId,
    services: Vec<ServiceId>,
) -> PrepareEnvironmentRequest {
    PrepareEnvironmentRequest {
        workspace_id,
        services,
        profile: None,
        group: None,
        branch: None,
        // The user selected exactly these services to build; do not fetch, do
        // not pull the dependency closure, do not migrate, do not start.
        sync: false,
        git_fetch: false,
        git_checkout: false,
        git_pull: false,
        include_dependencies: false,
        build_libraries: false,
        run_liquibase: false,
        migration_preset: None,
        build_operation: None,
        build: true,
        start: false,
        restart: false,
        stop: false,
        wait_for_health: true,
        health_checks: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bulk_build_request_only_enables_the_build_phase() {
        let workspace = service_orchestrator_domain::WorkspaceId::new("demo").expect("valid");
        let services = vec![ServiceId::new("orders-api").expect("valid")];
        let request = build_environment_request(workspace, services);
        assert!(request.build);
        assert!(!request.sync);
        assert!(!request.start);
        assert!(!request.build_libraries);
        assert!(!request.run_liquibase);
        assert!(!request.include_dependencies);
        assert_eq!(request.services.len(), 1);
    }

    #[test]
    fn invalid_inputs_are_rejected_before_any_ipc_call() {
        assert_eq!(
            parse_workspace_id("").expect_err("empty").code,
            "INVALID_INPUT"
        );
        assert_eq!(
            parse_service_id("").expect_err("empty").code,
            "INVALID_INPUT"
        );
        assert_eq!(
            parse_service_ids(&[]).expect_err("empty selection").code,
            "INVALID_INPUT"
        );
    }
}
