//! Repository discovery, bulk import, and saved-preset use cases behind the
//! desktop commands (`tasks/032.md` §7–§15, §26, §36–§38).
//!
//! These are thin adapters over the shared [`OrchestratorClient`] — the same
//! interface the TUI and MCP adapters use. Discovery, template matching, port
//! assignment, configuration composition, and validation all run in the
//! daemon's application core; the adapter validates identifiers, maps the
//! frontend's input onto the core's request types, and forwards the result
//! unchanged, so the GUI never re-implements a planning rule
//! (`tasks/032.md` §10, §14, §39).
//!
//! # Authorization
//!
//! A bulk import that only creates entities is a safe write. One that replaces
//! an existing entity is **destructive** — it overwrites hand-written
//! configuration — so the frontend confirmation is forwarded as explicit
//! destructive intent and the daemon honors it only for an interactive client
//! connected as the same user. The policy engine still decides
//! (`tasks/032.md` §13, §19, §50).
//!
//! # Loading versus executing a preset
//!
//! Listing or loading a saved operation preset performs no work and carries no
//! authorization: it composes the request the Bulk Operations screen would show,
//! and execution then runs the ordinary plan/validate/authorize path
//! (`tasks/032.md` §37).

use serde::Deserialize;
use service_orchestrator_application::{
    Authorization, BulkImportCommit, BulkImportOverrides, BulkImportPlan, BulkImportRequest,
    BulkImportSelection, DiscoveryScanRequest, ExistingEntityPolicy, MigrationPresetSummary,
    OperationPresetSummary, PrepareEnvironmentRequest, RepositoryDiscoveryReport,
};
use service_orchestrator_client::OrchestratorClient;
use service_orchestrator_domain::{MigrationPresetId, OperationPresetId, Port};

use crate::error::CommandError;
use crate::validation::{
    parse_migration_preset_id, parse_operation_preset_id, parse_template_id, parse_workspace_id,
};

/// The validated, wire-level request for a bounded discovery scan.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct DiscoveryScanInput {
    /// The workspace whose configured root is scanned.
    pub workspace_id: String,
    /// An optional depth override; the daemon clamps it to the hard cap.
    pub max_depth: Option<usize>,
    /// Additional directory names never entered, on top of the built-ins and
    /// the configured exclusions.
    pub exclude: Vec<String>,
    /// Whether symlinked directories are followed at all.
    pub follow_symlinks: Option<bool>,
    /// An optional repository-count override; the daemon clamps it.
    pub max_repositories: Option<usize>,
}

/// The GUI-shaped bulk import request.
///
/// Identifiers arrive as strings so the adapter can validate them through the
/// domain before anything crosses the IPC boundary; the overrides are the
/// core's own type because their values are plain strings and enums that the
/// planner validates field by field.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct BulkImportInput {
    /// The workspace the repositories are imported into.
    pub workspace_id: String,
    /// The selected repositories.
    pub repositories: Vec<BulkImportSelectionInput>,
    /// Shared overrides applied to every selected repository.
    pub bulk: BulkImportOverrides,
    /// The first port assigned to rows that need one.
    pub port_base: Option<u16>,
}

/// One selected repository, with its template id as a string.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct BulkImportSelectionInput {
    /// The repository path relative to the workspace root.
    pub relative_path: String,
    /// The explicitly assigned template id.
    pub template: Option<String>,
    /// How an existing configured entity at the same path is handled.
    pub existing: ExistingEntityPolicy,
    /// Per-service overrides.
    pub overrides: BulkImportOverrides,
}

/// Maps validated GUI input onto the core's discovery request.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected workspace identifier.
pub fn build_discovery_scan(
    input: &DiscoveryScanInput,
) -> Result<DiscoveryScanRequest, CommandError> {
    let workspace_id = parse_workspace_id(&input.workspace_id)?;
    Ok(DiscoveryScanRequest {
        workspace_id,
        max_depth: input.max_depth,
        exclude: input
            .exclude
            .iter()
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .collect(),
        follow_symlinks: input.follow_symlinks,
        max_repositories: input.max_repositories,
    })
}

