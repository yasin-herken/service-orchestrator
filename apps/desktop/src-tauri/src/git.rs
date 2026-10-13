//! Git read and write use cases behind the desktop commands.
//!
//! These are thin adapters over the shared [`OrchestratorClient`]: reading a
//! repository list, inspecting working-tree status, listing branches, previewing
//! a bulk synchronisation, and submitting the existing Git commands as tasks or
//! workflows. No Git command is ever executed here — the daemon and the Git
//! manager own that (`tasks/020.md` §3, §6, §9).
//!
//! The repository *list* is configuration-derived and performs no Git operation,
//! so the screen stays responsive with 40–100+ repositories; real working-tree
//! state is observed on demand through `inspect_repositories`
//! (`tasks/020.md` §4, §35).

use service_orchestrator_client::OrchestratorClient;
use service_orchestrator_domain::{Branch, RepositoryStatus, Service};

use crate::dto::{
    BranchView, CommandErrorView, GitStatusView, GitSyncPreview, GitSyncPreviewEntry,
    OperationOutcome, RepositoryInspectionView, RepositorySummary,
};
use crate::error::CommandError;
use crate::validation::{parse_branch, parse_service_id, parse_service_ids, parse_workspace_id};

/// The bulk Git mode, matching the coordinated operations the core exposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SyncMode {
    /// Fetch, then check out the target branch, then fast-forward pull.
    Sync,
    /// Fetch remote refs only.
    Fetch,
    /// Check out the target branch only.
    Checkout,
}

impl SyncMode {
    fn parse(raw: &str) -> Result<Self, CommandError> {
        match raw.trim() {
            "sync" => Ok(Self::Sync),
            "fetch" => Ok(Self::Fetch),
            "checkout" => Ok(Self::Checkout),
            other => Err(CommandError::invalid_input(format!(
                "unknown git sync mode '{other}' (expected sync, fetch, or checkout)"
            ))),
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Fetch => "fetch",
            Self::Checkout => "checkout",
        }
    }

    fn operations(self) -> Vec<String> {
        match self {
            Self::Sync => vec!["fetch".to_owned(), "checkout".to_owned(), "pull".to_owned()],
            Self::Fetch => vec!["fetch".to_owned()],
            Self::Checkout => vec!["checkout".to_owned()],
        }
    }
}

/// Waits for one already-ready client future.
///
/// The daemon-backed client's `async` methods are synchronous `call(...)`
/// wrappers whose futures are ready on the first poll, so this only crosses the
/// sync/async boundary; it never blocks on real I/O (`tasks/020.md` §3). It runs
/// on the command's blocking pool, never on an async worker.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tauri::async_runtime::block_on(future)
}

/// Lists a workspace's repositories as Git rows.
///
/// The row set is derived from configuration; the last-observed Git state comes
/// from the in-memory workspace status. No Git operation runs here, so opening
/// the Git screen is cheap regardless of repository count
/// (`tasks/020.md` §4, §35).
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected workspace id, or the daemon's
/// structured error for anything else.
pub fn list_repositories(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
) -> Result<Vec<RepositorySummary>, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let services = client
        .list_services(&workspace_id)
        .map_err(CommandError::from)?;
    let status = client
        .get_workspace_status(&workspace_id)
        .map_err(CommandError::from)?;
    let workspace = workspace_id.as_str();

    let repositories: Vec<Service> = services
        .into_iter()
        .filter(|service| service.repository.is_some())
        .collect();
    Ok(repositories
        .iter()
        .map(|service| {
            let observed = status
                .services
                .iter()
                .find(|candidate| candidate.service_id == service.id);
            RepositorySummary::new(workspace, service, observed)
        })
        .collect())
}

/// Inspects one repository's working-tree status on demand.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected service id, `ServiceNotFound` for an
/// unknown service, `InvalidOperation` when it has no repository, or a `Git`
/// error when the repository cannot be inspected.
pub fn get_git_status(
    client: &dyn OrchestratorClient,
    service_id: &str,
) -> Result<GitStatusView, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let status = block_on(client.get_git_status(&service_id)).map_err(CommandError::from)?;
    Ok(GitStatusView::new(service_id.as_str(), &status))
}

/// Inspects several repositories, reporting each result independently.
///
/// A repository that cannot be inspected yields an entry with an `error` rather
/// than failing the whole batch, so the results view can show exactly which
/// repositories failed and why (`tasks/020.md` §11).
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected workspace or service identifier.
pub fn inspect_repositories(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
    service_ids: &[String],
) -> Result<Vec<RepositoryInspectionView>, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let service_ids = parse_service_ids(service_ids)?;
    let workspace = workspace_id.as_str();

    Ok(service_ids
        .iter()
        .map(|service_id| {
            let repository_id = format!("{workspace}:{service_id}");
            match block_on(client.get_git_status(service_id)) {
                Ok(status) => RepositoryInspectionView {
                    repository_id,
                    service_id: service_id.to_string(),
                    status: Some(GitStatusView::new(service_id.as_str(), &status)),
                    error: None,
                },
                Err(error) => RepositoryInspectionView {
                    repository_id,
                    service_id: service_id.to_string(),
                    status: None,
                    error: Some(CommandErrorView::from(CommandError::from(error))),
                },
            }
        })
        .collect())
}

