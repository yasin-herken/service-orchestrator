//! Logical process state.
//!
//! This is the state of a *service process* as observed by the application, not
//! a reflection of any particular OS process API. PIDs, signals, and child
//! process trees are managed entirely by the `process` infrastructure crate and
//! are never stored in the domain.
//!
//! A process being [`Running`](ProcessState::Running) does not imply the service
//! is healthy; health is a separate concern expressed by [`HealthCheck`].
//!
//! [`HealthCheck`]: crate::health::HealthCheck

use std::fmt;

use serde::{Deserialize, Serialize};

/// The logical state of a local service process.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    /// No process is associated with the service.
    NotRunning,
    /// The process is starting but is not yet ready.
    Starting,
    /// The process is running.
    Running,
    /// The process is shutting down.
    Stopping,
    /// The process has stopped.
    Stopped,
    /// The process exited unexpectedly or could not start.
    Failed,
    /// The state could not be determined.
    #[default]
    Unknown,
}

impl ProcessState {
    /// Returns `true` if a process currently exists for the service.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Stopping)
    }

    /// Returns `true` if the service is fully running.
    #[must_use]
    pub const fn is_running(self) -> bool {
        matches!(self, Self::Running)
    }

    /// Returns `true` if the process has reached a terminal state.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Stopped | Self::Failed)
    }
}

impl fmt::Display for ProcessState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::NotRunning => "not_running",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        };
        f.write_str(label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_active_states() {
        assert!(ProcessState::Starting.is_active());
        assert!(ProcessState::Running.is_active());
        assert!(ProcessState::Stopping.is_active());
        assert!(!ProcessState::NotRunning.is_active());
        assert!(!ProcessState::Stopped.is_active());
    }

    #[test]
    fn classifies_terminal_states() {
        assert!(ProcessState::Stopped.is_terminal());
        assert!(ProcessState::Failed.is_terminal());
        assert!(!ProcessState::Running.is_terminal());
    }

    #[test]
    fn running_is_not_the_default() {
        assert_eq!(ProcessState::default(), ProcessState::Unknown);
        assert!(ProcessState::Running.is_running());
    }
}
