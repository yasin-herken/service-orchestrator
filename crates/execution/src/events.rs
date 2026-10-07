//! Task state events.
//!
//! The engine publishes a [`TaskEvent`] for every meaningful state change. The
//! event model is interface-neutral: it carries domain identifiers and values,
//! never terminal rows, MCP response shapes, or log formatting. Future
//! consumers — the Ratatui TUI, the MCP server, structured logging, and
//! telemetry — each subscribe and render independently.
//!
//! Delivery uses a bounded [`tokio::sync::broadcast`] channel. It is in-process
//! only: there is no external broker, and a subscriber that falls behind simply
//! observes a `Lagged` error and resynchronises from the task store.

use service_orchestrator_domain::{Task, TaskFailure, TaskId, TaskKind, TaskProgress, Workflow};
use tokio::sync::broadcast;

/// A single observable change in the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskEvent {
    /// A task was created and entered its initial state.
    TaskCreated {
        /// A snapshot of the newly created task.
        task: Box<Task>,
    },
    /// A task began executing.
    TaskStarted {
        /// The task that started.
        id: TaskId,
        /// The kind of work it performs.
        kind: TaskKind,
    },
    /// A running task reported progress.
    TaskProgress {
        /// The task that reported progress.
        id: TaskId,
        /// The new progress.
        progress: TaskProgress,
    },
    /// A running task reported a message.
    TaskMessage {
        /// The task that reported the message.
        id: TaskId,
        /// The message text.
        message: String,
    },
    /// A task attempt failed and will be retried.
    TaskRetrying {
        /// The task being retried.
        id: TaskId,
        /// The upcoming attempt number (one-based).
        attempt: u32,
    },
    /// A task completed successfully.
    TaskSucceeded {
        /// The task that succeeded.
        id: TaskId,
    },
    /// A task failed permanently.
    TaskFailed {
        /// The task that failed.
        id: TaskId,
        /// The structured failure.
        failure: TaskFailure,
    },
    /// A task was cancelled.
    TaskCancelled {
        /// The task that was cancelled.
        id: TaskId,
    },
    /// A task was deliberately not run (for example, a failed dependency).
    TaskSkipped {
        /// The task that was skipped.
        id: TaskId,
    },
    /// A task is waiting on its dependencies.
    TaskBlocked {
        /// The task that is blocked.
        id: TaskId,
    },
    /// A workflow was created and its tasks scheduled.
    WorkflowCreated {
        /// A snapshot of the newly created workflow.
        workflow: Box<Workflow>,
    },
}

/// The engine's bounded broadcast event channel.
#[derive(Clone)]
pub(crate) struct TaskEventBus {
    sender: broadcast::Sender<TaskEvent>,
}

impl TaskEventBus {
    /// Creates an event bus with the given channel capacity.
    pub(crate) fn new(capacity: usize) -> Self {
        let (sender, _receiver) = broadcast::channel(capacity.max(1));
        Self { sender }
    }

    /// Publishes an event, ignoring the absence of subscribers.
    pub(crate) fn publish(&self, event: TaskEvent) {
        // A send error only means there are currently no receivers.
        let _ = self.sender.send(event);
    }

    /// Subscribes to future events.
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<TaskEvent> {
        self.sender.subscribe()
    }
}
