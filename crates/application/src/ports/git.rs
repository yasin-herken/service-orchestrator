//! Git management boundary.
//!
//! The application does not run `git` itself. It describes *which repository*
//! (a workspace-aware [`ResolvedRepository`]) and *what* to do, and a
//! [`GitManager`] — implemented by the infrastructure `git` adapter over the
//! process manager — performs the operation and returns a structured result.
//!
//! # Why a port
//!
//! Every interface (TUI, MCP) drives the same use cases, and those use cases
//! must reach Git without knowing how Git is implemented. The port fixes that
//! boundary once. The concrete manager is injected by the composition root; a
//! [`NullGitManager`] keeps the application constructible (and its existing
//! tests unchanged) when no manager is wired.
//!
//! # Repository identity
//!
//! A repository is never addressed by a bare name: service names are not
//! globally unique. [`ResolvedRepository`] pairs the owning workspace with the
//! repository's absolute checkout path, so a Git task submitted to the engine
//! can be executed later without re-reading configuration. The value round-trips
//! through task parameters via [`ResolvedRepository::to_parameters`] and
//! [`ResolvedRepository::from_parameters`].

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use service_orchestrator_domain::{
    AbsolutePath, Branch, BranchInfo, GitOperation, GitOperationResult, RepoDiff, RepositoryStatus,
    WorkspaceId,
};
use service_orchestrator_execution::Cancellation;
use thiserror::Error;

/// Parameter keys used to carry a [`ResolvedRepository`] through the task engine.
pub mod parameter {
    /// The owning workspace identifier.
    pub const WORKSPACE: &str = "git.workspace";
    /// The repository identifier (a service or library id).
    pub const NAME: &str = "git.name";
    /// The clone URL.
    pub const URL: &str = "git.url";
    /// The absolute local checkout path.
    pub const PATH: &str = "git.path";
    /// The workspace root, used to validate the checkout stays inside it.
    pub const WORKSPACE_ROOT: &str = "git.workspace_root";
    /// The default branch.
    pub const DEFAULT_BRANCH: &str = "git.default_branch";
    /// A target branch for a checkout or sync step.
    pub const BRANCH: &str = "git.branch";
}

/// A repository resolved against a validated configuration.
///
/// This is the unit of work every [`GitManager`] operation takes. It is
/// workspace-aware: two workspaces may legitimately contain a service with the
/// same id, and their checkouts are distinct.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedRepository {
    /// The workspace that owns the repository.
    pub workspace_id: WorkspaceId,
    /// The repository identifier (a service or library id).
    pub name: String,
    /// The clone URL. Credentials must never be embedded.
    pub url: String,
    /// The absolute local checkout path.
    pub path: AbsolutePath,
    /// The workspace root; the checkout must remain inside it.
    pub workspace_root: AbsolutePath,
    /// The branch used when no other branch is requested.
    pub default_branch: Branch,
}

impl ResolvedRepository {
    /// Returns `true` when `path` is inside the workspace root.
    #[must_use]
    pub fn is_inside_workspace(&self) -> bool {
        let root = self.workspace_root.as_str().trim_end_matches('/');
        let path = self.path.as_str();
        path == root || path.starts_with(&format!("{root}/"))
    }

    /// Encodes the repository as task parameters.
    #[must_use]
    pub fn to_parameters(&self) -> BTreeMap<String, String> {
        let mut parameters = BTreeMap::new();
        parameters.insert(
            parameter::WORKSPACE.to_owned(),
            self.workspace_id.to_string(),
        );
        parameters.insert(parameter::NAME.to_owned(), self.name.clone());
        parameters.insert(parameter::URL.to_owned(), self.url.clone());
        parameters.insert(parameter::PATH.to_owned(), self.path.to_string());
        parameters.insert(
            parameter::WORKSPACE_ROOT.to_owned(),
            self.workspace_root.to_string(),
        );
        parameters.insert(
            parameter::DEFAULT_BRANCH.to_owned(),
            self.default_branch.to_string(),
        );
        parameters
    }

