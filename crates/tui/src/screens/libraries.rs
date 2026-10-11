//! Shared library management.
//!
//! The list is read from configuration; inspection columns are populated only
//! after the developer asks for them (`v` verify, `a` alignment, `r` status), so
//! the screen never inspects artifacts or scans repositories on its own.

use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::{Cell, Row, Table};
use ratatui::Frame;

use crate::app::App;
use crate::components::table;
use crate::theme;

/// Draws the library list.
pub(crate) fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let libraries = app.visible_libraries();
    let rows: Vec<Row<'_>> = libraries
        .iter()
        .map(|library| {
            let id = &library.id;
            let selected = app.state.library_selection.contains(id);
            let artifact = library
                .artifact
                .as_ref()
                .map_or_else(|| "-".to_owned(), ToString::to_string);
            let build = library
                .build_system
                .map_or_else(|| "-".to_owned(), |system| system.label().to_owned());
            let git = app
                .state
                .library_status
                .get(id)
                .map_or_else(|| "-".to_owned(), |status| status.git_state.to_string());
            let present = match app.state.artifact_verification.get(id) {
                Some(verification) if verification.present => {
                    Cell::from("present").style(theme::success())
                }
                Some(_) => Cell::from("missing").style(theme::warning()),
                None => Cell::from("-").style(theme::muted()),
            };
            let alignment = match app.state.alignment.get(id) {
                Some(plan) if plan.is_fully_aligned() => {
                    Cell::from("aligned").style(theme::success())
                }
                Some(plan) => Cell::from(format!(
                    "{}/{} aligned",
                    plan.aligned_count(),
                    plan.consumers.len()
                ))
                .style(theme::warning()),
                None => Cell::from("-").style(theme::muted()),
            };
            Row::new(vec![
                table::marker(selected),
                Cell::from(library.display_name.clone()),
                Cell::from(artifact),
                Cell::from(build),
                Cell::from(git),
                present,
                alignment,
            ])
        })
        .collect();

    let title = format!(
        "Libraries  {} · {} selected",
        libraries.len(),
        app.state.library_selection.len()
    );
    let widths = [
        Constraint::Length(3),
        Constraint::Min(16),
        Constraint::Length(28),
        Constraint::Length(8),
        Constraint::Length(9),
        Constraint::Length(9),
        Constraint::Length(14),
    ];
    let table = Table::new(rows, widths)
        .header(table::header(&[
            "",
            "Library",
            "Artifact",
            "Build",
            "Git",
            "Artifact",
            "Consumers",
        ]))
        .block(super::panel(app, title))
        .row_highlight_style(theme::selected_row())
        .highlight_symbol("> ");
    let mut state = table::state(app.state.cursor, app.state.list_offset);
    frame.render_stateful_widget(table, area, &mut state);
}
