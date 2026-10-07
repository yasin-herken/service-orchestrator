//! Execution context handed to a [`TaskExecutor`](crate::TaskExecutor).
//!
//! The context is deliberately narrow. An executor may observe cancellation and
//! report progress and messages; it may **not** reach into the scheduler, the
//! task store, or the application container. This keeps executors replaceable
//! and prevents infrastructure from depending on engine internals.

use std::sync::Arc;

use service_orchestrator_domain::{TaskId, TaskKind};

use crate::cancellation::Cancellation;
use crate::error::TaskEngineError;
use crate::task_manager::TaskTarget;

/// The engine-side sink that progress and message reports are forwarded to.
///
/// This is an internal boundary so the context can be constructed without
/// exposing the engine's shared state type.
pub(crate) trait ProgressSink: Send + Sync {
    /// Records progress for a task.
    fn set_progress(
        &self,
        id: &TaskId,
        current: u32,
        total: Option<u32>,
        message: Option<String>,
    ) -> Result<(), TaskEngineError>;

    /// Records the latest execution message for a task.
    fn set_message(&self, id: &TaskId, message: String);
}

/// Reports progress and messages for the currently executing task.
///
/// Cloning a `ProgressReporter` produces another handle to the same task.
#[derive(Clone)]
pub struct ProgressReporter {
    sink: Arc<dyn ProgressSink>,
    id: TaskId,
}

impl ProgressReporter {
    /// Creates a reporter bound to a task and an engine sink.
    pub(crate) fn new(sink: Arc<dyn ProgressSink>, id: TaskId) -> Self {
        Self { sink, id }
    }

    /// Reports determinate or indeterminate progress.
    ///
    /// Pass `None` for `total` when the total is unknown. When `total` is known
    /// and `current` exceeds it, the report is rejected.
    ///
    /// # Errors
    ///
    /// Returns a [`TaskEngineError`] if the progress is inconsistent (for
    /// example `current` greater than `total`) or the task no longer exists.
    pub fn set_progress(
        &self,
        current: u32,
        total: Option<u32>,
        message: Option<String>,
    ) -> Result<(), TaskEngineError> {
        self.sink.set_progress(&self.id, current, total, message)
    }

    /// Reports a short execution message such as `"Running Maven build"`.
    ///
    /// # Errors
    ///
    /// Returns a [`TaskEngineError`] if the task no longer exists.
    pub fn message(&self, message: impl Into<String>) -> Result<(), TaskEngineError> {
        self.sink.set_message(&self.id, message.into());
        Ok(())
    }

    /// Returns the identifier of the task this reporter is bound to.
    #[must_use]
    pub fn task_id(&self) -> &TaskId {
        &self.id
    }
}

impl std::fmt::Debug for ProgressReporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProgressReporter")
            .field("task_id", &self.id)
            .finish_non_exhaustive()
    }
}

/// Everything an executor needs to run one task attempt.
pub struct TaskContext {
    id: TaskId,
    kind: TaskKind,
    target: TaskTarget,
    cancellation: Cancellation,
    progress: ProgressReporter,
}

impl TaskContext {
    /// Creates a context for a task attempt.
    pub(crate) fn new(
        id: TaskId,
        kind: TaskKind,
        target: TaskTarget,
        cancellation: Cancellation,
        progress: ProgressReporter,
    ) -> Self {
        Self {
            id,
            kind,
            target,
            cancellation,
            progress,
        }
    }

    /// The identifier of the task being executed.
    #[must_use]
    pub fn task_id(&self) -> &TaskId {
        &self.id
    }

    /// The kind of the task being executed.
    #[must_use]
    pub fn kind(&self) -> &TaskKind {
        &self.kind
    }

    /// The entity the task acts on.
    #[must_use]
    pub fn target(&self) -> &TaskTarget {
        &self.target
    }

    /// The progress reporter for this task.
    #[must_use]
    pub fn progress(&self) -> &ProgressReporter {
        &self.progress
    }

    /// The cancellation token for this task.
    #[must_use]
    pub fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }

    /// Returns `true` if cancellation has been signalled for this task.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    /// Waits until cancellation is signalled for this task.
    pub async fn cancelled(&self) {
        self.cancellation.cancelled().await
    }
}

impl std::fmt::Debug for TaskContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskContext")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}