    /// Decodes a repository from task parameters.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::InvalidParameter`] if a required parameter is missing
    /// or does not parse as its domain type.
    pub fn from_parameters(parameters: &BTreeMap<String, String>) -> Result<Self, GitError> {
        let workspace_id =
            WorkspaceId::new(required(parameters, parameter::WORKSPACE)?).map_err(|error| {
                GitError::invalid_parameter(parameter::WORKSPACE, error.to_string())
            })?;
        let path = AbsolutePath::new(required(parameters, parameter::PATH)?)
            .map_err(|error| GitError::invalid_parameter(parameter::PATH, error.to_string()))?;
        let workspace_root = AbsolutePath::new(required(parameters, parameter::WORKSPACE_ROOT)?)
            .map_err(|error| {
                GitError::invalid_parameter(parameter::WORKSPACE_ROOT, error.to_string())
            })?;
        let default_branch = Branch::new(required(parameters, parameter::DEFAULT_BRANCH)?)
            .map_err(|error| {
                GitError::invalid_parameter(parameter::DEFAULT_BRANCH, error.to_string())
            })?;
        Ok(Self {
            workspace_id,
            name: required(parameters, parameter::NAME)?.to_owned(),
            url: required(parameters, parameter::URL)?.to_owned(),
            path,
            workspace_root,
            default_branch,
        })
    }
}

fn required<'a>(parameters: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, GitError> {
    parameters
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| GitError::MissingParameter {
            key: key.to_owned(),
        })
}

/// Common options for a mutating Git operation.
#[derive(Clone, Debug, Default)]
pub struct GitOptions {
    /// An optional timeout for the underlying `git` process.
    pub timeout: Option<Duration>,
    /// An optional cancellation token, propagated from the task engine.
    pub cancellation: Option<Cancellation>,
    /// Whether a fetch should prune deleted remote-tracking refs.
    pub prune: bool,
}

impl GitOptions {
    /// Creates options with no timeout or cancellation.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets the cancellation token.
    #[must_use]
    pub fn with_cancellation(mut self, cancellation: Cancellation) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    /// Enables prune.
    #[must_use]
    pub fn with_prune(mut self, prune: bool) -> Self {
        self.prune = prune;
        self
    }
}

/// Options for a diff inspection.
#[derive(Clone, Debug)]
pub struct DiffOptions {
    /// Whether to diff staged (`true`) or unstaged (`false`) changes.
    pub staged: bool,
    /// The maximum number of patch bytes to return.
    pub max_bytes: usize,
    /// An optional timeout for the underlying `git` process.
    pub timeout: Option<Duration>,
    /// An optional cancellation token.
    pub cancellation: Option<Cancellation>,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            staged: false,
            max_bytes: 64 * 1024,
            timeout: None,
            cancellation: None,
        }
    }
}

impl DiffOptions {
    /// Creates unstaged-diff options with the given size limit.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Selects the staged diff.
    #[must_use]
    pub fn staged(mut self, staged: bool) -> Self {
        self.staged = staged;
        self
    }

    /// Sets the maximum patch size, in bytes.
    #[must_use]
    pub fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    /// Sets the timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets the cancellation token.
    #[must_use]
    pub fn with_cancellation(mut self, cancellation: Cancellation) -> Self {
        self.cancellation = Some(cancellation);
        self
    }
}

