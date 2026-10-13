//! The health manager.
//!
//! [`HealthManager`] is the health adapter's coordinator. It reads the
//! validated configuration, resolves the providers through a
//! [`HealthCheckerRegistry`], runs one or more probes with per-check timeouts,
//! aggregates the outcomes deterministically, and records the result in a
//! [`HealthStore`]. It publishes [`HealthEvent`]s and mirrors the aggregate into
//! the application's [`RuntimeStateStore`], so status queries and diagnostics see
//! the same observation.
//!
//! # Responsibilities are not duplicated
//!
//! The manager owns *interpretation* and *bookkeeping* only. The process manager
//! owns process lifecycle, the providers own the probe mechanics, the task
//! engine owns scheduling, timeouts, and cancellation, and the application core
//! owns the use cases on top (`tasks/012.md` §3). The manager is driven either
//! by a task executor ([`crate::executor`]) or by the periodic
//! [`HealthMonitor`](crate::monitor).
//!
//! # Deterministic aggregation
//!
//! The rules are fixed and never average into a percentage
//! (`tasks/012.md` §14):
//!
//! - any required check **unhealthy** → `unhealthy`;
//! - otherwise any required check **unreachable** → `unreachable`;
//! - otherwise any required check **undetermined** (unknown/checking) → `checking`;
//! - otherwise any optional check failed → `degraded`;
//! - otherwise → `healthy`;
//! - no checks at all → `unknown`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use service_orchestrator_application::{
    HealthCheckOutcome, HealthCheckScope, HealthIndex, HealthState, RuntimeStateStore,
    ServiceHealth, ServiceRuntimeIndex,
};
use service_orchestrator_config::{Config, ConfigManager, StaticConfigManager};
use service_orchestrator_domain::{
    HealthCheck, HealthCheckKind, ServiceId, Timestamp, WorkspaceId,
};
use service_orchestrator_execution::Cancellation;

use crate::checker::{check_target, CheckType, HealthCheckContext};
use crate::error::HealthError;
use crate::events::{HealthEvent, HealthEventBus, HealthEventStream};
use crate::history::HealthStore;
use crate::registry::HealthCheckerRegistry;

/// The default timeout applied to a check that does not configure one.
pub const DEFAULT_CHECK_TIMEOUT_SECONDS: u64 = 5;

/// The default interval between readiness polls, in milliseconds.
pub const DEFAULT_POLL_INTERVAL_MILLIS: u64 = 500;

/// The capacity of the manager's event bus.
const EVENT_BUS_CAPACITY: usize = 256;

/// Runs health checks, aggregates them, and records the result.
pub struct HealthManager {
    config: Arc<dyn ConfigManager>,
    runtime_state: Arc<dyn RuntimeStateStore>,
    checkers: Arc<HealthCheckerRegistry>,
    store: Arc<HealthStore>,
    events: HealthEventBus,
    default_timeout: Duration,
    default_interval: Duration,
}

impl HealthManager {
    /// Creates a manager over validated configuration and its read-only ports.
    ///
    /// The three standard providers are registered: the process provider reads
    /// managed-process state from `service_runtime`, and the TCP and HTTP
    /// providers are stateless.
    #[must_use]
    pub fn new(
        config: Arc<Config>,
        service_runtime: Arc<dyn ServiceRuntimeIndex>,
        runtime_state: Arc<dyn RuntimeStateStore>,
    ) -> Self {
        Self::with_config_manager(
            Arc::new(StaticConfigManager::new((*config).clone())),
            service_runtime,
            runtime_state,
        )
    }

    /// Replaces the configuration manager with a shared, live one.
    ///
    /// The daemon wires the single configuration manager here so a committed
    /// change is observed by the next health check (`tasks/023.md` §39, §40).
    #[must_use]
    pub fn with_shared_config(mut self, config: Arc<dyn ConfigManager>) -> Self {
        self.config = config;
        self
    }

    /// Creates a manager over a shared, live configuration.
    ///
    /// Every check reads the manager's current snapshot, so a committed
    /// configuration change is honoured immediately (`tasks/023.md` §39, §40).
    #[must_use]
    pub fn with_config_manager(
        config: Arc<dyn ConfigManager>,
        service_runtime: Arc<dyn ServiceRuntimeIndex>,
        runtime_state: Arc<dyn RuntimeStateStore>,
    ) -> Self {
        Self {
            config,
            runtime_state,
            checkers: Arc::new(HealthCheckerRegistry::standard(service_runtime)),
            store: Arc::new(HealthStore::default()),
            events: HealthEventBus::new(EVENT_BUS_CAPACITY),
            default_timeout: Duration::from_secs(DEFAULT_CHECK_TIMEOUT_SECONDS),
            default_interval: Duration::from_millis(DEFAULT_POLL_INTERVAL_MILLIS),
        }
    }

