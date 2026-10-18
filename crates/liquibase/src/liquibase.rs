//! The Liquibase migration provider.
//!
//! Liquibase is the initial concrete provider (`tasks/011.md` §4, §6). It knows
//! how to turn a validated [`MigrationContext`] into a structured
//! [`MigrationPlan`] for each provider-neutral operation, but it is still fully
//! provider-neutral from the outside: it declares its own capabilities, resolves
//! its own executable or Maven wrapper, and never spawns a process.
//!
//! # Execution modes
//!
//! - **Maven** (the default): Liquibase is invoked through the Maven Liquibase
//!   plugin. The provider resolves the Maven Wrapper (`./mvnw`) when it exists in
//!   the working directory and falls back to `mvn` from `PATH`, exactly as the
//!   [`MavenBuildProvider`](service_orchestrator_build::MavenBuildProvider) does.
//!   The plugin version is never hard-coded (`tasks/011.md` §6, §25).
//! - **CLI**: Liquibase is invoked through a CLI executable. Because the CLI is
//!   not guaranteed to be installed and its invocation flags are not universal,
//!   this mode is only usable when validated configuration supplies a command
//!   for the operation; otherwise the request is rejected with
//!   [`MigrationError::CommandMissing`] (`tasks/011.md` §6).
//!
//! # Configured commands win
//!
//! When configuration declares a command for the requested operation, it is
//! authoritative and used verbatim — still structured (a program plus an
//! argument vector, never a shell line). Only the provider's *default* Maven
//! plan needs Maven knowledge (`tasks/011.md` §8).
//!
//! # Secrets
//!
//! A plan carries environment *values*, which may be secrets; the plan's
//! [`Debug`](std::fmt::Debug) prints only their count
//! (`AGENTS.md` §31, `tasks/011.md` §11).

use service_orchestrator_application::MigrationError;
use service_orchestrator_domain::{
    CommandSpec, LiquibaseExecutionMode, MigrationOperation, MigrationProviderType,
};

use crate::context::{MigrationContext, MigrationPresetInvocation};
use crate::plan::{MigrationCapabilities, MigrationPlan};
use crate::provider::{
    command_working_dir, merge_environment, validate_inside_workspace, BuildHost, MigrationProvider,
};

/// The Maven Wrapper script, preferred over a system-wide Maven.
const WRAPPER: &str = "mvnw";

/// The system-wide Maven executable, resolved from `PATH`.
const MAVEN: &str = "mvn";

/// The Liquibase migration provider.
#[derive(Clone, Copy, Debug, Default)]
pub struct LiquibaseProvider;

