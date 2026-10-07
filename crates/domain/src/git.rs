//! Provider-independent Git state.
//!
//! This describes the *logical* state of a checkout. It is produced by the Git
//! infrastructure adapter and consumed by the application, TUI, and MCP layers.
//! The domain never depends on `git2`, on CLI output, or on any other concrete
//! Git mechanism.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The logical state of a local Git working copy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitState {
    /// The working copy matches the checked-out commit with no local changes.
    Clean,
    /// There are uncommitted local changes.
    Modified,
    /// The branch is ahead of its upstream.
    Ahead,
    /// The branch is behind its upstream.
    Behind,
    /// The branch has diverged from its upstream (both ahead and behind).
    Diverged,
    /// The state could not be determined.
    #[default]
    Unknown,
}

impl GitState {
    /// Returns `true` if the working copy has uncommitted local changes.
    ///
    /// Destructive operations must not silently discard these changes.
    #[must_use]
    pub const fn has_local_changes(self) -> bool {
        matches!(self, Self::Modified)
    }

    /// Returns `true` if the branch is not in sync with its upstream.
    #[must_use]
    pub const fn is_out_of_sync(self) -> bool {
        matches!(self, Self::Ahead | Self::Behind | Self::Diverged)
    }

    /// Returns `true` if the state is known and clean.
    #[must_use]
    pub const fn is_clean(self) -> bool {
        matches!(self, Self::Clean)
    }
}

impl fmt::Display for GitState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Clean => "clean",
            Self::Modified => "modified",
            Self::Ahead => "ahead",
            Self::Behind => "behind",
            Self::Diverged => "diverged",
            Self::Unknown => "unknown",
        };
        f.write_str(label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_local_changes() {
        assert!(GitState::Modified.has_local_changes());
        assert!(!GitState::Clean.has_local_changes());
        assert!(!GitState::Ahead.has_local_changes());
    }

    #[test]
    fn recognizes_out_of_sync_states() {
        assert!(GitState::Ahead.is_out_of_sync());
        assert!(GitState::Behind.is_out_of_sync());
        assert!(GitState::Diverged.is_out_of_sync());
        assert!(!GitState::Clean.is_out_of_sync());
    }

    #[test]
    fn unknown_is_the_default() {
        assert_eq!(GitState::default(), GitState::Unknown);
    }
}
