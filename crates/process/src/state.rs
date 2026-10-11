//! Process lifecycle state, exit information, and termination reasons.
//!
//! The process manager owns a richer state machine than the domain's
//! [`ProcessState`](service_orchestrator_domain::ProcessState): it distinguishes
//! *why* a process ended (a clean exit, a failure, a signal, a forced kill, a
//! timeout, or a cancellation). [`ProcessStatus::domain_state`] maps the rich
//! state back into the interface-neutral vocabulary the application and the TUI
//! already understand.
//!
//! # State machine
//!
//! ```text
//! Created ──> Starting ──> Running ──┬──> Stopping ──┬──> Stopped
//!                          ▲         │               ├──> Exited
//!                          │         │               ├──> Failed
//!                          │         │               └──> Killed
//!                          │         ├───────────────> Stopped
//!                          │         ├───────────────> Exited
//!                          │         ├───────────────> Failed
//!                          │         └───────────────> Killed
//!                          │
//!                          └────────────────────────────── Starting
//!                             (a restart begins a new execution)
//!
//! Starting ──> Failed            (the process could not be spawned)
//! terminal: Stopped | Exited | Failed | Killed   (no outgoing edges)
//! ```
//!
//! `Running -> Starting` and `Stopping -> Starting` are the single extra edge,
//! used only when a [`RestartPolicy`](crate::RestartPolicy) starts a fresh
//! execution instance on the same handle.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use service_orchestrator_domain::{ProcessState, Timestamp};

/// The lifecycle state of a managed process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStatus {
    /// The specification exists but nothing has been spawned yet.
    Created,
    /// A spawn is in progress.
    Starting,
    /// The process is alive and being monitored.
    Running,
    /// A graceful stop has been requested.
    Stopping,
    /// The process was stopped by the manager (for example after `SIGTERM`).
    Stopped,
    /// The process exited on its own with a success status.
    Exited,
    /// The process failed to start or exited with a failure status.
    Failed,
    /// The process was force-killed (`SIGKILL`).
    Killed,
}

impl ProcessStatus {
    /// Returns `true` if no further transition is possible.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Stopped | Self::Exited | Self::Failed | Self::Killed
        )
    }

    /// Returns `true` if a process exists or is expected to.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(
            self,
            Self::Created | Self::Starting | Self::Running | Self::Stopping
        )
    }

    /// Returns `true` if the process is currently alive.
    #[must_use]
    pub const fn is_running(self) -> bool {
        matches!(self, Self::Running)
    }

    /// Returns `true` if `next` is a legal successor of this status.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        use ProcessStatus::{
            Created, Exited, Failed, Killed, Running, Starting, Stopped, Stopping,
        };
        matches!(
            (self, next),
            (Created, Starting)
                | (Starting, Running)
                | (Starting, Failed)
                | (Starting, Stopped)
                | (Starting, Killed)
                | (Running, Starting) // restart begins a new execution
                | (Running, Stopping)
                | (Running, Stopped)
                | (Running, Exited)
                | (Running, Failed)
                | (Running, Killed)
                | (Stopping, Starting) // restart after a stop
                | (Stopping, Stopped)
                | (Stopping, Exited)
                | (Stopping, Failed)
                | (Stopping, Killed)
        )
    }

    /// Maps this rich state into the interface-neutral domain vocabulary.
    #[must_use]
    pub const fn domain_state(self) -> ProcessState {
        match self {
            Self::Created => ProcessState::NotRunning,
            Self::Starting => ProcessState::Starting,
            Self::Running => ProcessState::Running,
            Self::Stopping => ProcessState::Stopping,
            Self::Stopped => ProcessState::Stopped,
            Self::Exited => ProcessState::Stopped,
            Self::Failed => ProcessState::Failed,
            Self::Killed => ProcessState::Stopped,
        }
    }

    /// Returns a short, stable, human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Exited => "exited",
            Self::Failed => "failed",
            Self::Killed => "killed",
        }
    }
}

impl fmt::Display for ProcessStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Why a process execution ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitReason {
    /// The process exited on its own with a success status.
    Exited,
    /// The process exited on its own with a non-zero status.
    Failed,
    /// The process was terminated by a signal the manager did not send.
    Signalled,
    /// The manager stopped the process gracefully (for example `SIGTERM`).
    Terminated,
    /// The process was stopped because its task was cancelled.
    Cancelled,
    /// The process was force-killed after a graceful stop failed.
    Killed,
    /// The process exceeded its configured timeout.
    TimedOut,
    /// The process could not be started.
    SpawnFailed,
}

impl ExitReason {
    /// Returns the terminal status implied by this reason.
    #[must_use]
    pub const fn status(self) -> ProcessStatus {
        match self {
            Self::Exited => ProcessStatus::Exited,
            Self::Failed | Self::Signalled | Self::SpawnFailed => ProcessStatus::Failed,
            Self::Terminated | Self::Cancelled => ProcessStatus::Stopped,
            Self::Killed | Self::TimedOut => ProcessStatus::Killed,
        }
    }

    /// Returns `true` if the execution ended in failure.
    #[must_use]
    pub const fn is_failure(self) -> bool {
        matches!(self, Self::Failed | Self::Signalled | Self::SpawnFailed)
    }

