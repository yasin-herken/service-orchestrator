//! Migration execution through the process manager.
//!
//! [`MigrationRunner`] is the *only* place in the migration adapter that starts a
//! process, and it never starts one itself: it turns a [`MigrationPlan`] into a
//! structured [`ProcessSpec`] and hands it to the [`ProcessManager`], observing
//! the manager's events. This preserves the rule that migration tooling does not
//! spawn operating-system processes directly (`AGENTS.md` §15) and keeps every
//! migration process supervised, stoppable, and cleaned up.
//!
//! # Environment
//!
//! Unlike a build plan, a migration plan carries environment *values*: the
//! database configuration a migration needs is supplied through the environment
//! (`tasks/011.md` §11). The runner therefore applies the plan environment to the
//! child process. The domain [`Environment`](service_orchestrator_domain::Environment)
//! redacts secret values whenever the specification is displayed or serialized,
//! so a credential is never written to a log.
//!
//! [`MigrationPlan`]: crate::plan::MigrationPlan

use service_orchestrator_execution::Cancellation;
use service_orchestrator_process::{
    ExitReason, ProcessError, ProcessEvent, ProcessManager, ProcessSpec,
};
use tokio::sync::broadcast::error::RecvError;

use crate::plan::MigrationPlan;

/// The result of running one migration process to completion.
#[derive(Clone, Debug)]
pub(crate) struct CommandOutput {
    /// The process exit code, when the platform reported one.
    pub(crate) code: Option<i32>,
    /// Why the process ended.
    pub(crate) reason: ExitReason,
    /// Captured standard output, with lines re-joined by `\n`.
    pub(crate) stdout: String,
    /// Captured standard error, with lines re-joined by `\n`.
    pub(crate) stderr: String,
    /// Whether output was dropped because the subscriber lagged.
    pub(crate) truncated: bool,
}

impl CommandOutput {
    /// Returns `true` if the process exited successfully with complete output.
    pub(crate) fn succeeded(&self) -> bool {
        !self.truncated && self.reason == ExitReason::Exited && self.code == Some(0)
    }
}

/// Runs migration plans through the process manager.
#[derive(Clone)]
pub(crate) struct MigrationRunner {
    processes: ProcessManager,
}

impl MigrationRunner {
    /// Creates a runner over the given process manager.
    pub(crate) fn new(processes: ProcessManager) -> Self {
        Self { processes }
    }

    /// Runs `plan` to completion, stopping it if `cancellation` is signalled.
    ///
    /// `timeout` (from the task) takes precedence over the plan's own declared
    /// timeout when both are present.
    ///
    /// # Errors
    ///
    /// Returns the process manager's error when the specification is invalid,
    /// the manager is shutting down, or the operating system refuses the spawn.
    pub(crate) async fn run(
        &self,
        plan: &MigrationPlan,
        timeout: Option<std::time::Duration>,
        cancellation: Option<Cancellation>,
    ) -> Result<CommandOutput, ProcessError> {
        let mut spec = ProcessSpec::new(plan.program.clone())?
            .args(plan.args.clone())
            .working_dir(plan.working_dir.as_str())
            .with_environment(plan.environment.clone());
        if let Some(timeout) = timeout.or(plan.timeout) {
            spec = spec.with_timeout(timeout);
        }

        // Subscribe before spawning so no early output line is missed.
        let mut events = self.processes.subscribe_events();

        let handle = match cancellation {
            Some(cancellation) => self.processes.spawn_with_cancellation(spec, async move {
                cancellation.cancelled().await;
            })?,
            None => self.processes.spawn(spec)?,
        };
        let id = handle.id().clone();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut truncated = false;

        loop {
            match events.recv().await {
                Ok(event) => {
                    if event.process_id() != &id {
                        continue;
                    }
                    match event {
                        ProcessEvent::StdoutLine(line) => stdout.push(line.line),
                        ProcessEvent::StderrLine(line) => stderr.push(line.line),
                        ProcessEvent::Stopped { exit, .. }
                        | ProcessEvent::Exited { exit, .. }
                        | ProcessEvent::Failed { exit, .. }
                        | ProcessEvent::Killed { exit, .. } => {
                            return Ok(CommandOutput {
                                code: exit.code,
                                reason: exit.reason,
                                stdout: stdout.join("\n"),
                                stderr: stderr.join("\n"),
                                truncated,
                            });
                        }
                        _ => {}
                    }
                }
                Err(RecvError::Lagged(_)) => {
                    // Output can no longer be reconstructed faithfully; keep
                    // draining lifecycle events so the process is still reaped.
                    truncated = true;
                }
                Err(RecvError::Closed) => break,
            }
        }

        // The event channel closed (manager dropped). Fall back to the exit
        // snapshot so the caller still receives a result.
        let snapshot = handle.wait().await?;
        let (code, reason) = snapshot
            .exit
            .map_or((None, ExitReason::Failed), |exit| (exit.code, exit.reason));
        Ok(CommandOutput {
            code,
            reason,
            stdout: stdout.join("\n"),
            stderr: stderr.join("\n"),
            truncated,
        })
    }
}

