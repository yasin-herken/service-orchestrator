//! Line-based process log streaming.
//!
//! The manager captures `stdout` and `stderr` independently and never merges
//! them. Each line becomes a [`LogLine`] carrying the process identifier, the
//! originating stream, the text, and a timestamp. Consumers subscribe through
//! [`LogStream`] and receive lines as an async channel; the same lines are also
//! published as [`ProcessEvent::StdoutLine`](crate::ProcessEvent::StdoutLine)
//! and [`ProcessEvent::StderrLine`](crate::ProcessEvent::StderrLine) so a single
//! event subscription can observe lifecycle *and* output.
//!
//! Delivery uses a bounded [`tokio::sync::broadcast`] channel. It is in-process
//! and lossy under backpressure: a consumer that falls behind observes a
//! `Lagged` error. Log persistence is a future concern owned by the logging
//! subsystem (`AGENTS.md` §35).

use serde::{Deserialize, Serialize};
use service_orchestrator_domain::Timestamp;
use tokio::sync::broadcast;

use crate::event::ProcessEvent;
use crate::id::ProcessId;

/// Which output stream a line came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStreamKind {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

impl LogStreamKind {
    /// Returns a short, stable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

/// A single captured line of process output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    /// The process that produced the line.
    pub process: ProcessId,
    /// Which stream the line came from.
    pub stream: LogStreamKind,
    /// The line text, without the trailing newline.
    pub line: String,
    /// When the line was captured.
    pub timestamp: Timestamp,
}

/// A filtered, async view of a process's log lines.
///
/// A `LogStream` returned by the manager observes every process; one returned
/// by a [`ProcessHandle`](crate::ProcessHandle) observes only that process.
/// Non-log events are skipped transparently.
pub struct LogStream {
    filter: Option<ProcessId>,
    receiver: broadcast::Receiver<ProcessEvent>,
}

impl LogStream {
    /// Creates a stream, optionally restricted to one process.
    pub(crate) fn new(
        filter: Option<ProcessId>,
        receiver: broadcast::Receiver<ProcessEvent>,
    ) -> Self {
        Self { filter, receiver }
    }

    /// Receives the next line for the subscribed process.
    ///
    /// # Errors
    ///
    /// Returns [`broadcast::error::RecvError::Lagged`] when the consumer fell
    /// behind and lines were dropped, or
    /// [`broadcast::error::RecvError::Closed`] when the manager has shut down.
    pub async fn recv(&mut self) -> Result<LogLine, broadcast::error::RecvError> {
        loop {
            match self.receiver.recv().await {
                Ok(ProcessEvent::StdoutLine(line) | ProcessEvent::StderrLine(line)) => {
                    if self.matches(&line) {
                        return Ok(line);
                    }
                }
                Ok(_) => {}
                Err(error) => return Err(error),
            }
        }
    }

    fn matches(&self, line: &LogLine) -> bool {
        match &self.filter {
            Some(id) => &line.process == id,
            None => true,
        }
    }
}

impl std::fmt::Debug for LogStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogStream")
            .field("filter", &self.filter)
            .finish_non_exhaustive()
    }
}