/// A structured Git failure.
///
/// Every variant carries the context an interface needs to explain what
/// happened — repository, operation, branch, and exit code — without exposing
/// credentials. Repository URLs are never included verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GitError {
    /// The repository's local path does not exist; it has not been cloned.
    #[error("repository '{repository}' has not been cloned to '{path}'")]
    NotCloned {
        /// The repository identifier.
        repository: String,
        /// The missing local path.
        path: String,
    },

    /// The local path exists but is not a valid Git working copy.
    #[error("'{path}' is not a valid Git repository for '{repository}'")]
    InvalidRepository {
        /// The repository identifier.
        repository: String,
        /// The offending path.
        path: String,
    },

    /// The repository is already cloned: the clone destination is occupied.
    ///
    /// Cloning must never overwrite an existing non-empty directory without
    /// explicit authorization (`AGENTS.md` §30).
    #[error("clone destination '{path}' for '{repository}' already exists and is not empty")]
    DestinationNotEmpty {
        /// The repository identifier.
        repository: String,
        /// The occupied destination path.
        path: String,
    },

    /// The repository could not be inspected.
    #[error("repository '{repository}' is unavailable: {message}")]
    Unavailable {
        /// The repository identifier.
        repository: String,
        /// A human-readable explanation.
        message: String,
    },

    /// The operation cannot proceed because the working tree has local changes.
    #[error("repository '{repository}' has local changes that would be affected")]
    WorkingTreeDirty {
        /// The repository identifier.
        repository: String,
        /// A human-readable summary of the changes.
        detail: String,
    },

    /// The requested branch does not exist.
    #[error("branch '{branch}' was not found in repository '{repository}'")]
    BranchNotFound {
        /// The repository identifier.
        repository: String,
        /// The requested branch.
        branch: String,
    },

    /// The repository is in a detached-HEAD state.
    #[error("repository '{repository}' has a detached HEAD")]
    DetachedHead {
        /// The repository identifier.
        repository: String,
    },

    /// The current branch has no upstream configured.
    #[error("branch '{branch}' in repository '{repository}' has no upstream")]
    UpstreamNotConfigured {
        /// The repository identifier.
        repository: String,
        /// The branch.
        branch: String,
    },

    /// The branch has diverged from its upstream.
    #[error(
        "branch '{branch}' in repository '{repository}' has diverged (ahead {ahead}, behind {behind})"
    )]
    DivergedBranch {
        /// The repository identifier.
        repository: String,
        /// The branch.
        branch: String,
        /// How many commits ahead.
        ahead: u32,
        /// How many commits behind.
        behind: u32,
    },

    /// A merge or rebase produced conflicts.
    #[error("repository '{repository}' has merge conflicts")]
    MergeConflict {
        /// The repository identifier.
        repository: String,
        /// A human-readable summary.
        detail: String,
    },

    /// Authentication with the remote failed.
    #[error("authentication failed for repository '{repository}': {message}")]
    AuthenticationFailed {
        /// The repository identifier.
        repository: String,
        /// A human-readable explanation.
        message: String,
    },

    /// The remote was unreachable.
    #[error("remote for repository '{repository}' is unavailable: {message}")]
    RemoteUnavailable {
        /// The repository identifier.
        repository: String,
        /// A human-readable explanation.
        message: String,
    },

    /// A `git` command failed with a non-zero exit status.
    #[error("git {operation} failed for '{repository}': {message}")]
    CommandFailed {
        /// The repository identifier.
        repository: String,
        /// The operation that failed.
        operation: GitOperation,
        /// The process exit code, when available.
        exit_code: Option<i32>,
        /// A human-readable summary.
        message: String,
        /// More detailed output, already bounded.
        detail: Option<String>,
    },

    /// The operation exceeded its timeout.
    #[error("git {operation} for '{repository}' timed out after {timeout_seconds}s")]
    Timeout {
        /// The repository identifier.
        repository: String,
        /// The operation that timed out.
        operation: GitOperation,
        /// The timeout, in seconds.
        timeout_seconds: u64,
    },

    /// The operation was cancelled.
    #[error("git {operation} for '{repository}' was cancelled")]
    Cancelled {
        /// The repository identifier.
        repository: String,
        /// The operation that was cancelled.
        operation: GitOperation,
    },

    /// The repository is held by another mutation.
    #[error("repository '{repository}' is busy with another operation")]
    Busy {
        /// The repository identifier.
        repository: String,
    },

    /// A repository parameter was missing from a task.
    #[error("missing Git parameter '{key}'")]
    MissingParameter {
        /// The missing parameter key.
        key: String,
    },

    /// A repository parameter was invalid.
    #[error("invalid Git parameter '{key}': {message}")]
    InvalidParameter {
        /// The offending parameter key.
        key: String,
        /// Why it is invalid.
        message: String,
    },

    /// A Git reference (branch, revision) was invalid.
    #[error("invalid Git reference '{reference}': {message}")]
    InvalidReference {
        /// The offending reference.
        reference: String,
        /// Why it is invalid.
        message: String,
    },

    /// The configured checkout path escapes its workspace.
    #[error("repository '{repository}' resolves outside its workspace")]
    PathOutsideWorkspace {
        /// The repository identifier.
        repository: String,
    },

    /// The `git` executable is missing or unsupported.
    #[error("Git is unavailable: {message}")]
    GitUnavailable {
        /// A human-readable explanation.
        message: String,
    },

    /// The manager itself failed.
    #[error("git manager error: {message}")]
    Internal {
        /// A human-readable description of the failure.
        message: String,
    },
}

impl GitError {
    /// Builds an [`GitError::InvalidParameter`].
    #[must_use]
    pub fn invalid_parameter(key: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidParameter {
            key: key.into(),
            message: message.into(),
        }
    }

    /// Builds a [`GitError::Internal`].
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    /// Builds a [`GitError::InvalidReference`].
    #[must_use]
    pub fn invalid_reference(reference: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidReference {
            reference: reference.into(),
            message: message.into(),
        }
    }

