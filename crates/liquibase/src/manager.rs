//! The Migration Manager.
//!
//! [`MigrationManager`] is the provider-agnostic orchestrator for database
//! migrations. Given a workspace-aware
//! [`ResolvedMigrationTarget`](service_orchestrator_application::ResolvedMigrationTarget),
//! it:
//!
//! 1. reads the service's validated migration configuration from
//!    [`Config`] at execution time (never from the task parameters, so no
//!    credential is ever stored on a task);
//! 2. applies the environment safety policy: only an explicitly configured local
//!    target may mutate a database without explicit authorization
//!    (`tasks/011.md` §10);
//! 3. resolves the [`MigrationProvider`] and rejects an operation it does not
//!    support;
//! 4. validates the target and asks the provider for a structured
//!    [`MigrationPlan`];
//! 5. acquires the database-target coordination lock so two writers never race
//!    over one configured database (`tasks/011.md` §15); and
//! 6. hands the plan to the [`MigrationRunner`], which executes it through the
//!    process manager.
//!
//! It contains no Liquibase-specific knowledge — every such detail lives in a
//! provider — and no scheduling logic: a migration is always run as a task, and
//! the task engine remains the only concurrency authority (`AGENTS.md` §9, §21,
//! `tasks/011.md` §3).
//!
//! # Failure semantics
//!
//! A failed migration never claims the database was rolled back
//! (`tasks/011.md` §17). A lock-related failure is recognized and reported as
//! [`MigrationError::LockConflict`]; the manager never releases a Liquibase lock
//! itself (`tasks/011.md` §16). The manager never retries a failed apply: the
//! task is submitted with [`RetryPolicy::none`](service_orchestrator_domain),
//! and a retry requires a new, explicitly inspected operation
//! (`tasks/011.md` §19).

use std::sync::Arc;
use std::time::Duration;

use service_orchestrator_application::{MigrationError, ResolvedMigrationTarget};
use service_orchestrator_config::{Config, ConfigManager, LiquibaseConfig, StaticConfigManager};
use service_orchestrator_domain::{
    redact, CommandKind, Environment, MigrationOperation, MigrationProviderType,
    MigrationTargetEnvironment, ServiceId,
};
use service_orchestrator_execution::Cancellation;
use service_orchestrator_process::{ExitReason, ProcessError, ProcessManager};

use crate::context::{MigrationContext, MigrationPresetInvocation};
use crate::coordinator::DatabaseCoordinator;
use crate::plan::MigrationPlan;
use crate::provider::{BuildHost, SystemBuildHost};
use crate::registry::MigrationProviderRegistry;
use crate::runner::{CommandOutput, MigrationRunner};

/// The maximum number of bytes of migration output kept as failure detail.
const MAX_DETAIL_BYTES: usize = 4096;

/// Substrings (lower-cased) that identify a Liquibase database-lock conflict.
///
/// Liquibase's human-readable messages differ across versions, so this is only
/// used to *classify a failure for diagnostics* — never to drive behavior. The
/// manager never releases a lock (`tasks/011.md` §16, §24).
const LOCK_MARKERS: &[&str] = &[
    "could not acquire change log lock",
    "lockexception",
    "waiting for changelog lock",
];

/// Substrings (lower-cased) whose lines are dropped from any captured output.
///
/// A migration tool may echo connection details on failure; dropping the whole
/// line is a conservative way to guarantee no credential reaches a log, a task
/// result, or an interface (`AGENTS.md` §31, `tasks/011.md` §11).
const SECRET_MARKERS: &[&str] = &["jdbc:", "password=", "pwd=", "--password"];

/// The result of a successful migration invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationOutcome {
    /// The service whose database was migrated.
    pub service_id: ServiceId,
    /// The provider that ran.
    pub provider: MigrationProviderType,
    /// The operation that ran.
    pub operation: MigrationOperation,
    /// A display-safe identity of the target database.
    pub database_identity: String,
    /// The process exit code, when the platform reported one.
    pub exit_code: Option<i32>,
    /// A bounded, credential-free capture of standard output.
    pub stdout: String,
    /// A bounded, credential-free capture of standard error.
    pub stderr: String,
}

