//! Database migration management.
//!
//! The list is the configuration-derived migration setup of each
//! migration-enabled service. Live status (`pending`, `locked`) is an explicit
//! unknown here; running the provider is a task, triggered with `s` or `p`.

use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::{Cell, Row, Table};
use ratatui::Frame;

use crate::app::App;
use crate::components::table;
use crate::theme;

/// Draws the migration screen.
pub(crate) fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let migrations = app.visible_migrations();
    let rows: Vec<Row<'_>> = migrations
        .iter()
        .map(|migration| {
            let enabled = if migration.enabled {
                Cell::from("enabled").style(theme::success())
            } else {
                Cell::from("disabled").style(theme::muted())
            };
            let auth = if migration.requires_authorization {
                Cell::from("required").style(theme::warning())
            } else {
                Cell::from("no").style(theme::muted())
            };
            Row::new(vec![
                Cell::from(migration.service_id.as_str().to_owned()),
                enabled,
                Cell::from(migration.provider.label().to_owned()),
                Cell::from(migration.execution_mode.label().to_owned()),
                Cell::from(migration.environment.label().to_owned()),
                Cell::from(migration.database_identity.clone()),
                auth,
            ])
        })
        .collect();

    let requiring_auth = migrations
        .iter()
        .filter(|migration| migration.requires_authorization)
        .count();
    let title = format!(
        "Migrations  {} configured · {} need authorization",
        migrations.len(),
        requiring_auth
    );
    let widths = [
        Constraint::Min(16),
        Constraint::Length(9),
        Constraint::Length(10),
        Constraint::Length(7),
        Constraint::Length(19),
        Constraint::Min(22),
        Constraint::Length(11),
    ];
    let table = Table::new(rows, widths)
        .header(table::header(&[
            "Service",
            "Enabled",
            "Provider",
            "Mode",
            "Environment",
            "Target",
            "Auth",
        ]))
        .block(super::panel(app, title))
        .row_highlight_style(theme::selected_row())
        .highlight_symbol("> ");
    let mut state = table::state(app.state.cursor, app.state.list_offset);
    frame.render_stateful_widget(table, area, &mut state);
}
