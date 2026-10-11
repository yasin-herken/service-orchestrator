//! Provider-independent Git state.
//!
//! This describes the *logical* state of a checkout. It is produced by the Git
//! infrastructure adapter and consumed by the application, TUI, and MCP layers.
//! The domain never depends on `git2`, on CLI output, or on any other concrete
//! Git mechanism.
//!
//! Two families of types live here:
//!
//! - [`GitState`], the compact state the runtime-state store records for a
//!   service; and
//! - the richer [`RepositoryStatus`], [`RepositoryLifecycle`], [`BranchInfo`],
//!   and [`GitOperationResult`] produced by the Git manager and returned to
//!   interfaces.
//!
//! [`RepositoryStatus`] deliberately keeps independent properties independent: a
//! repository can be modified *and* ahead of *and* behind its upstream at the
//! same time, so collapsing everything into one enum would lose information.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::repository::Branch;

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

/// The lifecycle of a configured repository on disk.
///
/// This is deliberately *not* the working-tree status ([`RepositoryStatus`] is).
/// A directory may exist without being a Git repository ([`Invalid`]), a path
/// may be missing entirely ([`NotCloned`]), or the location may be temporarily
/// unreadable ([`Unavailable`]). Collapsing those into "dirty" would be wrong.
///
/// [`Invalid`]: RepositoryLifecycle::Invalid
/// [`NotCloned`]: RepositoryLifecycle::NotCloned
/// [`Unavailable`]: RepositoryLifecycle::Unavailable
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryLifecycle {
    /// The local path does not exist yet; the repository has not been cloned.
    NotCloned,
    /// A valid Git working copy exists at the configured path.
    Available,
    /// The path exists but is not a valid Git working copy.
    Invalid,
    /// The repository exists but could not be inspected (permissions, I/O).
    Unavailable,
    /// The repository is currently held by another mutation.
    Busy,
    /// The lifecycle could not be determined.
    #[default]
    Unknown,
}

impl RepositoryLifecycle {
    /// Returns `true` when the repository can be operated on.
    #[must_use]
    pub const fn is_available(self) -> bool {
        matches!(self, Self::Available)
    }

    /// Returns a short, stable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NotCloned => "not_cloned",
            Self::Available => "available",
            Self::Invalid => "invalid",
            Self::Unavailable => "unavailable",
            Self::Busy => "busy",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for RepositoryLifecycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A structured snapshot of a repository's working-tree status.
///
/// Multiple independent properties may hold at once: a repository can have
/// staged *and* unstaged changes *and* be ahead *and* behind. Each is therefore
/// represented separately rather than folded into a single state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryStatus {
    /// The lifecycle of the repository on disk.
    pub lifecycle: RepositoryLifecycle,
    /// The checked-out branch, or `None` when detached or unborn.
    pub branch: Option<Branch>,
    /// The current commit SHA, or `None` for an unborn branch.
    pub commit: Option<String>,
    /// Whether `HEAD` is detached.
    pub detached_head: bool,
    /// Whether the branch is unborn (a fresh repository with no commits).
    pub unborn: bool,
    /// The configured upstream of the current branch, if any.
    pub upstream: Option<String>,
    /// How many commits the branch is ahead of its upstream, when known.
    pub ahead: Option<u32>,
    /// How many commits the branch is behind its upstream, when known.
    pub behind: Option<u32>,
    /// The number of staged changes.
    pub staged_changes: u32,
    /// The number of unstaged (working-tree) changes.
    pub unstaged_changes: u32,
    /// The number of untracked files.
    pub untracked_files: u32,
    /// The number of unmerged (conflicted) entries.
    pub conflicts: u32,
}

impl RepositoryStatus {
    /// Creates a status for a repository that has not been cloned.
    #[must_use]
    pub fn not_cloned() -> Self {
        Self {
            lifecycle: RepositoryLifecycle::NotCloned,
            ..Self::default()
        }
    }

    /// Returns `true` if the working tree has no staged, unstaged, untracked, or
    /// conflicted entries.
    #[must_use]
    pub const fn is_clean(&self) -> bool {
        self.staged_changes == 0
            && self.unstaged_changes == 0
            && self.untracked_files == 0
            && self.conflicts == 0
    }

    /// Returns `true` if any local change would be at risk from a destructive
    /// operation.
    ///
    /// Destructive operations must never silently discard these changes.
    #[must_use]
    pub const fn has_local_changes(&self) -> bool {
        !self.is_clean()
    }

    /// Returns `true` if the current branch is ahead of its upstream.
    #[must_use]
    pub const fn is_ahead(&self) -> bool {
        matches!(self.ahead, Some(a) if a > 0)
    }

    /// Returns `true` if the current branch is behind its upstream.
    #[must_use]
    pub const fn is_behind(&self) -> bool {
        matches!(self.behind, Some(b) if b > 0)
    }

    /// Returns `true` if the branch has diverged from its upstream.
    #[must_use]
    pub const fn is_diverged(&self) -> bool {
        self.is_ahead() && self.is_behind()
    }
}

/// The kind of branch in a listing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchKind {
    /// A branch in `refs/heads`.
    Local,
    /// A branch in `refs/remotes`.
    Remote,
}

/// A single branch in a repository listing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchInfo {
    /// The branch name, for example `main` or `origin/main`.
    pub name: String,
    /// Whether the branch is local or remote-tracking.
    pub kind: BranchKind,
    /// Whether the branch is currently checked out.
    pub current: bool,
    /// The upstream branch name, when one is configured.
    pub upstream: Option<String>,
    /// The commit the branch points at, when known.
    pub commit: Option<String>,
}

