//! Build execution through the process manager.
//!
//! [`BuildRunner`] is the *only* place in the build adapter that starts a
//! process, and it never starts one itself: it turns a [`BuildPlan`] into a
//! structured [`ProcessSpec`] and hands it to the [`ProcessManager`], observing
//! the manager's events. This keeps the rule that build tooling does not spawn
//! operating-system processes directly (`AGENTS.md` §15) intact and keeps every
//! build process supervised, stoppable, and cleaned up.
//!
//! # Environment
//!
//! A plan carries environment *requirements* (names) but never values
//! (`AGENTS.md` §31). The runner therefore sets no environment variables: the
//! build inherits the developer's process environment, where variables such as
//! `JAVA_HOME` live. Requirement enforcement happens in the manager, before a
//! process is started.
//!
//! [`BuildPlan`]: crate::plan::BuildPlan

use service_orchestrator_execution::Cancellation;
use service_orchestrator_process::{
    ExitReason, ProcessError, ProcessEvent, ProcessManager, ProcessSpec,
};
use tokio::sync::broadcast::error::RecvError;

use crate::plan::BuildPlan;

/// The result of running one build process to completion.
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

/// Runs build plans through the process manager.
#[derive(Clone)]
pub(crate) struct BuildRunner {
    processes: ProcessManager,
}

impl BuildRunner {
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
        plan: &BuildPlan,
        timeout: Option<std::time::Duration>,
        cancellation: Option<Cancellation>,
    ) -> Result<CommandOutput, ProcessError> {
        let mut spec = ProcessSpec::new(plan.program.clone())?
            .args(plan.args.clone())
            .working_dir(plan.working_dir.as_str());
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

impl std::fmt::Debug for BuildRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuildRunner").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_domain::{AbsolutePath, BuildOperation, BuildSystem};
    use std::time::Duration;

    #[cfg(unix)]
    const SLEEP: &str = "/bin/sleep";

    fn plan(program: &str, args: &[&str], dir: &str) -> BuildPlan {
        BuildPlan::new(
            BuildSystem::Maven,
            BuildOperation::Build,
            program,
            AbsolutePath::new(dir).unwrap(),
        )
        .with_args(args.iter().copied())
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn output_is_captured_and_joined() {
        let manager = ProcessManager::new().unwrap();
        let runner = BuildRunner::new(manager);
        let plan = plan("/bin/echo", &["first"], "/tmp");
        let output = runner.run(&plan, None, None).await.unwrap();
        assert!(output.succeeded());
        assert_eq!(output.stdout, "first");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn a_timed_out_process_is_reported_as_timed_out() {
        let manager = ProcessManager::new().unwrap();
        let runner = BuildRunner::new(manager);
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
        let runner = BuildRunner::new(manager);
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
