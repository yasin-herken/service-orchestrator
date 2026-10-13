//! The concrete Git manager.
//!
//! [`CliGitManager`] implements the application's [`GitManager`] port by driving
//! the installed Git CLI through the process manager. It is the only place in
//! the Git adapter that decides *what* Git command to run; the command shapes
//! live in [`crate::command`], the process interaction in [`crate::runner`], the
//! output parsing in [`crate::status`], and per-repository coordination in
//! [`crate::coordinator`].
//!
//! # Concurrency
//!
//! Mutating operations take a per-repository write lock; read-only operations
//! take a read lock. Two mutations of the same checkout therefore serialise,
//! while independent repositories and concurrent reads proceed in parallel. The
//! engine's `max_parallel_tasks` remains the only global bound.
//!
//! # Safety
//!
//! No operation silently discards developer work. `pull` and `sync` refuse a
//! dirty working tree, `checkout` lets Git refuse when local changes would be
//! overwritten, and the destructive operations (`reset_hard`, `clean`,
//! `discard_local_changes`) are only reachable after the application has
//! authorized them.

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use service_orchestrator_application::{
    DiffOptions, GitError, GitManager, GitOptions, ResolvedRepository,
};
use service_orchestrator_domain::{
    Branch, BranchInfo, DiffFileStat, GitOperation, GitOperationResult, RepoDiff,
    RepositoryLifecycle, RepositoryStatus,
};
use service_orchestrator_process::{ExitReason, ProcessError, ProcessManager, ProcessSpec};

use crate::command::{
    branch_list_args, checkout_args, checkout_track_args, clean_args, clone_args,
    diff_numstat_args, diff_patch_args, fetch_args, non_interactive_environment, pull_args,
    reset_hard_args, rev_parse_head_args, status_args, verify_local_branch_args,
    verify_remote_branch_args, GIT_PROGRAM, HARDENED_ENVIRONMENT_REMOVALS,
};
use crate::coordinator::RepositoryCoordinator;
use crate::runner::{CommandOutput, GitRunner};
use crate::status::{parse_branches, parse_status};

/// The maximum number of bytes of Git `stderr` kept as failure detail.
const MAX_DETAIL_BYTES: usize = 4096;

/// A [`GitManager`] backed by the installed Git CLI and the process manager.
#[derive(Clone)]
pub struct CliGitManager {
    runner: GitRunner,
    coordinator: RepositoryCoordinator,
}

impl CliGitManager {
    /// Creates a manager that runs Git through `processes`.
    #[must_use]
    pub fn new(processes: ProcessManager) -> Self {
        Self {
            runner: GitRunner::new(processes),
            coordinator: RepositoryCoordinator::new(),
        }
    }

    /// Returns the repository-scoped coordinator, for observability and tests.
    #[must_use]
    pub fn coordinator(&self) -> &RepositoryCoordinator {
        &self.coordinator
    }

    // -- validation -----------------------------------------------------------

    /// Rejects a repository whose checkout escapes its workspace.
    fn validate(&self, repository: &ResolvedRepository) -> Result<(), GitError> {
        if !repository.is_inside_workspace() {
            return Err(GitError::PathOutsideWorkspace {
                repository: repository.name.clone(),
            });
        }
        Ok(())
    }

