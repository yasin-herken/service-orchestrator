//! Permission and risk levels.
//!
//! Every operation exposed through the TUI or MCP is classified as
//! [`Read`](Permission::Read), [`SafeWrite`](Permission::SafeWrite), or
//! [`Destructive`](Permission::Destructive). The policy engine in the `policy`
//! crate uses this classification to decide whether an operation may proceed
//! implicitly or requires explicit confirmation; the domain only defines the
//! vocabulary so that both interfaces share one contract.
//!
//! The variants are ordered by increasing risk, so `max` yields the more
//! restrictive of two permissions.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The risk level of an operation.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Reads state without modifying anything (`list_services`, `git_status`).
    #[default]
    Read,
    /// Modifies local state in a recoverable way (`git_pull`, `build_service`).
    SafeWrite,
    /// Can destroy local work and must require confirmation (`git_reset`,
    /// `git_clean`, `discard_local_changes`, `delete_workspace`).
    Destructive,
}

impl Permission {
    /// Returns `true` if the operation may modify local state.
    #[must_use]
    pub const fn allows_mutation(self) -> bool {
        !matches!(self, Self::Read)
    }

    /// Returns `true` if the operation may destroy local work.
    #[must_use]
    pub const fn is_destructive(self) -> bool {
        matches!(self, Self::Destructive)
    }

    /// Returns `true` if the operation requires explicit user confirmation.
    ///
    /// Only destructive operations require confirmation today. The policy
    /// engine may add further conditions later without changing this contract.
    #[must_use]
    pub const fn requires_confirmation(self) -> bool {
        self.is_destructive()
    }

    /// Returns the more restrictive of two permissions.
    #[must_use]
    pub fn most_restrictive(self, other: Self) -> Self {
        self.max(other)
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Read => "read",
            Self::SafeWrite => "safe_write",
            Self::Destructive => "destructive",
        };
        f.write_str(label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_read_operations() {
        let permission = Permission::Read;
        assert!(!permission.allows_mutation());
        assert!(!permission.is_destructive());
        assert!(!permission.requires_confirmation());
    }

    #[test]
    fn classifies_safe_write_operations() {
        let permission = Permission::SafeWrite;
        assert!(permission.allows_mutation());
        assert!(!permission.is_destructive());
        assert!(!permission.requires_confirmation());
    }

    #[test]
    fn classifies_destructive_operations() {
        let permission = Permission::Destructive;
        assert!(permission.allows_mutation());
        assert!(permission.is_destructive());
        assert!(permission.requires_confirmation());
    }

    #[test]
    fn risk_levels_are_ordered() {
        assert!(Permission::Read < Permission::SafeWrite);
        assert!(Permission::SafeWrite < Permission::Destructive);
        assert_eq!(
            Permission::Read.most_restrictive(Permission::Destructive),
            Permission::Destructive
        );
        assert_eq!(
            Permission::Destructive.most_restrictive(Permission::SafeWrite),
            Permission::Destructive
        );
    }
}
