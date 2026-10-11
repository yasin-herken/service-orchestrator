//! Build management boundary.
//!
//! The application does not run Maven, npm, or any other build tool itself. It
//! describes *which entity* is being built (a workspace-aware
//! [`ResolvedBuildTarget`]), *which build-system-neutral operation* is being
//! performed, and *which configured command* (if any) should be preferred; a
//! build manager — implemented by the infrastructure `build` adapter over the
//! process manager — turns that into a structured build plan and executes it.
//!
//! # Why a port value, not a trait
//!
//! Every build operation is a task: the application validates preconditions,
//! resolves a [`ResolvedBuildTarget`], and submits a task carrying its
//! parameters to the engine. There is no read-only build query that would need a
//! direct manager call, so — unlike Git — no `BuildManager` trait is declared
//! here (see [`crate::ports`]). The value below is the contract the task
//! executor decodes, exactly as [`ResolvedRepository`](crate::ports::ResolvedRepository)
//! is for Git.
//!
//! # Entity identity
//!
//! A build target is never addressed by a bare name: service names are not
//! globally unique. [`ResolvedBuildTarget`] pairs the owning workspace with the
//! entity's absolute working directory, so a build task submitted to the engine
//! can be executed later without re-reading configuration. It round-trips
//! through task parameters via [`ResolvedBuildTarget::to_parameters`] and
//! [`ResolvedBuildTarget::from_parameters`].
//!
//! # Secrets
//!
//! A configured command may carry environment variables, some of them secret.
//! Environment *values* are deliberately not part of the target: task parameters
//! are stored on the task and may be surfaced through the interfaces, so
//! embedding configuration values there would risk leaking a credential
//! (`AGENTS.md` §31). Build-system runtime requirements (such as `JAVA_HOME`
//! for Maven) are declared by the provider as *names*, never resolved values.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use service_orchestrator_domain::{
    AbsolutePath, BuildOperation, BuildSystem, RelativePath, WorkspaceId,
};
use thiserror::Error;

/// Parameter keys used to carry a [`ResolvedBuildTarget`] through the task
/// engine.
pub mod parameter {
    /// The owning workspace identifier.
    pub const WORKSPACE: &str = "build.workspace";
    /// The entity identifier (a service or library id).
    pub const NAME: &str = "build.name";
    /// The build system label, for example `maven`.
    pub const SYSTEM: &str = "build.system";
    /// The build operation label, for example `build`.
    pub const OPERATION: &str = "build.operation";
    /// The absolute working directory.
    pub const PATH: &str = "build.path";
    /// The workspace root, used to validate the working directory stays inside
    /// it.
    pub const WORKSPACE_ROOT: &str = "build.workspace_root";
    /// The configured command's program, when one exists for the operation.
    pub const COMMAND_PROGRAM: &str = "build.command.program";
    /// The number of configured command arguments.
    pub const COMMAND_ARG_COUNT: &str = "build.command.arg_count";
    /// The prefix for indexed configured command arguments:
    /// `build.command.arg.0`, `build.command.arg.1`, ...
    pub const COMMAND_ARG_PREFIX: &str = "build.command.arg.";
    /// The configured command's working directory, relative to the workspace.
    pub const COMMAND_WORKING_DIR: &str = "build.command.working_dir";
}

/// A configured command preferred for a build operation.
///
/// When an entity declares a command for the requested operation (for example a
/// `build` entry in `[services.commands]`), it is authoritative: the provider
/// uses it verbatim instead of a build-system default. The command is always
/// structured — a program plus an argument vector — never a shell line
/// (`AGENTS.md` §32).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedBuildCommand {
    /// The executable to run.
    pub program: String,
    /// The arguments passed to the executable, in order.
    pub args: Vec<String>,
    /// The directory the command runs in, relative to the workspace root.
    pub working_dir: Option<RelativePath>,
}

/// A build target resolved against a validated configuration.
///
/// This is the unit of work a build manager consumes. It is workspace-aware:
/// two workspaces may legitimately contain a service with the same id, and
/// their working directories are distinct.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedBuildTarget {
    /// The workspace that owns the entity.
    pub workspace_id: WorkspaceId,
    /// The entity identifier (a service or library id).
    pub name: String,
    /// The build system to use.
    pub build_system: BuildSystem,
    /// The build-system-neutral operation to perform.
    pub operation: BuildOperation,
    /// The absolute working directory of the entity.
    pub path: AbsolutePath,
    /// The workspace root; the working directory must remain inside it.
    pub workspace_root: AbsolutePath,
    /// The configured command preferred for this operation, if any.
    pub command: Option<ResolvedBuildCommand>,
}

