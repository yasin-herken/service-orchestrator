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
    MigrationOperation, Permission, ServiceId, TaskKind, WorkflowKind, WorkspaceId,
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
    /// provider, execution mode, commands, database target, and environment are
    /// read from validated configuration by the migration adapter at execution
    /// time, so no credential is ever stored on a task (`tasks/011.md` §11).
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

        let non_local_authorized = self.authorize_migration(operation, config)?;

        let target = ResolvedMigrationTarget {
            workspace_id: workspace.workspace.id.clone(),
            service_id: service_id.clone(),
            operation,
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
            .unwrap_or_else(|| self.default_timeout_seconds());
        Ok((target, timeout))
    }

    /// Applies the permission a migration operation requires, returning whether
    /// a non-local database target was explicitly authorized.
    fn authorize_migration(
        &self,
        operation: MigrationOperation,
        config: &service_orchestrator_config::LiquibaseConfig,
    ) -> Result<bool, ApplicationError> {
        if !operation.is_mutating() {
            self.authorize(Permission::Read)?;
            return Ok(false);
        }
        if config.database.environment.requires_authorization() {
            self.authorize(Permission::Destructive)?;
            Ok(true)
        } else {
            self.authorize(Permission::SafeWrite)?;
            Ok(false)
        }
    }
}