    /// Replaces the checker registry.
    #[must_use]
    pub fn with_checkers(mut self, checkers: Arc<HealthCheckerRegistry>) -> Self {
        self.checkers = checkers;
        self
    }

    /// Replaces the health store.
    #[must_use]
    pub fn with_store(mut self, store: Arc<HealthStore>) -> Self {
        self.store = store;
        self
    }

    /// Overrides the default per-check timeout.
    #[must_use]
    pub fn with_check_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    /// Overrides the default readiness poll interval.
    #[must_use]
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.default_interval = interval;
        self
    }

    /// Returns the health store the manager writes to.
    #[must_use]
    pub fn store(&self) -> Arc<HealthStore> {
        Arc::clone(&self.store)
    }

    /// Subscribes to health events.
    #[must_use]
    pub fn subscribe(&self) -> HealthEventStream {
        self.events.subscribe()
    }

    /// Returns the default per-check timeout.
    #[must_use]
    pub fn check_timeout(&self) -> Duration {
        self.default_timeout
    }

    /// Runs the configured checks selected by `scope` and records the result.
    ///
    /// Every selected check is executed — an ordinary failing observation is a
    /// fact, not an error. A [`HealthError`] is returned only when the check
    /// could not be attempted at all: the service is unknown, no check matches
    /// the scope, a check kind has no provider, or a provider reports a
    /// configuration failure (`tasks/012.md` §35).
    ///
    /// # Errors
    ///
    /// Returns a [`HealthError`] as described above.
    pub async fn check(
        &self,
        service_id: &ServiceId,
        scope: HealthCheckScope,
    ) -> Result<ServiceHealth, HealthError> {
        let config = self.config.current();
        let workspace = config.workspace_of_service(service_id).ok_or_else(|| {
            HealthError::ServiceNotFound {
                service: service_id.clone(),
            }
        })?;
        let workspace_id = workspace.workspace.id.clone();
        let service = workspace
            .workspace
            .service(service_id)
            .expect("workspace_of_service returned a workspace that owns the service");

        let checks: Vec<HealthCheck> = service
            .health_checks
            .iter()
            .filter(|check| scope_includes(scope, &check.kind))
            .cloned()
            .collect();
        if checks.is_empty() {
            return Err(HealthError::HealthConfigurationInvalid {
                service: service_id.clone(),
                message: format!(
                    "service '{service_id}' has no '{}' health checks configured",
                    scope.label()
                ),
            });
        }
        self.run(&workspace_id, service_id, &checks).await
    }

    /// Polls a service's required checks until it is ready or `timeout` elapses.
    ///
    /// The process liveness, network reachability, and application readiness
    /// stages are represented by the configured required checks. A service is
    /// ready when the aggregate of its required checks is serving
    /// (`tasks/012.md` §12, §16).
    ///
    /// # Errors
    ///
    /// Returns [`HealthError::HealthNotReady`] if the deadline elapses first,
    /// [`HealthError::OperationCancelled`] if cancellation is observed, or
    /// another [`HealthError`] if a check cannot be attempted.
    pub async fn wait_until_ready(
        &self,
        service_id: &ServiceId,
        timeout: Duration,
        cancellation: Option<Cancellation>,
    ) -> Result<ServiceHealth, HealthError> {
        let config = self.config.current();
        let workspace = config.workspace_of_service(service_id).ok_or_else(|| {
            HealthError::ServiceNotFound {
                service: service_id.clone(),
            }
        })?;
        let workspace_id = workspace.workspace.id.clone();
        let service = workspace
            .workspace
            .service(service_id)
            .expect("workspace_of_service returned a workspace that owns the service");
        let required: Vec<HealthCheck> = service
            .health_checks
            .iter()
            .filter(|check| check.required)
            .cloned()
            .collect();
        if required.is_empty() {
            return Err(HealthError::HealthConfigurationInvalid {
                service: service_id.clone(),
                message: format!("service '{service_id}' has no required health checks"),
            });
        }

        let deadline = Instant::now() + timeout;
        let interval = self.poll_interval(&required);
        loop {
            if cancellation
                .as_ref()
                .is_some_and(Cancellation::is_cancelled)
            {
                return Err(HealthError::OperationCancelled {
                    service: service_id.clone(),
                });
            }

            let health = self.run(&workspace_id, service_id, &required).await?;
            if health.state.is_serving() {
                return Ok(health);
            }
            if Instant::now() >= deadline {
                return Err(HealthError::HealthNotReady {
                    service: service_id.clone(),
                    message: format!(
                        "the service is still {} after {} ms",
                        health.state.label(),
                        timeout.as_millis()
                    ),
                });
            }
            if !self.sleep_or_cancel(interval, cancellation.as_ref()).await {
                return Err(HealthError::OperationCancelled {
                    service: service_id.clone(),
                });
            }
        }
    }

    /// Runs a concrete list of checks and records the aggregate.
    async fn run(
        &self,
        workspace_id: &WorkspaceId,
        service_id: &ServiceId,
        checks: &[HealthCheck],
    ) -> Result<ServiceHealth, HealthError> {
        let mut outcomes = Vec::with_capacity(checks.len());
        for check in checks {
            outcomes.push(self.run_check(workspace_id, service_id, check).await?);
        }

        let state = aggregate_health(&outcomes);
        let previous = self
            .store
            .service_health(service_id)
            .map_or(HealthState::Unknown, |health| health.state);

        // Never leave the service reporting `checking` once a run completes;
        // `checking` is only meaningful while a probe is in flight.
        let checked_at = Timestamp::now();
        let health = self.store.record(
            workspace_id.clone(),
            service_id.clone(),
            state,
            outcomes,
            checked_at,
        );

        if previous != state {
            self.events.publish(HealthEvent::ServiceHealthChanged {
                workspace_id: workspace_id.clone(),
                service_id: service_id.clone(),
                previous,
                current: state,
            });
        }
        self.update_runtime_state(service_id, state);
        Ok(health)
    }

    /// Runs one check, publishing started/finished events.
    async fn run_check(
        &self,
        workspace_id: &WorkspaceId,
        service_id: &ServiceId,
        check: &HealthCheck,
    ) -> Result<HealthCheckOutcome, HealthError> {
        let check_type = CheckType::from_kind(&check.kind).ok_or_else(|| {
            HealthError::HealthCheckUnsupported {
                service: service_id.clone(),
                check_type: check.kind.label().to_owned(),
            }
        })?;
        let checker =
            self.checkers
                .get(check_type)
                .ok_or_else(|| HealthError::HealthCheckUnsupported {
                    service: service_id.clone(),
                    check_type: check_type.label().to_owned(),
                })?;

        self.events.publish(HealthEvent::HealthCheckStarted {
            workspace_id: workspace_id.clone(),
            service_id: service_id.clone(),
            check_type: check_type.label().to_owned(),
        });

        let timeout = check.timeout.unwrap_or(self.default_timeout);
        let context = HealthCheckContext {
            workspace_id,
            service_id,
            check,
            timeout,
        };
        let started = Instant::now();
        let (status, target, message) =
            match tokio::time::timeout(timeout, checker.check(&context)).await {
                Ok(Ok(result)) => (result.status, result.target, result.message),
                // A provider error means the check could not be attempted; the
                // manager reports it rather than inventing an outcome.
                Ok(Err(error)) => return Err(error),
                Err(_elapsed) => (
                    HealthState::Unreachable,
                    check_target(&check.kind),
                    Some(format!(
                        "the {} check timed out after {} ms",
                        check_type.label(),
                        timeout.as_millis()
                    )),
                ),
            };
        let duration_ms = millis(started.elapsed());

        let outcome = HealthCheckOutcome {
            check_type: check_type.label().to_owned(),
            target: target.or_else(|| check_target(&check.kind)),
            required: check.required,
            status,
            duration_ms,
            message: message.clone(),
            checked_at: Timestamp::now(),
        };

        if outcome.is_healthy() {
            self.events.publish(HealthEvent::HealthCheckSucceeded {
                workspace_id: workspace_id.clone(),
                service_id: service_id.clone(),
                check_type: outcome.check_type.clone(),
                duration_ms,
            });
        } else {
            self.events.publish(HealthEvent::HealthCheckFailed {
                workspace_id: workspace_id.clone(),
                service_id: service_id.clone(),
                check_type: outcome.check_type.clone(),
                status,
                message: message
                    .unwrap_or_else(|| format!("the {} check did not pass", check_type.label())),
                duration_ms,
            });
        }
        Ok(outcome)
    }

    /// Mirrors the aggregate health into the application's runtime state store.
    fn update_runtime_state(&self, service_id: &ServiceId, state: HealthState) {
        let current = self
            .runtime_state
            .service_state(service_id)
            .unwrap_or_default();
        self.runtime_state
            .set_service_state(service_id, current.with_health(state));
    }

    /// Returns the smallest configured check interval, or the default.
    fn poll_interval(&self, checks: &[HealthCheck]) -> Duration {
        checks
            .iter()
            .filter_map(|check| check.interval)
            .min()
            .unwrap_or(self.default_interval)
    }

    /// Sleeps for `interval`, returning `false` if cancellation is observed.
    async fn sleep_or_cancel(
        &self,
        interval: Duration,
        cancellation: Option<&Cancellation>,
    ) -> bool {
        match cancellation {
            Some(cancellation) => tokio::select! {
                () = tokio::time::sleep(interval) => true,
                () = cancellation.cancelled() => false,
            },
            None => {
                tokio::time::sleep(interval).await;
                true
            }
        }
    }
}