/// Lists a repository's local and remote-tracking branches.
///
/// # Errors
///
/// Same as [`get_git_status`].
pub fn list_branches(
    client: &dyn OrchestratorClient,
    service_id: &str,
) -> Result<Vec<BranchView>, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let branches = block_on(client.list_branches(&service_id)).map_err(CommandError::from)?;
    Ok(branches.iter().map(BranchView::from).collect())
}

/// Builds a read-only preview of a bulk Git operation over a selection.
///
/// For each selected repository this aggregates the real working-tree status and
/// branch list, then reports dirty working trees, a missing target branch, and
/// diverged histories — without stashing, resetting, or discarding anything. The
/// daemon still owns and enforces safety at execution time
/// (`tasks/020.md` §7, §8, §9).
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected workspace, service, branch, or mode.
pub fn preview_git_sync(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
    service_ids: &[String],
    target_branch: &str,
    mode: &str,
) -> Result<GitSyncPreview, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let service_ids = parse_service_ids(service_ids)?;
    let target_branch = parse_branch(target_branch)?;
    let mode = SyncMode::parse(mode)?;
    let workspace = workspace_id.as_str();

    let mut entries = Vec::with_capacity(service_ids.len());
    for service_id in &service_ids {
        entries.push(preview_entry(
            client,
            workspace,
            service_id,
            &target_branch,
            mode,
        ));
    }

    let mean = |predicate: &dyn Fn(&GitSyncPreviewEntry) -> bool| {
        entries.iter().filter(|entry| predicate(entry)).count()
    };
    Ok(GitSyncPreview {
        workspace_id: workspace.to_owned(),
        target_branch: target_branch.to_string(),
        mode: mode.label().to_owned(),
        total: entries.len(),
        ready: mean(&|entry| !entry.blocked && entry.warnings.is_empty()),
        dirty: mean(&|entry| !entry.clean),
        missing_branch: mean(&|entry| !entry.target_branch_exists),
        diverged: mean(&|entry| entry.diverged),
        unavailable: mean(&|entry| entry.lifecycle != "available"),
        entries,
    })
}

/// Builds one preview entry, inspecting status and branches best-effort.
fn preview_entry(
    client: &dyn OrchestratorClient,
    workspace: &str,
    service_id: &service_orchestrator_domain::ServiceId,
    target_branch: &Branch,
    mode: SyncMode,
) -> GitSyncPreviewEntry {
    let repository_id = format!("{workspace}:{service_id}");
    let operations = mode.operations();

    let mut warnings = Vec::new();
    let status = block_on(client.get_git_status(service_id));

    let (lifecycle, git_state, clean, ahead, behind, diverged, current_branch, blocked) =
        match status {
            Ok(status) => {
                let clean = status.is_clean();
                let diverged = status.is_diverged();
                if !clean {
                    warnings.push("working tree has uncommitted local changes".to_owned());
                }
                if diverged {
                    warnings.push("branch has diverged from its upstream".to_owned());
                }
                let lifecycle = status.lifecycle.label().to_owned();
                let blocked = !status.lifecycle.is_available();
                if blocked {
                    warnings.push(format!("repository is {lifecycle}"));
                }
                (
                    lifecycle,
                    map_git_state(&status),
                    clean,
                    status.ahead,
                    status.behind,
                    diverged,
                    status.branch.as_ref().map(ToString::to_string),
                    blocked,
                )
            }
            Err(error) => {
                warnings.push(format!("repository could not be inspected: {error}"));
                (
                    "unknown".to_owned(),
                    "unknown".to_owned(),
                    false,
                    None,
                    None,
                    false,
                    None,
                    true,
                )
            }
        };

    let (target_branch_exists, target_branch_kind) =
        match block_on(client.list_branches(service_id)) {
            Ok(branches) => resolve_target_branch(&branches, target_branch),
            Err(error) => {
                if !blocked {
                    warnings.push(format!("branches could not be listed: {error}"));
                }
                (false, None)
            }
        };

    let mut blocked = blocked;
    if matches!(mode, SyncMode::Sync | SyncMode::Checkout) && !target_branch_exists {
        warnings.push(format!("target branch '{target_branch}' was not found"));
        blocked = true;
    }

    GitSyncPreviewEntry {
        repository_id,
        service_id: service_id.to_string(),
        name: service_id.to_string(),
        current_branch,
        target_branch_exists,
        target_branch_kind,
        lifecycle,
        git_state,
        clean,
        ahead,
        behind,
        diverged,
        expected_operations: operations,
        warnings,
        blocked,
    }
}