    /// Inspects the filesystem to decide the repository lifecycle cheaply.
    ///
    /// A path that exists as a directory is only *tentatively* available; the
    /// caller confirms it is a Git working copy when the Git command runs.
    fn lifecycle_precheck(
        repository: &ResolvedRepository,
    ) -> Result<RepositoryLifecycle, GitError> {
        match std::fs::metadata(repository.path.as_str()) {
            Ok(meta) if meta.is_dir() => Ok(RepositoryLifecycle::Available),
            Ok(_) => Ok(RepositoryLifecycle::Invalid),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(RepositoryLifecycle::NotCloned)
            }
            Err(error) => Err(GitError::Unavailable {
                repository: repository.name.clone(),
                message: error.to_string(),
            }),
        }
    }

    /// Requires the checkout to exist and be a directory.
    fn ensure_repository(&self, repository: &ResolvedRepository) -> Result<(), GitError> {
        match Self::lifecycle_precheck(repository)? {
            RepositoryLifecycle::Available => Ok(()),
            RepositoryLifecycle::NotCloned => Err(GitError::NotCloned {
                repository: repository.name.clone(),
                path: repository.path.to_string(),
            }),
            RepositoryLifecycle::Invalid => Err(GitError::InvalidRepository {
                repository: repository.name.clone(),
                path: repository.path.to_string(),
            }),
            other => Err(GitError::Unavailable {
                repository: repository.name.clone(),
                message: format!("repository lifecycle is '{other}'"),
            }),
        }
    }

    /// Turns a lifecycle value into the matching structured error.
    fn require_available(
        repository: &ResolvedRepository,
        lifecycle: RepositoryLifecycle,
    ) -> Result<(), GitError> {
        match lifecycle {
            RepositoryLifecycle::Available => Ok(()),
            RepositoryLifecycle::NotCloned => Err(GitError::NotCloned {
                repository: repository.name.clone(),
                path: repository.path.to_string(),
            }),
            RepositoryLifecycle::Invalid => Err(GitError::InvalidRepository {
                repository: repository.name.clone(),
                path: repository.path.to_string(),
            }),
            RepositoryLifecycle::Busy => Err(GitError::Busy {
                repository: repository.name.clone(),
            }),
            other => Err(GitError::Unavailable {
                repository: repository.name.clone(),
                message: format!("repository lifecycle is '{other}'"),
            }),
        }
    }

    /// The coordinator key for a repository: its absolute checkout path.
    fn lock_key(repository: &ResolvedRepository) -> String {
        repository.path.as_str().to_owned()
    }

    // -- process execution ----------------------------------------------------

    /// Runs one Git command, mapping process-level failures to Git errors.
    ///
    /// A non-zero Git exit status is *not* an error here; callers decide
    /// whether the exit status matters (for example `rev-parse --verify
    /// --quiet` uses a non-zero status to mean "absent").
    async fn run(
        &self,
        repository: &ResolvedRepository,
        operation: GitOperation,
        args: Vec<String>,
        options: &GitOptions,
        working_dir: &str,
    ) -> Result<CommandOutput, GitError> {
        let mut spec = ProcessSpec::new(GIT_PROGRAM)
            .map_err(|error| GitError::GitUnavailable {
                message: error.to_string(),
            })?
            .args(args)
            .working_dir(working_dir)
            .with_environment(non_interactive_environment())
            // An inherited `GIT_DIR`, `GIT_WORK_TREE`, `GIT_SSH_COMMAND`, and
            // friends could redirect or bend this operation; remove them
            // (`AGENTS.md` §21, §32).
            .remove_envs(HARDENED_ENVIRONMENT_REMOVALS.iter().copied());
        if let Some(timeout) = options.timeout {
            spec = spec.with_timeout(timeout);
        }

        let output = self
            .runner
            .run(spec, options.cancellation.clone())
            .await
            .map_err(map_process_error)?;

        match output.reason {
            ExitReason::Cancelled => Err(GitError::Cancelled {
                repository: repository.name.clone(),
                operation,
            }),
            ExitReason::TimedOut => Err(GitError::Timeout {
                repository: repository.name.clone(),
                operation,
                timeout_seconds: options.timeout.map(|d| d.as_secs()).unwrap_or(0),
            }),
            _ => Ok(output),
        }
    }

    /// Runs one Git command, requiring a successful exit status.
    async fn run_checked(
        &self,
        repository: &ResolvedRepository,
        operation: GitOperation,
        args: Vec<String>,
        options: &GitOptions,
        working_dir: &str,
    ) -> Result<CommandOutput, GitError> {
        let output = self
            .run(repository, operation, args, options, working_dir)
            .await?;
        if output.succeeded() {
            Ok(output)
        } else {
            Err(classify_failure(repository, operation, &output))
        }
    }

    /// Returns the current commit, or `None` for an unborn branch.
    async fn head_commit(
        &self,
        repository: &ResolvedRepository,
        operation: GitOperation,
        options: &GitOptions,
    ) -> Result<Option<String>, GitError> {
        let output = self
            .run(
                repository,
                operation,
                rev_parse_head_args(),
                options,
                repository.path.as_str(),
            )
            .await?;
        if output.succeeded() {
            let commit = output.stdout.trim();
            Ok(if commit.is_empty() {
                None
            } else {
                Some(commit.to_owned())
            })
        } else {
            Ok(None)
        }
    }

    /// Returns whether a local branch reference exists.
    async fn local_branch_exists(
        &self,
        repository: &ResolvedRepository,
        branch: &str,
        options: &GitOptions,
    ) -> Result<bool, GitError> {
        self.reference_exists(
            repository,
            verify_local_branch_args(branch),
            branch,
            options,
        )
        .await
    }

    /// Returns whether a remote-tracking branch reference exists.
    async fn remote_branch_exists(
        &self,
        repository: &ResolvedRepository,
        branch: &str,
        options: &GitOptions,
    ) -> Result<bool, GitError> {
        self.reference_exists(
            repository,
            verify_remote_branch_args(branch),
            branch,
            options,
        )
        .await
    }

    async fn reference_exists(
        &self,
        repository: &ResolvedRepository,
        args: Vec<String>,
        branch: &str,
        options: &GitOptions,
    ) -> Result<bool, GitError> {
        let output = self
            .run(
                repository,
                GitOperation::Checkout,
                args,
                options,
                repository.path.as_str(),
            )
            .await?;
        if output.truncated {
            return Err(truncated_error(repository, GitOperation::Checkout));
        }
        if output.succeeded() {
            return Ok(true);
        }
        // A non-zero status can mean "absent" or a broken repository; treat a
        // repository-level failure as an inspection error.
        let stderr = output.stderr.to_ascii_lowercase();
        if not_a_work_tree(&stderr) {
            return Err(GitError::InvalidRepository {
                repository: repository.name.clone(),
                path: repository.path.to_string(),
            });
        }
        let _ = branch;
        Ok(false)
    }

    // -- status helpers -------------------------------------------------------

    /// Inspects status without acquiring the coordinator lock.
    ///
    /// Callers must already hold the appropriate read or write lock. This exists
    /// so a mutation can inspect the working tree under its own write lock
    /// without re-locking (the coordinator lock is not reentrant).
    async fn status_unlocked(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<RepositoryStatus, GitError> {
        match Self::lifecycle_precheck(repository)? {
            RepositoryLifecycle::Available => {}
            other => {
                return Ok(RepositoryStatus {
                    lifecycle: other,
                    ..RepositoryStatus::default()
                })
            }
        }

        let output = self
            .run(
                repository,
                GitOperation::Status,
                status_args(),
                options,
                repository.path.as_str(),
            )
            .await?;

        if output.succeeded() {
            return Ok(parse_status(&output.stdout));
        }
        if not_a_work_tree(&output.stderr.to_ascii_lowercase()) {
            return Ok(RepositoryStatus {
                lifecycle: RepositoryLifecycle::Invalid,
                ..RepositoryStatus::default()
            });
        }
        Err(classify_failure(repository, GitOperation::Status, &output))
    }

    /// Confirms the working tree is safe to mutate: available and clean.
    async fn require_clean(
        &self,
        repository: &ResolvedRepository,
        status: &RepositoryStatus,
    ) -> Result<(), GitError> {
        Self::require_available(repository, status.lifecycle)?;
        if status.detached_head {
            return Err(GitError::DetachedHead {
                repository: repository.name.clone(),
            });
        }
        if status.has_local_changes() {
            return Err(GitError::WorkingTreeDirty {
                repository: repository.name.clone(),
                detail: describe_changes(status),
            });
        }
        Ok(())
    }
}

