//! The service lifecycle task executors.
//!
//! Three thin adapters — start, stop, restart — connect the task engine to the
//! [`ServiceManager`]. Each decodes the workspace-aware
//! [`ResolvedServiceTarget`] from the task's parameters, reports a human message,
//! applies the task's cancellation token, delegates to the manager, and maps the
//! structured outcome (or [`ServiceRuntimeError`]) back onto a [`TaskOutcome`] /
//! [`TaskExecutorError`].
//!
//! No lifecycle meaning lives here: runtime selection, validation, planning, and
//! process ownership all stay in the manager and its runtimes, so the TUI and MCP
//! reach identical behaviour through the same engine (`AGENTS.md` §21).
//!
//! [`register_service_executors`] wires the three executors for the
//! `StartService`, `StopService`, and `RestartService` task kinds; the
//! composition root calls it once at startup with the same manager the
//! application reads through.

use std::sync::Arc;

use async_trait::async_trait;
use service_orchestrator_application::{ResolvedServiceTarget, ServiceRuntimeError};
use service_orchestrator_domain::TaskKind;
use service_orchestrator_execution::{
    ExecutorRegistry, TaskContext, TaskExecutor, TaskExecutorError, TaskOutcome,
};

use crate::manager::{RestartOutcome, ServiceManager, StartOutcome, StopOutcome};

/// Starts services on behalf of the task engine.
pub struct StartServiceExecutor {
    manager: Arc<ServiceManager>,
}

impl StartServiceExecutor {
    /// Creates a start executor over a service manager.
    #[must_use]
    pub fn new(manager: Arc<ServiceManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl TaskExecutor for StartServiceExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        let target = decode_target(&context)?;
        let _ = context
            .progress()
            .message(format!("starting service '{}'", target.service_id));
        self.manager
            .start(&target, Some(context.cancellation().clone()))
            .await
            .map(start_outcome)
            .map_err(to_executor_error)
    }
}

impl std::fmt::Debug for StartServiceExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartServiceExecutor")
            .finish_non_exhaustive()
    }
}

/// Stops services on behalf of the task engine.
pub struct StopServiceExecutor {
    manager: Arc<ServiceManager>,
}

impl StopServiceExecutor {
    /// Creates a stop executor over a service manager.
    #[must_use]
    pub fn new(manager: Arc<ServiceManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl TaskExecutor for StopServiceExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        let target = decode_target(&context)?;
        let _ = context
            .progress()
            .message(format!("stopping service '{}'", target.service_id));
        self.manager
            .stop(&target.service_id)
            .await
            .map(stop_outcome)
            .map_err(to_executor_error)
    }
}

impl std::fmt::Debug for StopServiceExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StopServiceExecutor")
            .finish_non_exhaustive()
    }
}

/// Restarts services on behalf of the task engine.
pub struct RestartServiceExecutor {
    manager: Arc<ServiceManager>,
}

impl RestartServiceExecutor {
    /// Creates a restart executor over a service manager.
    #[must_use]
    pub fn new(manager: Arc<ServiceManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl TaskExecutor for RestartServiceExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        let target = decode_target(&context)?;
        let _ = context
            .progress()
            .message(format!("restarting service '{}'", target.service_id));
        self.manager
            .restart(&target, Some(context.cancellation().clone()))
            .await
            .map(restart_outcome)
            .map_err(to_executor_error)
    }
}

impl std::fmt::Debug for RestartServiceExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestartServiceExecutor")
            .finish_non_exhaustive()
    }
}

/// Registers the start, stop, and restart executors against the engine.
///
/// The composition root calls this once with the registry it hands to the
/// engine and the manager the application reads through.
pub fn register_service_executors(registry: &mut ExecutorRegistry, manager: Arc<ServiceManager>) {
    let start: Arc<dyn TaskExecutor> = Arc::new(StartServiceExecutor::new(Arc::clone(&manager)));
    let stop: Arc<dyn TaskExecutor> = Arc::new(StopServiceExecutor::new(Arc::clone(&manager)));
    let restart: Arc<dyn TaskExecutor> = Arc::new(RestartServiceExecutor::new(manager));
    registry.register(TaskKind::StartService, start);
    registry.register(TaskKind::StopService, stop);
    registry.register(TaskKind::RestartService, restart);
}

