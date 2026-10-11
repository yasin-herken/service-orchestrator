//! Git repository management.
//!
//! The screen lists every service in repository order and overlays the last
//! inspected [`RepositoryStatus`](service_orchestrator_domain::RepositoryStatus)
//! when one is available. Status is only inspected on demand (`r`), so the
//! screen never scans every repository on each frame.

use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::{Cell, Row, Table};
use ratatui::Frame;

use crate::app::App;
use crate::components::{status, table};
use crate::theme;

/// Draws the Git screen.
pub(crate) fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let services = app.visible_services();
    let rows: Vec<Row<'_>> = services
        .iter()
        .map(|service| {
            let id = &service.service_id;
            let branch = app
                .state
                .git_status
                .get(id)
                .and_then(|status| status.branch.as_ref())
                .map(|branch| branch.as_str().to_owned())
                .or_else(|| {
                    service
                        .branch
                        .as_ref()
                        .map(|branch| branch.as_str().to_owned())
                })
                .unwrap_or_else(|| "-".to_owned());
            let (lifecycle, commit, changes) = match app.state.git_status.get(id) {
                Some(status) => {
                    let changes = format!(
                        "{}s {}u {}? {}c",
                        status.staged_changes,
                        status.unstaged_changes,
                        status.untracked_files,
                        status.conflicts,
                    );
                    let commit = status
                        .commit
                        .as_deref()
                        .map(|commit| commit.chars().take(8).collect::<String>())
                        .unwrap_or_else(|| "-".to_owned());
                    (status.lifecycle.label().to_owned(), commit, changes)
                }
                None => ("-".to_owned(), "-".to_owned(), "-".to_owned()),
            };
            Row::new(vec![
                Cell::from(service.display_name.clone()),
                Cell::from(lifecycle),
                Cell::from(branch),
                Cell::from(commit),
                Cell::from(changes),
                Cell::from(status::git(service.git_state)),
            ])
        })
        .collect();

    let tracked = services
        .iter()
        .filter(|service| app.snapshot.repositories.contains(&service.service_id))
        .count();
    let title = format!(
        "Git  {} repositories · {} with status",
        tracked,
        app.state.git_status.len()
    );
    let widths = [
        Constraint::Min(16),
        Constraint::Length(12),
        Constraint::Length(16),
        Constraint::Length(10),
        Constraint::Length(18),
        Constraint::Length(10),
    ];
    let table = Table::new(rows, widths)
        .header(table::header(&[
            "Service",
            "Lifecycle",
            "Branch",
            "Commit",
            "Changes",
            "State",
        ]))
        .block(super::panel(app, title))
        .row_highlight_style(theme::selected_row())
        .highlight_symbol("> ");
    let mut state = table::state(app.state.cursor, app.state.list_offset);
    frame.render_stateful_widget(table, area, &mut state);
}
