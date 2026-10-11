//! Task monitoring.
//!
//! Every row is a task the engine reported. The screen never keeps a registry of
//! its own; it renders the engine's list, and the snapshot is refreshed on task
//! events and on a periodic tick.

use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::{Cell, Row, Table};
use ratatui::Frame;

use crate::app::App;
use crate::components::{status, table};
use crate::theme;

/// Draws the task dashboard.
pub(crate) fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let tasks = app.visible_tasks();
    let rows: Vec<Row<'_>> = tasks
        .iter()
        .map(|task| {
            let target = task
                .service
                .as_ref()
                .map(ToString::to_string)
                .or_else(|| task.library.as_ref().map(ToString::to_string))
                .unwrap_or_else(|| "-".to_owned());
            let progress = progress_cell(task);
            let start = task.started_at.unwrap_or(task.created_at);
            let duration = super::format_duration(start, task.finished_at);
            let message = task
                .error
                .as_ref()
                .map(|failure| format!("{}: {}", failure.code, failure.message))
                .or_else(|| task.progress.message.clone())
                .unwrap_or_default();
            Row::new(vec![
                Cell::from(task.id.as_str().to_owned()),
                Cell::from(task.kind.label().to_owned()),
                Cell::from(target),
                Cell::from(status::task_status(task.status)),
                progress,
                Cell::from(duration),
                Cell::from(message),
            ])
        })
        .collect();

    let active = tasks.iter().filter(|task| task.status.is_active()).count();
    let title = format!(
        "Tasks  {} shown · {} active · filter: {}",
        tasks.len(),
        active,
        app.state.task_filter.label()
    );
    let widths = [
        Constraint::Length(12),
        Constraint::Length(18),
        Constraint::Length(16),
        Constraint::Length(10),
        Constraint::Length(14),
        Constraint::Length(9),
        Constraint::Min(20),
    ];
    let table = Table::new(rows, widths)
        .header(table::header(&[
            "Task", "Kind", "Target", "Status", "Progress", "Elapsed", "Message",
        ]))
        .block(super::panel(app, title))
        .row_highlight_style(theme::selected_row())
        .highlight_symbol("> ");
    let mut state = table::state(app.state.cursor, app.state.list_offset);
    frame.render_stateful_widget(table, area, &mut state);
}

/// Renders a task's real progress; it is never invented.
fn progress_cell(task: &service_orchestrator_domain::Task) -> Cell<'static> {
    match (task.progress.fraction(), task.progress.total) {
        (Some(fraction), Some(total)) => Cell::from(format!(
            "{:.0}% ({}/{})",
            fraction * 100.0,
            task.progress.current,
            total
        )),
        (None, Some(total)) if total > 0 => {
            Cell::from(format!("{}/{}", task.progress.current, total))
        }
        _ => Cell::from("indeterminate").style(theme::muted()),
    }
}
