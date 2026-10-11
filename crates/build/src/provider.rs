//! The `BuildProvider` abstraction.
//!
//! A [`BuildProvider`] knows *how* one build system works: which operations it
//! supports, what to validate before planning, how to resolve its executable or
//! wrapper, and how to turn a validated
//! [`ResolvedBuildTarget`](service_orchestrator_application::ResolvedBuildTarget)
//! into a structured [`BuildPlan`]. It deliberately does **not** spawn a process
//! or schedule a task: the process manager and the task engine own execution
//! (`tasks/008.md`). Everything a provider needs to *observe* about its
//! environment is asked through the tiny [`BuildHost`] trait, which keeps
//! providers pure and trivially testable.
//!
//! # Why an abstraction at all
//!
//! Maven is the primary build system of the current services, but the domain and
//! the application name build systems and operations neutrally. A provider is
//! the single place where build-system-specific knowledge lives, so adding a
//! new system is a new provider and a registry entry — not a change to
//! orchestration, the task engine, the interfaces, or the application core
//! (`AGENTS.md` §11, `tasks/008.md`).
//!
//! # Configured commands win
//!
//! When configuration declares a command for the requested operation, it is
//! authoritative: the provider uses it verbatim (still structured — a program
//! plus an argument vector, never a shell line) instead of a build-system
//! default. Only the provider's *default* plan needs build-system knowledge.

use service_orchestrator_application::{BuildError, ResolvedBuildCommand, ResolvedBuildTarget};
use service_orchestrator_domain::{AbsolutePath, BuildOperation, BuildSystem};

use crate::plan::{BuildPlan, EnvironmentRequirement, ExpectedOutput};

/// The filesystem and environment questions a provider may ask while planning.
///
/// Providers never touch the filesystem or the process environment directly.
/// Routing those observations through a host keeps a provider a pure function of
/// its inputs and makes it deterministic to test: a test supplies a host whose
/// answers it controls.
pub trait BuildHost: Send + Sync {
    /// Returns `true` when `name` exists directly inside `directory`.
    fn file_exists(&self, directory: &AbsolutePath, name: &str) -> bool;

    /// Returns `true` when the named environment variable is set and non-empty.
    fn environment_has(&self, name: &str) -> bool;
}

/// The production [`BuildHost`], backed by the real filesystem and environment.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemBuildHost;

impl SystemBuildHost {
    /// Creates the system host.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl BuildHost for SystemBuildHost {
    fn file_exists(&self, directory: &AbsolutePath, name: &str) -> bool {
        std::path::Path::new(directory.as_str())
            .join(name)
            .is_file()
    }

    fn environment_has(&self, name: &str) -> bool {
        std::env::var_os(name).is_some_and(|value| !value.is_empty())
    }
}

/// How one build system is validated and planned.
///
/// Implementations are stateless; all of their inputs arrive as arguments. This
/// is what makes them cheap to share behind `Arc<dyn BuildProvider>` and easy to
/// unit test.
pub trait BuildProvider: Send + Sync {
    /// The build system this provider implements.
    fn system(&self) -> BuildSystem;

    /// The build-system-neutral operations this provider supports.
    ///
    /// Capabilities genuinely differ between systems: npm has no notion of
    /// publishing a local artifact, and Maven's `install` publishes to the local
    /// repository rather than merely resolving dependencies. An operation that
    /// is not listed is rejected with [`BuildError::UnsupportedOperation`].
    fn capabilities(&self) -> &[BuildOperation];

    /// Returns `true` when this provider supports `operation`.
    fn supports(&self, operation: BuildOperation) -> bool {
        self.capabilities().contains(&operation)
    }

    /// Validates a target against this build system's requirements.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError::InvalidTarget`] when the target cannot be built by
    /// this provider (for example its working directory escapes the workspace).
    fn validate(
        &self,
        target: &ResolvedBuildTarget,
        host: &dyn BuildHost,
    ) -> Result<(), BuildError>;

    /// Constructs the structured plan for the target's operation.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError::InvalidTarget`] when the target is invalid or
    /// [`BuildError::UnsupportedOperation`] when the operation is not supported.
    fn build_plan(
        &self,
        target: &ResolvedBuildTarget,
        host: &dyn BuildHost,
    ) -> Result<BuildPlan, BuildError>;
}

/// Rejects a target whose working directory escapes its workspace.
pub(crate) fn validate_inside_workspace(target: &ResolvedBuildTarget) -> Result<(), BuildError> {
    if target.is_inside_workspace() {
        Ok(())
    } else {
        Err(BuildError::InvalidTarget {
            name: target.name.clone(),
            message: format!(
                "working directory '{}' is outside workspace root '{}'",
                target.path, target.workspace_root
            ),
        })
    }
}

