//! Git process execution through the process manager.
//!
//! [`GitRunner`] is the *only* place in the Git adapter that starts a process,
//! and it never does so itself: it hands a structured [`ProcessSpec`] to the
//! [`ProcessManager`] and observes the manager's events. This keeps the rule
//! that Git Manager does not spawn operating-system processes directly
//! (`AGENTS.md` §15) intact.
//!
//! # Output capture
//!
//! The process manager publishes every `stdout`/`stderr` line as an event. The
//! runner subscribes *before* spawning and filters by the assigned process id,
//! reconstructing `stdout` by joining the captured lines with `\n`. Because the
//! manager reads output line by line, this reconstructs the original stream
//! (including `NUL`-separated Git records) exactly as long as the subscriber
//! keeps up with the bounded broadcast channel. Under extreme output volume a
//! subscriber can lag; the runner detects [`RecvError::Lagged`] and abandons the
//! capture rather than silently returning truncated output, and the process is
//! still reaped through the manager so nothing is orphaned.
//!
//! [`RecvError::Lagged`]: tokio::sync::broadcast::error::RecvError::Lagged

use service_orchestrator_execution::Cancellation;
use service_orchestrator_process::{ExitReason, ProcessEvent, ProcessManager, ProcessSpec};
use tokio::sync::broadcast::error::RecvError;

/// The result of running one Git process to completion.
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

/// Runs Git processes through the process manager.
#[derive(Clone)]
pub(crate) struct GitRunner {
    processes: ProcessManager,
}

impl GitRunner {
    /// Creates a runner over the given process manager.
    pub(crate) fn new(processes: ProcessManager) -> Self {
        Self { processes }
    }

    /// Runs `spec` to completion, stopping it if `cancellation` is signalled.
    ///
    /// # Errors
    ///
    /// Returns the process manager's error when the specification is invalid,
    /// the manager is shutting down, or the operating system refuses the spawn.
    pub(crate) async fn run(
        &self,
        spec: ProcessSpec,
        cancellation: Option<Cancellation>,
    ) -> Result<CommandOutput, service_orchestrator_process::ProcessError> {
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

impl std::fmt::Debug for GitRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitRunner").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_execution::Cancellation;
    use std::time::Duration;

    /// A program that sleeps without producing output, to exercise the
    /// manager's timeout and cancellation handling deterministically.
    #[cfg(unix)]
    const SLEEP: &str = "/bin/sleep";

    fn sleeping(seconds: &str) -> ProcessSpec {
        ProcessSpec::new(SLEEP).unwrap().arg(seconds)
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn a_cancelled_process_is_reported_as_cancelled() {
        let manager = ProcessManager::new().unwrap();
        let runner = GitRunner::new(manager);
        let cancellation = Cancellation::new();
        let token = cancellation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            token.cancel();
        });

        let output = runner
            .run(sleeping("30"), Some(cancellation))
            .await
            .unwrap();
        assert_eq!(output.reason, ExitReason::Cancelled);
        assert!(!output.succeeded());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn a_timed_out_process_is_reported_as_timed_out() {
        let manager = ProcessManager::new().unwrap();
        let runner = GitRunner::new(manager);
        let spec = sleeping("30").with_timeout(Duration::from_millis(100));

        let output = runner.run(spec, None).await.unwrap();
        assert_eq!(output.reason, ExitReason::TimedOut);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn output_is_captured_and_joined() {
        let manager = ProcessManager::new().unwrap();
        let runner = GitRunner::new(manager);
        let spec = ProcessSpec::new("/bin/sh")
            .unwrap()
            .args(["-c", "printf 'first\\nsecond\\n'"]);
        let output = runner.run(spec, None).await.unwrap();
        assert!(output.succeeded());
        assert_eq!(output.stdout, "first\nsecond");
    }
}