#[async_trait]
impl GitManager for CliGitManager {
    async fn status(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<RepositoryStatus, GitError> {
        self.validate(repository)?;
        let _guard = self.coordinator.read(&Self::lock_key(repository)).await;
        self.status_unlocked(repository, options).await
    }

    async fn branches(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<Vec<BranchInfo>, GitError> {
        self.validate(repository)?;
        let _guard = self.coordinator.read(&Self::lock_key(repository)).await;
        self.ensure_repository(repository)?;
        let output = self
            .run_checked(
                repository,
                GitOperation::Branches,
                branch_list_args(),
                options,
                repository.path.as_str(),
            )
            .await?;
        Ok(parse_branches(&output.stdout))
    }

    async fn diff(
        &self,
        repository: &ResolvedRepository,
        options: &DiffOptions,
    ) -> Result<RepoDiff, GitError> {
        self.validate(repository)?;
        let _guard = self.coordinator.read(&Self::lock_key(repository)).await;
        self.ensure_repository(repository)?;

        let git_options = GitOptions {
            timeout: options.timeout,
            cancellation: options.cancellation.clone(),
            prune: false,
        };
        let numstat = self
            .run_checked(
                repository,
                GitOperation::Diff,
                diff_numstat_args(options.staged),
                &git_options,
                repository.path.as_str(),
            )
            .await?;

        // Redirect the patch to a temporary file and read back a bounded
        // prefix. A large diff would otherwise overflow the process manager's
        // line-event channel and be reported as a capture failure rather than a
        // truncated patch.
        let patch_file = TempPatchFile::create()?;
        self.run_checked(
            repository,
            GitOperation::Diff,
            diff_patch_args(options.staged, patch_file.path()),
            &git_options,
            repository.path.as_str(),
        )
        .await?;

        let files = parse_numstat(&numstat.stdout);
        let (patch, truncated) = bound_patch(
            patch_file.read_prefix(options.max_bytes)?,
            options.max_bytes,
        );
        Ok(RepoDiff {
            staged: options.staged,
            files,
            bytes: patch.len(),
            patch,
            truncated,
        })
    }

    async fn fetch(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        self.validate(repository)?;
        let _guard = self.coordinator.write(&Self::lock_key(repository)).await;
        self.ensure_repository(repository)?;
        self.run_checked(
            repository,
            GitOperation::Fetch,
            fetch_args(options.prune),
            options,
            repository.path.as_str(),
        )
        .await?;
        // A fetch only updates remote-tracking refs; HEAD does not move.
        Ok(GitOperationResult::succeeded(
            repository.name.clone(),
            GitOperation::Fetch,
        ))
    }

    async fn pull(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        self.validate(repository)?;
        let _guard = self.coordinator.write(&Self::lock_key(repository)).await;

        let status = self.status_unlocked(repository, options).await?;
        self.require_clean(repository, &status).await?;

        // Fetch first so the ahead/behind counts reflect the remote; a fetch
        // never touches the working tree.
        self.run_checked(
            repository,
            GitOperation::Fetch,
            fetch_args(options.prune),
            options,
            repository.path.as_str(),
        )
        .await?;
        let status = self.status_unlocked(repository, options).await?;
        if status.upstream.is_none() {
            return Err(GitError::UpstreamNotConfigured {
                repository: repository.name.clone(),
                branch: branch_label(&status, repository),
            });
        }
        if status.is_diverged() {
            return Err(GitError::DivergedBranch {
                repository: repository.name.clone(),
                branch: branch_label(&status, repository),
                ahead: status.ahead.unwrap_or(0),
                behind: status.behind.unwrap_or(0),
            });
        }

        let previous = status.commit.clone();
        self.run_checked(
            repository,
            GitOperation::Pull,
            pull_args(),
            options,
            repository.path.as_str(),
        )
        .await?;
        let current = self
            .head_commit(repository, GitOperation::Pull, options)
            .await?;
        Ok(
            GitOperationResult::succeeded(repository.name.clone(), GitOperation::Pull)
                .with_branch(status.branch)
                .with_commits(previous, current),
        )
    }

    async fn checkout(
        &self,
        repository: &ResolvedRepository,
        branch: &Branch,
        options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        self.validate(repository)?;
        let _guard = self.coordinator.write(&Self::lock_key(repository)).await;
        self.ensure_repository(repository)?;

        let local = self
            .local_branch_exists(repository, branch.as_str(), options)
            .await?;
        let remote = if local {
            false
        } else {
            self.remote_branch_exists(repository, branch.as_str(), options)
                .await?
        };
        if !local && !remote {
            return Err(GitError::BranchNotFound {
                repository: repository.name.clone(),
                branch: branch.to_string(),
            });
        }

        let previous = self
            .head_commit(repository, GitOperation::Checkout, options)
            .await?;
        let args = if local {
            checkout_args(branch.as_str())
        } else {
            checkout_track_args(branch.as_str())
        };
        let output = self
            .run(
                repository,
                GitOperation::Checkout,
                args,
                options,
                repository.path.as_str(),
            )
            .await?;
        if !output.succeeded() {
            let stderr = output.stderr.to_ascii_lowercase();
            if would_overwrite_local_changes(&stderr) {
                return Err(GitError::WorkingTreeDirty {
                    repository: repository.name.clone(),
                    detail: first_line(&sanitize(&output.stderr, repository)),
                });
            }
            return Err(classify_failure(
                repository,
                GitOperation::Checkout,
                &output,
            ));
        }

        let current = self
            .head_commit(repository, GitOperation::Checkout, options)
            .await?;
        Ok(
            GitOperationResult::succeeded(repository.name.clone(), GitOperation::Checkout)
                .with_branch(Some(branch.clone()))
                .with_commits(previous, current),
        )
    }

    async fn clone(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        self.validate(repository)?;
        let _guard = self.coordinator.write(&Self::lock_key(repository)).await;

        // Never overwrite an existing non-empty directory.
        match std::fs::read_dir(repository.path.as_str()) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Err(GitError::DestinationNotEmpty {
                        repository: repository.name.clone(),
                        path: repository.path.to_string(),
                    });
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(GitError::Unavailable {
                    repository: repository.name.clone(),
                    message: error.to_string(),
                })
            }
        }

        let args = clone_args(
            &repository.url,
            repository.default_branch.as_str(),
            repository.path.as_str(),
        );
        self.run_checked(
            repository,
            GitOperation::Clone,
            args,
            options,
            repository.workspace_root.as_str(),
        )
        .await?;
        let current = self
            .head_commit(repository, GitOperation::Clone, options)
            .await?;
        Ok(
            GitOperationResult::succeeded(repository.name.clone(), GitOperation::Clone)
                .with_branch(Some(repository.default_branch.clone()))
                .with_commits(None, current),
        )
    }

