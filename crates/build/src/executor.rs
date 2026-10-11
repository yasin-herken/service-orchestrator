//! The build task executor.
//!
//! [`BuildTaskExecutor`] adapts a build operation to the task engine. It is
//! deliberately thin: it confirms the task is a build operation, decodes the
//! workspace-aware [`ResolvedBuildTarget`] from the task's parameters, applies
//! the task's timeout and cancellation token, delegates to the
//! [`BuildManager`], and maps the structured [`BuildOutcome`] (or
//! [`BuildError`]) back onto a [`TaskOutcome`] ([`TaskExecutorError`]).
//!
//! All build meaning — provider selection, capability checks, planning,
//! execution, classification — stays in the manager and its providers. The
//! executor contains no build logic of its own, so the TUI and MCP reach
//! identical behavior through the same engine (`AGENTS.md` §21).
//!
//! [`register_build_executors`] wires the executor into an [`ExecutorRegistry`]
//! for every build task kind; the composition root calls it once at startup.

use std::sync::Arc;

use async_trait::async_trait;
use service_orchestrator_application::{BuildError, ResolvedBuildTarget};
use service_orchestrator_domain::{BuildOperation, TaskKind};
use service_orchestrator_execution::{
    ExecutorRegistry, TaskContext, TaskExecutor, TaskExecutorError, TaskOutcome,
};

use crate::manager::{BuildManager, BuildOutcome};

/// Performs build operations on behalf of the task engine.
pub struct BuildTaskExecutor {
    manager: Arc<BuildManager>,
}

impl BuildTaskExecutor {
    /// Creates an executor over a build manager.
    #[must_use]
    pub fn new(manager: Arc<BuildManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl TaskExecutor for BuildTaskExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        if BuildOperation::from_task_kind(context.kind()).is_none() {
            return Err(TaskExecutorError::internal(format!(
                "task kind '{}' is not a build operation",
                context.kind()
            )));
        }

        let target = ResolvedBuildTarget::from_parameters(context.parameters())
            .map_err(to_executor_error)?;
        let _ = context.progress().message(format!(
            "{} {} for '{}'",
            target.build_system, target.operation, target.name
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

impl std::fmt::Debug for BuildTaskExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuildTaskExecutor").finish_non_exhaustive()
    }
}

/// Registers a build executor for every build task kind.
///
/// The composition root calls this once, passing the same registry it hands to
/// the engine. The single executor instance serves every kind; the kind is read
/// from the [`TaskContext`] at execution time and the operation from the
/// decoded target.
pub fn register_build_executors(registry: &mut ExecutorRegistry, manager: Arc<BuildManager>) {
    let executor: Arc<dyn TaskExecutor> = Arc::new(BuildTaskExecutor::new(manager));
    for kind in [
        TaskKind::BuildService,
        TaskKind::BuildLibrary,
        TaskKind::MavenBuild,
        TaskKind::NpmBuild,
        TaskKind::NpmInstall,
    ] {
        registry.register(kind, executor.clone());
    }
}

/// Converts a successful build result into structured task metadata.
fn outcome(result: BuildOutcome) -> TaskOutcome {
    let mut outcome = TaskOutcome::succeeded(Some(format!(
        "{} {} completed for '{}'",
        result.system, result.operation, result.name
    )))
    .with_metadata("name", result.name.clone())
    .with_metadata("system", result.system.label())
    .with_metadata("operation", result.operation.label());
    if let Some(code) = result.exit_code {
        outcome = outcome.with_metadata("exit_code", code.to_string());
    }
    outcome
}

/// Maps a structured build failure onto the engine's error vocabulary.
fn to_executor_error(error: BuildError) -> TaskExecutorError {
    let code = error.code();
    match error {
        BuildError::Cancelled { .. } => TaskExecutorError::Cancelled,
        BuildError::Internal { message } => TaskExecutorError::internal(message),
        BuildError::CommandFailed {
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
    use service_orchestrator_domain::BuildSystem;

    #[test]
    fn cancellation_maps_to_a_cancelled_attempt() {
        let error = BuildError::Cancelled {
            name: "auth-service".to_owned(),
            system: BuildSystem::Maven,
            operation: BuildOperation::Build,
        };
        assert_eq!(to_executor_error(error), TaskExecutorError::Cancelled);
    }

    #[test]
    fn structured_failures_preserve_their_code_and_detail() {
        let error = BuildError::CommandFailed {
            name: "auth-service".to_owned(),
            system: BuildSystem::Maven,
            operation: BuildOperation::Build,
            exit_code: Some(1),
            message: "BUILD FAILURE".to_owned(),
            detail: Some("compilation error".to_owned()),
        };
        match to_executor_error(error) {
            TaskExecutorError::Failed { code, detail, .. } => {
                assert_eq!(code, "BUILD_COMMAND_FAILED");
                assert_eq!(detail.as_deref(), Some("compilation error"));
            }
            other => panic!("expected a structured failure, got {other:?}"),
        }
    }

    #[test]
    fn internal_failures_remain_internal() {
        let mapped = to_executor_error(BuildError::internal("boom"));
        assert!(matches!(mapped, TaskExecutorError::Internal { .. }));
    }

    #[test]
    fn unsupported_operations_carry_a_structured_code() {
        let error = BuildError::UnsupportedOperation {
            system: BuildSystem::Npm,
            operation: BuildOperation::PublishLocalArtifact,
        };
        match to_executor_error(error) {
            TaskExecutorError::Failed { code, .. } => {
                assert_eq!(code, "BUILD_UNSUPPORTED_OPERATION");
            }
            other => panic!("expected a structured failure, got {other:?}"),
        }
    }
}
