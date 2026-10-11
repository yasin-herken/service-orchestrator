//! The migration task executor.
//!
//! [`MigrationTaskExecutor`] adapts a migration operation to the task engine. It
//! is deliberately thin: it confirms the task is a migration operation, decodes
//! the workspace-aware
//! [`ResolvedMigrationTarget`](service_orchestrator_application::ResolvedMigrationTarget)
//! from the task's parameters, applies the task's timeout and cancellation
//! token, delegates to the [`MigrationManager`], and maps the structured
//! [`MigrationOutcome`] (or [`MigrationError`]) back onto a
//! [`TaskOutcome`] ([`TaskExecutorError`]).
//!
//! All migration meaning — configuration lookup, environment safety, provider
//! selection, capability checks, planning, database coordination, execution,
//! classification — stays in the manager and its providers. The executor
//! contains no migration logic of its own, so the TUI and MCP reach identical
//! behavior through the same engine (`AGENTS.md` §21, `tasks/011.md` §20).
//!
//! [`register_migration_executors`] wires the executor into an
//! [`ExecutorRegistry`] for every migration task kind (including the legacy
//! [`TaskKind::Liquibase`] apply alias); the composition root calls it once at
//! startup.

use std::sync::Arc;

use async_trait::async_trait;
use service_orchestrator_application::{MigrationError, ResolvedMigrationTarget};
use service_orchestrator_domain::{MigrationOperation, TaskKind};
use service_orchestrator_execution::{
    ExecutorRegistry, TaskContext, TaskExecutor, TaskExecutorError, TaskOutcome,
};

use crate::manager::{MigrationManager, MigrationOutcome};

/// Performs migration operations on behalf of the task engine.
pub struct MigrationTaskExecutor {
    manager: Arc<MigrationManager>,
}

impl MigrationTaskExecutor {
    /// Creates an executor over a migration manager.
    #[must_use]
    pub fn new(manager: Arc<MigrationManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl TaskExecutor for MigrationTaskExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        if MigrationOperation::from_task_kind(context.kind()).is_none() {
            return Err(TaskExecutorError::internal(format!(
                "task kind '{}' is not a migration operation",
                context.kind()
            )));
        }
        let target = ResolvedMigrationTarget::from_parameters(context.parameters())
            .map_err(to_executor_error)?;
        let _ = context.progress().message(format!(
            "{} migration for '{}'",
            target.operation, target.service_id
        ));

        self.manager
            .execute(
                &target,
                context.timeout(),
                Some(context.cancellation().clone()),
            )
            .await
            .map(outcome)
            .map_err(to_executor_error)
    }
}

impl std::fmt::Debug for MigrationTaskExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MigrationTaskExecutor")
            .finish_non_exhaustive()
    }
}

/// Registers a migration executor for every migration task kind.
///
/// The composition root calls this once, passing the same registry it hands to
/// the engine. The single executor instance serves every kind; the kind is read
/// from the [`TaskContext`] at execution time and the operation from the decoded
/// target.
pub fn register_migration_executors(
    registry: &mut ExecutorRegistry,
    manager: Arc<MigrationManager>,
) {
    let executor: Arc<dyn TaskExecutor> = Arc::new(MigrationTaskExecutor::new(manager));
    for kind in [
        TaskKind::MigrationValidate,
        TaskKind::MigrationStatus,
        TaskKind::MigrationPending,
        TaskKind::MigrationPreview,
        TaskKind::MigrationApply,
        // Retained as an apply alias so pre-existing workflows keep working.
        TaskKind::Liquibase,
    ] {
        registry.register(kind, executor.clone());
    }
}

/// Converts a successful migration result into structured task metadata.
fn outcome(result: MigrationOutcome) -> TaskOutcome {
    let mut outcome = TaskOutcome::succeeded(Some(format!(
        "{} {} completed for '{}'",
        result.provider, result.operation, result.service_id
    )))
    .with_metadata("service", result.service_id.to_string())
    .with_metadata("provider", result.provider.label().to_owned())
    .with_metadata("operation", result.operation.label())
    .with_metadata("database", result.database_identity.clone());
    if let Some(code) = result.exit_code {
        outcome = outcome.with_metadata("exit_code", code.to_string());
    }
    outcome
}

/// Maps a structured migration failure onto the engine's error vocabulary.
fn to_executor_error(error: MigrationError) -> TaskExecutorError {
    let code = error.code();
    match error {
        MigrationError::Cancelled { .. } => TaskExecutorError::Cancelled,
        MigrationError::Internal { message } => TaskExecutorError::internal(message),
        MigrationError::Failed {
            message, detail, ..
        } => match detail {
            Some(detail) => TaskExecutorError::failed_with_detail(code, message, detail),
            None => TaskExecutorError::failed(code, message),
        },
        other => TaskExecutorError::failed(other.code(), other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_domain::ServiceId;

    fn service() -> ServiceId {
        ServiceId::new("auth-service").unwrap()
    }

    #[test]
    fn cancellation_maps_to_a_cancelled_attempt() {
        let error = MigrationError::Cancelled {
            service: service(),
            operation: MigrationOperation::Apply,
        };
        assert_eq!(to_executor_error(error), TaskExecutorError::Cancelled);
    }

    #[test]
    fn a_lock_conflict_preserves_its_code() {
        let error = MigrationError::LockConflict {
            service: service(),
            message: "locked".to_owned(),
        };
        match to_executor_error(error) {
            TaskExecutorError::Failed { code, .. } => {
                assert_eq!(code, "MIGRATION_LOCK_CONFLICT");
            }
            other => panic!("expected a structured failure, got {other:?}"),
        }
    }

    #[test]
    fn a_failed_migration_preserves_its_code_and_detail() {
        let error = MigrationError::Failed {
            service: service(),
            operation: MigrationOperation::Apply,
            exit_code: Some(1),
            message: "error".to_owned(),
            detail: Some("rolled back 0 changesets".to_owned()),
        };
        match to_executor_error(error) {
            TaskExecutorError::Failed { code, detail, .. } => {
                assert_eq!(code, "MIGRATION_FAILED");
                assert_eq!(detail.as_deref(), Some("rolled back 0 changesets"));
            }
            other => panic!("expected a structured failure, got {other:?}"),
        }
    }

    #[test]
    fn internal_failures_remain_internal() {
        let mapped = to_executor_error(MigrationError::internal("boom"));
        assert!(matches!(mapped, TaskExecutorError::Internal { .. }));
    }

    #[test]
    fn an_unauthorized_target_carries_a_structured_code() {
        let error = MigrationError::DatabaseTargetNotAuthorized {
            service: service(),
            environment: service_orchestrator_domain::MigrationTargetEnvironment::Production,
        };
        match to_executor_error(error) {
            TaskExecutorError::Failed { code, .. } => {
                assert_eq!(code, "MIGRATION_DATABASE_TARGET_NOT_AUTHORIZED");
            }
            other => panic!("expected a structured failure, got {other:?}"),
        }
    }
}