impl LiquibaseProvider {
    /// Creates the provider.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Returns the configured command for the context's operation, if any.
    ///
    /// A per-operation command always wins. For the apply operation, the legacy
    /// `[services.commands] liquibase` command is a fallback so configuration
    /// written before the migration manager existed keeps working
    /// (`tasks/011.md` §8).
    fn command_for<'a>(&self, context: &'a MigrationContext) -> Option<&'a CommandSpec> {
        context
            .commands
            .get(context.operation)
            .or(match context.operation {
                MigrationOperation::Apply => context.service_command.as_ref(),
                _ => None,
            })
    }

    /// The conventional Maven Liquibase goals for a provider-neutral operation.
    ///
    /// `-B` runs Maven in batch mode so it never waits for an interactive prompt
    /// an interface cannot answer (`AGENTS.md` §9). Every supported operation has
    /// a goal, so the returned slice is never empty.
    fn maven_goals(operation: MigrationOperation) -> &'static [&'static str] {
        match operation {
            MigrationOperation::Validate => &["-B", "liquibase:validate"],
            MigrationOperation::Status => &["-B", "liquibase:status"],
            MigrationOperation::Pending => &["-B", "-Dliquibase.verbose=true", "liquibase:status"],
            MigrationOperation::Preview => &["-B", "liquibase:updateSQL"],
            MigrationOperation::Apply => &["-B", "liquibase:update"],
        }
    }

    /// Resolves the Maven executable or wrapper for the context's working
    /// directory.
    fn resolve_program(context: &MigrationContext, host: &dyn BuildHost) -> String {
        if host.file_exists(&context.working_dir, WRAPPER) {
            format!("./{WRAPPER}")
        } else {
            MAVEN.to_owned()
        }
    }

    /// Builds a plan from an explicitly configured command.
    fn plan_from_command(
        &self,
        context: &MigrationContext,
        command: &CommandSpec,
    ) -> Result<MigrationPlan, MigrationError> {
        command
            .validate()
            .map_err(|error| MigrationError::InvalidTarget {
                service: context.service_id.clone(),
                message: format!("the configured migration command is invalid: {error}"),
            })?;
        let working_dir = command_working_dir(context, command.working_dir.as_ref())?;
        let environment = merge_environment(&context.environment, &command.environment);
        Ok(MigrationPlan::new(
            self.provider(),
            context.operation,
            command.program.clone(),
            working_dir,
        )
        .with_args(command.args.clone())
        .with_environment(environment))
    }

    /// Builds the provider's default plan for the context's execution mode.
    fn default_plan(
        &self,
        context: &MigrationContext,
        host: &dyn BuildHost,
    ) -> Result<MigrationPlan, MigrationError> {
        match context.execution_mode {
            LiquibaseExecutionMode::Maven => {
                let program = Self::resolve_program(context, host);
                Ok(MigrationPlan::new(
                    self.provider(),
                    context.operation,
                    program,
                    context.working_dir.clone(),
                )
                .with_args(Self::maven_goals(context.operation).iter().copied())
                .with_environment(context.environment.clone()))
            }
            // The CLI is not assumed to exist and its flags are not universal:
            // configuration must supply the invocation (`tasks/011.md` §6).
            LiquibaseExecutionMode::Cli => Err(MigrationError::CommandMissing {
                operation: context.operation,
            }),
        }
    }

    /// Builds a plan from a named preset's structured goals.
    ///
    /// The goals are validated tokens composed into one Maven argument vector —
    /// never a shell line — with `-B` for non-interactive execution and the
    /// effective contexts (`tasks/032.md` §16–§18). Declaring a preset is not
    /// authorization: the manager has already revalidated the target and the
    /// caller's explicit confirmation (`tasks/032.md` §19).
    fn preset_plan(
        &self,
        context: &MigrationContext,
        preset: &MigrationPresetInvocation,
        host: &dyn BuildHost,
    ) -> Result<MigrationPlan, MigrationError> {
        if context.execution_mode != LiquibaseExecutionMode::Maven {
            return Err(MigrationError::InvalidConfiguration {
                service: context.service_id.clone(),
                message: format!(
                    "migration preset '{}' requires the Maven execution mode",
                    preset.id
                ),
            });
        }
        let program = Self::resolve_program(context, host);
        let mut args: Vec<String> = Vec::with_capacity(preset.goals.len() + 2);
        args.push("-B".to_owned());
        args.extend(preset.goals.iter().cloned());
        let contexts = preset.effective_contexts(&context.database.contexts);
        if !contexts.is_empty() {
            args.push(format!("-Dliquibase.contexts={}", contexts.join(",")));
        }
        Ok(MigrationPlan::new(
            self.provider(),
            context.operation,
            program,
            context.working_dir.clone(),
        )
        .with_args(args)
        .with_environment(context.environment.clone()))
    }
}

impl MigrationProvider for LiquibaseProvider {
    fn provider(&self) -> MigrationProviderType {
        MigrationProviderType::Liquibase
    }

    fn capabilities(&self) -> MigrationCapabilities {
        MigrationCapabilities::all()
    }

