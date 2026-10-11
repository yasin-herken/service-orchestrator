//! The visual system: a small, consistent set of styles.
//!
//! Colours are deliberately conservative. Status is always expressed with a
//! colour *and* a text label, so the interface stays readable on a monochrome
//! terminal and for colour-blind developers.

use ratatui::style::{Color, Modifier, Style};
use service_orchestrator_application::{HealthState, LogLevel};
use service_orchestrator_domain::{GitState, ProcessState, TaskStatus};

/// The accent style used for titles and focus.
#[must_use]
pub(crate) fn accent() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

/// A dim style for secondary information.
#[must_use]
pub(crate) fn muted() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// The style for the currently focused panel border.
#[must_use]
pub(crate) fn focused_border() -> Style {
    Style::default().fg(Color::Cyan)
}

/// The style for an unfocused panel border.
#[must_use]
pub(crate) fn border() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// The success style.
#[must_use]
pub(crate) fn success() -> Style {
    Style::default().fg(Color::Green)
}

/// The warning style.
#[must_use]
pub(crate) fn warning() -> Style {
    Style::default().fg(Color::Yellow)
}

/// The error style.
#[must_use]
pub(crate) fn error() -> Style {
    Style::default().fg(Color::Red)
}

/// The style for the selected row in a table or list.
#[must_use]
pub(crate) fn selected_row() -> Style {
    Style::default()
        .bg(Color::Indexed(236))
        .add_modifier(Modifier::BOLD)
}

/// A style for a row that is both selected and multi-selected.
#[must_use]
pub(crate) fn selection_marker() -> Style {
    Style::default().fg(Color::Yellow)
}

/// The style for a process state.
#[must_use]
pub(crate) fn process_style(state: ProcessState) -> Style {
    match state {
        ProcessState::Running => success(),
        ProcessState::Starting | ProcessState::Stopping => warning(),
        ProcessState::Failed => error(),
        ProcessState::Stopped | ProcessState::NotRunning => muted(),
        ProcessState::Unknown => Style::default(),
    }
}

/// The style for a health state.
#[must_use]
pub(crate) fn health_style(state: HealthState) -> Style {
    match state {
        HealthState::Healthy => success(),
        HealthState::Degraded | HealthState::Checking | HealthState::Unreachable => warning(),
        HealthState::Unhealthy => error(),
        HealthState::Unknown => muted(),
    }
}

/// The style for a Git state.
#[must_use]
pub(crate) fn git_style(state: GitState) -> Style {
    match state {
        GitState::Clean => success(),
        GitState::Modified => warning(),
        GitState::Ahead | GitState::Behind | GitState::Diverged => warning(),
        GitState::Unknown => muted(),
    }
}

/// The style for a task status.
#[must_use]
pub(crate) fn task_status_style(status: TaskStatus) -> Style {
    match status {
        TaskStatus::Succeeded => success(),
        TaskStatus::Failed => error(),
        TaskStatus::Running => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        TaskStatus::Queued | TaskStatus::Blocked => warning(),
        TaskStatus::Cancelled | TaskStatus::Skipped => muted(),
    }
}

/// The style for a log level.
#[must_use]
pub(crate) fn level_style(level: LogLevel) -> Style {
    match level {
        LogLevel::Trace | LogLevel::Debug => muted(),
        LogLevel::Info => Style::default(),
        LogLevel::Warn => warning(),
        LogLevel::Error => error(),
    }
}
