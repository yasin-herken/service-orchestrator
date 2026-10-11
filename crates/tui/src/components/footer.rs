//! The footer: contextual shortcut hints, the live search bar, and toasts.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::state::{Focus, Screen, ToastLevel};
use crate::theme;

/// Draws the footer for the current screen.
pub(crate) fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let line = if let Some(search) = app.state.search.as_ref() {
        if search.active {
            Line::from(vec![
                Span::styled(format!("/{}", search.query), theme::accent()),
                Span::styled("  ", theme::muted()),
                Span::styled("[Enter] apply  [Esc] cancel", theme::muted()),
            ])
        } else {
            hint_line(app)
        }
    } else {
        hint_line(app)
    };
    frame.render_widget(Paragraph::new(line).style(theme::muted()), area);
}

/// Builds the hint line, prefixing an active toast when one is shown.
fn hint_line(app: &App) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    if let Some(toast) = app.state.toast.as_ref() {
        spans.push(Span::styled(
            toast.message.clone(),
            toast_style(toast.level),
        ));
        spans.push(Span::raw("   "));
    }
    spans.push(Span::styled(hints(app), theme::muted()));
    Line::from(spans)
}

/// The contextual shortcut hint for the focused screen.
fn hints(app: &App) -> &'static str {
    if app.state.focus == Focus::Nav {
        return "[j/k] navigate  [Enter] open  [/] search  [?] help  [q] quit";
    }
    match app.state.screen {
        Screen::Dashboard => {
            "[S] services  [G] git  [B] builds  [L] libraries  [M] migrations  [T] tasks  [O] logs  [H] health  [W] workspace"
        }
        Screen::Services | Screen::ServiceDetail => {
            "[Space] select  [Enter] details  [s]tart [x] stop [r]estart [b]uild [f]etch [y]sync [p]ull [h]ealth [l]ogs  [S/X/B/F/Y/H] selected  [t] filter"
        }
        Screen::Git => {
            "[c]heckout [f]etch [y]sync [p]ull [r]efresh [d] reset [D] discard [n] clean  [F/Y] selected"
        }
        Screen::Builds => "[Enter/b] build  [B] build selected  [l]ogs  [t]asks  [c] clear",
        Screen::Libraries => {
            "[b]uild [i]nstall [p]repare [y]sync [v]erify [a]lignment [r]efresh"
        }
        Screen::Migrations => "[s]tatus [p]ending [P]review [v]alidate [a]pply [l]ogs",
        Screen::Tasks => "[Enter/l]ogs [x] cancel [t] filter  [↑/↓] navigate",
        Screen::Logs => "[f]ollow [c]lear [v] level  [↑/↓] scroll  [Esc] back",
        Screen::Health => "[Enter/h] check  [H] check selected  [l]ogs  [c] clear",
        Screen::Settings => "[R]efresh  [W] workspace  [?] help  [q] quit",
    }
}

/// The style for a toast of the given severity.
fn toast_style(level: ToastLevel) -> Style {
    match level {
        ToastLevel::Info => theme::accent(),
        ToastLevel::Success => theme::success(),
        ToastLevel::Warning => theme::warning(),
        ToastLevel::Error => theme::error(),
    }
}