impl ResolvedBuildTarget {
    /// Returns `true` when `path` is inside the workspace root.
    #[must_use]
    pub fn is_inside_workspace(&self) -> bool {
        let root = self.workspace_root.as_str().trim_end_matches('/');
        let path = self.path.as_str();
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
        parameters.insert(parameter::NAME.to_owned(), self.name.clone());
        parameters.insert(
            parameter::SYSTEM.to_owned(),
            self.build_system.label().to_owned(),
        );
        parameters.insert(
            parameter::OPERATION.to_owned(),
            self.operation.label().to_owned(),
        );
        parameters.insert(parameter::PATH.to_owned(), self.path.to_string());
        parameters.insert(
            parameter::WORKSPACE_ROOT.to_owned(),
            self.workspace_root.to_string(),
        );
        if let Some(command) = &self.command {
            parameters.insert(
                parameter::COMMAND_PROGRAM.to_owned(),
                command.program.clone(),
            );
            parameters.insert(
                parameter::COMMAND_ARG_COUNT.to_owned(),
                command.args.len().to_string(),
            );
            for (index, arg) in command.args.iter().enumerate() {
                parameters.insert(
                    format!("{}{index}", parameter::COMMAND_ARG_PREFIX),
                    arg.clone(),
                );
            }
            if let Some(working_dir) = &command.working_dir {
                parameters.insert(
                    parameter::COMMAND_WORKING_DIR.to_owned(),
                    working_dir.to_string(),
                );
            }
        }
        parameters
    }

    /// Decodes a target from task parameters.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError::MissingParameter`] if a required parameter is
    /// missing and [`BuildError::InvalidParameter`] if one does not parse as its
    /// domain type.
    pub fn from_parameters(parameters: &BTreeMap<String, String>) -> Result<Self, BuildError> {
        let workspace_id =
            WorkspaceId::new(required(parameters, parameter::WORKSPACE)?).map_err(|error| {
                BuildError::invalid_parameter(parameter::WORKSPACE, error.to_string())
            })?;
        let build_system = BuildSystem::parse(required(parameters, parameter::SYSTEM)?)
            .ok_or_else(|| {
                BuildError::invalid_parameter(
                    parameter::SYSTEM,
                    "not a supported build system".to_owned(),
                )
            })?;
        let operation = BuildOperation::parse(required(parameters, parameter::OPERATION)?)
            .ok_or_else(|| {
                BuildError::invalid_parameter(
                    parameter::OPERATION,
                    "not a known build operation".to_owned(),
                )
            })?;
        let path = AbsolutePath::new(required(parameters, parameter::PATH)?)
            .map_err(|error| BuildError::invalid_parameter(parameter::PATH, error.to_string()))?;
        let workspace_root = AbsolutePath::new(required(parameters, parameter::WORKSPACE_ROOT)?)
            .map_err(|error| {
                BuildError::invalid_parameter(parameter::WORKSPACE_ROOT, error.to_string())
            })?;
        let command = decode_command(parameters)?;
        Ok(Self {
            workspace_id,
            name: required(parameters, parameter::NAME)?.to_owned(),
            build_system,
            operation,
            path,
            workspace_root,
            command,
        })
    }
}

/// Decodes the optional configured command from task parameters.
fn decode_command(
    parameters: &BTreeMap<String, String>,
) -> Result<Option<ResolvedBuildCommand>, BuildError> {
    let Some(program) = parameters.get(parameter::COMMAND_PROGRAM) else {
        return Ok(None);
    };
    let arg_count = match parameters.get(parameter::COMMAND_ARG_COUNT) {
        Some(value) => value.parse::<usize>().map_err(|_| {
            BuildError::invalid_parameter(parameter::COMMAND_ARG_COUNT, "not a number".to_owned())
        })?,
        None => 0,
    };
    let mut args = Vec::with_capacity(arg_count);
    for index in 0..arg_count {
        let key = format!("{}{index}", parameter::COMMAND_ARG_PREFIX);
        args.push(required(parameters, &key)?.to_owned());
    }
    let working_dir = match parameters.get(parameter::COMMAND_WORKING_DIR) {
        Some(value) => Some(RelativePath::new(value.clone()).map_err(|error| {
            BuildError::invalid_parameter(parameter::COMMAND_WORKING_DIR, error.to_string())
        })?),
        None => None,
    };
    Ok(Some(ResolvedBuildCommand {
        program: program.clone(),
        args,
        working_dir,
    }))
}

fn required<'a>(
    parameters: &'a BTreeMap<String, String>,
    key: &str,
) -> Result<&'a str, BuildError> {
    parameters
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| BuildError::MissingParameter {
            key: key.to_owned(),
        })
}