    async fn sync(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        self.validate(repository)?;
        let _guard = self.coordinator.write(&Self::lock_key(repository)).await;

        let status = self.status_unlocked(repository, options).await?;
        self.require_clean(repository, &status).await?;
        let previous = status.commit.clone();
        let target = repository.default_branch.clone();

        // Fetch first so a remote-only target branch can be checked out.
        self.run_checked(
            repository,
            GitOperation::Fetch,
            fetch_args(options.prune),
            options,
            repository.path.as_str(),
        )
        .await?;

        if status.branch.as_ref() != Some(&target) {
            let local = self
                .local_branch_exists(repository, target.as_str(), options)
                .await?;
            let remote = if local {
                false
            } else {
                self.remote_branch_exists(repository, target.as_str(), options)
                    .await?
            };
            if !local && !remote {
                return Err(GitError::BranchNotFound {
                    repository: repository.name.clone(),
                    branch: target.to_string(),
                });
            }
            let args = if local {
                checkout_args(target.as_str())
            } else {
                checkout_track_args(target.as_str())
            };
            self.run_checked(
                repository,
                GitOperation::Checkout,
                args,
                options,
                repository.path.as_str(),
            )
            .await?;
        }

        // Re-inspect the (possibly new) branch before pulling.
        let current = self.status_unlocked(repository, options).await?;
        self.require_clean(repository, &current).await?;
        if current.upstream.is_none() {
            return Err(GitError::UpstreamNotConfigured {
                repository: repository.name.clone(),
                branch: branch_label(&current, repository),
            });
        }
        if current.is_diverged() {
            return Err(GitError::DivergedBranch {
                repository: repository.name.clone(),
                branch: branch_label(&current, repository),
                ahead: current.ahead.unwrap_or(0),
                behind: current.behind.unwrap_or(0),
            });
        }

        self.run_checked(
            repository,
            GitOperation::Pull,
            pull_args(),
            options,
            repository.path.as_str(),
        )
        .await?;
        let after = self
            .head_commit(repository, GitOperation::Sync, options)
            .await?;
        Ok(
            GitOperationResult::succeeded(repository.name.clone(), GitOperation::Sync)
                .with_branch(current.branch)
                .with_commits(previous, after),
        )
    }