    /// Returns a stable, machine-readable code for the failure.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotCloned { .. } => "GIT_NOT_CLONED",
            Self::InvalidRepository { .. } => "GIT_INVALID_REPOSITORY",
            Self::DestinationNotEmpty { .. } => "GIT_DESTINATION_NOT_EMPTY",
            Self::Unavailable { .. } => "GIT_REPOSITORY_UNAVAILABLE",
            Self::WorkingTreeDirty { .. } => "GIT_WORKING_TREE_DIRTY",
            Self::BranchNotFound { .. } => "GIT_BRANCH_NOT_FOUND",
            Self::DetachedHead { .. } => "GIT_DETACHED_HEAD",
            Self::UpstreamNotConfigured { .. } => "GIT_UPSTREAM_NOT_CONFIGURED",
            Self::DivergedBranch { .. } => "GIT_DIVERGED_BRANCH",
            Self::MergeConflict { .. } => "GIT_MERGE_CONFLICT",
            Self::AuthenticationFailed { .. } => "GIT_AUTHENTICATION_FAILED",
            Self::RemoteUnavailable { .. } => "GIT_REMOTE_UNAVAILABLE",
            Self::CommandFailed { .. } => "GIT_COMMAND_FAILED",
            Self::Timeout { .. } => "GIT_TIMEOUT",
            Self::Cancelled { .. } => "GIT_CANCELLED",
            Self::Busy { .. } => "GIT_REPOSITORY_BUSY",
            Self::MissingParameter { .. } => "GIT_MISSING_PARAMETER",
            Self::InvalidParameter { .. } => "GIT_INVALID_PARAMETER",
            Self::InvalidReference { .. } => "GIT_INVALID_REFERENCE",
            Self::PathOutsideWorkspace { .. } => "GIT_PATH_OUTSIDE_WORKSPACE",
            Self::GitUnavailable { .. } => "GIT_UNAVAILABLE",
            Self::Internal { .. } => "GIT_INTERNAL_ERROR",
        }
    }
}