/// Maps validated GUI input onto the core's bulk import request.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected workspace or template identifier, or
/// for a zero port base.
pub fn build_import(input: &BulkImportInput) -> Result<BulkImportRequest, CommandError> {
    let workspace_id = parse_workspace_id(&input.workspace_id)?;
    let repositories = input
        .repositories
        .iter()
        .map(|selection| {
            let template = selection
                .template
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(parse_template_id)
                .transpose()?;
            Ok::<BulkImportSelection, CommandError>(BulkImportSelection {
                relative_path: selection.relative_path.trim().to_owned(),
                template,
                existing: selection.existing,
                overrides: selection.overrides.clone(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let port_base = input
        .port_base
        .map(|port| {
            Port::new(port)
                .map_err(|error| CommandError::invalid_input(format!("invalid port base: {error}")))
        })
        .transpose()?;
    Ok(BulkImportRequest {
        workspace_id,
        repositories,
        bulk: input.bulk.clone(),
        port_base,
    })
}

/// Scans a workspace root for repositories, read-only and bounded.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for rejected input, or the daemon's structured error
/// (for example `DISCOVERY_ROOT_MISSING`) when the scan cannot be attempted.
pub fn discover(
    client: &dyn OrchestratorClient,
    input: &DiscoveryScanInput,
) -> Result<RepositoryDiscoveryReport, CommandError> {
    let request = build_discovery_scan(input)?;
    client
        .discover_repositories(request)
        .map_err(CommandError::from)
}

/// Plans a bulk import without changing anything.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for rejected input, `CONFIG_CONFLICT` when the
/// configuration changed while planning, or the daemon's structured error.
pub fn plan(
    client: &dyn OrchestratorClient,
    input: &BulkImportInput,
) -> Result<BulkImportPlan, CommandError> {
    let request = build_import(input)?;
    client.plan_bulk_import(request).map_err(CommandError::from)
}

/// Validates and commits a bulk import atomically.
///
/// `expected_revision` is the revision of the plan the user reviewed; a stale
/// revision is reported instead of importing onto a different configuration.
/// `confirmed` forwards explicit destructive intent for a selection that
/// replaces an existing entity.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for rejected input, `CONFIGURATION_INVALID` when the
/// plan does not validate, `CONFIG_CONFLICT` for a stale revision,
/// `PermissionDenied` when an update was not authorized, or the daemon's
/// structured error.
pub fn commit(
    client: &dyn OrchestratorClient,
    input: &BulkImportInput,
    expected_revision: &str,
    confirmed: bool,
) -> Result<BulkImportCommit, CommandError> {
    let request = build_import(input)?;
    let result = if confirmed {
        let authorized = client.with_authorization(Authorization::allow_destructive());
        authorized.commit_bulk_import(request, expected_revision)
    } else {
        // No confirmation: creating entities is still allowed, while an
        // update is refused by the daemon's policy engine.
        client.commit_bulk_import(request, expected_revision)
    };
    result.map_err(CommandError::from)
}

/// Lists the configured migration presets and their safety classification.
///
/// # Errors
///
/// Returns the daemon's structured error when it cannot be reached.
pub fn migration_presets(
    client: &dyn OrchestratorClient,
) -> Result<Vec<MigrationPresetSummary>, CommandError> {
    client.list_migration_presets().map_err(CommandError::from)
}

/// Lists the saved bulk-operation presets that apply to a workspace.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected workspace id, or the daemon's
/// structured error (including an unknown workspace).
pub fn operation_presets(
    client: &dyn OrchestratorClient,
    workspace_id: Option<&str>,
) -> Result<Vec<OperationPresetSummary>, CommandError> {
    let workspace_id = workspace_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(parse_workspace_id)
        .transpose()?;
    client
        .list_operation_presets(workspace_id.as_ref())
        .map_err(CommandError::from)
}

/// Composes the environment request a saved operation preset describes.
///
/// Loading never executes anything and never carries an authorization
/// (`tasks/032.md` §37).
///
/// # Errors
///
/// Returns `INVALID_INPUT` for rejected identifiers, or the daemon's structured
/// error (including an unknown preset).
pub fn operation_preset_request(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
    preset_id: &str,
) -> Result<PrepareEnvironmentRequest, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let preset_id = parse_operation_preset_id(preset_id)?;
    client
        .operation_preset_request(&workspace_id, &preset_id)
        .map_err(CommandError::from)
}

/// Validates a migration-preset identifier for the GUI's own selection state.
///
/// # Errors
///
/// Returns `INVALID_INPUT` when the domain rejects the identifier.
pub fn parse_preset_reference(raw: &str) -> Result<MigrationPresetId, CommandError> {
    parse_migration_preset_id(raw)
}

/// Validates an operation-preset identifier for the GUI's own selection state.
///
/// # Errors
///
/// Returns `INVALID_INPUT` when the domain rejects the identifier.
pub fn parse_operation_reference(raw: &str) -> Result<OperationPresetId, CommandError> {
    parse_operation_preset_id(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn import_input() -> BulkImportInput {
        BulkImportInput {
            workspace_id: "demo".to_owned(),
            repositories: vec![BulkImportSelectionInput {
                relative_path: "services/catalog-service".to_owned(),
                template: Some("spring-boot-backend".to_owned()),
                existing: ExistingEntityPolicy::Skip,
                overrides: BulkImportOverrides::default(),
            }],
            bulk: BulkImportOverrides::default(),
            port_base: Some(8081),
        }
    }

    #[test]
    fn a_valid_import_maps_onto_the_core_request() {
        let request = build_import(&import_input()).expect("the input is valid");
        assert_eq!(request.workspace_id.as_str(), "demo");
        assert_eq!(request.repositories.len(), 1);
        assert_eq!(
            request.repositories[0]
                .template
                .as_ref()
                .map(service_orchestrator_domain::TemplateId::as_str),
            Some("spring-boot-backend")
        );
        assert_eq!(request.port_base.map(Port::value), Some(8081));
    }

    #[test]
    fn blank_optional_values_are_treated_as_unset() {
        let mut input = import_input();
        input.port_base = None;
        input.repositories[0].template = Some("  ".to_owned());
        let request = build_import(&input).expect("the input is valid");
        assert!(request.port_base.is_none());
        assert!(request.repositories[0].template.is_none());
    }

    #[test]
    fn invalid_identifiers_are_rejected_before_the_ipc_boundary() {
        let mut input = import_input();
        input.workspace_id = "two words".to_owned();
        assert_eq!(
            build_import(&input).expect_err("bad workspace").code,
            "INVALID_INPUT"
        );

        let mut input = import_input();
        input.repositories[0].template = Some("bad template".to_owned());
        assert_eq!(
            build_import(&input).expect_err("bad template").code,
            "INVALID_INPUT"
        );

        let mut input = import_input();
        input.port_base = Some(0);
        assert_eq!(
            build_import(&input).expect_err("bad port").code,
            "INVALID_INPUT"
        );
    }

    #[test]
    fn discovery_input_is_validated_and_trimmed() {
        let input = DiscoveryScanInput {
            workspace_id: "demo".to_owned(),
            max_depth: Some(4),
            exclude: vec![" vendor ".to_owned(), String::new()],
            follow_symlinks: Some(false),
            max_repositories: Some(10),
        };
        let request = build_discovery_scan(&input).expect("the input is valid");
        assert_eq!(request.max_depth, Some(4));
        assert_eq!(request.exclude, vec!["vendor".to_owned()]);
        assert_eq!(request.follow_symlinks, Some(false));

        let invalid = DiscoveryScanInput {
            workspace_id: String::new(),
            ..DiscoveryScanInput::default()
        };
        assert_eq!(
            build_discovery_scan(&invalid)
                .expect_err("empty workspace")
                .code,
            "INVALID_INPUT"
        );
    }

    #[test]
    fn blank_preset_references_are_reported_as_input_errors() {
        assert_eq!(
            parse_preset_reference("").expect_err("empty").code,
            "INVALID_INPUT"
        );
        assert_eq!(
            parse_operation_reference("bad id")
                .expect_err("spaces")
                .code,
            "INVALID_INPUT"
        );
        assert_eq!(
            parse_operation_reference("backend-local")
                .expect("valid")
                .as_str(),
            "backend-local"
        );
    }
}