    fn validate(
        &self,
        context: &MigrationContext,
        _host: &dyn BuildHost,
    ) -> Result<(), MigrationError> {
        validate_inside_workspace(context)?;
        if !self.supports(context.operation) {
            return Err(MigrationError::UnsupportedOperation {
                provider: self.provider(),
                operation: context.operation,
            });
        }
        if let Some(preset) = &context.preset {
            // A preset composes Maven goals and cannot express a CLI
            // invocation, so the execution mode must be Maven
            // (`tasks/032.md` §16, §18).
            if context.execution_mode != LiquibaseExecutionMode::Maven {
                return Err(MigrationError::InvalidConfiguration {
                    service: context.service_id.clone(),
                    message: format!(
                        "migration preset '{}' requires the Maven execution mode",
                        preset.id
                    ),
                });
            }
            return Ok(());
        }
        match self.command_for(context) {
            Some(command) => command
                .validate()
                .map_err(|error| MigrationError::InvalidTarget {
                    service: context.service_id.clone(),
                    message: format!("the configured migration command is invalid: {error}"),
                }),
            None if context.execution_mode == LiquibaseExecutionMode::Cli => {
                Err(MigrationError::CommandMissing {
                    operation: context.operation,
                })
            }
            None => Ok(()),
        }
    }

