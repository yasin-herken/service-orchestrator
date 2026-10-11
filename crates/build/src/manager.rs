//! The Build Manager.
//!
//! [`BuildManager`] is the build-system-agnostic orchestrator. Given a
//! workspace-aware [`ResolvedBuildTarget`], it:
//!
//! 1. resolves the [`BuildProvider`] for the target's build system;
//! 2. rejects an operation the provider does not support, with a structured
//!    error;
//! 3. validates the target and asks the provider for a structured
//!    [`BuildPlan`];
//! 4. enforces any *required* environment requirement the plan declares; and
//! 5. hands the plan to the [`BuildRunner`], which executes it through the
//!    process manager.
//!
//! It contains no Maven- or npm-specific knowledge: every such detail lives in a
//! provider. It also contains no scheduling logic: a build is always run as a
//! task, and the task engine remains the only concurrency authority
//! (`AGENTS.md` §9, §21, `tasks/008.md`).

use std::sync::Arc;
use std::time::Duration;

use service_orchestrator_application::{BuildError, ResolvedBuildTarget};
use service_orchestrator_domain::{BuildOperation, BuildSystem};
use service_orchestrator_execution::Cancellation;
use service_orchestrator_process::{ExitReason, ProcessError, ProcessManager};

use crate::plan::BuildPlan;
use crate::provider::{BuildHost, SystemBuildHost};
use crate::registry::BuildProviderRegistry;
use crate::runner::{BuildRunner, CommandOutput};

/// The maximum number of bytes of build output kept as failure detail.
const MAX_DETAIL_BYTES: usize = 4096;

/// The result of a successful build invocation.
#[derive(Clone, Debug)]
pub struct BuildOutcome {
    /// The entity that was built.
    pub name: String,
    /// The build system that ran.
    pub system: BuildSystem,
    /// The operation that ran.
    pub operation: BuildOperation,
    /// The process exit code, when the platform reported one.
    pub exit_code: Option<i32>,
    /// A bounded capture of standard output.
    pub stdout: String,
    /// A bounded capture of standard error.
    pub stderr: String,
}

/// Orchestrates build plans across build systems.
#[derive(Clone)]
pub struct BuildManager {
    registry: Arc<BuildProviderRegistry>,
    host: Arc<dyn BuildHost>,
    runner: BuildRunner,
}

impl BuildManager {
    /// Creates a manager with the standard providers and the system host.
    #[must_use]
    pub fn new(processes: ProcessManager) -> Self {
        Self::with_providers(processes, BuildProviderRegistry::standard())
    }