    async fn reset_hard(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        self.validate(repository)?;
        let _guard = self.coordinator.write(&Self::lock_key(repository)).await;
        self.ensure_repository(repository)?;
        let previous = self
            .head_commit(repository, GitOperation::ResetHard, options)
            .await?;
        self.run_checked(
            repository,
            GitOperation::ResetHard,
            reset_hard_args(),
            options,
            repository.path.as_str(),
        )
        .await?;
        let current = self
            .head_commit(repository, GitOperation::ResetHard, options)
            .await?;
        Ok(
            GitOperationResult::succeeded(repository.name.clone(), GitOperation::ResetHard)
                .with_commits(previous, current),
        )
    }

    async fn clean(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        self.validate(repository)?;
        let _guard = self.coordinator.write(&Self::lock_key(repository)).await;
        self.ensure_repository(repository)?;
        self.run_checked(
            repository,
            GitOperation::Clean,
            clean_args(),
            options,
            repository.path.as_str(),
        )
        .await?;
        Ok(GitOperationResult::succeeded(
            repository.name.clone(),
            GitOperation::Clean,
        ))
    }

    async fn discard_local_changes(
        &self,
        repository: &ResolvedRepository,
        options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        self.validate(repository)?;
        let _guard = self.coordinator.write(&Self::lock_key(repository)).await;
        self.ensure_repository(repository)?;
        let previous = self
            .head_commit(repository, GitOperation::DiscardChanges, options)
            .await?;
        self.run_checked(
            repository,
            GitOperation::DiscardChanges,
            reset_hard_args(),
            options,
            repository.path.as_str(),
        )
        .await?;
        self.run_checked(
            repository,
            GitOperation::DiscardChanges,
            clean_args(),
            options,
            repository.path.as_str(),
        )
        .await?;
        let current = self
            .head_commit(repository, GitOperation::DiscardChanges, options)
            .await?;
        Ok(
            GitOperationResult::succeeded(repository.name.clone(), GitOperation::DiscardChanges)
                .with_commits(previous, current),
        )
    }
}

