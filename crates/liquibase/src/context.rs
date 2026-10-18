//! The execution-time migration context.
//!
//! A [`MigrationContext`] is everything a [`MigrationProvider`](crate::MigrationProvider)
//! needs to validate a request and build a [`MigrationPlan`](crate::MigrationPlan):
//! the provider-neutral operation, the provider and execution mode selected in
//! configuration, the resolved working directory, the per-operation commands,
//! the declared target-database metadata, and the environment variables to apply
//! to the child process.
//!
//! It is assembled by the [`MigrationManager`](crate::MigrationManager) from
//! validated [`Config`](service_orchestrator_config::Config) at execution time.
//! It is deliberately **not** carried on a task: task parameters are stored and
//! may be surfaced through an interface, so they hold only identity, never a
//! credential (`AGENTS.md` §31, `tasks/011.md` §11).
//!
//! # Debug is redacted
//!
//! The context holds environment *values* (which may be secrets) and command
//! specifications. Its [`Debug`] implementation prints only display-safe fields,
//! so an accidental `{:?}` can never leak a credential.

use std::fmt;

use service_orchestrator_config::{MigrationCommandSet, MigrationDatabaseConfig};
use service_orchestrator_domain::{
    AbsolutePath, CommandSpec, Environment, LiquibaseExecutionMode, MigrationOperation,
    MigrationPresetId, MigrationProviderType, ServiceId,
};

use crate::plan::MigrationSafety;

/// A named migration preset resolved from validated configuration at execution
/// time (`tasks/032.md` §16–§21).
///
/// The context carries the *resolved* preset, never an authorization: the
/// safety rules are revalidated for every execution, and the `destructive`
/// flag only classifies the plan — it never grants permission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationPresetInvocation {
    /// The preset identifier.
    pub id: MigrationPresetId,
    /// The Liquibase goals to run, in order.
    pub goals: Vec<String>,
    /// The Liquibase contexts declared by the preset. When empty, the
    /// service's declared contexts apply.
    pub contexts: Vec<String>,
    /// Whether the preset destroys database objects.
    pub destructive: bool,
    /// Whether the preset only inspects the database.
    pub read_only: bool,
    /// The environments a destructive preset may target. Never a substitute for
    /// the execution-time authorization.
    pub allowed_environments: Vec<service_orchestrator_domain::MigrationTargetEnvironment>,
    /// The preset's timeout, in seconds.
    pub timeout_seconds: Option<u64>,
}

impl MigrationPresetInvocation {
    /// Returns `true` when the preset may be executed against `environment`.
    ///
    /// A non-destructive preset has no environment restriction. A destructive
    /// preset only runs against a declared, environment-classified target on
    /// its list; the list is a precondition, never authorization
    /// (`tasks/032.md` §19).
    #[must_use]
    pub fn allows_environment(
        &self,
        environment: &service_orchestrator_domain::MigrationTargetEnvironment,
    ) -> bool {
        !self.destructive || self.allowed_environments.contains(environment)
    }

    /// Returns the contexts that apply to the service.
    ///
    /// A preset with declared contexts overrides the service's list; otherwise
    /// the service's declared contexts apply (`tasks/032.md` §17).
    #[must_use]
    pub fn effective_contexts<'a>(&'a self, service_contexts: &'a [String]) -> &'a [String] {
        if self.contexts.is_empty() {
            service_contexts
        } else {
            &self.contexts
        }
    }
}

/// The resolved, execution-time description of a migration.
#[derive(Clone, PartialEq, Eq)]
pub struct MigrationContext {
    /// The service whose database is migrated.
    pub service_id: ServiceId,
    /// The provider-neutral operation to perform.
    pub operation: MigrationOperation,
    /// The provider selected in configuration.
    pub provider: MigrationProviderType,
    /// How the provider is invoked (Maven or CLI).
    pub execution_mode: LiquibaseExecutionMode,
    /// The absolute working directory of the service.
    pub working_dir: AbsolutePath,
    /// The workspace root, used to keep the working directory contained.
    pub workspace_root: AbsolutePath,
    /// The per-operation configured commands (authoritative when set).
    pub commands: MigrationCommandSet,
    /// The named preset selected for this migration, when the caller chose one.
    /// A preset composes the goals and replaces the per-operation commands
    /// (`tasks/032.md` §16, §21).
    pub preset: Option<MigrationPresetInvocation>,
    /// The legacy `[services.commands] liquibase` command, used for the apply
    /// operation when no per-operation apply command is configured.
    pub service_command: Option<CommandSpec>,
    /// The declared target-database metadata (never credentials).
    pub database: MigrationDatabaseConfig,
    /// Environment variables to apply to the migration process.
    pub environment: Environment,
    /// Whether the caller explicitly authorized a non-local target.
    pub non_local_authorized: bool,
}

impl MigrationContext {
    /// Returns the display-safe identity of the target database.
    #[must_use]
    pub fn database_identity(&self) -> String {
        self.database.display_identity()
    }

    /// Returns the stable key identifying the physical database target.
    ///
    /// Migrations that share this key are serialized by the
    /// [`DatabaseCoordinator`](crate::DatabaseCoordinator); see
    /// [`MigrationDatabaseConfig::target_key`].
    #[must_use]
    pub fn target_key(&self) -> String {
        self.database.target_key()
    }

    /// Returns `true` when the working directory stays inside the workspace root.
    #[must_use]
    pub fn is_inside_workspace(&self) -> bool {
        let root = self.workspace_root.as_str().trim_end_matches('/');
        let path = self.working_dir.as_str();
        path == root || path.starts_with(&format!("{root}/"))
    }