/// Orchestrates migration plans across providers.
pub struct MigrationManager {
    config: Arc<dyn ConfigManager>,
    registry: Arc<MigrationProviderRegistry>,
    host: Arc<dyn BuildHost>,
    runner: MigrationRunner,
    coordinator: DatabaseCoordinator,
}

impl MigrationManager {
    /// Creates a manager over a process manager and validated configuration.
    ///
    /// The standard provider registry (Liquibase) and the system host are used.
    #[must_use]
    pub fn new(processes: ProcessManager, config: Arc<Config>) -> Self {
        Self::with_config_manager(
            processes,
            Arc::new(StaticConfigManager::new((*config).clone())),
            MigrationProviderRegistry::standard(),
        )
    }

    /// Creates a manager over an explicit provider registry.
    #[must_use]
    pub fn with_providers(
        processes: ProcessManager,
        config: Arc<Config>,
        registry: MigrationProviderRegistry,
    ) -> Self {
        Self::with_config_manager(
            processes,
            Arc::new(StaticConfigManager::new((*config).clone())),
            registry,
        )
    }

    /// Replaces the configuration manager with a shared, live one.
    ///
    /// The daemon wires the single configuration manager here so a committed
    /// change is observed by the next migration operation
    /// (`tasks/023.md` §39, §40).
    #[must_use]
    pub fn with_shared_config(mut self, config: Arc<dyn ConfigManager>) -> Self {
        self.config = config;
        self
    }

    /// Creates a manager over a shared, live configuration.
    ///
    /// Every operation reads the manager's current snapshot, so a committed
    /// configuration change is honoured immediately (`tasks/023.md` §39, §40).
    #[must_use]
    pub fn with_config_manager(
        processes: ProcessManager,
        config: Arc<dyn ConfigManager>,
        registry: MigrationProviderRegistry,
    ) -> Self {
        Self {
            config,
            registry: Arc::new(registry),
            host: Arc::new(SystemBuildHost::new()),
            runner: MigrationRunner::new(processes),
            coordinator: DatabaseCoordinator::new(),
        }
    }

    /// Replaces the host providers plan against.
    ///
    /// This is primarily a testing seam: a host whose filesystem and environment
    /// answers are controlled makes provider behavior deterministic.
    #[must_use]
    pub fn with_host(mut self, host: Arc<dyn BuildHost>) -> Self {
        self.host = host;
        self
    }

    /// Replaces the database coordinator.
    ///
    /// Sharing one coordinator across managers serializes writes to the same
    /// configured database while leaving unrelated migrations concurrent
    /// (`tasks/011.md` §15).
    #[must_use]
    pub fn with_coordinator(mut self, coordinator: DatabaseCoordinator) -> Self {
        self.coordinator = coordinator;
        self
    }

    /// Returns the provider registry, for observability.
    #[must_use]
    pub fn registry(&self) -> &Arc<MigrationProviderRegistry> {
        &self.registry
    }

