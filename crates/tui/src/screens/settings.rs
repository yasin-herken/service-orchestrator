//! Read-oriented settings.
//!
//! The screen reports configuration the application loaded; it does not edit it.
//! A configuration editor would require application APIs that safely persist
//! validated changes, which do not exist yet.

use std::collections::BTreeSet;

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::state::LOG_BUFFER_CAPACITY;
use crate::theme;

/// Draws the settings screen.
pub(crate) fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let snapshot = &app.snapshot;
    let mut lines: Vec<Line<'_>> = vec![
        Line::from(Span::styled("Configuration", theme::accent())),
        field("Config file", app.config_path()),
        field("Schema version", &snapshot.config_version.to_string()),
        field("Workspaces", &snapshot.workspaces.len().to_string()),
        field("Active workspace", &snapshot.workspace_name),
    ];

    if let Some(root) = &snapshot.workspace_root {
        lines.push(field("Workspace root", root));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Execution", theme::accent())));
    lines.push(field(
        "Max parallel tasks",
        &snapshot.max_parallel_tasks.to_string(),
    ));
    lines.push(field(
        "Default timeout",
        &format!("{}s", snapshot.default_timeout_seconds),
    ));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Runtime requirements",
        theme::accent(),
    )));
    let runtimes: BTreeSet<String> = snapshot
        .services_config
        .values()
        .flat_map(|service| service.runtimes.iter())
        .map(ToString::to_string)
        .collect();
    if runtimes.is_empty() {
        lines.push(muted("  none declared"));
    } else {
        for runtime in runtimes {
            lines.push(Line::from(vec![
                Span::styled("  ", theme::muted()),
                Span::raw(runtime),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Logging", theme::accent())));
    lines.push(field(
        "Log buffer",
        &format!("{LOG_BUFFER_CAPACITY} entries"),
    ));
    lines.push(field("Level filter", "per view (v to cycle)"));

    frame.render_widget(
        Paragraph::new(lines)
            .block(super::panel(app, "Settings"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// A `label: value` line.
fn field(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<20}"), theme::muted()),
        Span::raw(value.to_owned()),
    ])
}

/// A muted placeholder line.
fn muted(text: &str) -> Line<'static> {
    Line::from(Span::styled(text.to_owned(), theme::muted()))
}
