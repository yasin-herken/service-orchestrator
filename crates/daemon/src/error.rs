//! Errors produced while starting or running the daemon.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// A failure to start or run the local daemon.
#[derive(Debug, Error)]
pub enum DaemonError {
    /// Another daemon already owns the runtime directory.
    #[error("another service-orchestrator daemon is already running at {0}")]
    AlreadyRunning(PathBuf),

    /// A filesystem or socket operation failed.
    #[error("{operation} failed for {}: {source}", path.display())]
    Io {
        /// What was being attempted.
        operation: &'static str,
        /// The path involved.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: io::Error,
    },

    /// The daemon could not be initialized.
    #[error("the daemon could not start: {0}")]
    Startup(String),
}

impl DaemonError {
    /// Builds an [`DaemonError::Io`].
    pub(crate) fn io(operation: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.into(),
            source,
        }
    }
}
