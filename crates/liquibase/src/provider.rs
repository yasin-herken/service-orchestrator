//! The `MigrationProvider` abstraction.
//!
//! A [`MigrationProvider`] knows *how* one migration tool works: which
//! provider-neutral operations it supports, what to validate before planning,
//! how to resolve its executable or wrapper, and how to turn a validated
//! [`MigrationContext`] into a structured [`MigrationPlan`]. It deliberately does
//! **not** spawn a process or schedule a task: the process manager owns execution
//! and the task engine owns scheduling (`AGENTS.md` §9, §15, §21,
//! `tasks/011.md` §3).
//!
//! Liquibase is the initial concrete provider, but the abstraction is what keeps
//! the [`MigrationManager`](crate::MigrationManager) and the application core
//! provider-agnostic: adding Flyway is a new provider plus a registry entry, not
//! a change to orchestration (`tasks/011.md` §4, §5).
//!
//! # The host seam
//!
//! Providers observe the filesystem and the process environment only through the
//! [`BuildHost`] trait, which the build adapter already defines and the build
//! providers already use. Reusing it keeps the provider a pure function of its
//! inputs — no direct filesystem access, no `std::env` — and trivially testable.

use service_orchestrator_application::MigrationError;
use service_orchestrator_domain::{
    AbsolutePath, Environment, MigrationOperation, MigrationProviderType,
};

use crate::context::MigrationContext;
use crate::plan::{MigrationCapabilities, MigrationPlan};

pub use service_orchestrator_build::{BuildHost, SystemBuildHost};

/// How one migration tool is validated and planned.
///
/// Implementations are stateless; all of their inputs arrive as arguments. This
/// is what makes them cheap to share behind `Arc<dyn MigrationProvider>` and
/// easy to unit test.
pub trait MigrationProvider: Send + Sync {
    /// The provider this implementation implements.
    fn provider(&self) -> MigrationProviderType;

    /// The provider-neutral operations this provider supports.
    fn capabilities(&self) -> MigrationCapabilities;

    /// Returns `true` when this provider supports `operation`.
    fn supports(&self, operation: MigrationOperation) -> bool {
        self.capabilities().supports(operation)
    }

    /// Validates a context against this provider's requirements.
    ///
    /// # Errors
    ///
    /// Returns [`MigrationError::InvalidTarget`] when the working directory
    /// escapes the workspace and [`MigrationError::CommandMissing`] when the
    /// configured execution mode requires a command that is not configured.
    fn validate(
        &self,
        context: &MigrationContext,
        host: &dyn BuildHost,
    ) -> Result<(), MigrationError>;

    /// Constructs the structured plan for the context's operation.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`MigrationProvider::validate`] plus
    /// [`MigrationError::UnsupportedOperation`] when the operation is not
    /// supported and [`MigrationError::CommandMissing`] when no command can be
    /// resolved.
    fn plan(
        &self,
        context: &MigrationContext,
        host: &dyn BuildHost,
    ) -> Result<MigrationPlan, MigrationError>;
}

/// Rejects a context whose working directory escapes its workspace.
pub(crate) fn validate_inside_workspace(context: &MigrationContext) -> Result<(), MigrationError> {
    if context.is_inside_workspace() {
        Ok(())
    } else {
        Err(MigrationError::InvalidTarget {
            service: context.service_id.clone(),
            message: format!(
                "working directory '{}' is outside workspace root '{}'",
                context.working_dir, context.workspace_root
            ),
        })
    }
}

/// Resolves the working directory for a configured command.
///
/// A configured command may run in a subdirectory of the workspace; otherwise it
/// runs in the service's own directory. The resolved path is re-checked to stay
/// inside the workspace.
pub(crate) fn command_working_dir(
    context: &MigrationContext,
    working_dir: Option<&service_orchestrator_domain::RelativePath>,
) -> Result<AbsolutePath, MigrationError> {
    let Some(relative) = working_dir else {
        return Ok(context.working_dir.clone());
    };
    let root = context.workspace_root.as_str().trim_end_matches('/');
    let joined = format!("{root}/{}", relative.as_str());
    let path = AbsolutePath::new(joined).map_err(|error| MigrationError::InvalidTarget {
        service: context.service_id.clone(),
        message: format!("configured working directory is invalid: {error}"),
    })?;
    let inside = path.as_str() == root || path.as_str().starts_with(&format!("{root}/"));
    if !inside {
        return Err(MigrationError::InvalidTarget {
            service: context.service_id.clone(),
            message: format!(
                "configured working directory '{}' escapes workspace root '{root}'",
                path.as_str()
            ),
        });
    }
    Ok(path)
}

/// Merges `overlay` on top of `base`, returning the combined environment.
///
/// A variable declared by a command overrides the same variable declared on the
/// provider configuration, so a command can specialize the environment without
/// losing the rest.
pub(crate) fn merge_environment(base: &Environment, overlay: &Environment) -> Environment {
    let mut merged = base.clone();
    for variable in overlay.iter() {
        // `set` validates the key, which is already valid on the domain type.
        let _ = merged.set(variable.clone());
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_config::{MigrationCommandSet, MigrationDatabaseConfig};
    use service_orchestrator_domain::{EnvironmentVariable, MigrationTargetEnvironment};

    fn context() -> MigrationContext {
        MigrationContext {
            service_id: service_orchestrator_domain::ServiceId::new("auth-service").unwrap(),
            operation: MigrationOperation::Apply,
            provider: MigrationProviderType::Liquibase,
            execution_mode: service_orchestrator_domain::LiquibaseExecutionMode::Maven,
            working_dir: AbsolutePath::new("/w/acme/backend/auth-service").unwrap(),
            workspace_root: AbsolutePath::new("/w/acme").unwrap(),
            commands: MigrationCommandSet::default(),
            service_command: None,
            database: MigrationDatabaseConfig {
                environment: MigrationTargetEnvironment::Local,
                ..MigrationDatabaseConfig::default()
            },
            environment: Environment::new(),
            non_local_authorized: false,
        }
    }

    #[test]
    fn a_context_outside_its_workspace_is_rejected() {
        let escaping = MigrationContext {
            working_dir: AbsolutePath::new("/other/auth-service").unwrap(),
            ..context()
        };
        let error = validate_inside_workspace(&escaping).unwrap_err();
        assert_eq!(error.code(), "MIGRATION_INVALID_TARGET");
    }

    #[test]
    fn merging_lets_a_command_override_configuration() {
        let mut base = Environment::new();
        base.set(EnvironmentVariable::new("APP_ENV", "local").unwrap())
            .unwrap();
        base.set(EnvironmentVariable::new("LOG_LEVEL", "info").unwrap())
            .unwrap();
        let mut overlay = Environment::new();
        overlay
            .set(EnvironmentVariable::new("LOG_LEVEL", "debug").unwrap())
            .unwrap();

        let merged = merge_environment(&base, &overlay);
        assert_eq!(merged.get("APP_ENV").unwrap().value, "local");
        assert_eq!(merged.get("LOG_LEVEL").unwrap().value, "debug");
    }

    #[test]
    fn a_command_working_directory_is_resolved_and_contained() {
        use service_orchestrator_domain::RelativePath;
        let resolved =
            command_working_dir(&context(), Some(&RelativePath::new("backend").unwrap())).unwrap();
        assert_eq!(resolved.as_str(), "/w/acme/backend");

        // A traversal segment cannot even be represented as a relative path.
        assert!(RelativePath::new("../../etc").is_err());
    }
}
