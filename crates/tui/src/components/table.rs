//! Table helpers.
//!
//! Ratatui's [`Table`] is stateful: the selected row and scroll offset live on
//! a [`TableState`]. The interface keeps those in [`UiState`] and rebuilds a
//! state value each frame, which keeps rendering stateless and testable.
//!
//! [`UiState`]: crate::state::UiState

use ratatui::widgets::{Cell, Row, TableState};

use crate::theme;

/// Builds a [`TableState`] from the interface's cursor and scroll offset.
#[must_use]
pub(crate) fn state(cursor: usize, offset: usize) -> TableState {
    TableState::new()
        .with_offset(offset)
        .with_selected(Some(cursor))
}

/// Builds a styled table header row.
#[must_use]
pub(crate) fn header(labels: &[&'static str]) -> Row<'static> {
    Row::new(labels.iter().map(|label| Cell::from(*label))).style(theme::accent())
}

/// A multi-selection marker cell: `[x]` when selected, `[ ]` otherwise.
#[must_use]
pub(crate) fn marker(selected: bool) -> Cell<'static> {
    if selected {
        Cell::from("[x]").style(theme::selection_marker())
    } else {
        Cell::from("[ ]").style(theme::muted())
    }
}
