//! Health inspection.
//!
//! Health is deliberately distinct from process state: a running process is not
//! assumed to be healthy. The screen renders the last observed aggregate and its
//! individual check outcomes; running a check again is a task (`h` / `H`).

use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::{Cell, Row, Table};
use ratatui::Frame;

use crate::app::App;
use crate::components::{status, table};
use crate::theme;

/// Draws the health screen.
pub(crate) fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let services = app.visible_services();
    let rows: Vec<Row<'_>> = services
        .iter()
        .map(|service| {
            let id = &service.service_id;
            let health = app.snapshot.health.get(id);
            let checked = health
                .and_then(|health| health.checked_at)
                .map_or_else(|| "-".to_owned(), |at| at.to_string());
            let failures = health.map_or(0, |health| health.consecutive_failures);
            let checks = match health {
                Some(health) if !health.checks.is_empty() => {
                    let passing = health
                        .checks
                        .iter()
                        .filter(|check| check.is_healthy())
                        .count();
                    format!("{passing}/{}", health.checks.len())
                }
                _ => "-".to_owned(),
            };
            Row::new(vec![
                Cell::from(service.display_name.clone()),
                Cell::from(status::process(service.process_state)),
                Cell::from(status::health(service.health_state)),
                Cell::from(checked),
                Cell::from(failures.to_string()),
                Cell::from(checks),
            ])
        })
        .collect();

    let unhealthy = services
        .iter()
        .filter(|service| !service.health_state.is_healthy())
        .count();
    let title = format!(
        "Health  {} services · {} not healthy",
        services.len(),
        unhealthy
    );
    let widths = [
        Constraint::Min(18),
        Constraint::Length(10),
        Constraint::Length(11),
        Constraint::Min(30),
        Constraint::Length(9),
        Constraint::Length(7),
    ];
    let table = Table::new(rows, widths)
        .header(table::header(&[
            "Service",
            "Process",
            "Health",
            "Last checked",
            "Failures",
            "Checks",
        ]))
        .block(super::panel(app, title))
        .row_highlight_style(theme::selected_row())
        .highlight_symbol("> ");
    let mut state = table::state(app.state.cursor, app.state.list_offset);
    frame.render_stateful_widget(table, area, &mut state);
}
