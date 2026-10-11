//! Log viewing.
//!
//! A bounded, filterable view over the log entries the application returned. The
//! view holds at most [`crate::state::LOG_BUFFER_CAPACITY`] entries, so a noisy
//! service can never grow the interface without limit.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::components::status;
use crate::state::LogTarget;
use crate::theme;

/// Draws the log viewer.
pub(crate) fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(view) = app.state.log_view.as_ref() else {
        frame.render_widget(
            Paragraph::new("No log source selected.\n\nOpen a service's logs from the Services or Health screen with [l].")
                .block(super::panel(app, "Logs"))
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    };

    let target = match &view.target {
        LogTarget::Service(id) => format!("service {id}"),
        LogTarget::Task(id) => format!("task {id}"),
    };
    let level = view
        .minimum_level
        .map_or_else(|| "all".to_owned(), |level| level.label().to_owned());
    let title = format!(
        "Logs — {target}  ·  {} entries  ·  level ≥ {level}  ·  {}",
        view.entries.len(),
        if view.follow { "FOLLOWING" } else { "paused" }
    );

    let lines: Vec<Line<'_>> = view
        .entries
        .iter()
        .map(|entry| {
            Line::from(vec![
                Span::styled(
                    format!("{} ", clock(entry.timestamp.epoch_millis())),
                    theme::muted(),
                ),
                Span::raw(" "),
                status::level(entry.level),
                Span::raw(" "),
                Span::styled(format!("{:<9}", entry.source.label()), theme::muted()),
                Span::raw(entry.message.clone()),
            ])
        })
        .collect();

    frame.render_widget(
        Paragraph::new(lines)
            .block(super::panel(app, title))
            .scroll((view.scroll, 0))
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// Formats a millisecond epoch as `HH:MM:SS` in UTC.
fn clock(epoch_millis: i128) -> String {
    let seconds = epoch_millis.div_euclid(1000);
    let day = seconds.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", day / 3600, (day % 3600) / 60, day % 60)
}
