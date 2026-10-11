//! Database-migration boundary.
//!
//! The application does not run Liquibase, Flyway, or any other migration tool
//! itself. It describes *which entity* is being migrated (a workspace-aware
//! [`ResolvedMigrationTarget`]) and *which provider-neutral operation*
//! ([`MigrationOperation`]) is requested; a migration manager — implemented by
//! the `liquibase` infrastructure adapter over the process manager — resolves
//! the provider, builds a structured plan, applies the target's safety policy,
//! and executes it.
//!
//! # Why a port value, not a trait
//!
//! Every migration operation is a task: the application validates
//! preconditions, resolves a [`ResolvedMigrationTarget`], and submits a task
//! carrying its parameters to the engine. There is no read-only migration query
//! (status inspection is itself a process execution), so — exactly like
//! [`ResolvedBuildTarget`](crate::ports::ResolvedBuildTarget) — no
//! `MigrationManager` trait is declared here. The value below is the contract
//! the task executor decodes.
//!
//! # Identity and secrets
//!
//! A migration target is addressed by the owning workspace plus the service id,
//! never by a bare name, because service names are not globally unique. The
//! target carries identity, the working directory, the provider-neutral
//! operation, and whether the caller authorized a non-local database; it
//! deliberately carries **no** database credentials, connection strings,
//! passwords, or configured environment values, because task parameters are
//! stored on the task and may be surfaced through the interfaces
//! (`AGENTS.md` §31, `tasks/011.md` §11). The manager reads the connection
//! metadata and environment from validated configuration at execution time.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use service_orchestrator_domain::{
    AbsolutePath, MigrationOperation, MigrationProviderType, MigrationTargetEnvironment, ServiceId,
    WorkspaceId,
};
use thiserror::Error;

/// Parameter keys used to carry a [`ResolvedMigrationTarget`] through the task
/// engine.
pub mod parameter {
    /// The owning workspace identifier.
    pub const WORKSPACE: &str = "migration.workspace";
    /// The service identifier.
    pub const SERVICE: &str = "migration.service";
    /// The provider-neutral operation label, for example `apply`.
    pub const OPERATION: &str = "migration.operation";
    /// The absolute working directory of the service.
    pub const PATH: &str = "migration.path";
    /// The workspace root, used to validate the working directory stays inside
    /// it.
    pub const WORKSPACE_ROOT: &str = "migration.workspace_root";
    /// Whether the caller explicitly authorized a non-local migration target.
    pub const NON_LOCAL_AUTHORIZED: &str = "migration.non_local_authorized";
}

/// A migration target resolved against a validated configuration.
///
/// This is the unit of work a migration manager consumes. It is workspace-aware:
/// two workspaces may legitimately contain a service with the same id, and
/// their working directories are distinct.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedMigrationTarget {
    /// The workspace that owns the service.
    pub workspace_id: WorkspaceId,
    /// The service whose database is migrated.
    pub service_id: ServiceId,
    /// The provider-neutral operation to perform.
    pub operation: MigrationOperation,
    /// The absolute working directory of the service.
    pub path: AbsolutePath,
    /// The workspace root; the working directory must remain inside it.
    pub workspace_root: AbsolutePath,
    /// Whether the caller explicitly authorized a non-local database target.
    ///
    /// This is set from the caller's [`Authorization`](service_orchestrator_policy::Authorization)
    /// when the operation mutates a database whose environment is not local.
    /// The manager re-checks it so the policy cannot be bypassed by a
    /// hand-crafted task (`tasks/011.md` §10, §27).
    pub non_local_authorized: bool,
}

impl ResolvedMigrationTarget {
    /// Returns `true` when `path` is inside the workspace root.
    #[must_use]
    pub fn is_inside_workspace(&self) -> bool {
        let root = self.workspace_root.as_str().trim_end_matches('/');
        let path = self.path.as_str();
        path == root || path.starts_with(&format!("{root}/"))
    }

    /// Returns `true` when the operation mutates a database.
    #[must_use]
    pub const fn is_mutating(&self) -> bool {
        self.operation.is_mutating()
    }

