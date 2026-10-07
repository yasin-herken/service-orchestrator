//! The task engine error model.
//!
//! [`TaskEngineError`] is the error type produced by the engine's own API. It
//! is deliberately flat and descriptive: each variant names a concrete failure
//! (a missing task, a dependency cycle, a full queue, a shutdown) rather than
//! wrapping infrastructure failures that the engine never produces.
//!
//! The application-facing [`TaskManager`] trait returns
//! [`TaskManagerError`](crate::TaskManagerError); the engine maps its richer
//! error into that smaller contract at the boundary so interfaces never observe
//! scheduler internals.

use service_orchestrator_domain::{DomainError, TaskId, TaskKind, WorkflowId};
use thiserror::Error;

/// Errors produced by the task engine.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TaskEngineError {
    /// The referenced task does not exist.
    #[error("task '{0}' was not found")]
    TaskNotFound(TaskId),

    /// The referenced workflow does not exist.
    #[error("workflow '{0}' was not found")]
    WorkflowNotFound(WorkflowId),

    /// Two tasks were submitted with the same identifier.
    #[error("duplicate task id '{0}'")]
    DuplicateTask(TaskId),

    /// A task declared a dependency that was not part of the submission and is
    /// not already known to the engine.
    #[error("task '{task}' depends on unknown task '{dependency}'")]
    DependencyNotFound {
        /// The task that declared the dependency.
        task: TaskId,
        /// The dependency that could not be resolved.
        dependency: TaskId,
    },

    /// The submitted dependency graph contains a cycle.
    #[error("dependency cycle detected involving: {nodes}")]
    DependencyCycle {
        /// A comma-separated list of the task identifiers forming the cycle.
        nodes: String,
    },

    /// A task was asked to move to a status the domain state machine forbids.
    #[error("task '{id}' cannot transition from '{from}' to '{to}'")]
    InvalidTransition {
        /// The task whose transition was rejected.
        id: String,
        /// The current status.
        from: &'static str,
        /// The requested status.
        to: &'static str,
    },

    /// No executor was registered for the task's kind.
    #[error("no executor is registered for task kind '{0}'")]
    ExecutorNotFound(TaskKind),

    /// The engine refused to accept more queued work.
    #[error("task queue is full (limit {limit})")]
    QueueFull {
        /// The configured maximum number of queued tasks.
        limit: usize,
    },

    /// The engine has begun shutting down and no longer accepts work.
    #[error("task engine is shutting down")]
    ShuttingDown,

    /// A submitted workflow is not well formed.
    #[error("invalid workflow: {message}")]
    InvalidWorkflow {
        /// Why the workflow was rejected.
        message: String,
    },

    /// The engine was built without an active Tokio runtime.
    #[error("task engine requires a Tokio runtime context to be built")]
    NoRuntime,

    /// The engine failed for an internal reason.
    #[error("task engine error: {message}")]
    Internal {
        /// A human-readable description of the failure.
        message: String,
    },
}

impl TaskEngineError {
    /// Builds a [`TaskEngineError::Internal`].
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }
}

impl From<DomainError> for TaskEngineError {
    fn from(error: DomainError) -> Self {
        match error {
            DomainError::InvalidTaskTransition { id, from, to } => {
                Self::InvalidTransition { id, from, to }
            }
            DomainError::DependencyCycle { nodes } => Self::DependencyCycle { nodes },
            other => Self::Internal {
                message: other.to_string(),
            },
        }
    }
}

impl From<TaskEngineError> for crate::TaskManagerError {
    fn from(error: TaskEngineError) -> Self {
        match error {
            TaskEngineError::TaskNotFound(id) => Self::NotFound {
                entity: "task",
                id: id.to_string(),
            },
            TaskEngineError::WorkflowNotFound(id) => Self::NotFound {
                entity: "workflow",
                id: id.to_string(),
            },
            TaskEngineError::Internal { message } => Self::Internal { message },
            TaskEngineError::NoRuntime => Self::Internal {
                message: "task engine requires a Tokio runtime".to_owned(),
            },
            other @ (TaskEngineError::DuplicateTask(_)
            | TaskEngineError::DependencyNotFound { .. }
            | TaskEngineError::DependencyCycle { .. }
            | TaskEngineError::InvalidTransition { .. }
            | TaskEngineError::ExecutorNotFound(_)
            | TaskEngineError::QueueFull { .. }
            | TaskEngineError::InvalidWorkflow { .. }
            | TaskEngineError::ShuttingDown) => Self::Rejected {
                reason: other.to_string(),
            },
        }
    }
}