    /// Builds the execution-time context for `target` from validated
    /// configuration.
    ///
    /// # Errors
    ///
    /// Returns [`MigrationError::NotConfigured`] when the service has no
    /// migration configuration, [`MigrationError::Disabled`] when migrations are
    /// disabled, and [`MigrationError::InvalidTarget`] when the workspace or
    /// service is unknown.
    pub fn context(
        &self,
        target: &ResolvedMigrationTarget,
    ) -> Result<MigrationContext, MigrationError> {
        let service_id = &target.service_id;
        let config = self.config.current();
        let workspace = config.workspace(&target.workspace_id).ok_or_else(|| {
            MigrationError::InvalidTarget {
                service: service_id.clone(),
                message: format!("workspace '{}' is not configured", target.workspace_id),
            }
        })?;
        let liquibase =
            workspace
                .liquibase
                .get(service_id)
                .ok_or_else(|| MigrationError::NotConfigured {
                    service: service_id.clone(),
                })?;
        if !liquibase.enabled {
            return Err(MigrationError::Disabled {
                service: service_id.clone(),
            });
        }
        let service = workspace.workspace.service(service_id).ok_or_else(|| {
            MigrationError::InvalidTarget {
                service: service_id.clone(),
                message: format!("service '{service_id}' is not declared in the workspace"),
            }
        })?;

        let environment = merged_environment(workspace, service, liquibase);
        let service_command = service.commands.get(&CommandKind::Liquibase).cloned();

        // Resolve the named preset, if the caller selected one, from the
        // *current* configuration: the task carries only its identifier, so a
        // commit or a re-read is honoured and no authorization state is ever
        // stored on the task (`tasks/032.md` §19, §21).
        let preset = match &target.preset {
            Some(id) => {
                let preset =
                    config
                        .migration_preset(id)
                        .ok_or_else(|| MigrationError::PresetNotFound {
                            service: service_id.clone(),
                            preset: id.clone(),
                        })?;
                Some(MigrationPresetInvocation {
                    id: id.clone(),
                    goals: preset.goals.clone(),
                    contexts: preset.contexts.clone(),
                    destructive: preset.destructive,
                    read_only: preset.read_only,
                    allowed_environments: preset.allowed_environments.clone(),
                    timeout_seconds: preset.timeout_seconds,
                })
            }
            None => None,
        };

        Ok(MigrationContext {
            service_id: service_id.clone(),
            operation: target.operation,
            provider: liquibase.provider.clone(),
            execution_mode: liquibase.execution_mode,
            working_dir: target.path.clone(),
            workspace_root: target.workspace_root.clone(),
            commands: liquibase.commands.clone(),
            preset,
            service_command,
            database: liquibase.database.clone(),
            environment,
            non_local_authorized: target.non_local_authorized,
        })
    }

    /// Enforces the environment safety policy for a context.
    ///
    /// Read-only operations and mutations of an explicitly configured local
    /// database are allowed. A mutation of any other target requires the
    /// caller's explicit authorization; an unidentified target is reported as
    /// [`MigrationError::DatabaseTargetUnknown`] so the interface can prompt for
    /// the missing declaration (`tasks/011.md` §10, §27).
    ///
    /// A named preset is revalidated here, immediately before planning or
    /// execution: a destructive preset always requires explicit authorization —
    /// even against a local database — and its `allowed_environments` list must
    /// cover the target. The list never grants authorization by itself
    /// (`tasks/032.md` §19).
    ///
    /// # Errors
    ///
    /// Returns [`MigrationError::DatabaseTargetUnknown`] or
    /// [`MigrationError::DatabaseTargetNotAuthorized`] when a mutating operation
    /// targets a non-local database the caller did not authorize, and
    /// [`MigrationError::PresetNotAllowed`] or
    /// [`MigrationError::InvalidConfiguration`] when a selected preset does not
    /// cover the target or the operation.
    pub fn check_safety(&self, context: &MigrationContext) -> Result<(), MigrationError> {
        if let Some(preset) = &context.preset {
            if preset.read_only && context.operation.is_mutating() {
                return Err(MigrationError::InvalidConfiguration {
                    service: context.service_id.clone(),
                    message: format!(
                        "migration preset '{}' is read-only and cannot run the '{}' operation",
                        preset.id,
                        context.operation.label()
                    ),
                });
            }
            if preset.destructive {
                if !context.operation.is_mutating() {
                    return Err(MigrationError::InvalidConfiguration {
                        service: context.service_id.clone(),
                        message: format!(
                            "destructive migration preset '{}' requires a mutating operation",
                            preset.id
                        ),
                    });
                }
                if !preset.allows_environment(&context.database.environment) {
                    return Err(match &context.database.environment {
                        MigrationTargetEnvironment::Unknown => {
                            MigrationError::DatabaseTargetUnknown {
                                service: context.service_id.clone(),
                            }
                        }
                        environment => MigrationError::PresetNotAllowed {
                            service: context.service_id.clone(),
                            preset: preset.id.clone(),
                            environment: environment.clone(),
                        },
                    });
                }
                if !context.non_local_authorized {
                    return Err(MigrationError::DatabaseTargetNotAuthorized {
                        service: context.service_id.clone(),
                        environment: context.database.environment.clone(),
                    });
                }
                return Ok(());
            }
        }
        if !context.operation.is_mutating() {
            return Ok(());
        }
        if context.database.environment.is_local() || context.non_local_authorized {
            return Ok(());
        }
        match &context.database.environment {
            MigrationTargetEnvironment::Unknown => Err(MigrationError::DatabaseTargetUnknown {
                service: context.service_id.clone(),
            }),
            environment => Err(MigrationError::DatabaseTargetNotAuthorized {
                service: context.service_id.clone(),
                environment: environment.clone(),
            }),
        }
    }