/// A structured build failure.
///
/// Every variant carries the context an interface needs to explain what
/// happened — entity, operation, exit code — without exposing credentials.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BuildError {
    /// A build system has no registered provider.
    #[error("no build provider is registered for build system '{system}'")]
    UnsupportedSystem {
        /// The unsupported build-system label.
        system: String,
    },

    /// The provider does not support the requested operation.
    #[error("build system '{system}' does not support the '{operation}' operation")]
    UnsupportedOperation {
        /// The build system.
        system: BuildSystem,
        /// The unsupported operation.
        operation: BuildOperation,
    },

    /// The target is not valid for the provider.
    #[error("invalid build target '{name}': {message}")]
    InvalidTarget {
        /// The entity identifier.
        name: String,
        /// Why the target is invalid.
        message: String,
    },

    /// The build command failed with a non-zero exit status.
    #[error("{system} {operation} failed for '{name}': {message}")]
    CommandFailed {
        /// The entity identifier.
        name: String,
        /// The build system.
        system: BuildSystem,
        /// The operation that failed.
        operation: BuildOperation,
        /// The process exit code, when available.
        exit_code: Option<i32>,
        /// A human-readable summary.
        message: String,
        /// More detailed output, already bounded.
        detail: Option<String>,
    },

    /// The build exceeded its timeout.
    #[error("{system} {operation} for '{name}' timed out after {timeout_seconds}s")]
    Timeout {
        /// The entity identifier.
        name: String,
        /// The build system.
        system: BuildSystem,
        /// The operation that timed out.
        operation: BuildOperation,
        /// The timeout, in seconds.
        timeout_seconds: u64,
    },

    /// The build was cancelled.
    #[error("{system} {operation} for '{name}' was cancelled")]
    Cancelled {
        /// The entity identifier.
        name: String,
        /// The build system.
        system: BuildSystem,
        /// The operation that was cancelled.
        operation: BuildOperation,
    },

    /// The process manager refused or failed to run the plan.
    #[error("could not run the build for '{name}': {message}")]
    Process {
        /// The entity identifier.
        name: String,
        /// A human-readable explanation.
        message: String,
    },

    /// A build parameter was missing from a task.
    #[error("missing build parameter '{key}'")]
    MissingParameter {
        /// The missing parameter key.
        key: String,
    },

    /// A build parameter was invalid.
    #[error("invalid build parameter '{key}': {message}")]
    InvalidParameter {
        /// The offending parameter key.
        key: String,
        /// Why it is invalid.
        message: String,
    },

    /// The manager itself failed.
    #[error("build manager error: {message}")]
    Internal {
        /// A human-readable description of the failure.
        message: String,
    },
}

impl BuildError {
    /// Builds a [`BuildError::InvalidParameter`].
    #[must_use]
    pub fn invalid_parameter(key: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidParameter {
            key: key.into(),
            message: message.into(),
        }
    }

    /// Builds a [`BuildError::Internal`].
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
            Self::UnsupportedSystem { .. } => "BUILD_UNSUPPORTED_SYSTEM",
            Self::UnsupportedOperation { .. } => "BUILD_UNSUPPORTED_OPERATION",
            Self::InvalidTarget { .. } => "BUILD_INVALID_TARGET",
            Self::CommandFailed { .. } => "BUILD_COMMAND_FAILED",
            Self::Timeout { .. } => "BUILD_TIMEOUT",
            Self::Cancelled { .. } => "BUILD_CANCELLED",
            Self::Process { .. } => "BUILD_PROCESS_FAILED",
            Self::MissingParameter { .. } => "BUILD_MISSING_PARAMETER",
            Self::InvalidParameter { .. } => "BUILD_INVALID_PARAMETER",
            Self::Internal { .. } => "BUILD_INTERNAL_ERROR",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> ResolvedBuildTarget {
        ResolvedBuildTarget {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            name: "auth-service".to_owned(),
            build_system: BuildSystem::Maven,
            operation: BuildOperation::Build,
            path: AbsolutePath::new("/Users/dev/acme/backend/auth-service").unwrap(),
            workspace_root: AbsolutePath::new("/Users/dev/acme").unwrap(),
            command: Some(ResolvedBuildCommand {
                program: "./mvnw".to_owned(),
                args: vec!["clean".to_owned(), "install".to_owned()],
                working_dir: None,
            }),
        }
    }

    #[test]
    fn parameters_round_trip() {
        let original = target();
        let decoded = ResolvedBuildTarget::from_parameters(&original.to_parameters()).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn parameters_round_trip_without_a_command() {
        let original = ResolvedBuildTarget {
            command: None,
            ..target()
        };
        let decoded = ResolvedBuildTarget::from_parameters(&original.to_parameters()).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn missing_parameters_are_reported() {
        let mut parameters = target().to_parameters();
        parameters.remove(parameter::PATH);
        let error = ResolvedBuildTarget::from_parameters(&parameters).unwrap_err();
        assert!(matches!(error, BuildError::MissingParameter { .. }));
        assert_eq!(error.code(), "BUILD_MISSING_PARAMETER");
    }

    #[test]
    fn unknown_systems_are_rejected() {
        let mut parameters = target().to_parameters();
        parameters.insert(parameter::SYSTEM.to_owned(), "gradle".to_owned());
        let error = ResolvedBuildTarget::from_parameters(&parameters).unwrap_err();
        assert_eq!(error.code(), "BUILD_INVALID_PARAMETER");
    }

    #[test]
    fn detects_paths_outside_the_workspace() {
        assert!(target().is_inside_workspace());
        let escaping = ResolvedBuildTarget {
            path: AbsolutePath::new("/Users/dev/other/auth-service").unwrap(),
            ..target()
        };
        assert!(!escaping.is_inside_workspace());
    }

    #[test]
    fn error_codes_are_stable() {
        let error = BuildError::UnsupportedOperation {
            system: BuildSystem::Npm,
            operation: BuildOperation::PublishLocalArtifact,
        };
        assert_eq!(error.code(), "BUILD_UNSUPPORTED_OPERATION");
    }
}
