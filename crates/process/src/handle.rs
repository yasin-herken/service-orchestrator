//! A handle to a managed process.
//!
//! [`ProcessHandle`] is the strongly typed, cloneable handle returned when a
//! process is spawned. It exposes status and pid, graceful and forced stop, an
//! awaitable terminal wait, and subscriptions to events and log lines. Raw Tokio
//! process types are never exposed outside this crate.

use std::sync::Arc;

use crate::entry::{ProcessEntry, ShutdownRequest};
use crate::error::ProcessError;
use crate::event::ProcessEventStream;
use crate::id::ProcessId;
use crate::log::LogStream;
use crate::snapshot::ProcessSnapshot;
use crate::spec::ProcessSpec;
use crate::state::{ExitReason, ProcessStatus};

/// A cloneable handle to one managed process.
#[derive(Clone)]
pub struct ProcessHandle {
    entry: Arc<ProcessEntry>,
}

impl ProcessHandle {
    /// Creates a handle from a shared entry.
    pub(crate) fn new(entry: Arc<ProcessEntry>) -> Self {
        Self { entry }
    }

    /// Returns the process identifier.
    #[must_use]
    pub fn id(&self) -> &ProcessId {
        self.entry.id()
    }

    /// Returns the current lifecycle status.
    #[must_use]
    pub fn status(&self) -> ProcessStatus {
        self.entry.status()
    }

    /// Returns the operating-system process id, while the process is alive.
    #[must_use]
    pub fn pid(&self) -> Option<u32> {
        self.entry.pid()
    }

    /// Returns the process specification.
    #[must_use]
    pub fn spec(&self) -> &ProcessSpec {
        self.entry.spec()
    }

    /// Returns `true` if the process has reached a terminal state.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        self.entry.status().is_terminal()
    }

    /// Returns `true` if the process is currently alive.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.entry.status().is_running()
    }

    /// Returns a point-in-time snapshot of the process.
    #[must_use]
    pub fn snapshot(&self) -> ProcessSnapshot {
        self.entry.snapshot()
    }

    /// Requests a graceful stop.
    ///
    /// The process is sent its configured graceful signal and is force-killed
    /// if it does not exit within the grace period. Stopping an already-terminal
    /// process is a no-op that returns `Ok`.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError`] only for an internal failure; a terminal process
    /// is treated as already stopped.
    pub fn stop(&self) -> Result<(), ProcessError> {
        self.request(ShutdownRequest::Graceful(ExitReason::Terminated))
    }

    /// Requests a stop because the owning task was cancelled.
    ///
    /// Equivalent to [`stop`](Self::stop), but the exit reason is recorded as
    /// [`ExitReason::Cancelled`].
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError`] only for an internal failure.
    pub fn cancel(&self) -> Result<(), ProcessError> {
        self.request(ShutdownRequest::Graceful(ExitReason::Cancelled))
    }

    /// Force-kills the process immediately with `SIGKILL`.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError`] only for an internal failure; a terminal process
    /// is treated as already stopped.
    pub fn kill(&self) -> Result<(), ProcessError> {
        self.request(ShutdownRequest::Force(ExitReason::Killed))
    }

    /// Waits until the process reaches a terminal state and returns a snapshot.
    ///
    /// # Errors
    ///
    /// This method does not fail for a normal process exit; it returns the
    /// terminal snapshot. The `Result` is reserved for future use and matches
    /// the engine's awaitable APIs.
    pub async fn wait(&self) -> Result<ProcessSnapshot, ProcessError> {
        Ok(self.entry.wait_terminal().await)
    }

    /// Subscribes to this process's events.
    #[must_use]
    pub fn subscribe_events(&self) -> ProcessEventStream {
        self.entry.subscribe_events()
    }

    /// Subscribes to this process's `stdout` and `stderr` lines.
    #[must_use]
    pub fn subscribe_logs(&self) -> LogStream {
        self.entry.subscribe_logs()
    }

    fn request(&self, request: ShutdownRequest) -> Result<(), ProcessError> {
        match self.entry.request_shutdown(request) {
            Ok(()) | Err(ProcessError::AlreadyTerminal { .. }) => Ok(()),
            Err(error) => Err(error),
        }
    }
}

impl std::fmt::Debug for ProcessHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessHandle")
            .field("id", self.id())
            .field("status", &self.status())
            .field("pid", &self.pid())
            .finish()
    }
}
