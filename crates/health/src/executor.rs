//! The health task executors.
//!
//! A thin adapter connects the task engine to the [`HealthManager`]. It decodes
//! the workspace-aware [`ResolvedServiceTarget`] from the task's parameters,
//! reports a human message, maps the task kind onto a check scope, delegates to
//! the manager, and maps the structured result back onto a [`TaskOutcome`] or
//! [`TaskExecutorError`].
//!
//! No probe meaning lives here: provider selection, timeouts, aggregation,
//! history, and events all stay in the manager, so the TUI and MCP reach
//! identical behaviour through the same engine (`AGENTS.md` §21,
//! `tasks/012.md` §33).
//!
//! [`register_health_executors`] wires the five health task kinds; the
//! composition root calls it once with the same manager the application reads
//! through.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use service_orchestrator_application::{
    HealthCheckScope, HealthState, ResolvedServiceTarget, ServiceHealth,
};
use service_orchestrator_domain::{ServiceId, TaskKind};
use service_orchestrator_execution::{
    ExecutorRegistry, TaskContext, TaskExecutor, TaskExecutorError, TaskOutcome,
};

use crate::error::HealthError;
use crate::manager::HealthManager;

/// The default deadline, in seconds, for a readiness task.
pub const DEFAULT_READINESS_TIMEOUT_SECONDS: u64 = 60;

/// Runs health checks on behalf of the task engine.
pub struct HealthTaskExecutor {
    manager: Arc<HealthManager>,
}

impl HealthTaskExecutor {
    /// Creates a health executor over a manager.
    #[must_use]
    pub fn new(manager: Arc<HealthManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl TaskExecutor for HealthTaskExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        let target =
            ResolvedServiceTarget::from_parameters(context.parameters()).map_err(|error| {
                TaskExecutorError::failed("HEALTH_TARGET_INVALID", error.to_string())
            })?;
        let service_id = target.service_id;

        match context.kind() {
            TaskKind::HealthCheck => {
                self.run_scope(&service_id, HealthCheckScope::All, &context)
                    .await
            }
            TaskKind::ProcessHealthCheck => {
                self.run_scope(&service_id, HealthCheckScope::Process, &context)
                    .await
            }
            TaskKind::TcpHealthCheck => {
                self.run_scope(&service_id, HealthCheckScope::Tcp, &context)
                    .await
            }
            TaskKind::HttpHealthCheck => {
                self.run_scope(&service_id, HealthCheckScope::Http, &context)
                    .await
            }
            TaskKind::ReadinessCheck => self.run_readiness(&service_id, &context).await,
            other => Err(TaskExecutorError::internal(format!(
                "the health executor cannot handle task kind '{other}'"
            ))),
        }
    }
}

impl HealthTaskExecutor {
    /// Runs the checks selected by `scope` and evaluates the aggregate.
    async fn run_scope(
        &self,
        service_id: &ServiceId,
        scope: HealthCheckScope,
        context: &TaskContext,
    ) -> Result<TaskOutcome, TaskExecutorError> {
        let _ = context.progress().message(format!(
            "running the '{}' health checks for service '{service_id}'",
            scope.label()
        ));
        let health = self
            .manager
            .check(service_id, scope)
            .await
            .map_err(map_health_error)?;
        outcome_for(service_id, &health)
    }

    /// Waits until the service is ready, or the deadline elapses.
    async fn run_readiness(
        &self,
        service_id: &ServiceId,
        context: &TaskContext,
    ) -> Result<TaskOutcome, TaskExecutorError> {
        let _ = context.progress().message(format!(
            "waiting for service '{service_id}' to become ready"
        ));
        let timeout = context
            .timeout()
            .unwrap_or_else(|| Duration::from_secs(DEFAULT_READINESS_TIMEOUT_SECONDS));
        let health = self
            .manager
            .wait_until_ready(service_id, timeout, Some(context.cancellation().clone()))
            .await
            .map_err(map_health_error)?;
        Ok(
            TaskOutcome::succeeded(Some(format!("service '{service_id}' is ready")))
                .with_metadata("service", service_id.to_string())
                .with_metadata("health", health.state.label())
                .with_metadata("checks", health.checks.len().to_string()),
        )
    }
}

impl std::fmt::Debug for HealthTaskExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HealthTaskExecutor").finish_non_exhaustive()
    }
}

/// Registers the health executors against the engine.
///
/// One executor instance is registered for every health task kind so the
/// decoding and mapping logic exists once.
pub fn register_health_executors(registry: &mut ExecutorRegistry, manager: Arc<HealthManager>) {
    let executor: Arc<dyn TaskExecutor> = Arc::new(HealthTaskExecutor::new(manager));
    for kind in [
        TaskKind::HealthCheck,
        TaskKind::ProcessHealthCheck,
        TaskKind::TcpHealthCheck,
        TaskKind::HttpHealthCheck,
        TaskKind::ReadinessCheck,
    ] {
        registry.register(kind, Arc::clone(&executor));
    }
}

