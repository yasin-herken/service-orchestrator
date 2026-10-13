//! The application error model.
//!
//! [`ApplicationError`] is the single error type returned by the application
//! use cases. It is deliberately independent of any interface: the TUI and MCP
//! adapters map it into their own response shapes but neither can observe a
//! lower-level detail it does not understand.
//!
//! The variants fall into three groups:
//!
//! - **not found** — a referenced workspace, service, library, profile, group,
//!   task, or workflow does not exist;
//! - **precondition / validation** — the operation is not valid for the current
//!   configuration or runtime state;
//! - **boundary** — a port (task engine, log service) failed, with its context
//!   preserved but no low-level leak.

use serde::{Deserialize, Serialize};
use service_orchestrator_domain::Permission;
use service_orchestrator_domain::{
    GroupId, LibraryId, ProfileId, ServiceId, TaskId, WorkflowId, WorkspaceId,
};
use thiserror::Error;

/// Errors produced by application use cases.
///
/// The type derives `serde` because a client-side [`Application`] may be a thin
/// proxy over a local daemon: a structured error crosses the IPC boundary as-is
/// rather than being flattened into a string
/// ([`Application`]: crate::Application).
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
pub enum ApplicationError {
    /// The requested workspace does not exist.
    #[error("workspace '{0}' was not found")]
    WorkspaceNotFound(WorkspaceId),

    /// The requested service does not exist.
    #[error("service '{0}' was not found")]
    ServiceNotFound(ServiceId),

    /// The requested library does not exist.
    #[error("library '{0}' was not found")]
    LibraryNotFound(LibraryId),

    /// The requested profile does not exist.
    #[error("profile '{0}' was not found")]
    ProfileNotFound(ProfileId),

    /// The requested service group does not exist.
    #[error("service group '{0}' was not found")]
    GroupNotFound(GroupId),

    /// The requested task does not exist.
    #[error("task '{0}' was not found")]
    TaskNotFound(TaskId),

    /// The requested workflow does not exist.
    #[error("workflow '{0}' was not found")]
    WorkflowNotFound(WorkflowId),

    /// A precondition for the operation was not met.
    #[error("invalid operation: {message}")]
    InvalidOperation {
        /// Why the operation cannot proceed.
        message: String,
    },

    /// A referenced dependency could not be resolved.
    #[error("'{owner}' depends on '{target}', which was not found")]
    DependencyNotFound {
        /// The entity that declared the dependency.
        owner: String,
        /// The dependency target that could not be resolved.
        target: String,
    },

    /// The configuration is not valid for the requested operation.
    #[error("configuration is invalid: {message}")]
    ConfigurationInvalid {
        /// What is wrong with the configuration.
        message: String,
    },

    /// A configuration transaction failed with a structured error.
    ///
    /// Produced by the configuration-management use cases. The `code` is stable
    /// and machine-readable — for example `CONFIG_CONFLICT` when an edit lost a
    /// race with another writer, `CONFIG_ENTITY_NOT_FOUND`, or
    /// `CONFIG_WRITE_FAILED` — so an interface can react without scraping the
    /// message (`tasks/023.md` §28, §47).
    #[error("configuration error ({code}): {message}")]
    Configuration {
        /// A stable, machine-readable code.
        code: String,
        /// A human-readable, actionable summary.
        message: String,
    },

    /// A repository inspection failed with a structured error.
    #[error("repository inspection error ({code}): {message}")]
    RepositoryInspection {
        /// A stable, machine-readable code such as `INSPECTION_UNAVAILABLE`.
        code: String,
        /// A human-readable summary.
        message: String,
    },

    /// A caller-supplied path was rejected as unsafe or invalid.
    #[error("invalid path: {message}")]
    InvalidPath {
        /// A human-readable explanation.
        message: String,
    },

    /// A constructed workflow is not well formed.
    #[error("workflow is invalid: {message}")]
    WorkflowInvalid {
        /// What is wrong with the workflow.
        message: String,
    },

    /// The operation requires explicit confirmation and was denied.
    #[error("permission denied: {message}")]
    PermissionDenied {
        /// The permission that was required.
        permission: Permission,
        /// A human-readable explanation.
        message: String,
    },

    /// The task engine refused or failed the operation.
    #[error("task engine error: {message}")]
    TaskManager {
        /// The underlying engine message.
        message: String,
    },

    /// The log service failed.
    #[error("log service error: {message}")]
    LogService {
        /// The underlying log service message.
        message: String,
    },

    /// A Git operation failed with a structured error.
    #[error("git error ({code}): {message}")]
    Git {
        /// A stable, machine-readable code such as `GIT_WORKING_TREE_DIRTY`.
        code: String,
        /// A human-readable summary.
        message: String,
    },

    /// A build operation failed with a structured error.
    #[error("build error ({code}): {message}")]
    Build {
        /// A stable, machine-readable code such as `BUILD_PATH_OUTSIDE_WORKSPACE`.
        code: String,
        /// A human-readable summary.
        message: String,
    },

    /// A library's configuration is not valid for the requested operation.
    #[error("library '{library}' is not configured for this operation: {message}")]
    LibraryConfigurationInvalid {
        /// The library identifier.
        library: String,
        /// What is wrong with the configuration.
        message: String,
    },

    /// The library's build provider does not support the requested capability.
    #[error("the build provider for library '{library}' does not support '{operation}'")]
    BuildProviderUnsupported {
        /// The library identifier.
        library: String,
        /// The unsupported capability or operation.
        operation: String,
    },

    /// The expected artifact is not present in the local repository.
    #[error("artifact '{coordinate}' for library '{library}' was not found")]
    ArtifactNotFound {
        /// The library identifier.
        library: String,
        /// The artifact coordinate that was not found.
        coordinate: String,
    },