    /// Resolves, validates, and builds the plan for `target`.
    ///
    /// # Errors
    ///
    /// Returns a structured [`MigrationError`] for a missing or disabled
    /// configuration, an unauthorized target, an unregistered provider, an
    /// unsupported operation, or an invalid plan.
    pub fn plan(&self, target: &ResolvedMigrationTarget) -> Result<MigrationPlan, MigrationError> {
        let context = self.context(target)?;
        self.check_safety(&context)?;
        let provider = self.registry.resolve(&context.provider)?;
        if !provider.supports(context.operation) {
            return Err(MigrationError::UnsupportedOperation {
                provider: context.provider.clone(),
                operation: context.operation,
            });
        }
        provider.validate(&context, self.host.as_ref())?;
        provider.plan(&context, self.host.as_ref())
    }

    /// Executes `target`, stopping it if `cancellation` is signalled.
    ///
    /// `timeout`, when present, is the task's deadline and takes precedence over
    /// the plan's own declared timeout.
    ///
    /// Migrations against the same configured database are serialized: the
    /// manager holds the target's coordination lock for the duration of the run,
    /// so two writers never race over one database. Unrelated targets are
    /// unaffected (`tasks/011.md` §15). The lock is in-process only; see
    /// [`DatabaseCoordinator`].
    ///
    /// # Errors
    ///
    /// Returns the structured [`MigrationError`] for an invalid or unauthorized
    /// target, a process failure, a non-zero exit, a lock conflict, a timeout, or
    /// cancellation.
    pub async fn execute(
        &self,
        target: &ResolvedMigrationTarget,
        timeout: Option<Duration>,
        cancellation: Option<Cancellation>,
    ) -> Result<MigrationOutcome, MigrationError> {
        let context = self.context(target)?;
        self.check_safety(&context)?;
        let provider = self.registry.resolve(&context.provider)?;
        if !provider.supports(context.operation) {
            return Err(MigrationError::UnsupportedOperation {
                provider: context.provider.clone(),
                operation: context.operation,
            });
        }
        provider.validate(&context, self.host.as_ref())?;
        let plan = provider.plan(&context, self.host.as_ref())?;
        let effective_timeout = timeout.or(plan.timeout);

        let _coordination = self.coordinator.acquire(&plan.target_key).await;
        let output = self
            .runner
            .run(&plan, timeout, cancellation)
            .await
            .map_err(|error| map_process_error(&context, error))?;
        classify(&context, output, effective_timeout)
    }
}