/// Decodes a lifecycle target from a task's parameters.
fn decode_target(context: &TaskContext) -> Result<ResolvedServiceTarget, TaskExecutorError> {
    ResolvedServiceTarget::from_parameters(context.parameters()).map_err(to_executor_error)
}

/// Converts a successful start into structured task metadata.
fn start_outcome(result: StartOutcome) -> TaskOutcome {
    let summary = if result.already_running {
        format!("service '{}' was already running", result.service_id)
    } else {
        format!("service '{}' started", result.service_id)
    };
    let mut outcome = TaskOutcome::succeeded(Some(summary))
        .with_metadata("service", result.service_id.to_string())
        .with_metadata("runtime", result.runtime.label())
        .with_metadata("command", result.command)
        .with_metadata("process_id", result.process_id)
        .with_metadata("already_running", result.already_running.to_string());
    if let Some(pid) = result.pid {
        outcome = outcome.with_metadata("pid", pid.to_string());
    }
    outcome
}

/// Converts a successful stop into structured task metadata.
fn stop_outcome(result: StopOutcome) -> TaskOutcome {
    let summary = if result.already_stopped {
        format!("service '{}' was not running", result.service_id)
    } else {
        format!("service '{}' stopped", result.service_id)
    };
    let mut outcome = TaskOutcome::succeeded(Some(summary))
        .with_metadata("service", result.service_id.to_string())
        .with_metadata("already_stopped", result.already_stopped.to_string());
    if let Some(process_id) = result.process_id {
        outcome = outcome.with_metadata("process_id", process_id);
    }
    if let Some(code) = result.exit_code {
        outcome = outcome.with_metadata("exit_code", code.to_string());
    }
    if let Some(reason) = result.exit_reason {
        outcome = outcome.with_metadata("exit_reason", reason);
    }
    outcome
}

/// Converts a successful restart into structured task metadata.
fn restart_outcome(result: RestartOutcome) -> TaskOutcome {
    let summary = format!("service '{}' restarted", result.service_id);
    let mut outcome = TaskOutcome::succeeded(Some(summary))
        .with_metadata("service", result.service_id.to_string())
        .with_metadata("runtime", result.started.runtime.label())
        .with_metadata("command", result.started.command)
        .with_metadata("process_id", result.started.process_id)
        .with_metadata(
            "previous_stopped",
            result.stopped.already_stopped.to_string(),
        );
    if let Some(pid) = result.started.pid {
        outcome = outcome.with_metadata("pid", pid.to_string());
    }
    outcome
}

/// Maps a structured runtime failure onto the engine's error vocabulary.
fn to_executor_error(error: ServiceRuntimeError) -> TaskExecutorError {
    match error {
        ServiceRuntimeError::OperationCancelled { .. } => TaskExecutorError::Cancelled,
        ServiceRuntimeError::Internal { message } => TaskExecutorError::internal(message),
        other => TaskExecutorError::failed(other.code(), other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_domain::ServiceId;

    #[test]
    fn cancellation_maps_to_a_cancelled_attempt() {
        let error = ServiceRuntimeError::OperationCancelled {
            service: ServiceId::new("auth-service").unwrap(),
        };
        assert_eq!(to_executor_error(error), TaskExecutorError::Cancelled);
    }

    #[test]
    fn structured_failures_preserve_their_code() {
        let error = ServiceRuntimeError::PortAlreadyInUse {
            service: ServiceId::new("auth-service").unwrap(),
            port: 8081,
        };
        match to_executor_error(error) {
            TaskExecutorError::Failed { code, .. } => {
                assert_eq!(code, "RUNTIME_PORT_ALREADY_IN_USE");
            }
            other => panic!("expected a structured failure, got {other:?}"),
        }
    }

    #[test]
    fn internal_failures_remain_internal() {
        let mapped = to_executor_error(ServiceRuntimeError::internal("boom"));
        assert!(matches!(mapped, TaskExecutorError::Internal { .. }));
    }
}
