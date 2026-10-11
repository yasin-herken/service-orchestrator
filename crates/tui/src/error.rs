//! Error type for the TUI adapter.
//!
//! The TUI never invents application failures: application errors are rendered
//! as structured dialogs (see [`crate::state::Dialog`]). This error type exists
//! only for failures *of the interface itself* — terminal setup, input reading,
//! and drawing.

use std::io;

use thiserror::Error;

/// A failure of the terminal interface.
#[derive(Debug, Error)]
pub enum TuiError {
    /// The terminal could not be initialized or restored.
    #[error("terminal error: {0}")]
    Terminal(#[from] io::Error),

    /// The application failed to start.
    #[error("startup error: {0}")]
    Startup(String),

    /// The runtime terminated the interface unexpectedly.
    #[error("the interface stopped unexpectedly: {0}")]
    Runtime(String),
}
