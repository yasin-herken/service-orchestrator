//! The top-level renderer.
//!
//! [`draw`] composes the whole frame from already-read state: the header, the
//! navigation panel, the active screen, the status/footer bar, and any modal
//! dialog. It never mutates state and never calls the application core, which is
//! what keeps rendering cheap and testable (a `TestBackend` can drive it).

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::components::{dialog, footer};
use crate::screens;
use crate::state::{Focus, Screen, MIN_HEIGHT, MIN_WIDTH};
use crate::theme;

/// The height, in rows, of the header area.
pub(crate) const HEADER_HEIGHT: u16 = 2;

/// The height, in rows, of the footer area.
pub(crate) const FOOTER_HEIGHT: u16 = 1;

/// The width, in columns, of the navigation panel.
pub(crate) const NAV_WIDTH: u16 = 22;

/// The number of list rows visible in the content panel for a terminal height.
///
/// The event loop stores this in the UI state so page movement matches the
/// rendered viewport.
#[must_use]
pub(crate) fn list_rows(terminal_height: u16) -> usize {
    let content = terminal_height
        .saturating_sub(HEADER_HEIGHT + FOOTER_HEIGHT)
        .saturating_sub(2) // panel border
        .saturating_sub(1); // table header
    usize::from(content.max(1))
}

/// Draws an entire frame.
pub(crate) fn draw(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        draw_too_small(frame, area);
        return;
    }

    let [header, body, footer_area] = Layout::vertical([
        Constraint::Length(HEADER_HEIGHT),
        Constraint::Min(5),
        Constraint::Length(FOOTER_HEIGHT),
    ])
    .areas(area);

    draw_header(frame, header, app);

    let [nav, content] =
        Layout::horizontal([Constraint::Length(NAV_WIDTH), Constraint::Min(20)]).areas(body);

    draw_nav(frame, nav, app);
    screens::draw(frame, content, app);
    footer::draw(frame, footer_area, app);
    dialog::draw(frame, area, app);
}

/// Draws the title and workspace summary.
fn draw_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let total = app.snapshot.services.len();
    let running = app
        .snapshot
        .services
        .iter()
        .filter(|service| service.process_state.is_active())
        .count();
    let active_tasks = app
        .snapshot
        .tasks
        .iter()
        .filter(|task| task.status.is_active())
        .count();
    let workspace = if app.snapshot.workspace_name.is_empty() {
        "(no workspace)"
    } else {
        app.snapshot.workspace_name.as_str()
    };

    let lines = vec![
        Line::from(Span::styled("SERVICE ORCHESTRATOR", theme::accent())),
        Line::from(vec![
            Span::styled("Workspace: ", theme::muted()),
            Span::raw(workspace.to_owned()),
            Span::styled("    Running: ", theme::muted()),
            Span::styled(format!("{running}/{total}"), theme::success()),
            Span::styled("    Tasks: ", theme::muted()),
            Span::raw(active_tasks.to_string()),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

/// Draws the navigation panel.
fn draw_nav(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let items: Vec<ListItem<'_>> = Screen::NAV
        .iter()
        .map(|screen| ListItem::new(screen.title()))
        .collect();
    let border_style = if app.state.focus == Focus::Nav {
        theme::focused_border()
    } else {
        theme::border()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled("Navigation", theme::accent()));
    let list = List::new(items)
        .block(block)
        .highlight_style(theme::selected_row())
        .highlight_symbol("> ");
    let mut state = ListState::default().with_selected(Some(app.state.nav_index));
    frame.render_stateful_widget(list, area, &mut state);
}

/// Draws the minimum-size notice.
fn draw_too_small(frame: &mut Frame<'_>, area: Rect) {
    let lines = vec![
        Line::from(Span::styled("Terminal too small", theme::warning())),
        Line::from(""),
        Line::from(format!(
            "Need at least {MIN_WIDTH}x{MIN_HEIGHT}, have {}x{}.",
            area.width, area.height
        )),
        Line::from("Resize the terminal to continue."),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::warning()),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}
