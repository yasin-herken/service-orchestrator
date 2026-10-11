//! Terminal input, abstracted away from the concrete backend.
//!
//! Tests drive the state machine with [`InputEvent`] values directly; only this
//! module knows about `crossterm`. Keeping the abstraction small means a future
//! backend swap cannot reach into the state machine.

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// A key press the interface understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A printable character.
    Char(char),
    /// The return key.
    Enter,
    /// The escape key.
    Esc,
    /// The tab key.
    Tab,
    /// Shift+Tab.
    BackTab,
    /// Backspace.
    Backspace,
    /// Forward delete.
    Delete,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Ctrl+C.
    CtrlC,
    /// Ctrl+D.
    CtrlD,
}

/// A terminal input event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEvent {
    /// A key press.
    Key(Key),
    /// The terminal was resized.
    Resize {
        /// The new width, in columns.
        width: u16,
        /// The new height, in rows.
        height: u16,
    },
}

impl InputEvent {
    /// Creates a key-press event.
    #[must_use]
    pub const fn key(key: Key) -> Self {
        InputEvent::Key(key)
    }

    /// Creates a character key-press event.
    #[must_use]
    pub const fn ch(c: char) -> Self {
        InputEvent::Key(Key::Char(c))
    }
}

/// Translates a backend event into an [`InputEvent`].
///
/// Returns `None` for events the interface ignores, such as key releases, mouse
/// events, and focus changes.
pub(crate) fn translate(event: Event) -> Option<InputEvent> {
    match event {
        Event::Key(key) => translate_key(key).map(InputEvent::Key),
        Event::Resize(width, height) => Some(InputEvent::Resize { width, height }),
        _ => None,
    }
}

fn translate_key(key: KeyEvent) -> Option<Key> {
    if matches!(key.kind, KeyEventKind::Release) {
        return None;
    }
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    Some(match key.code {
        KeyCode::Char('c') | KeyCode::Char('C') if control => Key::CtrlC,
        KeyCode::Char('d') | KeyCode::Char('D') if control => Key::CtrlD,
        KeyCode::Char(c) if !control => Key::Char(c),
        KeyCode::Char(_) => return None,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Tab => Key::Tab,
        KeyCode::BackTab => Key::BackTab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_keys_are_distinguished() {
        let event = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(translate(event), Some(InputEvent::Key(Key::CtrlC)));
    }

    #[test]
    fn plain_characters_are_translated() {
        let event = Event::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(translate(event), Some(InputEvent::ch('j')));
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut key = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        key.kind = KeyEventKind::Release;
        assert_eq!(translate(Event::Key(key)), None);
    }

    #[test]
    fn resizes_are_translated() {
        assert_eq!(
            translate(Event::Resize(120, 40)),
            Some(InputEvent::Resize {
                width: 120,
                height: 40
            })
        );
    }
}
