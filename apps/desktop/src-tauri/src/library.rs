//! Library read and write use cases behind the desktop commands.
//!
//! The GUI never runs Maven: it asks the application core for a library's
//! configuration and best-effort repository status, its dependents, its local
//! artifact presence, and its consumer version alignment, then submits builds,
//! installs, syncs, and preparation workflows through the existing use cases
//! (`tasks/020.md` §20–§26).

use service_orchestrator_application::{LibraryPrepareOptions, PrepareSelectedLibrariesRequest};
use service_orchestrator_client::OrchestratorClient;

use crate::dto::{
    ArtifactVerificationView, LibraryDependentView, LibrarySummary, OperationOutcome,
    VersionAlignmentView,
};
use crate::error::CommandError;
use crate::validation::{parse_branch, parse_library_id, parse_library_ids, parse_workspace_id};

/// Waits for one already-ready client future (see `git::block_on`).
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tauri::async_runtime::block_on(future)
}

/// Lists a workspace's libraries with observed state and dependents.
///
/// Repository state is inspected best-effort per library, so a library whose
/// checkout cannot be read still appears with a structured `repository_error`
/// rather than failing the whole list (`tasks/020.md` §20).
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected workspace id, or the daemon's
/// structured error for anything else.
pub fn list_libraries(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
) -> Result<Vec<LibrarySummary>, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let libraries = client
        .list_libraries(&workspace_id)
        .map_err(CommandError::from)?;

    let mut summaries = Vec::with_capacity(libraries.len());
    for library in &libraries {
        let status = block_on(client.get_library_status(&library.id)).ok();
        let dependents = client
            .find_library_dependents(&library.id)
            .map_err(CommandError::from)?;
        summaries.push(LibrarySummary::new(library, status.as_ref(), &dependents));
    }
    Ok(summaries)
}

/// Lists the services that depend on one library.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected library id or `LibraryNotFound` for
/// an unknown library.
pub fn get_library_dependents(
    client: &dyn OrchestratorClient,
    library_id: &str,
) -> Result<Vec<LibraryDependentView>, CommandError> {
    let library_id = parse_library_id(library_id)?;
    let dependents = client
        .find_library_dependents(&library_id)
        .map_err(CommandError::from)?;
    Ok(dependents.iter().map(LibraryDependentView::from).collect())
}

/// Verifies a library's artifact in the effective local repository.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected library id, `LibraryNotFound` for an
/// unknown library, or the daemon's structured error for anything else.
pub fn verify_library_artifact(
    client: &dyn OrchestratorClient,
    library_id: &str,
) -> Result<ArtifactVerificationView, CommandError> {
    let library_id = parse_library_id(library_id)?;
    let verification =
        block_on(client.verify_library_artifact(&library_id)).map_err(CommandError::from)?;
    Ok(ArtifactVerificationView::from(&verification))
}

/// Inspects a library's consumer dependency version alignment.
///
/// This never rewrites a POM or changes a version: it reports each consumer as
/// aligned, mismatched, unknown, or unresolved (`tasks/020.md` §23, §24).
///
/// # Errors
///
/// Same as [`verify_library_artifact`].
pub fn inspect_version_alignment(
    client: &dyn OrchestratorClient,
    library_id: &str,
) -> Result<VersionAlignmentView, CommandError> {
    let library_id = parse_library_id(library_id)?;
    let plan = block_on(client.inspect_library_version_alignment(&library_id))
        .map_err(CommandError::from)?;
    Ok(VersionAlignmentView::from(&plan))
}

/// Builds one library.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected library id, otherwise the daemon's
/// structured error.
pub fn build_library(
    client: &dyn OrchestratorClient,
    library_id: &str,
) -> Result<OperationOutcome, CommandError> {
    let library_id = parse_library_id(library_id)?;
    let result = client
        .build_library(&library_id)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Installs one library's artifact into the local repository.
///
/// # Errors
///
/// Same as [`build_library`].
pub fn install_library(
    client: &dyn OrchestratorClient,
    library_id: &str,
) -> Result<OperationOutcome, CommandError> {
    let library_id = parse_library_id(library_id)?;
    let result = client
        .install_library(&library_id)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Synchronizes one library's repository.
///
/// # Errors
///
/// Same as [`build_library`].
pub fn sync_library(
    client: &dyn OrchestratorClient,
    library_id: &str,
) -> Result<OperationOutcome, CommandError> {
    let library_id = parse_library_id(library_id)?;
    let result = client
        .sync_library(&library_id)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Runs the prepare-library workflow for one library.
///
/// Synchronization is optional so a developer can build the current local
/// branch with uncommitted changes; when `branch` is given it is checked out
/// after synchronizing (`tasks/020.md` §25, §26).
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected library id or branch, otherwise the daemon's
/// structured error.
pub fn prepare_library(
    client: &dyn OrchestratorClient,
    library_id: &str,
    sync: bool,
    branch: Option<&str>,
    build: bool,
    install: bool,
    verify: bool,
) -> Result<OperationOutcome, CommandError> {
    let library_id = parse_library_id(library_id)?;
    let options = prepare_options(sync, branch, build, install, verify)?;
    let result = client
        .prepare_library(&library_id, &options)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Prepares several libraries as one workflow, in dependency order.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected workspace, library, or branch identifier (or
/// an empty selection), otherwise the daemon's structured error.
#[allow(clippy::too_many_arguments)]
pub fn prepare_libraries(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
    library_ids: &[String],
    sync: bool,
    branch: Option<&str>,
    build: bool,
    install: bool,
    verify: bool,
) -> Result<OperationOutcome, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let library_ids = parse_library_ids(library_ids)?;
    let options = prepare_options(sync, branch, build, install, verify)?;
    let request = PrepareSelectedLibrariesRequest::new(workspace_id)
        .with_libraries(library_ids)
        .with_options(options);
    let result = client
        .prepare_selected_libraries(request)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Builds validated prepare options from the GUI's toggles.
fn prepare_options(
    sync: bool,
    branch: Option<&str>,
    build: bool,
    install: bool,
    verify: bool,
) -> Result<LibraryPrepareOptions, CommandError> {
    let mut options = LibraryPrepareOptions::new()
        .with_sync(sync)
        .with_build(build)
        .with_install(install)
        .with_verify(verify);
    if let Some(branch) = branch.filter(|branch| !branch.trim().is_empty()) {
        options = options.with_branch(parse_branch(branch)?);
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_options_reflect_the_toggles_and_optional_branch() {
        let options = prepare_options(false, None, true, true, true).expect("valid");
        assert!(!options.sync);
        assert!(options.branch.is_none());
        assert!(options.build);

        let with_branch = prepare_options(true, Some("main"), true, false, false).expect("valid");
        assert!(with_branch.sync);
        assert_eq!(
            with_branch.branch.as_ref().map(ToString::to_string),
            Some("main".to_owned())
        );
        assert!(!with_branch.install);
    }

    #[test]
    fn an_empty_branch_is_treated_as_absent() {
        let options = prepare_options(true, Some("  "), true, true, true).expect("valid");
        assert!(options.branch.is_none());
    }

    #[test]
    fn invalid_inputs_are_rejected_before_any_ipc_call() {
        assert_eq!(
            parse_library_id("").expect_err("empty").code,
            "INVALID_INPUT"
        );
        assert_eq!(
            parse_workspace_id("bad id").expect_err("space").code,
            "INVALID_INPUT"
        );
        assert_eq!(
            parse_branch("bad branch").expect_err("space").code,
            "INVALID_INPUT"
        );
    }
}
