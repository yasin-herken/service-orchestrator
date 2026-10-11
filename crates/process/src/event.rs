//! Interface-neutral process events.
//!
//! The manager publishes a [`ProcessEvent`] for every meaningful change: a
//! process starting, exiting, failing, being killed, and every line of `stdout`
//! or `stderr`. The model carries identifiers, structured exit information, and
//! log lines — never terminal rows or MCP response shapes — so the Ratatui TUI,
//! the MCP server, and the logging subsystem can each subscribe and render
//! independently.
//!
//! Delivery uses a bounded [`tokio::sync::broadcast`] channel and is in-process
//! only. A subscriber that falls behind observes a `Lagged` error; the registry
//! remains authoritative for current state.

use tokio::sync::broadcast;

use crate::id::ProcessId;
use crate::log::LogLine;
use crate::state::ExitInfo;

/// A single observable change in the process manager.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessEvent {
    /// A process specification was accepted.
    Created {
        /// The new process identifier.
        id: ProcessId,
        /// The redacted command description.
        program: String,
    },
    /// A spawn is in progress.
    Starting {
        /// The process being started.
        id: ProcessId,
    },
    /// A process is alive and monitored.
    Started {
        /// The process that started.
        id: ProcessId,
        /// Its operating-system process id.
        pid: u32,
    },
    /// A process is being restarted (a fresh execution instance).
    Restarting {
        /// The process being restarted.
        id: ProcessId,
        /// The one-based restart attempt number.
        attempt: u32,
    },
    /// A graceful stop has been requested.
    Stopping {
        /// The process being stopped.
        id: ProcessId,
    },
    /// A process ended after a requested stop.
    Stopped {
        /// The process that stopped.
        id: ProcessId,
        /// Structured exit information.
        exit: ExitInfo,
    },
    /// A process ended on its own with a success status.
    Exited {
        /// The process that exited.
        id: ProcessId,
        /// Structured exit information.
        exit: ExitInfo,
    },
    /// A process failed to start or exited with a failure status.
    Failed {
        /// The process that failed.
        id: ProcessId,
        /// Structured exit information.
        exit: ExitInfo,
    },
    /// A process was force-killed.
    Killed {
        /// The process that was killed.
        id: ProcessId,
        /// Structured exit information.
        exit: ExitInfo,
    },
    /// A line was captured from standard output.
    StdoutLine(LogLine),
    /// A line was captured from standard error.
    StderrLine(LogLine),
}

impl ProcessEvent {
    /// Returns the identifier of the process this event concerns.
    #[must_use]
    pub fn process_id(&self) -> &ProcessId {
        match self {
            Self::Created { id, .. }
            | Self::Starting { id }
            | Self::Started { id, .. }
            | Self::Restarting { id, .. }
            | Self::Stopping { id }
            | Self::Stopped { id, .. }
            | Self::Exited { id, .. }
            | Self::Failed { id, .. }
            | Self::Killed { id, .. } => id,
            Self::StdoutLine(line) | Self::StderrLine(line) => &line.process,
        }
    }
}

/// The manager's bounded broadcast event channel.
#[derive(Clone)]
pub(crate) struct EventBus {
    sender: broadcast::Sender<ProcessEvent>,
}

impl EventBus {
    /// Creates an event bus with the given channel capacity.
    pub(crate) fn new(capacity: usize) -> Self {
        let (sender, _receiver) = broadcast::channel(capacity.max(1));
        Self { sender }
    }

    /// Publishes an event, ignoring the absence of subscribers.
    pub(crate) fn publish(&self, event: ProcessEvent) {
        // A send error only means there are currently no receivers.
        let _ = self.sender.send(event);
    }

    /// Subscribes to future events.
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<ProcessEvent> {
        self.sender.subscribe()
    }
}

/// A filtered, async view of process events.
///
/// A `ProcessEventStream` returned by the manager observes every process; one
/// returned by a [`ProcessHandle`](crate::ProcessHandle) observes only that
/// process.
pub struct ProcessEventStream {
    filter: Option<ProcessId>,
    receiver: broadcast::Receiver<ProcessEvent>,
}

impl ProcessEventStream {
    /// Creates a stream, optionally restricted to one process.
    pub(crate) fn new(
        filter: Option<ProcessId>,
        receiver: broadcast::Receiver<ProcessEvent>,
    ) -> Self {
        Self { filter, receiver }
    }

    /// Receives the next event for the subscribed process.
    ///
    /// # Errors
    ///
    /// Returns [`broadcast::error::RecvError`] when the consumer fell behind or
    /// the manager has shut down.
    pub async fn recv(&mut self) -> Result<ProcessEvent, broadcast::error::RecvError> {
        loop {
            match self.receiver.recv().await {
                Ok(event) => {
                    if self.matches(&event) {
                        return Ok(event);
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn matches(&self, event: &ProcessEvent) -> bool {
        match &self.filter {
            Some(id) => event.process_id() == id,
            None => true,
        }
    }
}

impl std::fmt::Debug for ProcessEventStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessEventStream")
            .field("filter", &self.filter)
            .finish_non_exhaustive()
    }
}