    /// Encodes the target as task parameters.
    #[must_use]
    pub fn to_parameters(&self) -> BTreeMap<String, String> {
        let mut parameters = BTreeMap::new();
        parameters.insert(
            parameter::WORKSPACE.to_owned(),
            self.workspace_id.to_string(),
        );
        parameters.insert(parameter::SERVICE.to_owned(), self.service_id.to_string());
        parameters.insert(
            parameter::OPERATION.to_owned(),
            self.operation.label().to_owned(),
        );
        parameters.insert(parameter::PATH.to_owned(), self.path.to_string());
        parameters.insert(
            parameter::WORKSPACE_ROOT.to_owned(),
            self.workspace_root.to_string(),
        );
        parameters.insert(
            parameter::NON_LOCAL_AUTHORIZED.to_owned(),
            self.non_local_authorized.to_string(),
        );
        parameters
    }

    /// Decodes a target from task parameters.
    ///
    /// # Errors
    ///
    /// Returns [`MigrationError::MissingParameter`] if a required parameter is
    /// missing and [`MigrationError::InvalidParameter`] if one does not parse as
    /// its domain type.
    pub fn from_parameters(parameters: &BTreeMap<String, String>) -> Result<Self, MigrationError> {
        let workspace_id =
            WorkspaceId::new(required(parameters, parameter::WORKSPACE)?).map_err(|error| {
                MigrationError::invalid_parameter(parameter::WORKSPACE, error.to_string())
            })?;
        let service_id =
            ServiceId::new(required(parameters, parameter::SERVICE)?).map_err(|error| {
                MigrationError::invalid_parameter(parameter::SERVICE, error.to_string())
            })?;
        let operation = MigrationOperation::parse(required(parameters, parameter::OPERATION)?)
            .ok_or_else(|| {
                MigrationError::invalid_parameter(
                    parameter::OPERATION,
                    "not a known migration operation".to_owned(),
                )
            })?;
        let path = AbsolutePath::new(required(parameters, parameter::PATH)?).map_err(|error| {
            MigrationError::invalid_parameter(parameter::PATH, error.to_string())
        })?;
        let workspace_root = AbsolutePath::new(required(parameters, parameter::WORKSPACE_ROOT)?)
            .map_err(|error| {
                MigrationError::invalid_parameter(parameter::WORKSPACE_ROOT, error.to_string())
            })?;
        let non_local_authorized = parameters
            .get(parameter::NON_LOCAL_AUTHORIZED)
            .map(|value| value == "true")
            .unwrap_or(false);
        Ok(Self {
            workspace_id,
            service_id,
            operation,
            path,
            workspace_root,
            non_local_authorized,
        })
    }
}

fn required<'a>(
    parameters: &'a BTreeMap<String, String>,
    key: &str,
) -> Result<&'a str, MigrationError> {
    parameters
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| MigrationError::MissingParameter {
            key: key.to_owned(),
        })
}

