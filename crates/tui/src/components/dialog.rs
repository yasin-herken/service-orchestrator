//! The modal dialog system.
//!
//! Dialogs are drawn last, over the whole frame, and are drawn from the same
//! read-only state as the screens. Destructive confirmations render the shared
//! [`ConfirmDialog`] produced by [`crate::app::App`] after the policy engine
//! denied an operation.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::state::{ConfirmDialog, Dialog, InputDialog, MessageDialog, SelectDialog};
use crate::theme;

/// Draws the active dialog, if any.
pub(crate) fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(dialog) = app.state.dialog.as_ref() else {
        return;
    };
    match dialog {
        Dialog::Help => draw_help(frame, area),
        Dialog::Confirm(confirm) => draw_confirm(frame, area, confirm),
        Dialog::Info(message) => draw_message(frame, area, message, false),
        Dialog::Error(message) => draw_message(frame, area, message, true),
        Dialog::Input(input) => draw_input(frame, area, input),
        Dialog::Select(select) => draw_select(frame, area, select),
    }
}

/// Centres a rectangle of the requested size within `area`.
fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// The rounded-then-cleared area a dialog occupies.
fn dialog_area(area: Rect, lines: usize, width: u16) -> Rect {
    let height = (lines as u16).saturating_add(2).min(area.height);
    centered(area, width.min(area.width), height)
}

fn draw_confirm(frame: &mut Frame<'_>, area: Rect, confirm: &ConfirmDialog) {
    let lines: Vec<Line<'_>> = confirm
        .lines
        .iter()
        .map(|line| Line::from(line.clone()))
        .collect();
    let mut all = lines;
    all.push(Line::from(""));
    all.push(Line::from(vec![
        Span::styled("[Enter] Confirm", theme::warning()),
        Span::raw("   "),
        Span::styled("[Esc] Cancel", theme::muted()),
    ]));
    let rect = dialog_area(area, all.len(), 72);
    let title_style = if confirm.destructive {
        theme::warning()
    } else {
        theme::accent()
    };
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(title_style)
        .title(Span::styled(confirm.title.clone(), title_style));
    frame.render_widget(
        Paragraph::new(all).block(block).wrap(Wrap { trim: false }),
        rect,
    );
}

fn draw_message(frame: &mut Frame<'_>, area: Rect, message: &MessageDialog, is_error: bool) {
    let mut lines: Vec<Line<'_>> = message
        .lines
        .iter()
        .map(|line| Line::from(line.clone()))
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("[Esc] Close", theme::muted())));
    let rect = dialog_area(area, lines.len(), 72);
    let style = if is_error {
        theme::error()
    } else {
        theme::accent()
    };
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(style)
        .title(Span::styled(message.title.clone(), style));
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        rect,
    );
}

fn draw_input(frame: &mut Frame<'_>, area: Rect, input: &InputDialog) {
    let lines = vec![
        Line::from(input.prompt.clone()),
        Line::from(""),
        Line::from(vec![
            Span::styled("> ", theme::accent()),
            Span::raw(input.value.clone()),
            Span::styled("_", theme::accent()),
        ]),
        Line::from(""),
        Line::from(Span::styled("[Enter] OK   [Esc] Cancel", theme::muted())),
    ];
    let rect = dialog_area(area, lines.len(), 60);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::accent())
        .title(Span::styled("Input", theme::accent()));
    frame.render_widget(Paragraph::new(lines).block(block), rect);
}

fn draw_select(frame: &mut Frame<'_>, area: Rect, select: &SelectDialog) {
    let items: Vec<ListItem<'_>> = select
        .options
        .iter()
        .map(|option| ListItem::new(option.label.clone()))
        .collect();
    let height = (select.options.len() as u16 + 4).min(area.height);
    let rect = centered(area, 60.min(area.width), height);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::accent())
        .title(Span::styled(select.title.clone(), theme::accent()));
    let list = List::new(items)
        .block(block)
        .highlight_style(theme::selected_row())
        .highlight_symbol("> ");
    let mut state = ListState::default().with_selected(Some(select.index));
    frame.render_stateful_widget(list, rect, &mut state);
}

fn draw_help(frame: &mut Frame<'_>, area: Rect) {
    let lines: Vec<Line<'_>> = HELP_LINES
        .iter()
        .map(|line| {
            if let Some((key, rest)) = line.split_once("  ") {
                Line::from(vec![
                    Span::styled(format!("{key:<14}"), theme::accent()),
                    Span::raw(rest.to_owned()),
                ])
            } else {
                Line::from(Span::styled((*line).to_owned(), theme::accent()))
            }
        })
        .collect();
    let rect = dialog_area(area, lines.len(), 72);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::accent())
        .title(Span::styled("Help", theme::accent()));
    frame.render_widget(Paragraph::new(lines).block(block), rect);
}

/// The static help text. A blank entry starts a new section.
const HELP_LINES: &[&str] = &[
    "Global",
    "j / Down  move down",
    "k / Up  move up",
    "Tab  switch panel",
    "Enter  open / confirm",
    "Esc  back / close dialog",
    "Space  select row",
    "/  search",
    "R  refresh",
    "W  cycle workspace",
    "?  this help",
    "q  quit",
    "Services",
    "s  start    x  stop    r  restart",
    "b  build    f  fetch   y  sync    p  pull",
    "h  health   l  logs",
    "S / X / B / F / Y / H  apply to selection",
    "a  select visible   c  clear selection   t  filter",
    "Git",
    "c  checkout  r  refresh status",
    "d  hard reset   D  discard changes   n  clean",
    "Libraries",
    "b  build  i  install  p  prepare  y  sync",
    "v  verify artifact  a  version alignment",
    "Migrations",
    "s  status  p  pending  P  preview  v  validate  a  apply",
    "Tasks",
    "Enter / l  view logs   x  cancel   t  filter",
    "Logs",
    "f  toggle follow   c  clear   v  cycle level",
];
