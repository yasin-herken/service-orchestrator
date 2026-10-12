//! Workspace validation models.
//!
//! A validation report is a *read-only* description of what a configured
//! workspace looks like on this machine right now: which repositories exist,
//! which dependencies resolve, which runtimes and build tools are available,
//! and how configured ports relate to actual occupancy (`tasks/017.md` §7, §8).
//!
//! Validation never checks out branches, pulls, builds, installs, migrates,
//! starts, stops, or rewrites configuration.

use serde::{Deserialize, Serialize};
use service_orchestrator_domain::{LibraryId, Port, ServiceId, WorkspaceId};

/// The outcome of validating one workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceValidationReport {
    /// The validated workspace.
    pub workspace_id: WorkspaceId,
    /// The workspace display name.
    pub name: String,
    /// The workspace root, as configured.
    pub root: String,
    /// Aggregate counts.
    pub totals: ValidationTotals,
    /// Per-repository checks, in stable order.
    pub repositories: Vec<RepositoryValidation>,
    /// Structured findings, in stable order.
    pub diagnostics: Vec<ValidationDiagnostic>,
    /// Ports declared by more than one service (configuration conflicts).
    pub port_conflicts: Vec<ConfiguredPortConflict>,
    /// Declared ports that currently accept connections (runtime occupancy).
    pub occupied_ports: Vec<OccupiedPort>,
}

impl WorkspaceValidationReport {
    /// Returns `true` when at least one diagnostic is an error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == ValidationSeverity::Error)
    }
}

/// Aggregate counts for a validation report.
///
/// These are the headline numbers a report must contain
/// (`tasks/017.md` §8). The per-finding detail lives in
/// [`WorkspaceValidationReport::diagnostics`]; the aggregate counts here let
/// an interface summarise a workspace without replaying every diagnostic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationTotals {
    /// Total configured services.
    pub services: usize,
    /// Services of the backend kind.
    pub backend_services: usize,
    /// Services of the frontend kind.
    pub frontend_services: usize,
    /// Total configured libraries.
    pub libraries: usize,
    /// Repositories whose checkout exists with Git metadata.
    pub repositories_valid: usize,
    /// Repositories whose checkout does not exist.
    pub repositories_missing: usize,
    /// Repositories that exist but are unusable (not a directory, or no Git
    /// metadata).
    pub repositories_invalid: usize,
    /// Configurations that cannot work as written (error diagnostics).
    ///
    /// A stopped service is never counted here.
    pub invalid_configurations: usize,
    /// Diagnostics about a runtime or build tool that is not installed.
    pub missing_runtimes: usize,
    /// Diagnostics about unresolvable or circular dependencies.
    pub dependency_errors: usize,
    /// Diagnostics about port configuration conflicts (not occupancy).
    pub port_conflicts: usize,
    /// Total error diagnostics.
    pub errors: usize,
    /// Total warning diagnostics.
    pub warnings: usize,
}

/// The entity a repository check belongs to.
///
/// Adjacently tagged (`{"kind":"service","id":"auth-service"}`) because an
/// internally tagged newtype variant cannot hold a bare string, and these
/// reports cross the IPC boundary as JSON.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum RepositoryOwner {
    /// A service's repository.
    Service(ServiceId),
    /// A library's repository.
    Library(LibraryId),
}

/// The result of checking one repository checkout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryCheckStatus {
    /// The checkout exists and contains Git metadata.
    Valid,
    /// The configured checkout path does not exist.
    Missing,
    /// The checkout path exists but is not a directory.
    NotADirectory,
    /// The directory exists but contains no `.git` metadata.
    MissingGitMetadata,
}

/// A per-repository validation result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryValidation {
    /// The entity the repository belongs to.
    pub owner: RepositoryOwner,
    /// The absolute checkout path, as resolved from configuration.
    pub path: String,
    /// The observed status.
    pub status: RepositoryCheckStatus,
}

/// How severe a diagnostic is.
///
/// An `Error` means a configured operation cannot work as written (a missing
/// repository, an unresolvable dependency, a configuration port conflict). A
/// `Warning` flags something that degrades or blocks *some* operations but is
/// not by itself a configuration contradiction (a missing local runtime, an
/// occupied port). A service that is merely stopped is never an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationSeverity {
    /// A configured operation cannot work as written.
    Error,
    /// Something degrades or blocks some operations.
    Warning,
}

/// The category of a validation diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationCode {
    /// The workspace root does not exist on this machine.
    MissingWorkspaceRoot,
    /// A service's working directory does not exist.
    MissingServicePath,
    /// A library's working directory does not exist.
    MissingLibraryPath,
    /// A configured path exists but is not a directory.
    NotADirectory,
    /// A repository checkout does not exist.
    MissingRepository,
    /// A repository checkout exists without Git metadata.
    MissingGitMetadata,
    /// A service depends on a service that is not configured.
    UnknownServiceDependency,
    /// A service or library depends on a library that is not configured.
    UnknownLibraryDependency,
    /// The dependency graph contains a cycle.
    DependencyCycle,
    /// Two services declare the same port.
    DuplicateConfiguredPort,
    /// A declared port currently accepts connections.
    PortCurrentlyOccupied,
    /// A runtime required by a service or library is not installed.
    MissingRuntime,
    /// No usable build tool was found for a service or library.
    MissingBuildTool,
    /// A start command's working directory does not exist.
    MissingStartDirectory,
    /// A health check references a port the service does not declare.
    UndeclaredHealthPort,
}

/// What a diagnostic is about.
///
/// Adjacently tagged so the payload survives JSON IPC: an internally tagged
/// newtype variant cannot hold a bare identifier string.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum ValidationScope {
    /// The workspace as a whole.
    Workspace,
    /// One service.
    Service(ServiceId),
    /// One library.
    Library(LibraryId),
}

/// One structured validation finding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationDiagnostic {
    /// How severe the finding is.
    pub severity: ValidationSeverity,
    /// The category.
    pub code: ValidationCode,
    /// What the finding is about.
    pub scope: ValidationScope,
    /// A human-readable, actionable explanation.
    pub message: String,
}

/// A port declared by more than one service — a configuration conflict,
/// independent of what is currently running.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfiguredPortConflict {
    /// The conflicting port.
    pub port: Port,
    /// The services that declare it, in stable order.
    pub services: Vec<ServiceId>,
}

/// A declared port that currently accepts TCP connections on loopback.
///
/// This is *actual runtime occupancy* and deliberately distinct from a
/// [`ConfiguredPortConflict`]: an occupied port may simply be an unrelated
/// process, or one of the declaring services already running.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OccupiedPort {
    /// The occupied port.
    pub port: Port,
    /// The services that declare it, in stable order.
    pub services: Vec<ServiceId>,
}