/// A structured database-migration failure.
///
/// Every variant carries the context an interface needs to explain what
/// happened — service, provider, operation, environment, exit code — without
/// ever exposing a credential or a connection string (`tasks/011.md` §11, §28).
/// Codes are stable and machine-readable so an interface can branch on them
/// without parsing text.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MigrationError {
    /// No provider is registered for the requested provider type.
    #[error("no migration provider is registered for provider '{provider}'")]
    ProviderNotFound {
        /// The requested provider label.
        provider: String,
    },

    /// Migrations are disabled for the service.
    #[error("migrations are disabled for service '{service}'")]
    Disabled {
        /// The service identifier.
        service: ServiceId,
    },

    /// The service has no migration configuration at all.
    #[error("service '{service}' has no migration configuration")]
    NotConfigured {
        /// The service identifier.
        service: ServiceId,
    },

    /// The provider does not support the requested operation.
    #[error("migration provider '{provider}' does not support the '{operation}' operation")]
    UnsupportedOperation {
        /// The provider.
        provider: MigrationProviderType,
        /// The unsupported operation.
        operation: MigrationOperation,
    },

    /// The configured migration command for the operation is missing.
    #[error("no migration command is configured for the '{operation}' operation")]
    CommandMissing {
        /// The operation that has no command.
        operation: MigrationOperation,
    },

    /// The migration configuration is not valid for the operation.
    #[error("invalid migration configuration for service '{service}': {message}")]
    InvalidConfiguration {
        /// The service identifier.
        service: ServiceId,
        /// Why the configuration is invalid.
        message: String,
    },

    /// The migration target is invalid for the provider.
    #[error("invalid migration target for service '{service}': {message}")]
    InvalidTarget {
        /// The service identifier.
        service: ServiceId,
        /// Why the target is invalid.
        message: String,
    },

    /// The target database's environment could not be identified.
    #[error("the target database for service '{service}' is not identified")]
    DatabaseTargetUnknown {
        /// The service identifier.
        service: ServiceId,
    },

    /// A non-local database target was not authorized.
    #[error(
        "migrating the '{environment}' database for service '{service}' requires explicit authorization"
    )]
    DatabaseTargetNotAuthorized {
        /// The service identifier.
        service: ServiceId,
        /// The declared (non-local) environment.
        environment: MigrationTargetEnvironment,
    },

    /// Inspecting status is not supported for this target.
    #[error("migration status is unavailable for service '{service}': {message}")]
    StatusUnavailable {
        /// The service identifier.
        service: ServiceId,
        /// Why status is unavailable.
        message: String,
    },

    /// Generating a SQL preview is not supported for this target.
    #[error("a migration preview is unsupported for service '{service}': {message}")]
    PreviewUnsupported {
        /// The service identifier.
        service: ServiceId,
        /// Why the preview is unsupported.
        message: String,
    },

    /// A Liquibase database lock blocked the migration.
    #[error(
        "the target database for service '{service}' is locked by another migration: {message}"
    )]
    LockConflict {
        /// The service identifier.
        service: ServiceId,
        /// A credential-free explanation.
        message: String,
    },

    /// The migration command failed with a non-zero exit status.
    #[error("{operation} migration failed for service '{service}': {message}")]
    Failed {
        /// The service identifier.
        service: ServiceId,
        /// The operation that failed.
        operation: MigrationOperation,
        /// The process exit code, when available.
        exit_code: Option<i32>,
        /// A credential-free summary.
        message: String,
        /// More detailed, already-bounded output.
        detail: Option<String>,
    },

    /// The migration exceeded its timeout.
    #[error("{operation} migration for service '{service}' timed out after {timeout_seconds}s")]
    Timeout {
        /// The service identifier.
        service: ServiceId,
        /// The operation that timed out.
        operation: MigrationOperation,
        /// The timeout, in seconds.
        timeout_seconds: u64,
    },

    /// The migration was cancelled.
    #[error("{operation} migration for service '{service}' was cancelled")]
    Cancelled {
        /// The service identifier.
        service: ServiceId,
        /// The operation that was cancelled.
        operation: MigrationOperation,
    },

    /// The process manager refused or failed to run the plan.
    #[error("could not run the migration for service '{service}': {message}")]
    Process {
        /// The service identifier.
        service: ServiceId,
        /// A credential-free explanation.
        message: String,
    },

    /// A migration parameter was missing from a task.
    #[error("missing migration parameter '{key}'")]
    MissingParameter {
        /// The missing parameter key.
        key: String,
    },

    /// A migration parameter was invalid.
    #[error("invalid migration parameter '{key}': {message}")]
    InvalidParameter {
        /// The offending parameter key.
        key: String,
        /// Why it is invalid.
        message: String,
    },

    /// The manager itself failed.
    #[error("migration manager error: {message}")]
    Internal {
        /// A credential-free description of the failure.
        message: String,
    },
}

impl MigrationError {
    /// Builds a [`MigrationError::InvalidParameter`].
    #[must_use]
    pub fn invalid_parameter(key: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidParameter {
            key: key.into(),
            message: message.into(),
        }
    }