/// Manages Git repositories through the process manager.
///
/// Read-only methods (`status`, `branches`, `diff`) are safe to run
/// concurrently; mutating methods acquire an exclusive repository-scoped lock so
/// two mutations of the same checkout never overlap, while independent
/// repositories proceed in parallel.
#[async_trait]
pub trait GitManager: Send + Sync {
    /// Inspects a repository's status.
    ///
    /// # Errors
    ///
    /// Returns a [`GitError`] if the repository cannot be inspected.
    async fn status(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<RepositoryStatus, GitError>;

    /// Lists local and remote-tracking branches.
    ///
    /// # Errors
    ///
    /// Returns a [`GitError`] if the repository cannot be inspected.
    async fn branches(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<Vec<BranchInfo>, GitError>;

    /// Returns a bounded diff of the working tree or the index.
    ///
    /// # Errors
    ///
    /// Returns a [`GitError`] if the repository cannot be inspected.
    async fn diff(
        &self,
        repository: &ResolvedRepository,
        options: &DiffOptions,
    ) -> Result<RepoDiff, GitError>;

    /// Fetches remote refs without touching the working tree.
    ///
    /// # Errors
    ///
    /// Returns a [`GitError`] if the fetch fails.
    async fn fetch(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<GitOperationResult, GitError>;

    /// Pulls the current branch, fast-forward only.
    ///
    /// # Errors
    ///
    /// Returns a [`GitError`] if the pull fails or is unsafe.
    async fn pull(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<GitOperationResult, GitError>;

    /// Checks out `branch`.
    ///
    /// # Errors
    ///
    /// Returns a [`GitError`] if the branch does not exist or the working tree is
    /// dirty.
    async fn checkout(
        &self,
        repository: &ResolvedRepository,
        branch: &Branch,
        options: &GitOptions,
    ) -> Result<GitOperationResult, GitError>;

    /// Clones the repository into its configured path.
    ///
    /// # Errors
    ///
    /// Returns a [`GitError`] if the destination exists or the clone fails.
    async fn clone(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<GitOperationResult, GitError>;

    /// Synchronizes the repository: fetch, checkout, fast-forward.
    ///
    /// # Errors
    ///
    /// Returns a [`GitError`] if the repository is unsafe to update.
    async fn sync(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<GitOperationResult, GitError>;

    /// Discards the working tree with `git reset --hard`.
    ///
    /// This is destructive and must only be invoked after the application has
    /// authorized it.
    ///
    /// # Errors
    ///
    /// Returns a [`GitError`] if the reset fails.
    async fn reset_hard(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<GitOperationResult, GitError>;

    /// Removes untracked files with `git clean -fd`.
    ///
    /// This is destructive and must only be invoked after authorization.
    ///
    /// # Errors
    ///
    /// Returns a [`GitError`] if the clean fails.
    async fn clean(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<GitOperationResult, GitError>;

    /// Discards every local change (reset and clean).
    ///
    /// This is destructive and must only be invoked after authorization.
    ///
    /// # Errors
    ///
    /// Returns a [`GitError`] if the operation fails.
    async fn discard_local_changes(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<GitOperationResult, GitError>;
}

/// A [`GitManager`] that is never wired to a real implementation.
///
/// It reports [`GitError::GitUnavailable`] for every operation so the
/// application remains constructible without a manager. The composition root
/// replaces it with the concrete `git` adapter.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullGitManager;

fn unavailable() -> GitError {
    GitError::GitUnavailable {
        message: "no Git manager is configured".to_owned(),
    }
}

#[async_trait]
impl GitManager for NullGitManager {
    async fn status(
        &self,
        _repository: &ResolvedRepository,
        _options: &GitOptions,
    ) -> Result<RepositoryStatus, GitError> {
        Err(unavailable())
    }

    async fn branches(
        &self,
        _repository: &ResolvedRepository,
        _options: &GitOptions,
    ) -> Result<Vec<BranchInfo>, GitError> {
        Err(unavailable())
    }

    async fn diff(
        &self,
        _repository: &ResolvedRepository,
        _options: &DiffOptions,
    ) -> Result<RepoDiff, GitError> {
        Err(unavailable())
    }

    async fn fetch(
        &self,
        _repository: &ResolvedRepository,
        _options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        Err(unavailable())
    }

    async fn pull(
        &self,
        _repository: &ResolvedRepository,
        _options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        Err(unavailable())
    }

    async fn checkout(
        &self,
        _repository: &ResolvedRepository,
        _branch: &Branch,
        _options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        Err(unavailable())
    }

    async fn clone(
        &self,
        _repository: &ResolvedRepository,
        _options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        Err(unavailable())
    }

    async fn sync(
        &self,
        _repository: &ResolvedRepository,
        _options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        Err(unavailable())
    }

    async fn reset_hard(
        &self,
        _repository: &ResolvedRepository,
        _options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        Err(unavailable())
    }

    async fn clean(
        &self,
        _repository: &ResolvedRepository,
        _options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        Err(unavailable())
    }

    async fn discard_local_changes(
        &self,
        _repository: &ResolvedRepository,
        _options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        Err(unavailable())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository() -> ResolvedRepository {
        ResolvedRepository {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            name: "auth-service".to_owned(),
            url: "git@github.com:acme/auth-service.git".to_owned(),
            path: AbsolutePath::new("/Users/dev/acme/backend/auth-service").unwrap(),
            workspace_root: AbsolutePath::new("/Users/dev/acme").unwrap(),
            default_branch: Branch::new("main").unwrap(),
        }
    }

    #[test]
    fn parameters_round_trip() {
        let original = repository();
        let decoded = ResolvedRepository::from_parameters(&original.to_parameters()).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn missing_parameters_are_reported() {
        let mut parameters = repository().to_parameters();
        parameters.remove(parameter::PATH);
        let error = ResolvedRepository::from_parameters(&parameters).unwrap_err();
        assert!(matches!(error, GitError::MissingParameter { .. }));
        assert_eq!(error.code(), "GIT_MISSING_PARAMETER");
    }

    #[test]
    fn detects_paths_outside_the_workspace() {
        assert!(repository().is_inside_workspace());
        let escaping = ResolvedRepository {
            path: AbsolutePath::new("/Users/dev/other/auth-service").unwrap(),
            ..repository()
        };
        assert!(!escaping.is_inside_workspace());
    }

    #[test]
    fn error_codes_are_stable() {
        assert_eq!(
            GitError::invalid_parameter("git.path", "bad").code(),
            "GIT_INVALID_PARAMETER"
        );
        assert_eq!(
            GitError::NotCloned {
                repository: "auth".to_owned(),
                path: "/tmp/auth".to_owned(),
            }
            .code(),
            "GIT_NOT_CLONED"
        );
    }
}