impl std::fmt::Debug for CliGitManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CliGitManager")
            .field("repositories", &self.coordinator.len())
            .finish_non_exhaustive()
    }
}

/// Maps a process-manager failure into a structured Git error.
fn map_process_error(error: ProcessError) -> GitError {
    match error {
        ProcessError::Spawn { message, .. } => GitError::GitUnavailable {
            message: format!("could not start the git executable: {message}"),
        },
        other => GitError::Internal {
            message: other.to_string(),
        },
    }
}

/// Classifies a failed Git invocation into a specific structured error.
fn classify_failure(
    repository: &ResolvedRepository,
    operation: GitOperation,
    output: &CommandOutput,
) -> GitError {
    if output.truncated {
        return truncated_error(repository, operation);
    }
    let stderr = output.stderr.trim();
    let lower = stderr.to_ascii_lowercase();

    if not_a_work_tree(&lower) {
        return GitError::InvalidRepository {
            repository: repository.name.clone(),
            path: repository.path.to_string(),
        };
    }
    if is_authentication_failure(&lower) {
        return GitError::AuthenticationFailed {
            repository: repository.name.clone(),
            message: summarize(stderr, repository),
        };
    }
    if is_remote_unavailable(&lower) {
        return GitError::RemoteUnavailable {
            repository: repository.name.clone(),
            message: summarize(stderr, repository),
        };
    }

    let detail = sanitize(stderr, repository);
    GitError::CommandFailed {
        repository: repository.name.clone(),
        operation,
        exit_code: output.code,
        message: first_line(&detail),
        detail: if detail.is_empty() {
            None
        } else {
            Some(bound(&detail, MAX_DETAIL_BYTES))
        },
    }
}

/// Builds the error for output too large to capture faithfully.
fn truncated_error(repository: &ResolvedRepository, operation: GitOperation) -> GitError {
    GitError::CommandFailed {
        repository: repository.name.clone(),
        operation,
        exit_code: None,
        message: "git output exceeded the capture buffer and could not be read completely"
            .to_owned(),
        detail: None,
    }
}

/// Returns `true` when stderr indicates the path is not a Git working copy.
fn not_a_work_tree(stderr_lowercase: &str) -> bool {
    stderr_lowercase.contains("not a git repository")
        || stderr_lowercase.contains("must be run in a work tree")
}

/// Returns `true` for the Git messages that mean authentication failed.
fn is_authentication_failure(stderr_lowercase: &str) -> bool {
    [
        "authentication failed",
        "permission denied (publickey",
        "permission denied (publickey)",
        "could not read username",
        "could not read password",
        "terminal prompts disabled",
        "invalid username or password",
        "support for password authentication was removed",
    ]
    .iter()
    .any(|needle| stderr_lowercase.contains(needle))
}