    fn plan(
        &self,
        context: &MigrationContext,
        host: &dyn BuildHost,
    ) -> Result<MigrationPlan, MigrationError> {
        self.validate(context, host)?;
        let plan = match &context.preset {
            Some(preset) => self.preset_plan(context, preset, host)?,
            None => match self.command_for(context) {
                Some(command) => self.plan_from_command(context, command)?,
                None => self.default_plan(context, host)?,
            },
        };
        Ok(plan
            .with_target(context.database_identity(), context.target_key())
            .with_safety(context.safety()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_config::{MigrationCommandSet, MigrationDatabaseConfig};
    use service_orchestrator_domain::{
        AbsolutePath, Environment, MigrationTargetEnvironment, ServiceId,
    };

    /// A host whose answers the test controls.
    struct FakeHost {
        wrapper: bool,
    }

    impl FakeHost {
        fn without_wrapper() -> Self {
            Self { wrapper: false }
        }

        fn with_wrapper() -> Self {
            Self { wrapper: true }
        }
    }

    impl BuildHost for FakeHost {
        fn file_exists(&self, _directory: &AbsolutePath, name: &str) -> bool {
            self.wrapper && name == WRAPPER
        }

        fn environment_has(&self, _name: &str) -> bool {
            false
        }
    }

    fn context(operation: MigrationOperation) -> MigrationContext {
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
            database: MigrationDatabaseConfig {
                environment: MigrationTargetEnvironment::Local,
                ..MigrationDatabaseConfig::default()
            },
            environment: Environment::new(),
            non_local_authorized: false,
        }
    }

    fn reset_local() -> MigrationPresetInvocation {
        MigrationPresetInvocation {
            id: service_orchestrator_domain::MigrationPresetId::new("reset_local").unwrap(),
            goals: vec![
                "liquibase:dropAll".to_owned(),
                "liquibase:update".to_owned(),
            ],
            contexts: vec!["sql".to_owned(), "data".to_owned(), "onlyDev".to_owned()],
            destructive: true,
            read_only: false,
            allowed_environments: vec![MigrationTargetEnvironment::Local],
            timeout_seconds: None,
        }
    }

    #[test]
    fn declares_its_capabilities() {
        let provider = LiquibaseProvider::new();
        assert_eq!(provider.provider(), MigrationProviderType::Liquibase);
        for operation in [
            MigrationOperation::Validate,
            MigrationOperation::Status,
            MigrationOperation::Pending,
            MigrationOperation::Preview,
            MigrationOperation::Apply,
        ] {
            assert!(provider.supports(operation));
        }
    }

    #[test]
    fn prefers_the_wrapper_when_it_exists() {
        let provider = LiquibaseProvider::new();
        let plan = provider
            .plan(
                &context(MigrationOperation::Status),
                &FakeHost::with_wrapper(),
            )
            .unwrap();
        assert_eq!(plan.program, "./mvnw");
    }

    #[test]
    fn falls_back_to_the_system_maven() {
        let provider = LiquibaseProvider::new();
        let plan = provider
            .plan(
                &context(MigrationOperation::Apply),
                &FakeHost::without_wrapper(),
            )
            .unwrap();
        assert_eq!(plan.program, "mvn");
    }

    #[test]
    fn maps_operations_to_maven_goals() {
        let provider = LiquibaseProvider::new();
        let host = FakeHost::without_wrapper();
        for (operation, expected) in [
            (
                MigrationOperation::Validate,
                vec!["-B", "liquibase:validate"],
            ),
            (MigrationOperation::Status, vec!["-B", "liquibase:status"]),
            (
                MigrationOperation::Pending,
                vec!["-B", "-Dliquibase.verbose=true", "liquibase:status"],
            ),
            (
                MigrationOperation::Preview,
                vec!["-B", "liquibase:updateSQL"],
            ),
            (MigrationOperation::Apply, vec!["-B", "liquibase:update"]),
        ] {
            let plan = provider.plan(&context(operation), &host).unwrap();
            assert_eq!(plan.args, expected);
        }
    }

    #[test]
    fn a_configured_command_is_used_verbatim() {
        let provider = LiquibaseProvider::new();
        let commands = MigrationCommandSet {
            update: Some(CommandSpec::new("./mvnw", ["liquibase:update", "-Dverbose"]).unwrap()),
            ..MigrationCommandSet::default()
        };
        let context = MigrationContext {
            commands,
            ..context(MigrationOperation::Apply)
        };
        let plan = provider
            .plan(&context, &FakeHost::without_wrapper())
            .unwrap();
        assert_eq!(plan.program, "./mvnw");
        assert_eq!(plan.args, vec!["liquibase:update", "-Dverbose"]);
    }

    #[test]
    fn a_preset_composes_goals_and_contexts_structurally() {
        let provider = LiquibaseProvider::new();
        let context = MigrationContext {
            preset: Some(reset_local()),
            ..context(MigrationOperation::Apply)
        };
        let plan = provider
            .plan(&context, &FakeHost::without_wrapper())
            .unwrap();
        assert_eq!(plan.program, "mvn");
        assert_eq!(
            plan.args,
            vec![
                "-B",
                "liquibase:dropAll",
                "liquibase:update",
                "-Dliquibase.contexts=sql,data,onlyDev",
            ]
        );
        assert_eq!(plan.safety, crate::plan::MigrationSafety::Destructive);
        assert!(plan.safety.is_destructive());
    }

    #[test]
    fn a_preset_without_contexts_uses_the_service_contexts() {
        let provider = LiquibaseProvider::new();
        let context = MigrationContext {
            preset: Some(MigrationPresetInvocation {
                contexts: Vec::new(),
                destructive: false,
                ..reset_local()
            }),
            database: MigrationDatabaseConfig {
                contexts: vec!["sql".to_owned(), "data".to_owned()],
                ..context(MigrationOperation::Apply).database
            },
            ..context(MigrationOperation::Apply)
        };
        let plan = provider
            .plan(&context, &FakeHost::without_wrapper())
            .unwrap();
        assert_eq!(
            plan.args,
            vec![
                "-B",
                "liquibase:dropAll",
                "liquibase:update",
                "-Dliquibase.contexts=sql,data",
            ]
        );
    }

    #[test]
    fn a_preset_never_uses_the_per_operation_command() {
        let provider = LiquibaseProvider::new();
        let context = MigrationContext {
            preset: Some(MigrationPresetInvocation {
                goals: vec!["liquibase:update".to_owned()],
                contexts: Vec::new(),
                destructive: false,
                read_only: false,
                ..reset_local()
            }),
            commands: MigrationCommandSet {
                update: Some(CommandSpec::new("./mvnw", ["something-else"]).unwrap()),
                ..MigrationCommandSet::default()
            },
            ..context(MigrationOperation::Apply)
        };
        let plan = provider
            .plan(&context, &FakeHost::without_wrapper())
            .unwrap();
        assert_eq!(plan.args, vec!["-B", "liquibase:update"]);
    }

    #[test]
    fn a_preset_with_a_cli_execution_mode_is_rejected() {
        let provider = LiquibaseProvider::new();
        let context = MigrationContext {
            preset: Some(reset_local()),
            execution_mode: LiquibaseExecutionMode::Cli,
            ..context(MigrationOperation::Apply)
        };
        let error = provider
            .plan(&context, &FakeHost::without_wrapper())
            .unwrap_err();
        assert_eq!(error.code(), "MIGRATION_INVALID_CONFIGURATION");
    }

    #[test]
    fn a_status_preset_composes_a_read_only_plan() {
        let provider = LiquibaseProvider::new();
        let context = MigrationContext {
            preset: Some(MigrationPresetInvocation {
                id: service_orchestrator_domain::MigrationPresetId::new("status").unwrap(),
                goals: vec!["liquibase:status".to_owned()],
                contexts: Vec::new(),
                destructive: false,
                read_only: true,
                allowed_environments: Vec::new(),
                timeout_seconds: None,
            }),
            ..context(MigrationOperation::Status)
        };
        let plan = provider
            .plan(&context, &FakeHost::without_wrapper())
            .unwrap();
        assert_eq!(plan.args, vec!["-B", "liquibase:status"]);
        assert_eq!(plan.safety, crate::plan::MigrationSafety::ReadOnly);
    }

    #[test]
    fn the_legacy_service_command_is_used_for_apply() {
        let provider = LiquibaseProvider::new();
        let context = MigrationContext {
            service_command: Some(CommandSpec::new("liquibase", ["update"]).unwrap()),
            ..context(MigrationOperation::Apply)
        };
        let plan = provider
            .plan(&context, &FakeHost::without_wrapper())
            .unwrap();
        assert_eq!(plan.program, "liquibase");
        assert_eq!(plan.args, vec!["update"]);
    }

    #[test]
    fn a_cli_request_without_a_command_is_rejected() {
        let provider = LiquibaseProvider::new();
        let context = MigrationContext {
            execution_mode: LiquibaseExecutionMode::Cli,
            ..context(MigrationOperation::Status)
        };
        let error = provider
            .plan(&context, &FakeHost::without_wrapper())
            .unwrap_err();
        assert_eq!(error.code(), "MIGRATION_COMMAND_MISSING");
    }

    #[test]
    fn a_cli_request_with_a_command_is_planned() {
        let provider = LiquibaseProvider::new();
        let commands = MigrationCommandSet {
            status: Some(CommandSpec::new("liquibase", ["status"]).unwrap()),
            ..MigrationCommandSet::default()
        };
        let context = MigrationContext {
            execution_mode: LiquibaseExecutionMode::Cli,
            commands,
            ..context(MigrationOperation::Status)
        };
        let plan = provider
            .plan(&context, &FakeHost::without_wrapper())
            .unwrap();
        assert_eq!(plan.program, "liquibase");
        assert_eq!(plan.args, vec!["status"]);
    }

    #[test]
    fn the_plan_carries_the_target_identity_and_safety() {
        let provider = LiquibaseProvider::new();
        let plan = provider
            .plan(
                &context(MigrationOperation::Apply),
                &FakeHost::without_wrapper(),
            )
            .unwrap();
        assert_eq!(plan.safety, crate::plan::MigrationSafety::LocalWrite);
        assert!(plan.database_identity.contains("local"));
        assert!(!plan.target_key.is_empty());
    }

    #[test]
    fn planning_does_not_spawn_a_process() {
        // The wrapper program does not exist; if planning tried to run it, this
        // would fail. It cannot, because the provider is a pure mapping.
        let provider = LiquibaseProvider::new();
        let plan = provider
            .plan(
                &context(MigrationOperation::Apply),
                &FakeHost::without_wrapper(),
            )
            .unwrap();
        assert_eq!(provider.capabilities(), MigrationCapabilities::all());
        assert!(!plan.command().contains("hunter2"));
    }
}
