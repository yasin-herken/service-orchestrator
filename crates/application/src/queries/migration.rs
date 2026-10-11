//! Database-migration queries.
//!
//! Migration *inspection* — status, pending changesets, SQL preview — is a task,
//! because it runs the provider against a database (`commands/migration.rs`).
//! The one genuinely side-effect-free query is this configuration snapshot: it
//! reads validated configuration and never touches a database, a process, or a
//! credential. Live state that would require running Liquibase is reported as an
//! explicit unknown (`None`), never guessed (`tasks/011.md` §7, §14).

use service_orchestrator_domain::ServiceId;

use crate::app::Application;
use crate::error::ApplicationError;
use crate::models::MigrationStatus;

impl Application {
    /// Returns the configured migration setup of a service.
    ///
    /// The snapshot is derived entirely from validated configuration: the
    /// provider, execution mode, declared target environment, a display-safe
    /// database identity, and the configured timeout. It performs no I/O. The
    /// live [`MigrationStatus::pending`] and [`MigrationStatus::locked`] values
    /// are left `None`; use [`Application::get_migration_status`] or
    /// [`Application::inspect_migration_pending`] to run the provider for a real
    /// answer.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] for an unknown service, or
    /// [`ApplicationError::InvalidOperation`] when the service has no migration
    /// configuration.
    pub fn migration_status(
        &self,
        service_id: &ServiceId,
    ) -> Result<MigrationStatus, ApplicationError> {
        let (workspace, _service) = self.locate_service(service_id)?;
        let config = self
            .liquibase_config(workspace, service_id)
            .ok_or_else(|| {
                ApplicationError::invalid_operation(format!(
                    "service '{service_id}' has no migration configuration"
                ))
            })?;
        Ok(MigrationStatus {
            service_id: service_id.clone(),
            workspace_id: workspace.workspace.id.clone(),
            enabled: config.enabled,
            provider: config.provider.clone(),
            execution_mode: config.execution_mode,
            environment: config.database.environment.clone(),
            database_identity: config.database.display_identity(),
            requires_authorization: config.database.environment.requires_authorization(),
            timeout_seconds: config.timeout_seconds,
            pending: None,
            locked: None,
        })
    }
}