/// Returns `true` for the Git messages that mean the remote is unreachable.
fn is_remote_unavailable(stderr_lowercase: &str) -> bool {
    [
        "could not resolve host",
        "unable to access",
        "connection refused",
        "connection timed out",
        "repository not found",
        "does not appear to be a git repository",
        "could not read from remote repository",
        "no route to host",
        "network is unreachable",
    ]
    .iter()
    .any(|needle| stderr_lowercase.contains(needle))
}

/// Returns `true` when a checkout failed because local changes would be lost.
fn would_overwrite_local_changes(stderr_lowercase: &str) -> bool {
    stderr_lowercase.contains("local changes")
        || stderr_lowercase.contains("would be overwritten")
        || stderr_lowercase.contains("please commit your changes")
        || stderr_lowercase.contains("please, commit your changes")
}

/// Parses `git diff --numstat` output into per-file statistics.
fn parse_numstat(output: &str) -> Vec<DiffFileStat> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let insertions = fields.next()?;
            let deletions = fields.next()?;
            let path = fields.next()?;
            if path.is_empty() {
                return None;
            }
            Some(DiffFileStat {
                path: path.to_owned(),
                insertions: insertions.parse().unwrap_or(0),
                deletions: deletions.parse().unwrap_or(0),
            })
        })
        .collect()
}

/// A temporary file that holds a `git diff` patch for the lifetime of one
/// operation and is removed when dropped.
///
/// Using the file system as the sink keeps the patch from travelling through
/// the process manager's bounded line-event channel; the caller reads back only
/// the prefix it will return. The path is unique per process and call.
struct TempPatchFile {
    path: PathBuf,
}

impl TempPatchFile {
    /// Reserves a unique path in the system temporary directory.
    fn create() -> Result<Self, GitError> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("so-git-diff-{}-{unique}.patch", std::process::id()));
        if path.to_str().is_none() {
            return Err(GitError::internal(
                "the system temporary directory is not valid UTF-8",
            ));
        }
        Ok(Self { path })
    }

    /// The path as a string argument for `git diff --output=`.
    fn path(&self) -> &str {
        self.path
            .to_str()
            .expect("the temporary path was validated to be UTF-8")
    }

    /// Reads at most `max_bytes` (plus a single byte to detect truncation).
    ///
    /// A missing file means `git diff` produced no patch at all, which happens
    /// when the working tree is clean; that is an empty patch, not an error.
    fn read_prefix(&self, max_bytes: usize) -> Result<String, GitError> {
        let file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
            Err(error) => {
                return Err(GitError::internal(format!(
                    "could not read the captured diff: {error}"
                )))
            }
        };
        let limit = max_bytes.saturating_add(1) as u64;
        let mut bytes = Vec::new();
        file.take(limit).read_to_end(&mut bytes).map_err(|error| {
            GitError::internal(format!("could not read the captured diff: {error}"))
        })?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

impl Drop for TempPatchFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Truncates a patch to `max_bytes` on a character boundary.
fn bound_patch(patch: String, max_bytes: usize) -> (String, bool) {
    if patch.len() <= max_bytes {
        return (patch, false);
    }
    let mut end = max_bytes;
    while end > 0 && !patch.is_char_boundary(end) {
        end -= 1;
    }
    (patch[..end].to_owned(), true)
}

/// A concise human-readable summary of a dirty working tree.
fn describe_changes(status: &RepositoryStatus) -> String {
    format!(
        "{} staged, {} unstaged, {} untracked, {} conflicted",
        status.staged_changes, status.unstaged_changes, status.untracked_files, status.conflicts
    )
}

/// The branch label to use in an error, falling back to the default branch.
fn branch_label(status: &RepositoryStatus, repository: &ResolvedRepository) -> String {
    status
        .branch
        .as_ref()
        .map(Branch::to_string)
        .unwrap_or_else(|| repository.default_branch.to_string())
}

/// Returns the first non-empty line of `text`, bounded.
fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    bound(line.trim(), MAX_DETAIL_BYTES)
}

/// A concise message for an authentication or remote failure.
fn summarize(text: &str, repository: &ResolvedRepository) -> String {
    let sanitized = sanitize(text, repository);
    let summary = first_line(&sanitized);
    if summary.is_empty() {
        "no details were reported by git".to_owned()
    } else {
        summary
    }
}

