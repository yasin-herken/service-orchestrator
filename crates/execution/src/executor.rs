//! The task executor boundary.
//!
//! An executor answers *how* a task is performed. The scheduler answers *when*
//! it runs. Keeping the two apart is the central design rule of the engine:
//! the scheduler never knows how Maven, Git, or a process works, and an executor
//! never knows about queues, dependencies, or concurrency.
//!
//! Concrete executors (Git, build, process, health) are infrastructure and are
//! supplied to the engine through its [`ExecutorRegistry`](crate::ExecutorRegistry).
//! This crate ships no production executor and never touches the network,
//! filesystem, or a process.
//!
//! [`TaskExecutor`] is `async` because real operations are I/O bound and must
//! be cancellable without blocking a worker thread. Executors that perform
//! genuinely blocking work should run it on a blocking thread pool internally.

use std::collections::BTreeMap;

use async_trait::async_trait;
use service_orchestrator_domain::TaskFailure;
use thiserror::Error;

use crate::context::TaskContext;

/// The structured result of a completed task attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskOutcome {
    /// Whether the operation succeeded.
    pub success: bool,
    /// An optional human-readable summary.
    pub message: Option<String>,
    /// An optional reference to where detailed output is stored.
    ///
    /// Large raw output must not be embedded in the task result; an executor
    /// should write it to the logging subsystem and record a reference here.
    pub output: Option<String>,
    /// The structured failure, present when `success` is `false`.
    pub failure: Option<TaskFailure>,
    /// Optional structured metadata such as an exit code or artifact version.
    pub metadata: BTreeMap<String, String>,
}

impl TaskOutcome {
    /// A successful outcome with an optional summary.
    #[must_use]
    pub fn succeeded(message: Option<String>) -> Self {
        Self {
            success: true,
            message,
            output: None,
            failure: None,
            metadata: BTreeMap::new(),
        }
    }

    /// A failed outcome carrying a structured failure.
    #[must_use]
    pub fn failed(failure: TaskFailure) -> Self {
        Self {
            success: false,
            message: Some(failure.message.clone()),
            output: None,
            failure: Some(failure),
            metadata: BTreeMap::new(),
        }
    }

    /// Sets the output reference.
    #[must_use]
    pub fn with_output(mut self, output: impl Into<String>) -> Self {
        self.output = Some(output.into());
        self
    }

    /// Adds a metadata entry.
    #[must_use]
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

/// Errors an executor may return.
///
/// These are intentionally narrow: an executor either reports a structured
/// failure, reports that it observed cancellation, or reports an internal
/// error. The engine turns each into the correct terminal task state.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TaskExecutorError {
    /// The executor observed cancellation and stopped cooperatively.
    #[error("task was cancelled")]
    Cancelled,

    /// The operation failed with a structured, machine-readable code.
    #[error("{code}: {message}")]
    Failed {
        /// A stable, machine-readable code such as `MAVEN_BUILD_FAILED`.
        code: String,
        /// A human-readable summary.
        message: String,
        /// Optional additional detail.
        detail: Option<String>,
    },

    /// The executor itself failed unexpectedly.
    #[error("executor error: {message}")]
    Internal {
        /// A human-readable description of the failure.
        message: String,
    },
}

impl TaskExecutorError {
    /// Builds a [`TaskExecutorError::Failed`].
    #[must_use]
    pub fn failed(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Failed {
            code: code.into(),
            message: message.into(),
            detail: None,
        }
    }

    /// Builds a [`TaskExecutorError::Failed`] with extra detail.
    #[must_use]
    pub fn failed_with_detail(
        code: impl Into<String>,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::Failed {
            code: code.into(),
            message: message.into(),
            detail: Some(detail.into()),
        }
    }

    /// Builds a [`TaskExecutorError::Internal`].
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    /// Converts the error into the domain's structured [`TaskFailure`].
    #[must_use]
    pub fn to_failure(&self) -> TaskFailure {
        let (code, message, detail) = match self {
            Self::Cancelled => ("TASK_CANCELLED", "task was cancelled".to_owned(), None),
            Self::Failed {
                code,
                message,
                detail,
            } => (code.as_str(), message.clone(), detail.clone()),
            Self::Internal { message } => ("EXECUTOR_ERROR", message.clone(), None),
        };
        // Both the code and message are guaranteed non-empty by construction.
        TaskFailure::new(code, message, detail).unwrap_or_else(|_| TaskFailure {
            code: "EXECUTOR_ERROR".to_owned(),
            message: "executor failed".to_owned(),
            detail: None,
        })
    }
}

/// Performs the work described by a task.
///
/// Implementations are supplied to the engine and looked up by
/// [`TaskKind`](service_orchestrator_domain::TaskKind). An executor receives a
/// [`TaskContext`] and must:
///
/// - report progress and messages through the context;
/// - observe cancellation cooperatively;
/// - return a structured [`TaskOutcome`] or [`TaskExecutorError`].
#[async_trait]
pub trait TaskExecutor: Send + Sync {
    /// Executes a single task attempt.
    ///
    /// # Errors
    ///
    /// Returns a [`TaskExecutorError`] when the operation fails or observes
    /// cancellation.
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError>;
}