    /// Classifies the risk of running this operation against this target.
    #[must_use]
    pub fn safety(&self) -> MigrationSafety {
        if !self.operation.is_mutating() {
            return MigrationSafety::ReadOnly;
        }
        if self
            .preset
            .as_ref()
            .is_some_and(|preset| preset.destructive)
        {
            return MigrationSafety::Destructive;
        }
        if self.database.environment.is_local() {
            MigrationSafety::LocalWrite
        } else {
            MigrationSafety::AuthorizedWrite
        }
    }
}

impl fmt::Debug for MigrationContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MigrationContext")
            .field("service_id", &self.service_id)
            .field("operation", &self.operation)
            .field("provider", &self.provider)
            .field("execution_mode", &self.execution_mode)
            .field("working_dir", &self.working_dir)
            .field("database", &self.database_identity())
            .field("environment_variables", &self.environment.len())
            .field("preset", &self.preset.as_ref().map(|preset| &preset.id))
            .field("non_local_authorized", &self.non_local_authorized)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_domain::{EnvironmentVariable, MigrationTargetEnvironment};

    fn context(
        operation: MigrationOperation,
        environment: MigrationTargetEnvironment,
    ) -> MigrationContext {
        let database = MigrationDatabaseConfig {
            environment,
            ..MigrationDatabaseConfig::default()
        };
        MigrationContext {
            service_id: ServiceId::new("auth-service").unwrap(),
            operation,
            provider: MigrationProviderType::Liquibase,
            execution_mode: LiquibaseExecutionMode::Maven,
            working_dir: AbsolutePath::new("/w/acme/backend/auth-service").unwrap(),
            workspace_root: AbsolutePath::new("/w/acme").unwrap(),
            commands: MigrationCommandSet::default(),
            preset: None,
            service_command: None,
            database,
            environment: Environment::default(),
            non_local_authorized: false,
        }
    }

    #[test]
    fn read_only_operations_are_always_safe() {
        for environment in [
            MigrationTargetEnvironment::Local,
            MigrationTargetEnvironment::Production,
            MigrationTargetEnvironment::Unknown,
        ] {
            assert_eq!(
                context(MigrationOperation::Status, environment).safety(),
                MigrationSafety::ReadOnly
            );
        }
    }

    #[test]
    fn applying_to_a_local_target_is_a_local_write() {
        assert_eq!(
            context(MigrationOperation::Apply, MigrationTargetEnvironment::Local).safety(),
            MigrationSafety::LocalWrite
        );
    }

    #[test]
    fn applying_to_a_remote_target_is_an_authorized_write() {
        assert_eq!(
            context(
                MigrationOperation::Apply,
                MigrationTargetEnvironment::Production
            )
            .safety(),
            MigrationSafety::AuthorizedWrite
        );
    }

    #[test]
    fn a_destructive_preset_is_always_destructive() {
        let preset = MigrationPresetInvocation {
            id: MigrationPresetId::new("reset_local").unwrap(),
            goals: vec![
                "liquibase:dropAll".to_owned(),
                "liquibase:update".to_owned(),
            ],
            contexts: Vec::new(),
            destructive: true,
            read_only: false,
            allowed_environments: vec![MigrationTargetEnvironment::Local],
            timeout_seconds: None,
        };
        let mut context = context(MigrationOperation::Apply, MigrationTargetEnvironment::Local);
        context.preset = Some(preset.clone());
        assert_eq!(context.safety(), MigrationSafety::Destructive);
        assert!(context.safety().is_destructive());
        // A read-only operation never becomes destructive, even with a
        // destructive preset selected: the application rejects that pairing.
        context.operation = MigrationOperation::Status;
        assert_eq!(context.safety(), MigrationSafety::ReadOnly);
    }

    #[test]
    fn a_preset_without_contexts_inherits_the_service_contexts() {
        let service_contexts = vec!["sql".to_owned(), "data".to_owned()];
        let inheriting = MigrationPresetInvocation {
            id: MigrationPresetId::new("update").unwrap(),
            goals: vec!["liquibase:update".to_owned()],
            contexts: Vec::new(),
            destructive: false,
            read_only: false,
            allowed_environments: Vec::new(),
            timeout_seconds: None,
        };
        assert_eq!(
            inheriting.effective_contexts(&service_contexts),
            &service_contexts
        );

        let overriding = MigrationPresetInvocation {
            contexts: vec!["onlyDev".to_owned()],
            ..inheriting
        };
        assert_eq!(
            overriding.effective_contexts(&service_contexts),
            ["onlyDev"]
        );
    }

    #[test]
    fn detects_paths_outside_the_workspace() {
        assert!(
            context(MigrationOperation::Apply, MigrationTargetEnvironment::Local)
                .is_inside_workspace()
        );
        let escaping = MigrationContext {
            working_dir: AbsolutePath::new("/other/auth-service").unwrap(),
            ..context(MigrationOperation::Apply, MigrationTargetEnvironment::Local)
        };
        assert!(!escaping.is_inside_workspace());
    }

    #[test]
    fn debug_output_never_contains_a_secret() {
        let mut context = context(MigrationOperation::Apply, MigrationTargetEnvironment::Local);
        context
            .environment
            .set(EnvironmentVariable::secret("DB_PASSWORD", "hunter2").unwrap())
            .unwrap();
        let rendered = format!("{context:?}");
        assert!(!rendered.contains("hunter2"));
        assert!(rendered.contains("environment_variables"));
    }
}
