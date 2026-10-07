//! # TUI
//!
//! Ratatui terminal interface adapter.
//!
//! Presentation only: reads application state, renders it, and dispatches
//! intents to application use cases. It must not contain Git, build, process,
//! or workflow logic. It depends only on the application core.