impl HealthIndex for HealthManager {
    fn service_health(&self, service_id: &ServiceId) -> Option<ServiceHealth> {
        self.store.service_health(service_id)
    }

    fn services(&self) -> Vec<ServiceHealth> {
        self.store.services()
    }
}

impl std::fmt::Debug for HealthManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HealthManager")
            .field("checkers", &self.checkers.types())
            .field("default_timeout", &self.default_timeout)
            .field("default_interval", &self.default_interval)
            .finish_non_exhaustive()
    }
}

/// Returns `true` when `scope` selects checks of `kind`.
#[must_use]
pub fn scope_includes(scope: HealthCheckScope, kind: &HealthCheckKind) -> bool {
    match scope {
        HealthCheckScope::All => true,
        HealthCheckScope::Process => matches!(kind, HealthCheckKind::Process),
        HealthCheckScope::Tcp => matches!(kind, HealthCheckKind::Port(_)),
        HealthCheckScope::Http => matches!(kind, HealthCheckKind::Http(_)),
    }
}

/// Aggregates individual check outcomes into a service's overall health.
///
/// The rules are deterministic and deliberately never average into a
/// percentage (`tasks/012.md` §14). A required failure dominates an optional
/// one; an unreachable result dominates an undetermined one.
#[must_use]
pub fn aggregate_health(outcomes: &[HealthCheckOutcome]) -> HealthState {
    if outcomes.is_empty() {
        return HealthState::Unknown;
    }

    let mut required_unhealthy = false;
    let mut required_unreachable = false;
    let mut required_pending = false;
    let mut optional_failed = false;

    for outcome in outcomes {
        if outcome.required {
            match outcome.status {
                HealthState::Healthy => {}
                HealthState::Unhealthy | HealthState::Degraded => required_unhealthy = true,
                HealthState::Unreachable => required_unreachable = true,
                HealthState::Unknown | HealthState::Checking => required_pending = true,
            }
        } else if !outcome.status.is_healthy() {
            optional_failed = true;
        }
    }

    if required_unhealthy {
        HealthState::Unhealthy
    } else if required_unreachable {
        HealthState::Unreachable
    } else if required_pending {
        HealthState::Checking
    } else if optional_failed {
        HealthState::Degraded
    } else {
        HealthState::Healthy
    }
}