impl BranchInfo {
    /// Creates a branch listing entry.
    #[must_use]
    pub fn new(name: impl Into<String>, kind: BranchKind, current: bool) -> Self {
        Self {
            name: name.into(),
            kind,
            current,
            upstream: None,
            commit: None,
        }
    }
}

/// A repository-level Git operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitOperation {
    /// Inspect status.
    Status,
    /// List branches.
    Branches,
    /// Inspect a diff.
    Diff,
    /// Fetch remote refs.
    Fetch,
    /// Pull with fast-forward only.
    Pull,
    /// Check out a branch.
    Checkout,
    /// Clone a repository.
    Clone,
    /// Synchronize (fetch, checkout, fast-forward).
    Sync,
    /// Discard the working tree with a hard reset.
    ResetHard,
    /// Remove untracked files.
    Clean,
    /// Discard all local changes (reset and clean).
    DiscardChanges,
}

impl GitOperation {
    /// Returns a short, stable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Branches => "branches",
            Self::Diff => "diff",
            Self::Fetch => "fetch",
            Self::Pull => "pull",
            Self::Checkout => "checkout",
            Self::Clone => "clone",
            Self::Sync => "sync",
            Self::ResetHard => "reset_hard",
            Self::Clean => "clean",
            Self::DiscardChanges => "discard_changes",
        }
    }
}

impl fmt::Display for GitOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A structured result of a mutating Git operation.
///
/// The `previous_commit` / `current_commit` pair lets an interface show what
/// actually changed without re-reading the repository, and `changed` says
/// whether the operation moved `HEAD`. Operations that do not move `HEAD`
/// (a fetch that only updates remote-tracking refs, a checkout of the current
/// branch) report `changed == false` honestly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitOperationResult {
    /// The workspace-aware repository identifier.
    pub repository: String,
    /// The operation that produced this result.
    pub operation: GitOperation,
    /// Whether the operation completed successfully.
    pub success: bool,
    /// The branch the repository is on after the operation, when known.
    pub branch: Option<Branch>,
    /// The commit `HEAD` pointed at before the operation, when known.
    pub previous_commit: Option<String>,
    /// The commit `HEAD` points at after the operation, when known.
    pub current_commit: Option<String>,
    /// Whether the operation changed `HEAD`.
    pub changed: bool,
}

impl GitOperationResult {
    /// Creates a successful result.
    #[must_use]
    pub fn succeeded(repository: impl Into<String>, operation: GitOperation) -> Self {
        Self {
            repository: repository.into(),
            operation,
            success: true,
            branch: None,
            previous_commit: None,
            current_commit: None,
            changed: false,
        }
    }

    /// Sets the branch.
    #[must_use]
    pub fn with_branch(mut self, branch: Option<Branch>) -> Self {
        self.branch = branch;
        self
    }

    /// Sets the before/after commits, deriving `changed`.
    #[must_use]
    pub fn with_commits(mut self, previous: Option<String>, current: Option<String>) -> Self {
        self.changed = previous != current;
        self.previous_commit = previous;
        self.current_commit = current;
        self
    }
}

/// A per-file entry in a diff summary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffFileStat {
    /// The file path.
    pub path: String,
    /// The number of inserted lines, when the change is textual.
    pub insertions: u32,
    /// The number of deleted lines, when the change is textual.
    pub deletions: u32,
}

/// A bounded repository diff.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoDiff {
    /// Whether the diff covers staged (`true`) or unstaged (`false`) changes.
    pub staged: bool,
    /// The per-file change summary.
    pub files: Vec<DiffFileStat>,
    /// The unified patch text, bounded by the requested size limit.
    pub patch: String,
    /// Whether the patch was truncated to respect the size limit.
    pub truncated: bool,
    /// The number of bytes of patch text returned.
    pub bytes: usize,
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

    #[test]
    fn a_clean_tree_is_clean_and_has_no_local_changes() {
        let status = RepositoryStatus {
            lifecycle: RepositoryLifecycle::Available,
            ..RepositoryStatus::default()
        };
        assert!(status.is_clean());
        assert!(!status.has_local_changes());
    }

    #[test]
    fn independent_status_properties_coexist() {
        let status = RepositoryStatus {
            lifecycle: RepositoryLifecycle::Available,
            staged_changes: 1,
            unstaged_changes: 2,
            untracked_files: 3,
            ahead: Some(1),
            behind: Some(4),
            ..RepositoryStatus::default()
        };
        assert!(!status.is_clean());
        assert!(status.has_local_changes());
        assert!(status.is_ahead());
        assert!(status.is_behind());
        assert!(status.is_diverged());
    }

    #[test]
    fn not_cloned_status_is_not_available() {
        let status = RepositoryStatus::not_cloned();
        assert_eq!(status.lifecycle, RepositoryLifecycle::NotCloned);
        assert!(!status.lifecycle.is_available());
        assert!(status.is_clean());
    }

    #[test]
    fn operation_result_derives_changed_from_commits() {
        let result = GitOperationResult::succeeded("auth-service", GitOperation::Pull)
            .with_commits(Some("abc".to_owned()), Some("def".to_owned()));
        assert!(result.changed);
        let no_change = GitOperationResult::succeeded("auth-service", GitOperation::Fetch)
            .with_commits(Some("abc".to_owned()), Some("abc".to_owned()));
        assert!(!no_change.changed);
    }

    #[test]
    fn operation_labels_are_stable() {
        assert_eq!(GitOperation::Sync.to_string(), "sync");
        assert_eq!(RepositoryLifecycle::NotCloned.to_string(), "not_cloned");
    }
}
