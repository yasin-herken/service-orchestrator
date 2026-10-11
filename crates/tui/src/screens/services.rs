//! The service list.
//!
//! Renders the statuses the application core already produced. The screen never
//! derives business meaning: the filter is applied by [`crate::state`] and the
//! rows are read straight from the snapshot.

use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::{Cell, Row, Table};
use ratatui::Frame;

use crate::app::App;
use crate::components::{status, table};
use crate::theme;

/// Draws the service list.
pub(crate) fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let services = app.visible_services();
    let rows: Vec<Row<'_>> = services
        .iter()
        .map(|service| {
            let selected = app.state.selection.contains(&service.service_id);
            let branch = service
                .branch
                .as_ref()
                .map_or_else(|| "-".to_owned(), |branch| branch.as_str().to_owned());
            let port = service
                .ports
                .first()
                .map_or_else(|| "-".to_owned(), |port| format!(":{}", port.value()));
            Row::new(vec![
                table::marker(selected),
                Cell::from(service.display_name.clone()),
                Cell::from(service.kind.label().to_owned()),
                Cell::from(status::process(service.process_state)),
                Cell::from(status::health(service.health_state)),
                Cell::from(branch),
                Cell::from(port),
            ])
        })
        .collect();

    let title = format!(
        "Services  {} shown · {} selected · filter: {}",
        services.len(),
        app.state.selection.len(),
        app.state.service_filter.label()
    );
    let widths = [
        Constraint::Length(3),
        Constraint::Min(16),
        Constraint::Length(9),
        Constraint::Length(10),
        Constraint::Length(11),
        Constraint::Length(14),
        Constraint::Length(7),
    ];
    let table = Table::new(rows, widths)
        .header(table::header(&[
            "", "Service", "Type", "Process", "Health", "Branch", "Port",
        ]))
        .block(super::panel(app, title))
        .row_highlight_style(theme::selected_row())
        .highlight_symbol("> ");
    let mut state = table::state(app.state.cursor, app.state.list_offset);
    frame.render_stateful_widget(table, area, &mut state);
}
