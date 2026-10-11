//! Service runtime boundary.
//!
//! The application does not spawn, stop, or supervise service processes itself.
//! [`start_service`](crate::Application::start_service) and its siblings resolve
//! a workspace-aware [`ResolvedServiceTarget`] and submit a `StartService` /
//! `StopService` / `RestartService` task; the task executor decodes the target
//! and drives the infrastructure *Service Manager* (implemented by the
//! `runtime` adapter over the process manager).
//!
//! # Why a port value, not a trait — and one read-only trait
//!
//! Every lifecycle operation is a task, so — exactly like builds — there is no
//! `ServiceManager` trait for the *mutating* path: the value below is the
//! contract the task executor decodes. The application does need to answer
//! read-oriented questions (`get_service_runtime_status`, `list_running_services`)
//! from live managed-process state that lives in the runtime adapter, so
//! [`ServiceRuntimeIndex`] is a small, read-only port the adapter implements and
//! [`NullServiceRuntimeIndex`] keeps the application constructible without it.
//!
//! # Secrets
//!
//! A service's environment may carry secret values. Like the build target,
//! [`ResolvedServiceTarget`] deliberately carries no environment *values*: task
//! parameters are stored on the task and may be surfaced through the
//! interfaces, so embedding a credential there would leak it (`AGENTS.md` §31).
//! The executor's adapter reads environment from validated configuration at
//! execution time and hands it straight to the process manager, which redacts
//! secret values whenever a specification is displayed or serialized.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use service_orchestrator_domain::{AbsolutePath, ProcessState, ServiceId, Timestamp, WorkspaceId};
use thiserror::Error;

/// Parameter keys used to carry a [`ResolvedServiceTarget`] through the task
/// engine.
pub mod parameter {
    /// The owning workspace identifier.
    pub const WORKSPACE: &str = "service.workspace";
    /// The service identifier.
    pub const SERVICE: &str = "service.id";
    /// The service's absolute working directory, when it has one.
    pub const PATH: &str = "service.path";
    /// The workspace root, used to validate the working directory stays inside
    /// it.
    pub const WORKSPACE_ROOT: &str = "service.workspace_root";
}

/// A service lifecycle target resolved against a validated configuration.
///
/// This is the unit of work the Service Manager consumes. It is workspace-aware
/// (two workspaces may legitimately contain a service with the same id) and
/// carries the identity and working directory a lifecycle task needs, without
/// duplicating environment values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedServiceTarget {
    /// The workspace that owns the service.
    pub workspace_id: WorkspaceId,
    /// The service identifier.
    pub service_id: ServiceId,
    /// The service's absolute working directory, when it has one.
    ///
    /// `None` is allowed at the type level because a service need not declare a
    /// path to be *stoppable*; a start or restart rejects a missing path with
    /// [`ServiceRuntimeError::WorkingDirectoryInvalid`].
    pub path: Option<AbsolutePath>,
    /// The workspace root; a resolved working directory must stay inside it.
    pub workspace_root: AbsolutePath,
}

impl ResolvedServiceTarget {
    /// Returns `true` when a resolved path is inside the workspace root.
    ///
    /// A target with no path trivially satisfies containment (there is nothing
    /// to escape).
    #[must_use]
    pub fn is_inside_workspace(&self) -> bool {
        let Some(path) = &self.path else {
            return true;
        };
        let root = self.workspace_root.as_str().trim_end_matches('/');
        let path = path.as_str();
        path == root || path.starts_with(&format!("{root}/"))
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
        if let Some(path) = &self.path {
            parameters.insert(parameter::PATH.to_owned(), path.to_string());
        }
        parameters.insert(
            parameter::WORKSPACE_ROOT.to_owned(),
            self.workspace_root.to_string(),
        );
        parameters
    }

