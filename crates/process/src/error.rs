//! The process-manager error model.
//!
//! [`ProcessError`] is the error type produced by the process manager's own
//! API. It is deliberately narrow: it names process-management failures (a
//! missing process, a refused transition, a spawn failure, a missing runtime)
//! rather than wrapping errors from the rest of the system.
//!
//! The process manager is infrastructure: its errors are mapped into the
//! application error at the boundary, exactly like the other adapters.

use thiserror::Error;

use crate::id::ProcessId;
use crate::state::ProcessStatus;

/// Errors produced by the process manager.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProcessError {
    /// The referenced process does not exist in the registry.
    #[error("process '{0}' was not found")]
    NotFound(ProcessId),

    /// The manager was built without an active Tokio runtime.
    #[error("process manager requires a Tokio runtime context")]
    NoRuntime,

    /// A process specification was invalid.
    #[error("invalid process specification: {reason}")]
    InvalidSpec {
        /// Why the specification was rejected.
        reason: String,
    },

    /// The operating system refused to spawn the process.
    #[error("failed to spawn '{program}': {message}")]
    Spawn {
        /// The process that could not be started.
        id: ProcessId,
        /// The executable that was requested.
        program: String,
        /// The underlying operating-system error.
        message: String,
    },

    /// The manager was asked to act on a process that has already finished.
    #[error("process '{id}' is already {status}")]
    AlreadyTerminal {
        /// The process that has finished.
        id: ProcessId,
        /// Its terminal status.
        status: ProcessStatus,
    },

    /// A lifecycle transition was requested that the state machine forbids.
    #[error("process '{id}' cannot transition from '{from}' to '{to}'")]
    InvalidTransition {
        /// The process whose transition was rejected.
        id: ProcessId,
        /// The current status.
        from: ProcessStatus,
        /// The requested status.
        to: ProcessStatus,
    },

    /// The manager is shutting down and no longer accepts new processes.
    #[error("process manager is shutting down")]
    ShuttingDown,

    /// Signal delivery is not supported on this platform.
    #[error("process signalling is not supported on this platform")]
    UnsupportedSignal,

    /// An operating-system operation failed.
    #[error("process operation failed: {message}")]
    Io {
        /// A human-readable description of the failure.
        message: String,
    },

    /// The manager failed for an internal reason.
    #[error("process manager error: {message}")]
    Internal {
        /// A human-readable description of the failure.
        message: String,
    },
}

impl ProcessError {
    /// Builds a [`ProcessError::InvalidSpec`].
    #[must_use]
    pub fn invalid_spec(reason: impl Into<String>) -> Self {
        Self::InvalidSpec {
            reason: reason.into(),
        }
    }

    /// Builds a [`ProcessError::Internal`].
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    /// Builds a [`ProcessError::Io`].
    #[must_use]
    pub fn io(message: impl Into<String>) -> Self {
        Self::Io {
            message: message.into(),
        }
    }
}
