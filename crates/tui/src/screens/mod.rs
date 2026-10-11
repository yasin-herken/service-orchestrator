//! The screen router and shared panel chrome.
//!
//! Each screen is a small module that renders one view from already-read
//! application state. Screens never mutate application state and never call the
//! application core; the state machine in [`crate::app`] has already done that.

mod builds;
mod dashboard;
mod git;
mod health;
mod libraries;
mod logs;
mod migrations;
mod service_detail;
mod services;
mod settings;
mod tasks;

use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders};
use ratatui::Frame;
use service_orchestrator_domain::Timestamp;

use crate::app::App;
use crate::state::{Focus, Screen};
use crate::theme;

/// Draws the content panel for the active screen.
pub(crate) fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    match app.state.screen {
        Screen::Dashboard => dashboard::draw(frame, area, app),
        Screen::Services => services::draw(frame, area, app),
        Screen::ServiceDetail => service_detail::draw(frame, area, app),
        Screen::Git => git::draw(frame, area, app),
        Screen::Builds => builds::draw(frame, area, app),
        Screen::Libraries => libraries::draw(frame, area, app),
        Screen::Migrations => migrations::draw(frame, area, app),
        Screen::Tasks => tasks::draw(frame, area, app),
        Screen::Logs => logs::draw(frame, area, app),
        Screen::Health => health::draw(frame, area, app),
        Screen::Settings => settings::draw(frame, area, app),
    }
}

/// Builds the bordered panel used by every content screen.
pub(crate) fn panel(app: &App, title: impl Into<String>) -> Block<'static> {
    let border_style = if app.state.focus == Focus::Content {
        theme::focused_border()
    } else {
        theme::border()
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Line::from(title.into()).style(theme::accent()))
}

/// Formats an elapsed time between two timestamps, or since `start`.
///
/// Only observed task timestamps are used; the interface never invents a
/// duration for work the engine has not reported.
#[must_use]
pub(crate) fn format_duration(start: Timestamp, end: Option<Timestamp>) -> String {
    let end = end.unwrap_or_else(Timestamp::now);
    let millis = end
        .epoch_millis()
        .saturating_sub(start.epoch_millis())
        .max(0);
    let total_seconds = (millis / 1000) as u64;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    if minutes >= 60 {
        format!("{}h{:02}m", minutes / 60, minutes % 60)
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}