    /// Returns a short, stable, human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Exited => "exited",
            Self::Failed => "failed",
            Self::Signalled => "signalled",
            Self::Terminated => "terminated",
            Self::Cancelled => "cancelled",
            Self::Killed => "killed",
            Self::TimedOut => "timed_out",
            Self::SpawnFailed => "spawn_failed",
        }
    }
}

impl fmt::Display for ExitReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Structured information about how one process execution ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitInfo {
    /// The process exit code, when the platform reported one.
    pub code: Option<i32>,
    /// The terminating signal number, when the process was signalled.
    pub signal: Option<i32>,
    /// Why the execution ended.
    pub reason: ExitReason,
    /// When the execution started.
    pub started_at: Timestamp,
    /// When the execution finished.
    pub finished_at: Timestamp,
    /// How long the execution ran.
    pub duration: Duration,
}

impl ExitInfo {
    /// Builds exit information from its parts, computing the duration.
    #[must_use]
    pub fn new(
        code: Option<i32>,
        signal: Option<i32>,
        reason: ExitReason,
        started_at: Timestamp,
        finished_at: Timestamp,
    ) -> Self {
        let millis = (finished_at.epoch_millis() - started_at.epoch_millis()).max(0) as u64;
        Self {
            code,
            signal,
            reason,
            started_at,
            finished_at,
            duration: Duration::from_millis(millis),
        }
    }

    /// Returns the terminal status implied by this exit information.
    #[must_use]
    pub const fn status(&self) -> ProcessStatus {
        self.reason.status()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_and_active_are_disjoint() {
        for status in [
            ProcessStatus::Created,
            ProcessStatus::Starting,
            ProcessStatus::Running,
            ProcessStatus::Stopping,
            ProcessStatus::Stopped,
            ProcessStatus::Exited,
            ProcessStatus::Failed,
            ProcessStatus::Killed,
        ] {
            assert_ne!(status.is_terminal(), status.is_active());
        }
    }

    #[test]
    fn terminal_states_have_no_successors() {
        for status in [
            ProcessStatus::Stopped,
            ProcessStatus::Exited,
            ProcessStatus::Failed,
            ProcessStatus::Killed,
        ] {
            for next in [
                ProcessStatus::Created,
                ProcessStatus::Starting,
                ProcessStatus::Running,
                ProcessStatus::Stopping,
                ProcessStatus::Stopped,
                ProcessStatus::Exited,
                ProcessStatus::Failed,
                ProcessStatus::Killed,
            ] {
                assert!(!status.can_transition_to(next));
            }
        }
    }

    #[test]
    fn accepts_the_documented_transitions() {
        use ProcessStatus::*;
        assert!(Created.can_transition_to(Starting));
        assert!(Starting.can_transition_to(Running));
        assert!(Starting.can_transition_to(Failed));
        assert!(Running.can_transition_to(Stopping));
        assert!(Running.can_transition_to(Starting));
        assert!(Running.can_transition_to(Exited));
        assert!(Running.can_transition_to(Killed));
        assert!(Stopping.can_transition_to(Stopped));
        assert!(Stopping.can_transition_to(Killed));
        assert!(Stopping.can_transition_to(Starting));
    }

    #[test]
    fn rejects_illegal_transitions() {
        use ProcessStatus::*;
        assert!(!Created.can_transition_to(Running));
        assert!(!Running.can_transition_to(Created));
        assert!(!Stopped.can_transition_to(Running));
    }

    #[test]
    fn maps_to_domain_states() {
        assert_eq!(ProcessStatus::Running.domain_state(), ProcessState::Running);
        assert_eq!(
            ProcessStatus::Starting.domain_state(),
            ProcessState::Starting
        );
        assert_eq!(ProcessStatus::Killed.domain_state(), ProcessState::Stopped);
        assert_eq!(ProcessStatus::Failed.domain_state(), ProcessState::Failed);
    }

    #[test]
    fn reasons_map_to_terminal_statuses() {
        assert_eq!(ExitReason::Exited.status(), ProcessStatus::Exited);
        assert_eq!(ExitReason::Failed.status(), ProcessStatus::Failed);
        assert_eq!(ExitReason::Signalled.status(), ProcessStatus::Failed);
        assert_eq!(ExitReason::Terminated.status(), ProcessStatus::Stopped);
        assert_eq!(ExitReason::Cancelled.status(), ProcessStatus::Stopped);
        assert_eq!(ExitReason::Killed.status(), ProcessStatus::Killed);
        assert_eq!(ExitReason::TimedOut.status(), ProcessStatus::Killed);
    }

    #[test]
    fn exit_info_computes_duration() {
        use time::macros::datetime;
        let start = Timestamp::from_offset(datetime!(2026-10-07 12:00:00 UTC));
        let finish = Timestamp::from_offset(datetime!(2026-10-07 12:00:02 UTC));
        let info = ExitInfo::new(Some(0), None, ExitReason::Exited, start, finish);
        assert_eq!(info.duration, Duration::from_secs(2));
        assert_eq!(info.status(), ProcessStatus::Exited);
    }
}