    /// Creates a manager over an explicit provider registry.
    #[must_use]
    pub fn with_providers(processes: ProcessManager, registry: BuildProviderRegistry) -> Self {
        Self {
            registry: Arc::new(registry),
            host: Arc::new(SystemBuildHost::new()),
            runner: BuildRunner::new(processes),
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

    /// Returns the provider registry, for observability.
    #[must_use]
    pub fn registry(&self) -> &Arc<BuildProviderRegistry> {
        &self.registry
    }

    /// Resolves and validates the plan for `target`.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError::UnsupportedSystem`] when no provider is registered,
    /// [`BuildError::UnsupportedOperation`] when the provider does not support
    /// the operation, and [`BuildError::InvalidTarget`] when the target is
    /// invalid or a required environment variable is missing.
    pub fn plan(&self, target: &ResolvedBuildTarget) -> Result<BuildPlan, BuildError> {
        let provider = self.registry.resolve(target.build_system)?;
        if !provider.supports(target.operation) {
            return Err(BuildError::UnsupportedOperation {
                system: target.build_system,
                operation: target.operation,
            });
        }
        provider.validate(target, self.host.as_ref())?;
        let plan = provider.build_plan(target, self.host.as_ref())?;
        self.check_environment(target, &plan)?;
        Ok(plan)
    }

    /// Executes `target`, stopping it if `cancellation` is signalled.
    ///
    /// `timeout`, when present, is the task's deadline and takes precedence over
    /// the plan's own declared timeout.
    ///
    /// # Errors
    ///
    /// Returns the structured [`BuildError`] for an invalid or unsupported
    /// target, a process failure, a non-zero exit, a timeout, or cancellation.
    pub async fn execute(
        &self,
        target: &ResolvedBuildTarget,
        timeout: Option<Duration>,
        cancellation: Option<Cancellation>,
    ) -> Result<BuildOutcome, BuildError> {
        let plan = self.plan(target)?;
        let effective_timeout = timeout.or(plan.timeout);
        let output = self
            .runner
            .run(&plan, timeout, cancellation)
            .await
            .map_err(|error| map_process_error(target, error))?;
        classify(target, output, effective_timeout)
    }

    /// Rejects a plan whose required environment variables are absent.
    fn check_environment(
        &self,
        target: &ResolvedBuildTarget,
        plan: &BuildPlan,
    ) -> Result<(), BuildError> {
        for name in plan.required_environment() {
            if !self.host.environment_has(name) {
                return Err(BuildError::InvalidTarget {
                    name: target.name.clone(),
                    message: format!(
                        "the '{name}' environment variable is required by {} but is not set",
                        plan.system
                    ),
                });
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for BuildManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuildManager")
            .field("providers", &self.registry.systems().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// Maps a process-manager failure into a structured build error.
fn map_process_error(target: &ResolvedBuildTarget, error: ProcessError) -> BuildError {
    match error {
        ProcessError::Spawn { message, .. } => BuildError::Process {
            name: target.name.clone(),
            message: format!("could not start the build: {message}"),
        },
        other => BuildError::internal(other.to_string()),
    }
}

/// Turns a completed process into a build outcome or a structured failure.
fn classify(
    target: &ResolvedBuildTarget,
    output: CommandOutput,
    timeout: Option<Duration>,
) -> Result<BuildOutcome, BuildError> {
    match output.reason {
        ExitReason::Cancelled => Err(BuildError::Cancelled {
            name: target.name.clone(),
            system: target.build_system,
            operation: target.operation,
        }),
        ExitReason::TimedOut => Err(BuildError::Timeout {
            name: target.name.clone(),
            system: target.build_system,
            operation: target.operation,
            timeout_seconds: timeout.map(|value| value.as_secs()).unwrap_or(0),
        }),
        _ if output.succeeded() => Ok(BuildOutcome {
            name: target.name.clone(),
            system: target.build_system,
            operation: target.operation,
            exit_code: output.code,
            stdout: bound(&output.stdout),
            stderr: bound(&output.stderr),
        }),
        _ => Err(BuildError::CommandFailed {
            name: target.name.clone(),
            system: target.build_system,
            operation: target.operation,
            exit_code: output.code,
            message: failure_message(&output),
            detail: detail(&output),
        }),
    }
}

/// A concise, credential-free summary of a failed build.
fn failure_message(output: &CommandOutput) -> String {
    if output.truncated {
        return "build output exceeded the capture buffer and could not be read completely"
            .to_owned();
    }
    let first = output
        .stderr
        .lines()
        .chain(output.stdout.lines())
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    if first.trim().is_empty() {
        match output.code {
            Some(code) => format!("build exited with status {code}"),
            None => "build failed before reporting an exit status".to_owned(),
        }
    } else {
        bound(first.trim())
    }
}

/// Additional bounded detail for a failed build.
fn detail(output: &CommandOutput) -> Option<String> {
    let combined = if output.stderr.trim().is_empty() {
        output.stdout.trim()
    } else {
        output.stderr.trim()
    };
    if combined.is_empty() {
        None
    } else {
        Some(bound(combined))
    }
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
    use crate::plan::EnvironmentRequirement;
    use crate::provider::{BuildHost, BuildProvider};
    use service_orchestrator_application::BuildError;
    use service_orchestrator_domain::{AbsolutePath, WorkspaceId};

    /// A provider that requires an environment variable that is never set.
    struct RequiringProvider;

    impl BuildProvider for RequiringProvider {
        fn system(&self) -> BuildSystem {
            BuildSystem::Maven
        }

        fn capabilities(&self) -> &[BuildOperation] {
            &[BuildOperation::Build]
        }

        fn validate(&self, _: &ResolvedBuildTarget, _: &dyn BuildHost) -> Result<(), BuildError> {
            Ok(())
        }

        fn build_plan(
            &self,
            target: &ResolvedBuildTarget,
            _: &dyn BuildHost,
        ) -> Result<BuildPlan, BuildError> {
            Ok(BuildPlan::new(
                target.build_system,
                target.operation,
                "mvn",
                target.path.clone(),
            )
            .with_environment(EnvironmentRequirement::required("UNSET_REQUIRED_VARIABLE")))
        }
    }

    struct NoHost;

    impl BuildHost for NoHost {
        fn file_exists(&self, _: &AbsolutePath, _: &str) -> bool {
            false
        }

        fn environment_has(&self, _: &str) -> bool {
            false
        }
    }

    fn target() -> ResolvedBuildTarget {
        ResolvedBuildTarget {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            name: "auth-service".to_owned(),
            build_system: BuildSystem::Maven,
            operation: BuildOperation::Build,
            path: AbsolutePath::new("/w/acme/backend/auth-service").unwrap(),
            workspace_root: AbsolutePath::new("/w/acme").unwrap(),
            command: None,
        }
    }

    #[tokio::test]
    async fn a_missing_required_environment_variable_is_reported() {
        let mut registry = BuildProviderRegistry::new();
        registry.register(Arc::new(RequiringProvider));
        let manager = BuildManager::with_providers(ProcessManager::new().unwrap(), registry)
            .with_host(Arc::new(NoHost));
        let error = manager.plan(&target()).unwrap_err();
        assert_eq!(error.code(), "BUILD_INVALID_TARGET");
    }

    #[tokio::test]
    async fn an_unregistered_system_is_rejected() {
        let manager = BuildManager::with_providers(
            ProcessManager::new().unwrap(),
            BuildProviderRegistry::new(),
        );
        let error = manager.plan(&target()).unwrap_err();
        assert_eq!(error.code(), "BUILD_UNSUPPORTED_SYSTEM");
    }

    #[tokio::test]
    async fn an_unsupported_operation_is_rejected() {
        let manager = BuildManager::new(ProcessManager::new().unwrap());
        let target = ResolvedBuildTarget {
            operation: BuildOperation::InstallDependencies,
            ..target()
        };
        let error = manager.plan(&target).unwrap_err();
        assert_eq!(error.code(), "BUILD_UNSUPPORTED_OPERATION");
    }
}