/// Converts a [`Duration`] to whole milliseconds, saturating at `u64::MAX`.
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_application::CHECK_HTTP;

    fn outcome(status: HealthState, required: bool) -> HealthCheckOutcome {
        HealthCheckOutcome {
            check_type: CHECK_HTTP.to_owned(),
            target: None,
            required,
            status,
            duration_ms: 1,
            message: None,
            checked_at: Timestamp::now(),
        }
    }

    #[test]
    fn aggregation_prefers_required_failures() {
        assert_eq!(
            aggregate_health(&[outcome(HealthState::Healthy, true)]),
            HealthState::Healthy
        );
        assert_eq!(
            aggregate_health(&[
                outcome(HealthState::Healthy, true),
                outcome(HealthState::Unhealthy, false),
            ]),
            HealthState::Degraded
        );
        assert_eq!(
            aggregate_health(&[
                outcome(HealthState::Unhealthy, true),
                outcome(HealthState::Unhealthy, false),
            ]),
            HealthState::Unhealthy
        );
    }

    #[test]
    fn aggregation_distinguishes_unreachable_and_pending() {
        assert_eq!(
            aggregate_health(&[outcome(HealthState::Unreachable, true)]),
            HealthState::Unreachable
        );
        assert_eq!(
            aggregate_health(&[outcome(HealthState::Unknown, true)]),
            HealthState::Checking
        );
        assert_eq!(aggregate_health(&[]), HealthState::Unknown);
    }

    #[test]
    fn scope_selection_is_exhaustive() {
        assert!(scope_includes(
            HealthCheckScope::All,
            &HealthCheckKind::Process
        ));
        assert!(scope_includes(
            HealthCheckScope::Process,
            &HealthCheckKind::Process
        ));
        assert!(!scope_includes(
            HealthCheckScope::Http,
            &HealthCheckKind::Process
        ));
        assert!(scope_includes(
            HealthCheckScope::Tcp,
            &HealthCheckKind::Port(service_orchestrator_domain::Port::new(8081).unwrap())
        ));
    }
}
