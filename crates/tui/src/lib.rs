//! # TUI
//!
//! Ratatui terminal interface adapter.
//!
//! Presentation only: reads application state, renders it, and dispatches
//! intents to application use cases. It must not contain Git, build, process,
//! or workflow logic. It depends only on the application core.
//!
//! The interface is split into small, single-purpose modules:
//!
//! - [`App`] is the state machine: it turns an [`Action`] into either a change
//!   of interface state or an application-core call;
//! - [`state`] holds presentation state ([`UiState`]) and the read-only
//!   application-state cache ([`Snapshot`]);
//! - [`screens`] renders each screen; [`components`] holds the shared widgets;
//! - [`run`] owns the terminal lifecycle and the async event loop;
//! - [`testing`] provides in-memory fakes for tests.
//!
//! Nothing here executes infrastructure. Every mutating operation goes through
//! the application core; every long-running operation returns a task or
//! workflow identifier that the interface observes through the task engine.
//!
//! See `docs/tui.md` and `docs/adr/013-ratatui-tui.md`.

mod action;
mod app;
mod components;
mod error;
mod event;
mod input;
mod observer;
mod router;
mod screens;
mod state;
mod terminal;
mod theme;

pub mod testing;

pub use action::Action;
pub use app::App;
pub use error::TuiError;
pub use event::{AppEvent, AsyncOutcome, Effect};
pub use input::{InputEvent, Key};
pub use observer::TaskObserver;
pub use state::{
    ConfirmDialog, Dialog, Focus, InputDialog, InputPurpose, LogTarget, LogView, MessageDialog,
    Screen, SearchState, SelectDialog, SelectOption, ServiceFilter, Snapshot, TaskFilter, Toast,
    ToastLevel, UiState,
};
pub use terminal::run;
