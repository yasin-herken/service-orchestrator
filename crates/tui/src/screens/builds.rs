//! Build management.
//!
//! The screen shows the build system the application reports for each service
//! and whether a build command is configured; it never hard-codes Maven or npm
//! behaviour. Builds run through the application core as tasks.

use ratatui::layout::{Constraint, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Row, Table};
use ratatui::Frame;
use service_orchestrator_domain::{Task, TaskKind};

use crate::app::App;
use crate::components::{status, table};
use crate::theme;

/// Draws the build screen.
pub(crate) fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let services = app.visible_services();
    let rows: Vec<Row<'_>> = services
        .iter()
        .map(|service| {
            let id = &service.service_id;
            let config = app.snapshot.service_config(id);
            let build_system = config
                .and_then(|service| service.build_system)
                .map_or_else(|| "-".to_owned(), |system| system.label().to_owned());
            let command = match config {
                Some(service) if service.has_build_command() => {
                    Span::styled("configured", theme::success())
                }
                Some(_) => Span::styled("none", theme::muted()),
                None => Span::styled("-", theme::muted()),
            };
            let last = latest_build(app, id);
            let last_cell = match last {
                Some(task) => Cell::from(Line::from(vec![
                    status::task_status(task.status),
                    Span::raw(format!(" {}", task.kind.label())),
                ])),
                None => Cell::from("-"),
            };
            Row::new(vec![
                Cell::from(service.display_name.clone()),
                Cell::from(build_system),
                Cell::from(command),
                last_cell,
            ])
        })
        .collect();

    let buildable = services
        .iter()
        .filter(|service| {
            app.snapshot
                .service_config(&service.service_id)
                .is_some_and(service_orchestrator_domain::Service::has_build_command)
        })
        .count();
    let title = format!("Builds  {buildable} buildable of {} shown", services.len());
    let widths = [
        Constraint::Min(18),
        Constraint::Length(12),
        Constraint::Length(14),
        Constraint::Length(24),
    ];
    let table = Table::new(rows, widths)
        .header(table::header(&[
            "Service",
            "Build system",
            "Build command",
            "Last build",
        ]))
        .block(super::panel(app, title))
        .row_highlight_style(theme::selected_row())
        .highlight_symbol("> ");
    let mut state = table::state(app.state.cursor, app.state.list_offset);
    frame.render_stateful_widget(table, area, &mut state);
}

/// The most recent build-kind task that targeted `id`.
fn latest_build<'a>(app: &'a App, id: &service_orchestrator_domain::ServiceId) -> Option<&'a Task> {
    app.snapshot
        .tasks
        .iter()
        .filter(|task| task.service.as_ref() == Some(id) && is_build(&task.kind))
        .max_by_key(|task| task.created_at.epoch_millis())
}

/// Whether a task kind is a build step.
fn is_build(kind: &TaskKind) -> bool {
    matches!(
        kind,
        TaskKind::BuildService
            | TaskKind::MavenBuild
            | TaskKind::NpmInstall
            | TaskKind::NpmBuild
            | TaskKind::BuildLibrary
    )
}
