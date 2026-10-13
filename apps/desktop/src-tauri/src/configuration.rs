//! Configuration-management use cases behind the desktop commands.
//!
//! These are thin adapters over the shared [`OrchestratorClient`] — the same
//! interface the TUI and MCP adapters use. The daemon is the **only**
//! configuration writer: the adapter reads a snapshot, previews a change, and
//! asks the daemon to validate and persist it. The GUI parses no TOML, writes
//! no file, and never edits configuration on its own
//! (`tasks/023.md` §37, §38, §41).
//!
//! # Authorization
//!
//! A removal is a destructive operation. The frontend's confirmation only
//! *expresses user intent*: it is forwarded as the request's
//! `allow_destructive` flag, and the daemon honors it only for an interactive
//! client connected as the same user. The policy engine still decides, and a
//! denial surfaces as an actionable `PermissionDenied` error — a confirmation
//! boolean is never trusted authorization (`tasks/023.md` §29, §30, §46).

use service_orchestrator_application::{
    Authorization, ConfigurationCommit, ConfigurationReload, ConfigurationValidation,
    ProjectConfigurationSuggestion, RepositoryInspection,
};
use service_orchestrator_client::OrchestratorClient;
use service_orchestrator_config::{ConfigChange, ConfigurationPreview, ConfigurationSnapshot};

use crate::error::CommandError;

/// Reads the active configuration as a raw, editable snapshot.
///
/// # Errors
///
/// Returns the daemon's structured error when the file cannot be read, parsed,
/// or reached.
pub fn snapshot(client: &dyn OrchestratorClient) -> Result<ConfigurationSnapshot, CommandError> {
    client.configuration_snapshot().map_err(CommandError::from)
}

/// Validates the active configuration file without changing it.
///
/// # Errors
///
/// Returns the daemon's structured error when the file cannot be read or
/// reached.
pub fn validate(client: &dyn OrchestratorClient) -> Result<ConfigurationValidation, CommandError> {
    client.validate_configuration().map_err(CommandError::from)
}

/// Previews a proposed change without committing it.
///
/// The preview carries the structured diff, the validation issues, the
/// affected references, and the running-service impact — all computed by the
/// daemon's configuration layer and application core, never by the GUI
/// (`tasks/023.md` §32, §34).
///
/// # Errors
///
/// Returns `CONFIG_CONFLICT` when `expected_revision` is stale, a structural
/// error when the change cannot be applied, or the daemon's structured error.
pub fn preview(
    client: &dyn OrchestratorClient,
    change: &ConfigChange,
    expected_revision: &str,
) -> Result<ConfigurationPreview, CommandError> {
    client
        .preview_configuration(change, expected_revision)
        .map_err(CommandError::from)
}

/// Validates and persists a change, attaching destructive intent when
/// confirmed.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected change, `PermissionDenied` when a
/// removal was not authorized, `CONFIGURATION_INVALID` when the proposal does
/// not validate, `CONFIG_CONFLICT` when the revision is stale, or the daemon's
/// structured error.
pub fn commit(
    client: &dyn OrchestratorClient,
    change: &ConfigChange,
    expected_revision: &str,
    confirmed: bool,
) -> Result<ConfigurationCommit, CommandError> {
    if confirmed {
        let authorized = client.with_authorization(Authorization::allow_destructive());
        authorized
            .commit_configuration(change, expected_revision)
            .map_err(CommandError::from)
    } else {
        // No confirmation: standard authorization still allows a create or an
        // update, while a removal is refused by the daemon.
        client
            .commit_configuration(change, expected_revision)
            .map_err(CommandError::from)
    }
}

/// Re-reads and validates the configuration file from disk.
///
/// # Errors
///
/// Returns `CONFIGURATION_INVALID` when the on-disk file is invalid, or the
/// daemon's structured error when it cannot be read or reached.
pub fn reload(client: &dyn OrchestratorClient) -> Result<ConfigurationReload, CommandError> {
    client.reload_configuration().map_err(CommandError::from)
}

/// Inspects a local repository directory, read-only.
///
/// # Errors
///
/// Returns `INVALID_PATH` for a rejected path, or the daemon's structured error
/// when the directory cannot be inspected.
pub fn inspect(
    client: &dyn OrchestratorClient,
    path: &str,
) -> Result<RepositoryInspection, CommandError> {
    client.inspect_repository(path).map_err(CommandError::from)
}

/// Detects a suggested service configuration from a repository.
///
/// # Errors
///
/// Same as [`inspect`].
pub fn detect(
    client: &dyn OrchestratorClient,
    path: &str,
) -> Result<ProjectConfigurationSuggestion, CommandError> {
    client
        .detect_project_configuration(path)
        .map_err(CommandError::from)
}