    /// Decodes a target from task parameters.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceRuntimeError::MissingParameter`] if a required parameter
    /// is missing and [`ServiceRuntimeError::InvalidParameter`] if one does not
    /// parse as its domain type.
    pub fn from_parameters(
        parameters: &BTreeMap<String, String>,
    ) -> Result<Self, ServiceRuntimeError> {
        let workspace_id =
            WorkspaceId::new(required(parameters, parameter::WORKSPACE)?).map_err(|error| {
                ServiceRuntimeError::invalid_parameter(parameter::WORKSPACE, error.to_string())
            })?;
        let service_id =
            ServiceId::new(required(parameters, parameter::SERVICE)?).map_err(|error| {
                ServiceRuntimeError::invalid_parameter(parameter::SERVICE, error.to_string())
            })?;
        let path = match parameters.get(parameter::PATH) {
            Some(value) => Some(AbsolutePath::new(value.clone()).map_err(|error| {
                ServiceRuntimeError::invalid_parameter(parameter::PATH, error.to_string())
            })?),
            None => None,
        };
        let workspace_root = AbsolutePath::new(required(parameters, parameter::WORKSPACE_ROOT)?)
            .map_err(|error| {
                ServiceRuntimeError::invalid_parameter(parameter::WORKSPACE_ROOT, error.to_string())
            })?;
        Ok(Self {
            workspace_id,
            service_id,
            path,
            workspace_root,
        })
    }
}

fn required<'a>(
    parameters: &'a BTreeMap<String, String>,
    key: &str,
) -> Result<&'a str, ServiceRuntimeError> {
    parameters
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| ServiceRuntimeError::MissingParameter {
            key: key.to_owned(),
        })
}

/// A read-only snapshot of a managed service process.
///
/// This is the structured answer to `get_service_runtime_status` and
/// `list_running_services`. It is deliberately interface-neutral: the TUI and
/// MCP render it; neither is referenced here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceProcessInfo {
    /// The workspace that owns the service.
    pub workspace_id: WorkspaceId,
    /// The service identifier.
    pub service_id: ServiceId,
    /// The managed process identifier, when one exists.
    pub process_id: Option<String>,
    /// The operating-system process id, while the process is alive.
    pub pid: Option<u32>,
    /// The last observed logical process state.
    pub state: ProcessState,
    /// The runtime label that started the process, when known.
    pub runtime: Option<String>,
    /// A redacted identity of the startup command.
    pub command: Option<String>,
    /// When the current execution started.
    pub started_at: Option<Timestamp>,
    /// The exit code of the last execution, once it has ended.
    pub exit_code: Option<i32>,
    /// How the last execution ended, once it has ended.
    pub exit_reason: Option<String>,
    /// A structured explanation of the last failure, when there was one.
    pub last_error: Option<String>,
}

/// A read-only index of managed service processes.
///
/// The runtime adapter implements this over its in-memory runtime registry; the
/// application reads it so its queries never reach past a port. The
/// [`NullServiceRuntimeIndex`] answers "nothing is managed", which keeps the
/// application constructible and its existing tests unchanged when no runtime
/// adapter is wired.
pub trait ServiceRuntimeIndex: Send + Sync {
    /// Returns the managed snapshot for `service_id`, if one is known.
    fn runtime_state(&self, service_id: &ServiceId) -> Option<ServiceProcessInfo>;

    /// Lists snapshots of every service process the adapter is tracking.
    fn managed_services(&self) -> Vec<ServiceProcessInfo>;
}

/// A [`ServiceRuntimeIndex`] that knows about nothing.
///
/// It is the default so the application is always constructible. Queries then
/// fall back to the last observed [`RuntimeStateStore`](crate::RuntimeStateStore)
/// value rather than live managed-process detail.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullServiceRuntimeIndex;

impl ServiceRuntimeIndex for NullServiceRuntimeIndex {
    fn runtime_state(&self, _service_id: &ServiceId) -> Option<ServiceProcessInfo> {
        None
    }

    fn managed_services(&self) -> Vec<ServiceProcessInfo> {
        Vec::new()
    }
}

