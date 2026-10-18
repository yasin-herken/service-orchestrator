//! Database migration commands.
//!
//! Every migration operation — validate, status, pending, preview, apply — is a
//! task. These commands resolve a workspace-aware
//! [`ResolvedMigrationTarget`](crate::ports::ResolvedMigrationTarget), apply the
//! shared policy for the target's *declared* environment, and submit a task.
//! They never run Liquibase, Maven, or a database client themselves: the
//! migration adapter owns execution and the task engine owns scheduling
//! (`AGENTS.md` §21, `tasks/011.md` §3, §26).
//!
//! # Environment safety at the boundary
//!
//! Read-only operations require only [`Permission::Read`]. Applying to an
//! explicitly configured local database requires [`Permission::SafeWrite`].
//! Applying to any other declared environment — shared development, staging,
//! production, an unmodelled name, or an unidentified target — requires
//! [`Permission::Destructive`], the same explicit confirmation destructive Git
//! operations use (`tasks/011.md` §10, §27). The resolved target records whether
//! that authorization was granted; the manager re-checks it, so the policy
//! cannot be bypassed with a hand-crafted task.

use service_orchestrator_domain::{
    MigrationOperation, MigrationPresetId, MigrationTargetEnvironment, Permission, ServiceId,
    TaskKind, WorkflowKind, WorkspaceId,
};
use service_orchestrator_execution::TaskDefinition;

use crate::app::Application;
use crate::commands::WorkflowBuilder;
use crate::error::ApplicationError;
use crate::models::OperationResult;
use crate::ports::ResolvedMigrationTarget;

impl Application {
    /// Validates a service's migration configuration without touching the
    /// database.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] for an unknown service, or
    /// [`ApplicationError::InvalidOperation`] when the service has no enabled
    /// migration configuration.
    pub fn validate_migration(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.schedule_migration(
            service_id,
            MigrationOperation::Validate,
            TaskKind::MigrationValidate,
        )
    }

    /// Schedules a migration-status inspection.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] or
    /// [`ApplicationError::InvalidOperation`].
    pub fn get_migration_status(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.schedule_migration(
            service_id,
            MigrationOperation::Status,
            TaskKind::MigrationStatus,
        )
    }

    /// Schedules a pending-changeset inspection.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] or
    /// [`ApplicationError::InvalidOperation`].
    pub fn inspect_migration_pending(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.schedule_migration(
            service_id,
            MigrationOperation::Pending,
            TaskKind::MigrationPending,
        )
    }