/// Evaluates a completed check into a task outcome.
///
/// Healthy and degraded services are a successful observation: the task
/// succeeded even though the service is not fully healthy. A required failure
/// fails the task with a stable code so an interface can branch without parsing
/// the message (`tasks/012.md` §17, §35).
fn outcome_for(
    service_id: &ServiceId,
    health: &ServiceHealth,
) -> Result<TaskOutcome, TaskExecutorError> {
    let summary = format!("service '{service_id}' is {}", health.state.label());
    let metadata = |outcome: TaskOutcome| {
        outcome
            .with_metadata("service", service_id.to_string())
            .with_metadata("health", health.state.label())
            .with_metadata("checks", health.checks.len().to_string())
    };

    match health.state {
        HealthState::Healthy | HealthState::Degraded => {
            Ok(metadata(TaskOutcome::succeeded(Some(summary))))
        }
        HealthState::Unhealthy => Err(failed_with_check_detail(
            "HEALTH_CHECK_FAILED",
            summary,
            health,
        )),
        HealthState::Unreachable => Err(failed_with_check_detail(
            "HEALTH_UNREACHABLE",
            summary,
            health,
        )),
        HealthState::Unknown | HealthState::Checking => Err(failed_with_check_detail(
            "HEALTH_UNDETERMINED",
            summary,
            health,
        )),
    }
}

/// Builds a structured failure, attaching the first failing check's message.
fn failed_with_check_detail(
    code: &str,
    message: String,
    health: &ServiceHealth,
) -> TaskExecutorError {
    match health
        .checks
        .iter()
        .find(|check| !check.is_healthy())
        .and_then(|check| check.message.clone())
    {
        Some(detail) => TaskExecutorError::failed_with_detail(code, message, detail),
        None => TaskExecutorError::failed(code, message),
    }
}

/// Maps a structured health error onto the engine's error vocabulary.
fn map_health_error(error: HealthError) -> TaskExecutorError {
    match error {
        HealthError::OperationCancelled { .. } => TaskExecutorError::Cancelled,
        HealthError::Internal { message } => TaskExecutorError::internal(message),
        other => TaskExecutorError::failed(other.code(), other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_application::{HealthCheckOutcome, CHECK_HTTP};
    use service_orchestrator_domain::{TaskId, Timestamp, WorkspaceId};

    fn health(state: HealthState, check_status: HealthState) -> ServiceHealth {
        ServiceHealth {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            service_id: ServiceId::new("auth-service").unwrap(),
            state,
            checked_at: Some(Timestamp::now()),
            checks: vec![HealthCheckOutcome {
                check_type: CHECK_HTTP.to_owned(),
                target: None,
                required: true,
                status: check_status,
                duration_ms: 4,
                message: Some("connection refused".to_owned()),
                checked_at: Timestamp::now(),
            }],
            consecutive_failures: 0,
            last_healthy_at: None,
            last_unhealthy_at: None,
        }
    }

    fn service() -> ServiceId {
        ServiceId::new("auth-service").unwrap()
    }

    #[test]
    fn healthy_and_degraded_are_successful_observations() {
        assert!(outcome_for(
            &service(),
            &health(HealthState::Healthy, HealthState::Healthy)
        )
        .is_ok());
        assert!(outcome_for(
            &service(),
            &health(HealthState::Degraded, HealthState::Unhealthy)
        )
        .is_ok());
    }

    #[test]
    fn required_failures_map_to_stable_codes() {
        match outcome_for(
            &service(),
            &health(HealthState::Unhealthy, HealthState::Unhealthy),
        ) {
            Err(TaskExecutorError::Failed { code, detail, .. }) => {
                assert_eq!(code, "HEALTH_CHECK_FAILED");
                assert_eq!(detail.as_deref(), Some("connection refused"));
            }
            other => panic!("expected a structured failure, got {other:?}"),
        }
        match outcome_for(
            &service(),
            &health(HealthState::Unreachable, HealthState::Unreachable),
        ) {
            Err(TaskExecutorError::Failed { code, .. }) => assert_eq!(code, "HEALTH_UNREACHABLE"),
            other => panic!("expected a structured failure, got {other:?}"),
        }
    }

    #[test]
    fn cancellation_is_preserved() {
        let mapped = map_health_error(HealthError::OperationCancelled { service: service() });
        assert_eq!(mapped, TaskExecutorError::Cancelled);
    }

    #[test]
    fn registering_health_executors_wires_every_health_kind() {
        let mut registry = ExecutorRegistry::new();
        let config = service_orchestrator_config::load_from_str(
            "config_version = 1\n[[workspaces]]\nid = \"acme\"\nroot = \"/tmp/acme\"\n",
            "<test>",
        )
        .unwrap();
        let manager = Arc::new(HealthManager::new(
            Arc::new(config),
            Arc::new(service_orchestrator_application::NullServiceRuntimeIndex),
            Arc::new(service_orchestrator_application::InMemoryRuntimeStateStore::new()),
        ));
        register_health_executors(&mut registry, manager);
        for kind in [
            TaskKind::HealthCheck,
            TaskKind::ProcessHealthCheck,
            TaskKind::TcpHealthCheck,
            TaskKind::HttpHealthCheck,
            TaskKind::ReadinessCheck,
        ] {
            assert!(registry.contains(&kind), "missing executor for {kind}");
        }
        // A task id is not needed to assert registration, but pin the type use.
        let _ = TaskId::new("task-1").unwrap();
    }
}
