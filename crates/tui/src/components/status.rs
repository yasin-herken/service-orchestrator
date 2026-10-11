//! Status spans.
//!
//! Every status is rendered as a coloured *and* textual value, so a monochrome
//! terminal — or a colour-blind developer — can still read it.

use ratatui::text::Span;
use service_orchestrator_application::{HealthState, LogLevel};
use service_orchestrator_domain::{GitState, ProcessState, TaskStatus};

use crate::theme;

/// Renders a process state.
#[must_use]
pub(crate) fn process(state: ProcessState) -> Span<'static> {
    Span::styled(state.to_string(), theme::process_style(state))
}

/// Renders a health state.
#[must_use]
pub(crate) fn health(state: HealthState) -> Span<'static> {
    Span::styled(state.label().to_owned(), theme::health_style(state))
}

/// Renders a Git state.
#[must_use]
pub(crate) fn git(state: GitState) -> Span<'static> {
    Span::styled(state.to_string(), theme::git_style(state))
}

/// Renders a task status.
#[must_use]
pub(crate) fn task_status(status: TaskStatus) -> Span<'static> {
    Span::styled(status.label().to_owned(), theme::task_status_style(status))
}

/// Renders a log level.
#[must_use]
pub(crate) fn level(level: LogLevel) -> Span<'static> {
    Span::styled(level.label().to_owned(), theme::level_style(level))
}