    /// Schedules a dry-run SQL preview.
    ///
    /// A preview is informational: it performs no database mutation, so it
    /// requires only [`Permission::Read`].
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] or
    /// [`ApplicationError::InvalidOperation`].
    pub fn preview_migration(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.schedule_migration(
            service_id,
            MigrationOperation::Preview,
            TaskKind::MigrationPreview,
        )
    }

    /// Applies pending migrations for a service.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] for an unknown service,
    /// [`ApplicationError::InvalidOperation`] when migrations are not enabled,
    /// or [`ApplicationError::PermissionDenied`] when the target's environment
    /// requires confirmation that has not been granted.
    pub fn apply_migration(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.schedule_migration(
            service_id,
            MigrationOperation::Apply,
            TaskKind::MigrationApply,
        )
    }

    /// Applies migrations for several selected services as one workflow.
    ///
    /// Services without enabled migration configuration are skipped rather than
    /// failing the batch (`tasks/011.md` §21). Each enabled service becomes an
    /// independent step; the migration adapter serializes steps that share a
    /// database target, and unrelated targets run concurrently through the task
    /// engine (`tasks/011.md` §15).
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] for an unknown workspace,
    /// [`ApplicationError::ServiceNotFound`] for an unknown member, or
    /// [`ApplicationError::PermissionDenied`] when a non-local target was not
    /// authorized.
    pub fn apply_selected_migrations(
        &self,
        workspace_id: &WorkspaceId,
        service_ids: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError> {
        let workspace = self.require_workspace(workspace_id)?;
        let selected = self.selected_service_ids(workspace, service_ids)?;

        let kind = WorkflowKind::Custom("apply_migrations".to_owned());
        let mut builder = WorkflowBuilder::new(kind.clone());
        for id in &selected {
            let enabled = self
                .liquibase_config(workspace, id)
                .is_some_and(|config| config.enabled);
            if !enabled {
                // An explicitly skipped service: migration is disabled or
                // unconfigured, so no step is scheduled for it.
                continue;
            }
            let (target, _timeout) = self.resolve_migration(id, MigrationOperation::Apply)?;
            builder.add_with_parameters(
                TaskKind::MigrationApply,
                Vec::new(),
                Some(format!("apply migrations for {id}")),
                target.to_parameters(),
            );
        }

        self.finish_workflow(builder, TaskKind::MigrationApply)
    }

    /// Resolves and schedules a single migration operation.
    fn schedule_migration(
        &self,
        service_id: &ServiceId,
        operation: MigrationOperation,
        kind: TaskKind,
    ) -> Result<OperationResult, ApplicationError> {
        let (target, timeout) = self.resolve_migration(service_id, operation)?;
        self.schedule(
            TaskDefinition::new(kind)
                .for_service(service_id.clone())
                .with_parameters(target.to_parameters())
                .with_timeout(timeout),
        )
    }

    /// Resolves a workspace-aware migration target and its effective timeout.
    ///
    /// The returned target carries only identity and the working directory; the
    /// provider, execution mode, commands, preset, database target, and
    /// environment are read from validated configuration by the migration
    /// adapter at execution time, so no credential is ever stored on a task
    /// (`tasks/011.md` §11).
    ///
    /// The effective preset is the service's configured
    /// [`liquibase.preset`](service_orchestrator_config::LiquibaseConfig::preset):
    /// a read-only preset never turns an apply into a status, so an explicit
    /// apply falls back to the service's per-operation command instead.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`], a
    /// [`ApplicationError::Migration`] for an invalid target, or
    /// [`ApplicationError::PermissionDenied`] when the required permission for
    /// the target's environment was not granted.
    pub(crate) fn resolve_migration(
        &self,
        service_id: &ServiceId,
        operation: MigrationOperation,
    ) -> Result<(ResolvedMigrationTarget, u64), ApplicationError> {
        self.resolve_migration_impl(service_id, operation, None, true)
    }

    /// Resolves a migration target with an explicit named preset.
    ///
    /// The preset overrides the service's configured default. A destructive
    /// preset always requires [`Permission::Destructive`], and its
    /// `allowed_environments` list must cover the service's declared database
    /// environment; the list itself never grants authorization
    /// (`tasks/032.md` §19).
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Application::resolve_migration`], plus an
    /// [`ApplicationError::InvalidOperation`] when the preset is unknown,
    /// incompatible with the operation, or not allowed for the target
    /// environment.
    pub(crate) fn resolve_migration_with_preset(
        &self,
        service_id: &ServiceId,
        operation: MigrationOperation,
        preset: &MigrationPresetId,
    ) -> Result<(ResolvedMigrationTarget, u64), ApplicationError> {
        self.resolve_migration_impl(service_id, operation, Some(preset), true)
    }

    /// Resolves a migration target *without* applying the permission policy.
    ///
    /// Planning uses this to describe a database-mutating step and to report that
    /// it requires authorization; execution always uses
    /// [`Application::resolve_migration`], which enforces the policy. The
    /// returned target records `non_local_authorized = false`, so it is
    /// ineligible to run if it ever reached the migration adapter
    /// (`tasks/016.md` §35).
    pub(crate) fn resolve_migration_target(
        &self,
        service_id: &ServiceId,
        operation: MigrationOperation,
    ) -> Result<(ResolvedMigrationTarget, u64), ApplicationError> {
        self.resolve_migration_impl(service_id, operation, None, false)
    }

    /// Resolves a migration target with an explicit preset *without* applying
    /// the permission policy; see [`Application::resolve_migration_target`].
    pub(crate) fn resolve_migration_target_with_preset(
        &self,
        service_id: &ServiceId,
        operation: MigrationOperation,
        preset: &MigrationPresetId,
    ) -> Result<(ResolvedMigrationTarget, u64), ApplicationError> {
        self.resolve_migration_impl(service_id, operation, Some(preset), false)
    }

    /// Resolves a migration target, optionally enforcing the shared policy.
    fn resolve_migration_impl(
        &self,
        service_id: &ServiceId,
        operation: MigrationOperation,
        preset: Option<&MigrationPresetId>,
        authorize: bool,
    ) -> Result<(ResolvedMigrationTarget, u64), ApplicationError> {
        let (workspace, service) = self.locate_service(service_id)?;
        let config = self
            .liquibase_config(workspace, service_id)
            .ok_or_else(|| {
                ApplicationError::invalid_operation(format!(
                    "service '{service_id}' has no Liquibase configuration"
                ))
            })?;
        if !config.enabled {
            return Err(ApplicationError::invalid_operation(format!(
                "migrations are disabled for service '{service_id}'"
            )));
        }
        let relative = service.path.as_ref().ok_or_else(|| {
            ApplicationError::invalid_operation(format!(
                "service '{service_id}' has no path configured for migrations"
            ))
        })?;
        let path = self.absolute_entity_path(workspace, relative)?;

        // The effective preset: an explicit selection overrides the service's
        // configured default. A read-only default never silently turns an apply
        // into a status operation.
        let effective = match preset {
            Some(preset) => Some(preset.clone()),
            None if operation == MigrationOperation::Apply => config
                .preset
                .clone()
                .filter(|id| !self.migration_preset_is_read_only(id)),
            None => None,
        };
        let validated = self.validate_migration_preset(service_id, operation, config, effective)?;

        let non_local_authorized = if authorize {
            self.authorize_migration(operation, config, validated.as_ref())?
        } else {
            false
        };

        let target = ResolvedMigrationTarget {
            workspace_id: workspace.workspace.id.clone(),
            service_id: service_id.clone(),
            operation,
            preset: validated.as_ref().map(|preset| preset.id.clone()),
            path,
            workspace_root: workspace.workspace.root.clone(),
            non_local_authorized,
        };
        if !target.is_inside_workspace() {
            return Err(ApplicationError::Migration {
                code: "MIGRATION_INVALID_TARGET".to_owned(),
                message: format!(
                    "service '{service_id}' resolves to a working directory outside workspace root '{}'",
                    target.workspace_root
                ),
            });
        }

        let timeout = config
            .timeout_seconds
            .or_else(|| validated.as_ref().and_then(|preset| preset.timeout_seconds))
            .unwrap_or_else(|| self.default_timeout_seconds());
        Ok((target, timeout))
    }

    /// Returns `true` when a preset which may be configured on a service is
    /// read-only. An unknown preset is never read-only: configuration
    /// normalization already rejects an unknown reference, so this only
    /// protects against a stale cache.
    pub(crate) fn migration_preset_is_read_only(&self, preset: &MigrationPresetId) -> bool {
        self.config()
            .migration_preset(preset)
            .is_some_and(|preset| preset.read_only)
    }

    /// Validates a preset against the operation and the service's declared
    /// database environment.
    ///
    /// The rules mirror the configuration-level validation and are rechecked
    /// here because a request may select a preset the service does not
    /// configure: a read-only preset cannot mutate, a destructive preset
    /// requires a mutating operation and an environment its
    /// `allowed_environments` list covers, and the list never grants
    /// authorization by itself (`tasks/032.md` §19).
    fn validate_migration_preset(
        &self,
        service_id: &ServiceId,
        operation: MigrationOperation,
        config: &service_orchestrator_config::LiquibaseConfig,
        preset: Option<MigrationPresetId>,
    ) -> Result<Option<service_orchestrator_config::MigrationPreset>, ApplicationError> {
        let Some(id) = preset else {
            return Ok(None);
        };
        let preset = self.config().migration_preset(&id).ok_or_else(|| {
            ApplicationError::invalid_operation(format!("migration preset '{id}' does not exist"))
        })?;
        if preset.read_only && operation.is_mutating() {
            return Err(ApplicationError::invalid_operation(format!(
                "migration preset '{id}' is read-only and cannot run the '{}' operation",
                operation.label()
            )));
        }
        if preset.destructive {
            if !operation.is_mutating() {
                return Err(ApplicationError::invalid_operation(format!(
                    "destructive migration preset '{id}' requires a mutating operation"
                )));
            }
            let environment = &config.database.environment;
            if !preset.allows_environment(environment) {
                return Err(ApplicationError::invalid_operation(format!(
                    "migration preset '{id}' is not allowed for the '{environment}' database of service '{service_id}'{}",
                    if *environment == MigrationTargetEnvironment::Unknown {
                        ": the database target is not identified"
                    } else {
                        ""
                    }
                )));
            }
        }
        Ok(Some(preset))
    }

    /// Applies the permission a migration operation requires, returning whether
    /// a destructive database operation was explicitly authorized.
    ///
    /// A destructive preset always requires [`Permission::Destructive`] — even
    /// against a local database — because `dropAll` destroys objects regardless
    /// of the environment's name (`tasks/032.md` §19).
    fn authorize_migration(
        &self,
        operation: MigrationOperation,
        config: &service_orchestrator_config::LiquibaseConfig,
        preset: Option<&service_orchestrator_config::MigrationPreset>,
    ) -> Result<bool, ApplicationError> {
        if !operation.is_mutating() {
            self.authorize(Permission::Read)?;
            return Ok(false);
        }
        if preset.is_some_and(|preset| preset.destructive)
            || config.database.environment.requires_authorization()
        {
            self.authorize(Permission::Destructive)?;
            Ok(true)
        } else {
            self.authorize(Permission::SafeWrite)?;
            Ok(false)
        }
    }
}