    /// Builds a [`MigrationError::Internal`].
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    /// Returns a stable, machine-readable code for the failure.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::ProviderNotFound { .. } => "MIGRATION_PROVIDER_NOT_FOUND",
            Self::Disabled { .. } => "MIGRATION_DISABLED",
            Self::NotConfigured { .. } => "MIGRATION_NOT_CONFIGURED",
            Self::UnsupportedOperation { .. } => "MIGRATION_UNSUPPORTED_OPERATION",
            Self::CommandMissing { .. } => "MIGRATION_COMMAND_MISSING",
            Self::InvalidConfiguration { .. } => "MIGRATION_INVALID_CONFIGURATION",
            Self::InvalidTarget { .. } => "MIGRATION_INVALID_TARGET",
            Self::DatabaseTargetUnknown { .. } => "MIGRATION_DATABASE_TARGET_UNKNOWN",
            Self::DatabaseTargetNotAuthorized { .. } => "MIGRATION_DATABASE_TARGET_NOT_AUTHORIZED",
            Self::StatusUnavailable { .. } => "MIGRATION_STATUS_UNAVAILABLE",
            Self::PreviewUnsupported { .. } => "MIGRATION_PREVIEW_UNSUPPORTED",
            Self::LockConflict { .. } => "MIGRATION_LOCK_CONFLICT",
            Self::Failed { .. } => "MIGRATION_FAILED",
            Self::Timeout { .. } => "MIGRATION_TIMEOUT",
            Self::Cancelled { .. } => "MIGRATION_CANCELLED",
            Self::Process { .. } => "MIGRATION_PROCESS_FAILED",
            Self::MissingParameter { .. } => "MIGRATION_MISSING_PARAMETER",
            Self::InvalidParameter { .. } => "MIGRATION_INVALID_PARAMETER",
            Self::Internal { .. } => "MIGRATION_INTERNAL_ERROR",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> ResolvedMigrationTarget {
        ResolvedMigrationTarget {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            service_id: ServiceId::new("auth-service").unwrap(),
            operation: MigrationOperation::Apply,
            path: AbsolutePath::new("/Users/dev/acme/backend/auth-service").unwrap(),
            workspace_root: AbsolutePath::new("/Users/dev/acme").unwrap(),
            non_local_authorized: false,
        }
    }

    #[test]
    fn parameters_round_trip() {
        let original = target();
        let decoded = ResolvedMigrationTarget::from_parameters(&original.to_parameters()).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn authorization_round_trips() {
        let original = ResolvedMigrationTarget {
            non_local_authorized: true,
            ..target()
        };
        let decoded = ResolvedMigrationTarget::from_parameters(&original.to_parameters()).unwrap();
        assert!(decoded.non_local_authorized);
    }

    #[test]
    fn an_absent_authorization_defaults_to_false() {
        let mut parameters = target().to_parameters();
        parameters.remove(parameter::NON_LOCAL_AUTHORIZED);
        let decoded = ResolvedMigrationTarget::from_parameters(&parameters).unwrap();
        assert!(!decoded.non_local_authorized);
    }

    #[test]
    fn missing_parameters_are_reported() {
        let mut parameters = target().to_parameters();
        parameters.remove(parameter::PATH);
        let error = ResolvedMigrationTarget::from_parameters(&parameters).unwrap_err();
        assert!(matches!(error, MigrationError::MissingParameter { .. }));
        assert_eq!(error.code(), "MIGRATION_MISSING_PARAMETER");
    }

    #[test]
    fn unknown_operations_are_rejected() {
        let mut parameters = target().to_parameters();
        parameters.insert(parameter::OPERATION.to_owned(), "rollback".to_owned());
        let error = ResolvedMigrationTarget::from_parameters(&parameters).unwrap_err();
        assert_eq!(error.code(), "MIGRATION_INVALID_PARAMETER");
    }

    #[test]
    fn detects_paths_outside_the_workspace() {
        assert!(target().is_inside_workspace());
        let escaping = ResolvedMigrationTarget {
            path: AbsolutePath::new("/Users/dev/other/auth-service").unwrap(),
            ..target()
        };
        assert!(!escaping.is_inside_workspace());
    }

    #[test]
    fn only_apply_is_mutating() {
        assert!(target().is_mutating());
        let status = ResolvedMigrationTarget {
            operation: MigrationOperation::Status,
            ..target()
        };
        assert!(!status.is_mutating());
    }

    #[test]
    fn error_codes_are_stable() {
        let error = MigrationError::DatabaseTargetNotAuthorized {
            service: ServiceId::new("auth-service").unwrap(),
            environment: MigrationTargetEnvironment::Production,
        };
        assert_eq!(error.code(), "MIGRATION_DATABASE_TARGET_NOT_AUTHORIZED");
        let error = MigrationError::ProviderNotFound {
            provider: "flyway".to_owned(),
        };
        assert_eq!(error.code(), "MIGRATION_PROVIDER_NOT_FOUND");
    }
}
