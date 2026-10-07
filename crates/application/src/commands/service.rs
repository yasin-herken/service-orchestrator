//! Service commands.

use service_orchestrator_domain::{
    CommandKind, Permission, ProcessState, ServiceCapability, ServiceId, TaskKind,
};
use service_orchestrator_execution::TaskDefinition;

use crate::app::Application;
use crate::error::ApplicationError;
use crate::models::OperationResult;

impl Application {
    /// Synchronizes a service's repository.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, or [`ApplicationError::InvalidOperation`] if it has no repository.
    pub fn sync_service(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let (workspace, service) = self.locate_service(service_id)?;
        self.check_dependencies(workspace, service)?;
        if service.repository.is_none() {
            return Err(ApplicationError::invalid_operation(format!(
                "service '{service_id}' has no repository to synchronize"
            )));
        }
        self.schedule(TaskDefinition::new(TaskKind::GitSync).for_service(service_id.clone()))
    }

    /// Builds a service.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, or [`ApplicationError::InvalidOperation`] if it cannot be built
    /// (no build capability, no build command, or no path).
    pub fn build_service(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let (workspace, service) = self.locate_service(service_id)?;
        self.check_dependencies(workspace, service)?;
        if !service.supports(ServiceCapability::Build) {
            return Err(ApplicationError::invalid_operation(format!(
                "service '{service_id}' does not declare the build capability"
            )));
        }
        if !service.has_build_command() {
            return Err(ApplicationError::invalid_operation(format!(
                "service '{service_id}' has no build command configured"
            )));
        }
        if service.path.is_none() {
            return Err(ApplicationError::invalid_operation(format!(
                "service '{service_id}' has no workspace path configured"
            )));
        }
        self.schedule(
            TaskDefinition::new(TaskKind::BuildService)
                .for_service(service_id.clone())
                .with_timeout(self.default_timeout_seconds()),
        )
    }

    /// Starts a service.
    ///
    /// Starting a service that is already running is a no-op and returns
    /// [`OperationResult::AlreadyInDesiredState`].
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, or [`ApplicationError::InvalidOperation`] if it cannot be started
    /// or is currently transitioning.
    pub fn start_service(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let (workspace, service) = self.locate_service(service_id)?;
        self.check_dependencies(workspace, service)?;
        if !service.supports(ServiceCapability::Run) {
            return Err(ApplicationError::invalid_operation(format!(
                "service '{service_id}' does not declare the run capability"
            )));
        }
        if !service.has_start_command() {
            return Err(ApplicationError::invalid_operation(format!(
                "service '{service_id}' has no start command configured"
            )));
        }

        let current = self.runtime().service_state(service_id).unwrap_or_default();
        if current.process.is_running() {
            return Ok(OperationResult::AlreadyInDesiredState {
                kind: TaskKind::StartService,
            });
        }
        if current.process.is_active() {
            return Err(ApplicationError::invalid_operation(format!(
                "service '{service_id}' is currently {} and cannot be started",
                current.process
            )));
        }

        let result = self.schedule(
            TaskDefinition::new(TaskKind::StartService)
                .for_service(service_id.clone())
                .with_timeout(self.default_timeout_seconds()),
        )?;
        self.runtime()
            .set_service_state(service_id, current.with_process(ProcessState::Starting));
        Ok(result)
    }

    /// Stops a service.
    ///
    /// Stopping a service that is not running is a no-op and returns
    /// [`OperationResult::AlreadyInDesiredState`].
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, or [`ApplicationError::InvalidOperation`] if it cannot be run and
    /// therefore cannot be stopped.
    pub fn stop_service(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let (_workspace, service) = self.locate_service(service_id)?;
        if !service.supports(ServiceCapability::Run) {
            return Err(ApplicationError::invalid_operation(format!(
                "service '{service_id}' does not declare the run capability"
            )));
        }

        let current = self.runtime().service_state(service_id).unwrap_or_default();
        match current.process {
            ProcessState::Running | ProcessState::Starting | ProcessState::Unknown => {
                let result = self.schedule(
                    TaskDefinition::new(TaskKind::StopService)
                        .for_service(service_id.clone())
                        .with_timeout(self.default_timeout_seconds()),
                )?;
                self.runtime()
                    .set_service_state(service_id, current.with_process(ProcessState::Stopping));
                Ok(result)
            }
            ProcessState::Stopping
            | ProcessState::Stopped
            | ProcessState::Failed
            | ProcessState::NotRunning => Ok(OperationResult::AlreadyInDesiredState {
                kind: TaskKind::StopService,
            }),
        }
    }

    /// Restarts a service.
    ///
    /// A restart is unconditional: it always schedules work, even when the
    /// service is currently running or stopped.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, or [`ApplicationError::InvalidOperation`] if it cannot be run.
    pub fn restart_service(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let (workspace, service) = self.locate_service(service_id)?;
        self.check_dependencies(workspace, service)?;
        if !service.supports(ServiceCapability::Run) {
            return Err(ApplicationError::invalid_operation(format!(
                "service '{service_id}' does not declare the run capability"
            )));
        }
        if !service.has_start_command() {
            return Err(ApplicationError::invalid_operation(format!(
                "service '{service_id}' has no start command configured"
            )));
        }

        let current = self.runtime().service_state(service_id).unwrap_or_default();
        let result = self.schedule(
            TaskDefinition::new(TaskKind::RestartService)
                .for_service(service_id.clone())
                .with_timeout(self.default_timeout_seconds()),
        )?;
        self.runtime()
            .set_service_state(service_id, current.with_process(ProcessState::Starting));
        Ok(result)
    }

    /// Runs Liquibase migrations for a service.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, or [`ApplicationError::InvalidOperation`] if Liquibase is not
    /// enabled or has no command.
    pub fn run_liquibase(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let (workspace, service) = self.locate_service(service_id)?;
        self.check_dependencies(workspace, service)?;
        let Some(config) = self.liquibase_config(workspace, service_id) else {
            return Err(ApplicationError::invalid_operation(format!(
                "service '{service_id}' has no Liquibase configuration"
            )));
        };
        if !config.enabled {
            return Err(ApplicationError::invalid_operation(format!(
                "Liquibase is disabled for service '{service_id}'"
            )));
        }
        if config.command.is_none() && !service.commands.contains(&CommandKind::Liquibase) {
            return Err(ApplicationError::invalid_operation(format!(
                "service '{service_id}' has no Liquibase command configured"
            )));
        }
        self.schedule(
            TaskDefinition::new(TaskKind::Liquibase)
                .for_service(service_id.clone())
                .with_timeout(
                    config
                        .timeout_seconds
                        .unwrap_or_else(|| self.default_timeout_seconds()),
                ),
        )
    }

    /// Schedules a health check for a service.
    ///
    /// This is a read-oriented operation: it does not mutate the workspace, but
    /// it schedules a probe task so the last observed health can be refreshed.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, or [`ApplicationError::InvalidOperation`] if it has no configured
    /// health checks.
    pub fn health_check(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::Read)?;
        let (_workspace, service) = self.locate_service(service_id)?;
        if service.health_checks.is_empty() {
            return Err(ApplicationError::invalid_operation(format!(
                "service '{service_id}' has no health checks configured"
            )));
        }
        self.schedule(
            TaskDefinition::new(TaskKind::HealthCheck)
                .for_service(service_id.clone())
                .with_timeout(self.default_timeout_seconds()),
        )
    }
}