impl std::fmt::Debug for MigrationRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MigrationRunner").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_domain::{
        AbsolutePath, Environment, MigrationOperation, MigrationProviderType,
    };
    use std::time::Duration;

    #[cfg(unix)]
    const SLEEP: &str = "/bin/sleep";

    fn plan(program: &str, args: &[&str], dir: &str) -> MigrationPlan {
        MigrationPlan::new(
            MigrationProviderType::Liquibase,
            MigrationOperation::Apply,
            program,
            AbsolutePath::new(dir).unwrap(),
        )
        .with_args(args.iter().copied())
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn output_is_captured_and_joined() {
        let manager = ProcessManager::new().unwrap();
        let runner = MigrationRunner::new(manager);
        let plan = plan("/bin/echo", &["first"], "/tmp");
        let output = runner.run(&plan, None, None).await.unwrap();
        assert!(output.succeeded());
        assert_eq!(output.stdout, "first");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn the_plan_environment_is_applied_to_the_process() {
        // `sh -c 'echo "$MIGRATION_MARKER"'` prints the variable if the runner
        // forwarded the plan environment. The value is not secret.
        let manager = ProcessManager::new().unwrap();
        let runner = MigrationRunner::new(manager);
        let mut environment = Environment::new();
        environment
            .set(
                service_orchestrator_domain::EnvironmentVariable::new(
                    "MIGRATION_MARKER",
                    "applied",
                )
                .unwrap(),
            )
            .unwrap();
        let plan = plan("/bin/sh", &["-c", "echo \"$MIGRATION_MARKER\""], "/tmp")
            .with_environment(environment);
        let output = runner.run(&plan, None, None).await.unwrap();
        assert!(output.succeeded());
        assert_eq!(output.stdout, "applied");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn a_non_zero_exit_is_captured() {
        let manager = ProcessManager::new().unwrap();
        let runner = MigrationRunner::new(manager);
        let plan = plan("/bin/sh", &["-c", "exit 3"], "/tmp");
        let output = runner.run(&plan, None, None).await.unwrap();
        assert!(!output.succeeded());
        assert_eq!(output.code, Some(3));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn a_timed_out_process_is_reported_as_timed_out() {
        let manager = ProcessManager::new().unwrap();
        let runner = MigrationRunner::new(manager);
        let plan = plan(SLEEP, &["30"], "/tmp");
        let output = runner
            .run(&plan, Some(Duration::from_millis(100)), None)
            .await
            .unwrap();
        assert_eq!(output.reason, ExitReason::TimedOut);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn a_cancelled_process_is_reported_as_cancelled() {
        let manager = ProcessManager::new().unwrap();
        let runner = MigrationRunner::new(manager);
        let cancellation = Cancellation::new();
        let token = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            token.cancel();
        });

        let plan = plan(SLEEP, &["30"], "/tmp");
        let output = runner.run(&plan, None, Some(cancellation)).await.unwrap();
        assert_eq!(output.reason, ExitReason::Cancelled);
    }
}