/// A structured service runtime failure.
///
/// Every variant carries the context an interface needs — workspace, service,
/// runtime, command identity, exit code — to explain what happened without
/// exposing credentials (`AGENTS.md` §31, `tasks/010.md` §32).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ServiceRuntimeError {
    /// The requested workspace does not exist.
    #[error("workspace '{workspace}' was not found")]
    WorkspaceNotFound {
        /// The workspace identifier.
        workspace: WorkspaceId,
    },

    /// The requested service does not exist.
    #[error("service '{service}' was not found")]
    ServiceNotFound {
        /// The service identifier.
        service: ServiceId,
    },

    /// No runtime is registered for the service's configuration.
    #[error("no service runtime is registered for service '{service}'")]
    UnsupportedRuntime {
        /// The service identifier.
        service: ServiceId,
        /// The runtime declaration that could not be resolved, when known.
        runtime: Option<String>,
    },

    /// The required runtime executable is not available.
    #[error("the {runtime} runtime required by service '{service}' is not available: {message}")]
    RuntimeNotFound {
        /// The service identifier.
        service: ServiceId,
        /// The runtime label.
        runtime: String,
        /// An actionable explanation.
        message: String,
    },

    /// The installed runtime version is incompatible with the pinned version.
    #[error("service '{service}' requires {runtime} {expected} but {found} is installed")]
    RuntimeVersionMismatch {
        /// The service identifier.
        service: ServiceId,
        /// The runtime label.
        runtime: String,
        /// The version the service pins.
        expected: String,
        /// The version that is actually installed.
        found: String,
    },

    /// The service declares no start command.
    #[error("service '{service}' has no start command configured")]
    StartCommandMissing {
        /// The service identifier.
        service: ServiceId,
    },

    /// The service's working directory is missing or invalid.
    #[error("service '{service}' has an invalid working directory '{path}': {message}")]
    WorkingDirectoryInvalid {
        /// The service identifier.
        service: ServiceId,
        /// The offending path, when one was resolved.
        path: String,
        /// An actionable explanation.
        message: String,
    },

    /// A configured port is already occupied.
    #[error("service '{service}' cannot start: port {port} is already in use")]
    PortAlreadyInUse {
        /// The service identifier.
        service: ServiceId,
        /// The occupied port.
        port: u16,
    },

    /// A start is already in progress for the service.
    #[error("service '{service}' is already starting or transitioning")]
    ServiceAlreadyStarting {
        /// The service identifier.
        service: ServiceId,
    },

    /// The process manager refused or failed to spawn the process.
    #[error("service '{service}' failed to start: {message}")]
    ProcessStartFailed {
        /// The service identifier.
        service: ServiceId,
        /// A human-readable explanation.
        message: String,
    },

    /// The process manager refused or failed to stop the process.
    #[error("service '{service}' failed to stop: {message}")]
    ProcessStopFailed {
        /// The service identifier.
        service: ServiceId,
        /// A human-readable explanation.
        message: String,
    },

    /// A managed process exited unexpectedly.
    #[error("service '{service}' exited unexpectedly: {message}")]
    ProcessExitedUnexpectedly {
        /// The service identifier.
        service: ServiceId,
        /// The exit detail.
        message: String,
    },

    /// The lifecycle operation was cancelled before a process was started.
    #[error("the runtime operation for service '{service}' was cancelled")]
    OperationCancelled {
        /// The service identifier.
        service: ServiceId,
    },

    /// The lifecycle operation exceeded its timeout.
    #[error("the runtime operation for service '{service}' timed out")]
    OperationTimeout {
        /// The service identifier.
        service: ServiceId,
    },

    /// The configuration is not consistent with the requested operation.
    #[error("invalid service runtime configuration: {message}")]
    InvalidConfiguration {
        /// What is wrong with the configuration.
        message: String,
    },

    /// A runtime parameter was missing from a task.
    #[error("missing service runtime parameter '{key}'")]
    MissingParameter {
        /// The missing parameter key.
        key: String,
    },

    /// A runtime parameter was invalid.
    #[error("invalid service runtime parameter '{key}': {message}")]
    InvalidParameter {
        /// The offending parameter key.
        key: String,
        /// Why it is invalid.
        message: String,
    },

    /// The manager itself failed.
    #[error("service runtime error: {message}")]
    Internal {
        /// A human-readable description of the failure.
        message: String,
    },
}

impl ServiceRuntimeError {
    /// Builds a [`ServiceRuntimeError::InvalidParameter`].
    #[must_use]
    pub fn invalid_parameter(key: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidParameter {
            key: key.into(),
            message: message.into(),
        }
    }

    /// Builds a [`ServiceRuntimeError::Internal`].
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    /// Returns the service identifier the error refers to, when it has one.
    #[must_use]
    pub fn service(&self) -> Option<&ServiceId> {
        match self {
            Self::ServiceNotFound { service }
            | Self::UnsupportedRuntime { service, .. }
            | Self::RuntimeNotFound { service, .. }
            | Self::RuntimeVersionMismatch { service, .. }
            | Self::StartCommandMissing { service }
            | Self::WorkingDirectoryInvalid { service, .. }
            | Self::PortAlreadyInUse { service, .. }
            | Self::ServiceAlreadyStarting { service }
            | Self::ProcessStartFailed { service, .. }
            | Self::ProcessStopFailed { service, .. }
            | Self::ProcessExitedUnexpectedly { service, .. }
            | Self::OperationCancelled { service }
            | Self::OperationTimeout { service } => Some(service),
            Self::WorkspaceNotFound { .. }
            | Self::InvalidConfiguration { .. }
            | Self::MissingParameter { .. }
            | Self::InvalidParameter { .. }
            | Self::Internal { .. } => None,
        }
    }

