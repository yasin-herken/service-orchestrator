//! Database-migration queries.
//!
//! Migration *inspection* — status, pending changesets, SQL preview — is a task,
//! because it runs the provider against a database (`commands/migration.rs`).
//! The one genuinely side-effect-free query is this configuration snapshot: it
//! reads validated configuration and never touches a database, a process, or a
//! credential. Live state that would require running Liquibase is reported as an
//! explicit unknown (`None`), never guessed (`tasks/011.md` §7, §14).

use service_orchestrator_domain::{Permission, ServiceId};

use crate::app::Application;
use crate::error::ApplicationError;
use crate::models::{MigrationPresetSummary, MigrationStatus};

impl Application {
    /// Lists every named migration preset: the built-ins merged with the
    /// user-configured presets, in stable order (`tasks/032.md` §16).
    ///
    /// The listing is display data for a migration preset selector; it runs no
    /// provider and never authorizes a destructive preset.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::PermissionDenied`] without read permission.
    pub fn list_migration_presets(&self) -> Result<Vec<MigrationPresetSummary>, ApplicationError> {
        self.authorize(Permission::Read)?;
        Ok(self
            .config()
            .all_migration_presets()
            .into_iter()
            .map(|preset| MigrationPresetSummary {
                id: preset.id,
                name: preset.name,
                description: preset.description,
                contexts: preset.contexts,
                read_only: preset.read_only,
                destructive: preset.destructive,
                allowed_environments: preset.allowed_environments,
                timeout_seconds: preset.timeout_seconds,
                builtin: preset.builtin,
            })
            .collect())
    }

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