/// Constructs a plan from a target's configured command.
///
/// The command is authoritative and used verbatim; the provider still attaches
/// its own environment requirements and expected outputs so the plan remains
/// complete.
pub(crate) fn plan_from_command(
    target: &ResolvedBuildTarget,
    command: &ResolvedBuildCommand,
    environment: Vec<EnvironmentRequirement>,
    expected_outputs: Vec<ExpectedOutput>,
) -> Result<BuildPlan, BuildError> {
    Ok(BuildPlan {
        program: command.program.clone(),
        args: command.args.clone(),
        working_dir: command_working_dir(target)?,
        environment,
        timeout: None,
        expected_outputs,
        system: target.build_system,
        operation: target.operation,
    })
}

/// Resolves the working directory for a target's configured command.
///
/// A configured command may run in a subdirectory of the workspace; otherwise it
/// runs in the entity's own directory. The resolved path is re-checked to stay
/// inside the workspace.
fn command_working_dir(target: &ResolvedBuildTarget) -> Result<AbsolutePath, BuildError> {
    let Some(relative) = target
        .command
        .as_ref()
        .and_then(|command| command.working_dir.as_ref())
    else {
        return Ok(target.path.clone());
    };
    let root = target.workspace_root.as_str().trim_end_matches('/');
    let joined = format!("{root}/{}", relative.as_str());
    let path = AbsolutePath::new(joined).map_err(|error| BuildError::InvalidTarget {
        name: target.name.clone(),
        message: format!("configured working directory is invalid: {error}"),
    })?;
    let inside = path.as_str() == root || path.as_str().starts_with(&format!("{root}/"));
    if !inside {
        return Err(BuildError::InvalidTarget {
            name: target.name.clone(),
            message: format!(
                "configured working directory '{}' escapes workspace root '{root}'",
                path.as_str()
            ),
        });
    }
    Ok(path)
}

/// Builds the provider's plan, choosing the configured command when present.
///
/// `configured` builds the plan for an explicitly configured command;
/// `default` supplies the build-system default and is only consulted when
/// configuration declares no command for the operation.
pub(crate) fn resolve_plan(
    target: &ResolvedBuildTarget,
    default: impl FnOnce() -> Result<BuildPlan, BuildError>,
    configured: impl FnOnce(&ResolvedBuildCommand) -> Result<BuildPlan, BuildError>,
) -> Result<BuildPlan, BuildError> {
    validate_inside_workspace(target)?;
    match &target.command {
        Some(command) => configured(command),
        None => default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_domain::WorkspaceId;

    fn target(command: Option<ResolvedBuildCommand>) -> ResolvedBuildTarget {
        ResolvedBuildTarget {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            name: "auth-service".to_owned(),
            build_system: BuildSystem::Maven,
            operation: BuildOperation::Build,
            path: AbsolutePath::new("/w/acme/backend/auth-service").unwrap(),
            workspace_root: AbsolutePath::new("/w/acme").unwrap(),
            command,
        }
    }

    fn plan(target: &ResolvedBuildTarget) -> Result<BuildPlan, BuildError> {
        resolve_plan(
            target,
            || {
                Ok(BuildPlan::new(
                    target.build_system,
                    target.operation,
                    "mvn",
                    target.path.clone(),
                ))
            },
            |command| {
                plan_from_command(
                    target,
                    command,
                    vec![EnvironmentRequirement::optional("JAVA_HOME")],
                    Vec::new(),
                )
            },
        )
    }

    #[test]
    fn a_configured_command_is_used_verbatim() {
        let target = target(Some(ResolvedBuildCommand {
            program: "./mvnw".to_owned(),
            args: vec!["clean".to_owned(), "install".to_owned()],
            working_dir: None,
        }));
        let built = plan(&target).unwrap();
        assert_eq!(built.program, "./mvnw");
        assert_eq!(built.args, vec!["clean", "install"]);
        assert_eq!(built.working_dir, target.path);
    }

    #[test]
    fn a_configured_command_working_directory_is_resolved_against_the_workspace() {
        let target = target(Some(ResolvedBuildCommand {
            program: "./mvnw".to_owned(),
            args: Vec::new(),
            working_dir: Some(
                service_orchestrator_domain::RelativePath::new("backend/auth-service").unwrap(),
            ),
        }));
        let built = plan(&target).unwrap();
        assert_eq!(built.working_dir.as_str(), "/w/acme/backend/auth-service");
    }

    #[test]
    fn without_a_command_the_default_plan_is_used() {
        let target = target(None);
        let built = plan(&target).unwrap();
        assert_eq!(built.program, "mvn");
        assert!(built.args.is_empty());
    }

    #[test]
    fn a_target_outside_its_workspace_is_rejected() {
        let target = ResolvedBuildTarget {
            path: AbsolutePath::new("/w/other/auth-service").unwrap(),
            ..target(None)
        };
        let error = plan(&target).unwrap_err();
        assert_eq!(error.code(), "BUILD_INVALID_TARGET");
    }

    #[test]
    fn the_system_host_reports_missing_files() {
        let host = SystemBuildHost::new();
        let directory = AbsolutePath::new("/").unwrap();
        assert!(!host.file_exists(&directory, "definitely-not-a-real-file-xyz"));
    }
}
