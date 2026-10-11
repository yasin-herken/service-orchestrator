//! The Git task executor.
//!
//! [`GitTaskExecutor`] adapts a mutating Git operation to the task engine. It
//! is deliberately thin: it decodes the workspace-aware [`ResolvedRepository`]
//! from the task's parameters, applies the task's timeout and cancellation to a
//! [`GitOptions`] value, calls the corresponding [`GitManager`] method, and maps
//! the structured [`GitOperationResult`] (or [`GitError`]) back onto a
//! [`TaskOutcome`] ([`TaskExecutorError`]).
//!
//! All business meaning — validation, safety checks, concurrency, conflict
//! policy — stays in the manager. The executor contains no Git logic of its own,
//! so the TUI and MCP reach identical behavior through the same engine
//! (`AGENTS.md` §21).
//!
//! [`register_git_executors`] wires the executor into an [`ExecutorRegistry`]
//! for every Git task kind; the composition root calls it once at startup.

use std::sync::Arc;

use async_trait::async_trait;
use service_orchestrator_application::{
    parameter, GitError, GitManager, GitOptions, ResolvedRepository,
};
use service_orchestrator_domain::{Branch, GitOperationResult, TaskKind};
use service_orchestrator_execution::{
    ExecutorRegistry, TaskContext, TaskExecutor, TaskExecutorError, TaskOutcome,
};

use crate::manager::CliGitManager;

/// Performs mutating Git operations on behalf of the task engine.
pub struct GitTaskExecutor {
    manager: Arc<CliGitManager>,
}

impl GitTaskExecutor {
    /// Creates an executor over a Git manager.
    #[must_use]
    pub fn new(manager: Arc<CliGitManager>) -> Self {
        Self { manager }
    }

    /// Builds the options for one attempt from the task context.
    fn options(context: &TaskContext) -> GitOptions {
        let mut options = GitOptions::new().with_cancellation(context.cancellation().clone());
        if let Some(timeout) = context.timeout() {
            options = options.with_timeout(timeout);
        }
        options
    }
}

#[async_trait]
impl TaskExecutor for GitTaskExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        let repository =
            ResolvedRepository::from_parameters(context.parameters()).map_err(to_executor_error)?;
        let _ = context.progress().message(format!(
            "git {} for '{}'",
            context.kind().label(),
            repository.name
        ));
        let options = Self::options(&context);

        let result = match context.kind() {
            TaskKind::GitSync => self.manager.sync(&repository, &options).await,
            TaskKind::GitFetch => self.manager.fetch(&repository, &options).await,
            TaskKind::GitPull => self.manager.pull(&repository, &options).await,
            TaskKind::GitCheckout => {
                let branch = checkout_branch(&context)?;
                self.manager.checkout(&repository, &branch, &options).await
            }
            TaskKind::GitClone => {
                GitManager::clone(self.manager.as_ref(), &repository, &options).await
            }
            TaskKind::GitResetHard => self.manager.reset_hard(&repository, &options).await,
            TaskKind::GitClean => self.manager.clean(&repository, &options).await,
            TaskKind::GitDiscardChanges => {
                self.manager
                    .discard_local_changes(&repository, &options)
                    .await
            }
            other => {
                return Err(TaskExecutorError::internal(format!(
                    "task kind '{other}' is not a Git operation"
                )))
            }
        };

        result.map(outcome).map_err(to_executor_error)
    }
}

impl std::fmt::Debug for GitTaskExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitTaskExecutor").finish_non_exhaustive()
    }
}

/// Registers a Git executor for every Git task kind.
///
/// The composition root calls this once, passing the same registry it hands to
/// the engine. The single executor instance serves every Git kind; the kind is
/// read from the [`TaskContext`] at execution time.
pub fn register_git_executors(registry: &mut ExecutorRegistry, manager: Arc<CliGitManager>) {
    let executor: Arc<dyn TaskExecutor> = Arc::new(GitTaskExecutor::new(manager));
    for kind in [
        TaskKind::GitSync,
        TaskKind::GitFetch,
        TaskKind::GitPull,
        TaskKind::GitCheckout,
        TaskKind::GitClone,
        TaskKind::GitResetHard,
        TaskKind::GitClean,
        TaskKind::GitDiscardChanges,
    ] {
        registry.register(kind, executor.clone());
    }
}

/// Reads the target branch for a checkout task from its parameters.
fn checkout_branch(context: &TaskContext) -> Result<Branch, TaskExecutorError> {
    let value = context.parameter(parameter::BRANCH).ok_or_else(|| {
        TaskExecutorError::failed(
            "GIT_MISSING_PARAMETER",
            format!("checkout requires the '{}' parameter", parameter::BRANCH),
        )
    })?;
    Branch::new(value).map_err(|error| {
        TaskExecutorError::failed(
            "GIT_INVALID_REFERENCE",
            format!("invalid branch '{value}': {error}"),
        )
    })
}

/// Converts a successful Git result into structured task metadata.
fn outcome(result: GitOperationResult) -> TaskOutcome {
    let mut outcome = TaskOutcome::succeeded(Some(format!(
        "git {} completed for '{}'",
        result.operation.label(),
        result.repository
    )))
    .with_metadata("repository", result.repository.clone())
    .with_metadata("operation", result.operation.label())
    .with_metadata("changed", result.changed.to_string());
    if let Some(branch) = &result.branch {
        outcome = outcome.with_metadata("branch", branch.to_string());
    }
    if let Some(previous) = &result.previous_commit {
        outcome = outcome.with_metadata("previous_commit", previous.clone());
    }
    if let Some(current) = &result.current_commit {
        outcome = outcome.with_metadata("current_commit", current.clone());
    }
    outcome
}

/// Maps a structured Git failure onto the engine's error vocabulary.
fn to_executor_error(error: GitError) -> TaskExecutorError {
    match error {
        GitError::Cancelled { .. } => TaskExecutorError::Cancelled,
        GitError::Internal { message } => TaskExecutorError::internal(message),
        other => TaskExecutorError::failed(other.code(), other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_application::GitError;

    #[test]
    fn cancellation_maps_to_a_cancelled_attempt() {
        let error = GitError::Cancelled {
            repository: "auth-service".to_owned(),
            operation: service_orchestrator_domain::GitOperation::Pull,
        };
        assert_eq!(to_executor_error(error), TaskExecutorError::Cancelled);
    }

    #[test]
    fn structured_failures_preserve_their_code() {
        let error = GitError::BranchNotFound {
            repository: "auth-service".to_owned(),
            branch: "release/9".to_owned(),
        };
        let mapped = to_executor_error(error);
        match mapped {
            TaskExecutorError::Failed { code, .. } => assert_eq!(code, "GIT_BRANCH_NOT_FOUND"),
            other => panic!("expected a structured failure, got {other:?}"),
        }
    }

    #[test]
    fn internal_failures_remain_internal() {
        let mapped = to_executor_error(GitError::internal("boom"));
        assert!(matches!(mapped, TaskExecutorError::Internal { .. }));
    }
}