/// Truncates `text` to at most `max` bytes on a character boundary.
fn bound(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// Removes the configured URL and any URL userinfo from Git output.
///
/// Configuration already rejects credential-bearing URLs (`Repository::new`),
/// but Git itself can echo a credential supplied by a helper, so every URL
/// authority with userinfo is redacted defensively (`AGENTS.md` §31).
fn sanitize(text: &str, repository: &ResolvedRepository) -> String {
    let redacted = if repository.url.is_empty() {
        text.to_owned()
    } else {
        text.replace(&repository.url, "<remote>")
    };
    redact_url_userinfo(&redacted)
}

/// Replaces `scheme://userinfo@host` fragments with `scheme://***@host`.
fn redact_url_userinfo(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find("://") {
        let after = index + 3;
        result.push_str(&rest[..after]);
        let remainder = &rest[after..];
        let end = remainder
            .find(|c: char| c.is_whitespace() || c == '/')
            .unwrap_or(remainder.len());
        let authority = &remainder[..end];
        match authority.rsplit_once('@') {
            Some((_userinfo, host)) => {
                result.push_str("***@");
                result.push_str(host);
            }
            None => result.push_str(authority),
        }
        rest = &remainder[end..];
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_domain::{AbsolutePath, WorkspaceId};

    fn repository(url: &str) -> ResolvedRepository {
        ResolvedRepository {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            name: "auth-service".to_owned(),
            url: url.to_owned(),
            path: AbsolutePath::new("/Users/dev/acme/backend/auth-service").unwrap(),
            workspace_root: AbsolutePath::new("/Users/dev/acme").unwrap(),
            default_branch: Branch::new("main").unwrap(),
        }
    }

    #[test]
    fn not_a_work_tree_is_recognized() {
        assert!(not_a_work_tree(
            "fatal: not a git repository (or any of the parent directories)"
        ));
        assert!(not_a_work_tree(
            "fatal: this operation must be run in a work tree"
        ));
        assert!(!not_a_work_tree("fatal: could not resolve host"));
    }

    #[test]
    fn authentication_and_remote_failures_are_classified() {
        assert!(is_authentication_failure(
            "fatal: authentication failed for 'https://example.com/x.git'"
        ));
        assert!(is_authentication_failure(
            "git@github.com: permission denied (publickey)."
        ));
        assert!(is_remote_unavailable(
            "fatal: unable to access 'https://x/': could not resolve host: x"
        ));
        assert!(!is_remote_unavailable("fatal: authentication failed"));
    }

    #[test]
    fn numstat_parses_text_and_binary_entries() {
        let stats = parse_numstat("12\t3\tsrc/a.rs\n-\t-\tassets/logo.png\n");
        assert_eq!(stats.len(), 2);
        assert_eq!(stats[0].path, "src/a.rs");
        assert_eq!(stats[0].insertions, 12);
        assert_eq!(stats[0].deletions, 3);
        // Binary files report `-`; they are represented as zero-line changes.
        assert_eq!(stats[1].insertions, 0);
        assert_eq!(stats[1].deletions, 0);
    }

    #[test]
    fn patch_is_bounded_on_a_character_boundary() {
        let patch = "abcdef".to_owned();
        let (text, truncated) = bound_patch(patch.clone(), 4);
        assert_eq!(text, "abcd");
        assert!(truncated);
        let (full, not_truncated) = bound_patch(patch, 16);
        assert_eq!(full, "abcdef");
        assert!(!not_truncated);

        // A multi-byte character must not be split.
        let unicode = "é".repeat(4); // 8 bytes
        let (text, truncated) = bound_patch(unicode, 5);
        assert_eq!(text, "éé");
        assert!(truncated);
    }

    #[test]
    fn urls_and_userinfo_are_redacted() {
        let redacted = redact_url_userinfo(
            "fatal: could not read from https://user:token@example.com/acme/app.git",
        );
        assert!(!redacted.contains("token"));
        assert!(redacted.contains("***@example.com"));

        let repository = repository("git@github.com:acme/auth-service.git");
        let sanitized = sanitize(
            "fatal: repository 'git@github.com:acme/auth-service.git' not found",
            &repository,
        );
        assert!(!sanitized.contains("git@github.com"));
        assert!(sanitized.contains("<remote>"));
    }

    #[test]
    fn bound_appends_an_ellipsis() {
        assert_eq!(bound("hello", 16), "hello");
        assert_eq!(bound("hello world", 5), "hello…");
    }
}