    /// A consumer references a different artifact version than the library
    /// provides.
    #[error(
        "artifact version mismatch for library '{library}': expected '{expected}', found '{found}'"
    )]
    ArtifactVersionMismatch {
        /// The library identifier.
        library: String,
        /// The target version.
        expected: String,
        /// The version a consumer references.
        found: String,
    },

    /// A dependency relationship could not be resolved.
    #[error("could not resolve dependencies for library '{library}': {message}")]
    DependencyResolution {
        /// The library identifier.
        library: String,
        /// A human-readable explanation.
        message: String,
    },

    /// The selected libraries form a dependency cycle.
    #[error("library dependency cycle detected: {libraries}")]
    DependencyCycle {
        /// The libraries that form the cycle.
        libraries: String,
    },

    /// The selected services form a dependency cycle.
    #[error("service dependency cycle detected: {services}")]
    ServiceDependencyCycle {
        /// The services that form the cycle.
        services: String,
    },

    /// The effective dependency version could not be inspected.
    #[error("dependency version inspection is unsupported for library '{library}': {message}")]
    VersionInspectionUnsupported {
        /// The library identifier.
        library: String,
        /// A human-readable explanation.
        message: String,
    },

    /// An artifact inspection failed with a structured error.
    #[error("artifact inspection error ({code}): {message}")]
    ArtifactInspection {
        /// A stable, machine-readable code.
        code: String,
        /// A human-readable summary.
        message: String,
    },

    /// A service runtime operation failed with a structured error.
    #[error("service runtime error ({code}): {message}")]
    ServiceRuntime {
        /// A stable, machine-readable code such as `RUNTIME_PORT_ALREADY_IN_USE`.
        code: String,
        /// A human-readable summary.
        message: String,
    },

    /// A database migration operation failed with a structured error.
    #[error("migration error ({code}): {message}")]
    Migration {
        /// A stable, machine-readable code such as `MIGRATION_DATABASE_TARGET_NOT_AUTHORIZED`.
        code: String,
        /// A human-readable summary.
        message: String,
    },

    /// A health operation failed with a structured error.
    #[error("health error ({code}): {message}")]
    Health {
        /// A stable, machine-readable code such as `HEALTH_CHECK_TIMEOUT`.
        code: String,
        /// A human-readable summary.
        message: String,
    },

    /// Diagnostics could not be produced for a service.
    #[error("diagnostics are unavailable for service '{service}': {message}")]
    DiagnosticsUnavailable {
        /// The service identifier.
        service: String,
        /// A human-readable explanation.
        message: String,
    },

    /// A log query was rejected as invalid.
    #[error("invalid log query: {message}")]
    LogQueryInvalid {
        /// What is wrong with the query.
        message: String,
    },

    /// The shared application runtime is unavailable.
    ///
    /// A client-side proxy produces this when it cannot reach the local daemon,
    /// when the connection drops, or when the two speak incompatible protocol
    /// versions. It never originates inside the application core itself.
    #[error("{message}")]
    RuntimeUnavailable {
        /// A stable, machine-readable code such as `DAEMON_UNAVAILABLE`.
        code: String,
        /// A human-readable, actionable summary.
        message: String,
    },
}

impl ApplicationError {
    /// Builds an [`ApplicationError::InvalidOperation`].
    #[must_use]
    pub(crate) fn invalid_operation(message: impl Into<String>) -> Self {
        Self::InvalidOperation {
            message: message.into(),
        }
    }

    /// Maps a configuration error into a structured application error.
    ///
    /// The stable `code` lets an interface distinguish, for example, a stale
    /// revision (`CONFIG_CONFLICT`) from a write failure without parsing the
    /// message (`tasks/023.md` §28, §47).
    #[must_use]
    pub(crate) fn config(error: service_orchestrator_config::ConfigError) -> Self {
        use service_orchestrator_config::ConfigError;
        // A validation failure keeps its dedicated variant so the UI can render
        // it as a configuration problem rather than a generic transaction error.
        if let ConfigError::Validation(errors) = &error {
            return Self::ConfigurationInvalid {
                message: errors.to_string(),
            };
        }
        let code = match &error {
            ConfigError::Read { .. } => "CONFIG_READ_FAILED",
            ConfigError::Parse { .. } => "CONFIG_PARSE_FAILED",
            ConfigError::UnsupportedVersion { .. } => "CONFIG_VERSION_UNSUPPORTED",
            ConfigError::Conflict { .. } => "CONFIG_CONFLICT",
            ConfigError::EntityNotFound { .. } => "CONFIG_ENTITY_NOT_FOUND",
            ConfigError::EntityExists { .. } => "CONFIG_ENTITY_EXISTS",
            ConfigError::InvalidChange { .. } => "CONFIG_INVALID_CHANGE",
            ConfigError::Document { .. } => "CONFIG_DOCUMENT_INVALID",
            ConfigError::Write { .. } => "CONFIG_WRITE_FAILED",
            // `Validation` was handled above.
            ConfigError::Validation(_) => "CONFIG_INVALID",
        };
        Self::Configuration {
            code: code.to_owned(),
            message: error.to_string(),
        }
    }

    /// Builds an [`ApplicationError::RepositoryInspection`].
    #[must_use]
    pub(crate) fn inspection(error: crate::ports::RepositoryInspectionError) -> Self {
        Self::RepositoryInspection {
            code: error.code,
            message: error.message,
        }
    }

    /// Builds an [`ApplicationError::InvalidPath`].
    #[must_use]
    pub(crate) fn invalid_path(message: impl Into<String>) -> Self {
        Self::InvalidPath {
            message: message.into(),
        }
    }
}