    /// Returns a stable, machine-readable code for the failure.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::WorkspaceNotFound { .. } => "RUNTIME_WORKSPACE_NOT_FOUND",
            Self::ServiceNotFound { .. } => "RUNTIME_SERVICE_NOT_FOUND",
            Self::UnsupportedRuntime { .. } => "RUNTIME_UNSUPPORTED",
            Self::RuntimeNotFound { .. } => "RUNTIME_NOT_FOUND",
            Self::RuntimeVersionMismatch { .. } => "RUNTIME_VERSION_MISMATCH",
            Self::StartCommandMissing { .. } => "RUNTIME_START_COMMAND_MISSING",
            Self::WorkingDirectoryInvalid { .. } => "RUNTIME_WORKING_DIRECTORY_INVALID",
            Self::PortAlreadyInUse { .. } => "RUNTIME_PORT_ALREADY_IN_USE",
            Self::ServiceAlreadyStarting { .. } => "RUNTIME_ALREADY_STARTING",
            Self::ProcessStartFailed { .. } => "RUNTIME_PROCESS_START_FAILED",
            Self::ProcessStopFailed { .. } => "RUNTIME_PROCESS_STOP_FAILED",
            Self::ProcessExitedUnexpectedly { .. } => "RUNTIME_PROCESS_EXITED",
            Self::OperationCancelled { .. } => "RUNTIME_OPERATION_CANCELLED",
            Self::OperationTimeout { .. } => "RUNTIME_OPERATION_TIMEOUT",
            Self::InvalidConfiguration { .. } => "RUNTIME_INVALID_CONFIGURATION",
            Self::MissingParameter { .. } => "RUNTIME_MISSING_PARAMETER",
            Self::InvalidParameter { .. } => "RUNTIME_INVALID_PARAMETER",
            Self::Internal { .. } => "RUNTIME_INTERNAL_ERROR",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> ResolvedServiceTarget {
        ResolvedServiceTarget {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            service_id: ServiceId::new("auth-service").unwrap(),
            path: Some(AbsolutePath::new("/Users/dev/acme/backend/auth-service").unwrap()),
            workspace_root: AbsolutePath::new("/Users/dev/acme").unwrap(),
        }
    }

    #[test]
    fn parameters_round_trip() {
        let original = target();
        let decoded = ResolvedServiceTarget::from_parameters(&original.to_parameters()).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn parameters_round_trip_without_a_path() {
        let original = ResolvedServiceTarget {
            path: None,
            ..target()
        };
        let decoded = ResolvedServiceTarget::from_parameters(&original.to_parameters()).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn missing_parameters_are_reported() {
        let mut parameters = target().to_parameters();
        parameters.remove(parameter::SERVICE);
        let error = ResolvedServiceTarget::from_parameters(&parameters).unwrap_err();
        assert!(matches!(
            error,
            ServiceRuntimeError::MissingParameter { .. }
        ));
        assert_eq!(error.code(), "RUNTIME_MISSING_PARAMETER");
    }

    #[test]
    fn detects_paths_outside_the_workspace() {
        assert!(target().is_inside_workspace());
        let escaping = ResolvedServiceTarget {
            path: Some(AbsolutePath::new("/Users/dev/other/auth-service").unwrap()),
            ..target()
        };
        assert!(!escaping.is_inside_workspace());
    }

    #[test]
    fn a_missing_path_is_contained() {
        let target = ResolvedServiceTarget {
            path: None,
            ..target()
        };
        assert!(target.is_inside_workspace());
    }

    #[test]
    fn error_codes_are_stable() {
        let error = ServiceRuntimeError::PortAlreadyInUse {
            service: ServiceId::new("auth-service").unwrap(),
            port: 8081,
        };
        assert_eq!(error.code(), "RUNTIME_PORT_ALREADY_IN_USE");
    }
}