/// Reduces a status into the compact observed Git state for the preview label.
fn map_git_state(status: &RepositoryStatus) -> String {
    if !status.lifecycle.is_available() {
        return "unknown".to_owned();
    }
    if status.has_local_changes() {
        return "modified".to_owned();
    }
    if status.is_diverged() {
        return "diverged".to_owned();
    }
    if status.is_ahead() {
        return "ahead".to_owned();
    }
    if status.is_behind() {
        return "behind".to_owned();
    }
    "clean".to_owned()
}

/// Finds the target branch among a repository's refs, preferring a local ref.
fn resolve_target_branch(
    branches: &[service_orchestrator_domain::BranchInfo],
    target: &Branch,
) -> (bool, Option<String>) {
    use service_orchestrator_domain::BranchKind;

    let name = target.as_str();
    let local = branches
        .iter()
        .find(|branch| branch.kind == BranchKind::Local && branch.name == name);
    if local.is_some() {
        return (true, Some("local".to_owned()));
    }
    let remote = branches.iter().find(|branch| {
        branch.kind == BranchKind::Remote
            && (branch.name == format!("origin/{name}")
                || branch.name.ends_with(&format!("/{name}")))
    });
    match remote {
        Some(_) => (true, Some("remote".to_owned())),
        None => (false, None),
    }
}

/// Fetches one repository.
///
/// # Errors
///
/// Same as [`get_git_status`], plus the daemon's structured error when the
/// operation is rejected.
pub fn fetch_repository(
    client: &dyn OrchestratorClient,
    service_id: &str,
) -> Result<OperationOutcome, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let result = client
        .fetch_repository(&service_id)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Fast-forward pulls one repository.
///
/// # Errors
///
/// Same as [`fetch_repository`].
pub fn pull_repository(
    client: &dyn OrchestratorClient,
    service_id: &str,
) -> Result<OperationOutcome, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let result = client
        .pull_repository(&service_id)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Checks out a branch in one repository.
///
/// # Errors
///
/// Same as [`fetch_repository`], plus `INVALID_INPUT` for a rejected branch.
pub fn checkout_repository(
    client: &dyn OrchestratorClient,
    service_id: &str,
    branch: &str,
) -> Result<OperationOutcome, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let branch = parse_branch(branch)?;
    let result = client
        .checkout_repository(&service_id, &branch)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Synchronizes one repository (fetch, checkout, fast-forward).
///
/// # Errors
///
/// Same as [`fetch_repository`].
pub fn sync_repository(
    client: &dyn OrchestratorClient,
    service_id: &str,
) -> Result<OperationOutcome, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let result = client
        .sync_service(&service_id)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Fetches the selected repositories as one workflow.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected identifier or an empty selection, otherwise
/// the daemon's structured error.
pub fn fetch_repositories(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
    service_ids: &[String],
) -> Result<OperationOutcome, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let service_ids = parse_service_ids(service_ids)?;
    let result = client
        .fetch_repositories(&workspace_id, &service_ids)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Synchronizes the selected repositories as one workflow.
///
/// # Errors
///
/// Same as [`fetch_repositories`].
pub fn sync_repositories(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
    service_ids: &[String],
) -> Result<OperationOutcome, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let service_ids = parse_service_ids(service_ids)?;
    let result = client
        .sync_repositories(&workspace_id, &service_ids)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Checks out a branch in the selected repositories as one workflow.
///
/// # Errors
///
/// Same as [`fetch_repositories`], plus `INVALID_INPUT` for a rejected branch.
pub fn checkout_repositories(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
    service_ids: &[String],
    branch: &str,
) -> Result<OperationOutcome, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let service_ids = parse_service_ids(service_ids)?;
    let branch = parse_branch(branch)?;
    let result = client
        .checkout_repositories(&workspace_id, &service_ids, &branch)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_mode_parses_and_describes_its_operations() {
        assert_eq!(SyncMode::parse("sync").expect("valid").label(), "sync");
        assert_eq!(
            SyncMode::parse("sync").expect("valid").operations(),
            vec!["fetch".to_owned(), "checkout".to_owned(), "pull".to_owned()]
        );
        assert_eq!(
            SyncMode::parse("fetch").expect("valid").operations(),
            vec!["fetch".to_owned()]
        );
        assert_eq!(
            SyncMode::parse("checkout").expect("valid").operations(),
            vec!["checkout".to_owned()]
        );
        let error = SyncMode::parse("rebase").expect_err("unknown mode");
        assert_eq!(error.code, "INVALID_INPUT");
    }

    #[test]
    fn invalid_inputs_are_rejected_before_any_ipc_call() {
        assert_eq!(
            parse_service_id("").expect_err("empty").code,
            "INVALID_INPUT"
        );
        assert_eq!(
            parse_workspace_id("bad id").expect_err("space").code,
            "INVALID_INPUT"
        );
        assert_eq!(parse_branch("").expect_err("empty").code, "INVALID_INPUT");
    }
}