impl std::fmt::Debug for MigrationManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MigrationManager")
            .field("providers", &self.registry.providers().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// Merges workspace, service, and Liquibase environment layers.
///
/// Later layers override earlier ones. Secret values remain redacted whenever the
/// environment is displayed or serialized (`AGENTS.md` §31).
fn merged_environment(
    workspace: &service_orchestrator_config::ConfiguredWorkspace,
    service: &service_orchestrator_domain::Service,
    liquibase: &LiquibaseConfig,
) -> Environment {
    let mut environment = workspace.environment.clone();
    for variable in service.environment.iter() {
        let _ = environment.set(variable.clone());
    }
    for variable in liquibase.environment.iter() {
        let _ = environment.set(variable.clone());
    }
    environment
}

/// Maps a process-manager failure into a structured migration error.
fn map_process_error(context: &MigrationContext, error: ProcessError) -> MigrationError {
    match error {
        ProcessError::Spawn { message, .. } => MigrationError::Process {
            service: context.service_id.clone(),
            message: format!("could not start the migration: {message}"),
        },
        other => MigrationError::internal(other.to_string()),
    }
}

/// Turns a completed process into a migration outcome or a structured failure.
fn classify(
    context: &MigrationContext,
    output: CommandOutput,
    timeout: Option<Duration>,
) -> Result<MigrationOutcome, MigrationError> {
    match output.reason {
        ExitReason::Cancelled => Err(MigrationError::Cancelled {
            service: context.service_id.clone(),
            operation: context.operation,
        }),
        ExitReason::TimedOut => Err(MigrationError::Timeout {
            service: context.service_id.clone(),
            operation: context.operation,
            timeout_seconds: timeout.map(|value| value.as_secs()).unwrap_or(0),
        }),
        _ if output.succeeded() => Ok(MigrationOutcome {
            service_id: context.service_id.clone(),
            provider: context.provider.clone(),
            operation: context.operation,
            database_identity: context.database_identity(),
            exit_code: output.code,
            stdout: bound(&scrub(&output.stdout)),
            stderr: bound(&scrub(&output.stderr)),
        }),
        _ if is_lock_conflict(&output) => Err(MigrationError::LockConflict {
            service: context.service_id.clone(),
            message: "Liquibase reported that the change-log lock could not be acquired; \
                      another migration may be running. No lock was released automatically."
                .to_owned(),
        }),
        _ => Err(MigrationError::Failed {
            service: context.service_id.clone(),
            operation: context.operation,
            exit_code: output.code,
            message: failure_message(&output),
            detail: detail(&output),
        }),
    }
}

/// Returns `true` when the process output names a Liquibase lock conflict.
fn is_lock_conflict(output: &CommandOutput) -> bool {
    let combined = format!("{}\n{}", output.stderr, output.stdout).to_lowercase();
    LOCK_MARKERS.iter().any(|marker| combined.contains(marker))
}

/// A concise, credential-free summary of a failed migration.
fn failure_message(output: &CommandOutput) -> String {
    if output.truncated {
        return "migration output exceeded the capture buffer and could not be read completely"
            .to_owned();
    }
    let first = output
        .stderr
        .lines()
        .chain(output.stdout.lines())
        .map(str::trim)
        .find(|line| !line.is_empty() && !line_is_secret(line))
        .unwrap_or("");
    if first.is_empty() {
        match output.code {
            Some(code) => format!("migration exited with status {code}"),
            None => "migration failed before reporting an exit status".to_owned(),
        }
    } else {
        bound(first)
    }
}

/// Additional bounded, credential-free detail for a failed migration.
fn detail(output: &CommandOutput) -> Option<String> {
    let combined = if output.stderr.trim().is_empty() {
        output.stdout.trim()
    } else {
        output.stderr.trim()
    };
    let redacted = scrub(combined);
    if redacted.trim().is_empty() {
        None
    } else {
        Some(bound(redacted.trim()))
    }
}

/// Removes any line that looks like it carries a credential and then scrubs
/// secret-looking values from what remains.
///
/// Two layers are used deliberately: dropping a whole line when it names a
/// JDBC URL or a password flag is the surest way to remove a connection string,
/// while the shared [`redact`] rule catches `key=value` and credential-bearing
/// URL shapes inside lines that survive (`AGENTS.md` §31).
fn scrub(text: &str) -> String {
    text.lines()
        .filter(|line| !line_is_secret(line))
        .map(redact)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Returns `true` when a line contains a known credential marker.
fn line_is_secret(line: &str) -> bool {
    let lower = line.to_lowercase();
    SECRET_MARKERS.iter().any(|marker| lower.contains(marker))
}

/// Bounds `text` to [`MAX_DETAIL_BYTES`] on a character boundary.
fn bound(text: &str) -> String {
    if text.len() <= MAX_DETAIL_BYTES {
        return text.to_owned();
    }
    let mut end = MAX_DETAIL_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_config::load_from_str;
    use service_orchestrator_domain::{AbsolutePath, WorkspaceId};

    /// A configuration with one local, Maven-mode service.
    const CONFIG: &str = r#"
config_version = 1

[[workspaces]]
id = "acme"
root = "/tmp/acme"

[[services]]
id = "auth-service"
type = "backend"
workspace = "acme"
path = "backend/auth-service"

[services.commands]
build = "./mvnw clean install"
start = "./mvnw spring-boot:run"

[services.liquibase]
enabled = true

[services.liquibase.commands]
status = { program = "/bin/echo", args = ["status"] }
update = { program = "/bin/echo", args = ["applied"] }

[services.liquibase.database]
environment = "local"
"#;

    fn manager(text: &str) -> MigrationManager {
        let config = Arc::new(load_from_str(text, "<test>").expect("config should load"));
        MigrationManager::new(ProcessManager::new().unwrap(), config)
    }

    fn target(operation: MigrationOperation) -> ResolvedMigrationTarget {
        ResolvedMigrationTarget {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            service_id: ServiceId::new("auth-service").unwrap(),
            operation,
            preset: None,
            // `/tmp` exists on every supported platform, so a migration process
            // can actually be spawned; the build runner tests use the same.
            path: AbsolutePath::new("/tmp").unwrap(),
            workspace_root: AbsolutePath::new("/tmp").unwrap(),
            non_local_authorized: false,
        }
    }

    #[tokio::test]
    async fn read_only_operations_never_require_authorization_for_a_non_local_target() {
        let text = CONFIG.replace("environment = \"local\"", "environment = \"production\"");
        let manager = manager(&text);
        assert!(manager.plan(&target(MigrationOperation::Status)).is_ok());
    }

    #[tokio::test]
    async fn applying_to_a_local_target_needs_no_explicit_authorization() {
        let manager = manager(CONFIG);
        assert!(manager.plan(&target(MigrationOperation::Apply)).is_ok());
    }

    #[tokio::test]
    async fn applying_to_an_unknown_target_is_rejected() {
        let text = CONFIG.replace("environment = \"local\"", "environment = \"unknown\"");
        let manager = manager(&text);
        let error = manager
            .plan(&target(MigrationOperation::Apply))
            .unwrap_err();
        assert_eq!(error.code(), "MIGRATION_DATABASE_TARGET_UNKNOWN");
    }

    #[tokio::test]
    async fn applying_to_a_non_local_target_requires_authorization() {
        let text = CONFIG.replace("environment = \"local\"", "environment = \"production\"");
        let manager = manager(&text);
        let error = manager
            .plan(&target(MigrationOperation::Apply))
            .unwrap_err();
        assert_eq!(error.code(), "MIGRATION_DATABASE_TARGET_NOT_AUTHORIZED");
    }

    #[tokio::test]
    async fn applying_to_an_authorized_non_local_target_is_planned() {
        let text = CONFIG.replace("environment = \"local\"", "environment = \"production\"");
        let manager = manager(&text);
        let authorized = ResolvedMigrationTarget {
            non_local_authorized: true,
            ..target(MigrationOperation::Apply)
        };
        assert!(manager.plan(&authorized).is_ok());
    }

    #[tokio::test]
    async fn a_disabled_service_is_rejected() {
        let text = CONFIG.replace("enabled = true", "enabled = false");
        let manager = manager(&text);
        let error = manager
            .plan(&target(MigrationOperation::Apply))
            .unwrap_err();
        assert_eq!(error.code(), "MIGRATION_DISABLED");
    }

    #[tokio::test]
    async fn a_service_without_migration_configuration_is_rejected() {
        let text = r#"
config_version = 1

[[workspaces]]
id = "acme"
root = "/tmp/acme"

[[services]]
id = "auth-service"
type = "backend"
workspace = "acme"
path = "backend/auth-service"

[services.commands]
build = "./mvnw clean install"
start = "./mvnw spring-boot:run"
"#;
        let manager = manager(text);
        let error = manager
            .plan(&target(MigrationOperation::Apply))
            .unwrap_err();
        assert_eq!(error.code(), "MIGRATION_NOT_CONFIGURED");
    }

    #[tokio::test]
    async fn an_unregistered_provider_is_rejected() {
        let text = CONFIG.replace("enabled = true", "enabled = true\nprovider = \"dbmate\"");
        let manager = manager(&text);
        let error = manager
            .plan(&target(MigrationOperation::Apply))
            .unwrap_err();
        assert_eq!(error.code(), "MIGRATION_PROVIDER_NOT_FOUND");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn executes_a_successful_migration() {
        let manager = manager(CONFIG);
        let outcome = manager
            .execute(&target(MigrationOperation::Apply), None, None)
            .await
            .unwrap();
        assert_eq!(outcome.exit_code, Some(0));
        assert_eq!(outcome.stdout, "applied");
        assert!(outcome.database_identity.contains("local"));
    }

    /// A failed migration output for `classify`, without spawning a shell.
    fn failed_output(stderr: &str, code: i32) -> CommandOutput {
        CommandOutput {
            code: Some(code),
            reason: ExitReason::Failed,
            stdout: String::new(),
            stderr: stderr.to_owned(),
            truncated: false,
        }
    }

    #[tokio::test]
    async fn a_lock_conflict_is_classified() {
        // Classification is exercised directly so the test needs no shell
        // command (which validated configuration now refuses).
        let manager = manager(CONFIG);
        let context = manager
            .context(&target(MigrationOperation::Apply))
            .expect("the service is configured");
        let error = classify(
            &context,
            failed_output("Could not acquire change log lock", 1),
            None,
        )
        .unwrap_err();
        assert_eq!(error.code(), "MIGRATION_LOCK_CONFLICT");
    }

    #[tokio::test]
    async fn a_failed_migration_does_not_claim_a_rollback() {
        let manager = manager(CONFIG);
        let context = manager
            .context(&target(MigrationOperation::Apply))
            .expect("the service is configured");
        let error = classify(&context, failed_output("boom", 2), None).unwrap_err();
        match error {
            MigrationError::Failed {
                exit_code, message, ..
            } => {
                assert_eq!(exit_code, Some(2));
                assert!(!message.to_lowercase().contains("rollback"));
            }
            other => panic!("expected a structured failure, got {other:?}"),
        }
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn a_timed_out_migration_is_reported_as_a_timeout() {
        let text = CONFIG.replace(
            "update = { program = \"/bin/echo\", args = [\"applied\"] }",
            "update = { program = \"/bin/sleep\", args = [\"30\"] }",
        );
        let manager = manager(&text);
        let error = manager
            .execute(
                &target(MigrationOperation::Apply),
                Some(Duration::from_millis(100)),
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code(), "MIGRATION_TIMEOUT");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn a_cancelled_migration_is_reported_as_cancelled() {
        let text = CONFIG.replace(
            "update = { program = \"/bin/echo\", args = [\"applied\"] }",
            "update = { program = \"/bin/sleep\", args = [\"30\"] }",
        );
        let manager = manager(&text);
        let cancellation = Cancellation::new();
        let token = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            token.cancel();
        });
        let error = manager
            .execute(&target(MigrationOperation::Apply), None, Some(cancellation))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "MIGRATION_CANCELLED");
    }

    #[tokio::test]
    async fn redaction_drops_credential_lines() {
        let text = "starting\njdbc:postgresql://db/app?password=hunter2\nok";
        let redacted = scrub(text);
        assert!(!redacted.contains("hunter2"));
        assert!(redacted.contains("starting"));
        assert!(redacted.contains("ok"));
    }

    #[test]
    fn redaction_scrubs_secret_values_on_surviving_lines() {
        // A line without a known credential marker is kept, but a secret-looking
        // `key=value` value on it is still masked by the shared rule. (`token=`
        // is not one of this adapter's whole-line markers, so the line survives
        // to be value-scrubbed.)
        let text = "connecting with AUTH_TOKEN=abc123\nmigration complete";
        let redacted = scrub(text);
        assert!(!redacted.contains("abc123"));
        assert!(redacted.contains("AUTH_TOKEN=***"));
        assert!(redacted.contains("migration complete"));
    }
}
