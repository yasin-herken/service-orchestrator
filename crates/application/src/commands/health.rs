//! Health check commands.
//!
//! A health check is read-oriented: it does not mutate the workspace, but it
//! schedules a probe task so the last observed health can be refreshed. Running
//! the probe is the health adapter's job; the application only validates
//! preconditions, applies policy, and submits the task (`tasks/012.md` §33).

use service_orchestrator_domain::{Permission, ServiceId, TaskKind};
use service_orchestrator_execution::TaskDefinition;

use crate::app::Application;
use crate::error::ApplicationError;
use crate::models::{HealthCheckScope, OperationResult};

/// The default deadline, in seconds, for a readiness poll.
///
/// A readiness check polls until the service is ready or this deadline elapses,
/// so a startup task is never left running indefinitely (`tasks/012.md` §16).
pub(crate) const DEFAULT_READINESS_TIMEOUT_SECONDS: u64 = 60;

impl Application {
    /// Schedules a health check for a service.
    ///
    /// The scope selects which configured checks run; [`HealthCheckScope::All`]
    /// runs every configured check. The probe is a task, so its progress and
    /// result are observable through the task engine and the health index.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, or [`ApplicationError::InvalidOperation`] if it has no configured
    /// health checks.
    pub fn check_service_health(
        &self,
        service_id: &ServiceId,
        scope: HealthCheckScope,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::Read)?;
        let (workspace, service) = self.locate_service(service_id)?;
        if service.health_checks.is_empty() {
            return Err(ApplicationError::invalid_operation(format!(
                "service '{service_id}' has no health checks configured"
            )));
        }
        let kind = match scope {
            HealthCheckScope::All => TaskKind::HealthCheck,
            HealthCheckScope::Process => TaskKind::ProcessHealthCheck,
            HealthCheckScope::Tcp => TaskKind::TcpHealthCheck,
            HealthCheckScope::Http => TaskKind::HttpHealthCheck,
        };
        let target = self.service_runtime_target(workspace, service)?;
        self.schedule(
            TaskDefinition::new(kind)
                .for_service(service_id.clone())
                .with_parameters(target.to_parameters())
                .with_timeout(self.default_timeout_seconds()),
        )
    }

    /// Schedules a readiness check for a service.
    ///
    /// A readiness check polls the service's *required* checks until they pass,
    /// or the deadline elapses. It is a distinct operation from starting a
    /// service: a start task completes once the process is spawned, while a
    /// readiness check represents the wait until the service is actually
    /// serving (`tasks/012.md` §12, §16).
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, or [`ApplicationError::InvalidOperation`] if it has no required
    /// health checks.
    pub fn check_service_readiness(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::Read)?;
        let (workspace, service) = self.locate_service(service_id)?;
        if !service.health_checks.iter().any(|check| check.required) {
            return Err(ApplicationError::invalid_operation(format!(
                "service '{service_id}' has no required health checks to determine readiness"
            )));
        }
        let target = self.service_runtime_target(workspace, service)?;
        self.schedule(
            TaskDefinition::new(TaskKind::ReadinessCheck)
                .for_service(service_id.clone())
                .with_parameters(target.to_parameters())
                .with_timeout(DEFAULT_READINESS_TIMEOUT_SECONDS),
        )
    }
}
