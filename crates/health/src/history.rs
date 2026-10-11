//! Bounded health history and the read-only health index.
//!
//! [`HealthStore`] is the health adapter's memory. It keeps the latest
//! [`ServiceHealth`] for every service it has observed plus a bounded ring of
//! recent [`HealthSample`]s, and it is the concrete [`HealthIndex`] the
//! application reads through. Exactly like the runtime state store, reads are
//! side-effect free: a query returns the *last observed* health and never
//! probes anything (`tasks/012.md` §17, §18).
//!
//! # Bounded retention
//!
//! History is retained in memory, never in a database, and is bounded per
//! service. A service that has been polled for days holds at most
//! [`HealthStore::capacity`] samples, so a long-running process cannot exhaust
//! memory through health accumulation (`tasks/012.md` §18).

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, PoisonError};

use service_orchestrator_application::{
    HealthCheckOutcome, HealthIndex, HealthState, ServiceHealth,
};
use service_orchestrator_domain::{ServiceId, Timestamp, WorkspaceId};

/// The default number of health samples retained per service.
pub const DEFAULT_HISTORY_CAPACITY: usize = 50;

/// A single retained health observation.
///
/// A sample is a compact point in the history: the aggregate state at a moment,
/// when it was observed, and how many consecutive observations had failed by
/// then. The full per-check outcomes are kept only for the latest observation,
/// in [`ServiceHealth`], so history stays cheap even when a service has many
/// checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HealthSample {
    /// The aggregate state at the sampled moment.
    pub state: HealthState,
    /// When the sample was observed.
    pub checked_at: Timestamp,
    /// How many consecutive observations had failed by this point.
    pub consecutive_failures: u32,
}

/// One service's retained health.
#[derive(Debug, Default)]
struct ServiceRecord {
    latest: Option<ServiceHealth>,
    samples: VecDeque<HealthSample>,
    consecutive_failures: u32,
    last_healthy_at: Option<Timestamp>,
    last_unhealthy_at: Option<Timestamp>,
}

/// A bounded, shared store of observed service health.
///
/// It is safe to share across threads and cheap to clone behind an [`Arc`]; the
/// manager writes to it and the application reads it through [`HealthIndex`].
///
/// [`Arc`]: std::sync::Arc
#[derive(Debug)]
pub struct HealthStore {
    capacity: usize,
    records: Mutex<BTreeMap<ServiceId, ServiceRecord>>,
}

impl Default for HealthStore {
    fn default() -> Self {
        Self::new(DEFAULT_HISTORY_CAPACITY)
    }
}

impl HealthStore {
    /// Creates a store retaining at most `capacity` samples per service.
    ///
    /// A zero capacity is treated as one so a store always keeps the latest
    /// sample.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            records: Mutex::new(BTreeMap::new()),
        }
    }

    /// Returns the per-service sample capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<ServiceId, ServiceRecord>> {
        self.records.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records a completed observation and returns the resulting aggregate.
    ///
    /// The store owns the history bookkeeping: it counts consecutive failures,
    /// remembers the last healthy and unhealthy instants, and trims history to
    /// the configured capacity. The `state` is the manager's already-aggregated
    /// result; the store never re-derives it (`tasks/012.md` §14, §18).
    pub fn record(
        &self,
        workspace_id: WorkspaceId,
        service_id: ServiceId,
        state: HealthState,
        checks: Vec<HealthCheckOutcome>,
        checked_at: Timestamp,
    ) -> ServiceHealth {
        let mut records = self.lock();
        let record = records.entry(service_id.clone()).or_default();

        if state.is_healthy() {
            record.consecutive_failures = 0;
            record.last_healthy_at = Some(checked_at);
        } else {
            record.consecutive_failures = record.consecutive_failures.saturating_add(1);
            record.last_unhealthy_at = Some(checked_at);
        }

        let health = ServiceHealth {
            workspace_id,
            service_id,
            state,
            checked_at: Some(checked_at),
            checks,
            consecutive_failures: record.consecutive_failures,
            last_healthy_at: record.last_healthy_at,
            last_unhealthy_at: record.last_unhealthy_at,
        };
        record.latest = Some(health.clone());
        record.samples.push_back(HealthSample {
            state,
            checked_at,
            consecutive_failures: record.consecutive_failures,
        });
        while record.samples.len() > self.capacity {
            record.samples.pop_front();
        }
        health
    }

    /// Returns the retained samples for a service, oldest first.
    #[must_use]
    pub fn history(&self, service_id: &ServiceId) -> Vec<HealthSample> {
        self.lock()
            .get(service_id)
            .map(|record| record.samples.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Returns the number of samples retained for a service.
    #[must_use]
    pub fn history_len(&self, service_id: &ServiceId) -> usize {
        self.lock()
            .get(service_id)
            .map_or(0, |record| record.samples.len())
    }

    /// Removes every record, returning the store to its initial state.
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// Returns `true` when no service has been observed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }
}

impl HealthIndex for HealthStore {
    fn service_health(&self, service_id: &ServiceId) -> Option<ServiceHealth> {
        self.lock()
            .get(service_id)
            .and_then(|record| record.latest.clone())
    }

    fn services(&self) -> Vec<ServiceHealth> {
        self.lock()
            .values()
            .filter_map(|record| record.latest.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_application::{CHECK_HTTP, CHECK_PROCESS};

    fn service() -> ServiceId {
        ServiceId::new("auth-service").unwrap()
    }

    fn workspace() -> WorkspaceId {
        WorkspaceId::new("acme").unwrap()
    }

    fn outcome(check_type: &str, status: HealthState, required: bool) -> HealthCheckOutcome {
        HealthCheckOutcome {
            check_type: check_type.to_owned(),
            target: None,
            required,
            status,
            duration_ms: 1,
            message: None,
            checked_at: Timestamp::now(),
        }
    }

    #[test]
    fn recording_tracks_failures_and_timestamps() {
        let store = HealthStore::new(4);
        let healthy = store.record(
            workspace(),
            service(),
            HealthState::Healthy,
            vec![outcome(CHECK_PROCESS, HealthState::Healthy, true)],
            Timestamp::now(),
        );
        assert_eq!(healthy.consecutive_failures, 0);
        assert!(healthy.last_healthy_at.is_some());
        assert!(healthy.last_unhealthy_at.is_none());

        let unhealthy = store.record(
            workspace(),
            service(),
            HealthState::Unhealthy,
            vec![outcome(CHECK_HTTP, HealthState::Unhealthy, true)],
            Timestamp::now(),
        );
        assert_eq!(unhealthy.consecutive_failures, 1);
        assert!(unhealthy.last_unhealthy_at.is_some());

        let again = store.record(
            workspace(),
            service(),
            HealthState::Unreachable,
            vec![outcome(CHECK_HTTP, HealthState::Unreachable, true)],
            Timestamp::now(),
        );
        assert_eq!(again.consecutive_failures, 2);

        let recovered = store.record(
            workspace(),
            service(),
            HealthState::Healthy,
            vec![outcome(CHECK_PROCESS, HealthState::Healthy, true)],
            Timestamp::now(),
        );
        assert_eq!(recovered.consecutive_failures, 0);
    }

    #[test]
    fn history_is_bounded_to_capacity() {
        let store = HealthStore::new(2);
        for _ in 0..5 {
            store.record(
                workspace(),
                service(),
                HealthState::Healthy,
                Vec::new(),
                Timestamp::now(),
            );
        }
        assert_eq!(store.history_len(&service()), 2);
        assert_eq!(store.history(&service()).len(), 2);
    }

    #[test]
    fn zero_capacity_still_retains_the_latest_sample() {
        let store = HealthStore::new(0);
        store.record(
            workspace(),
            service(),
            HealthState::Healthy,
            Vec::new(),
            Timestamp::now(),
        );
        assert_eq!(store.history_len(&service()), 1);
    }

    #[test]
    fn the_index_reports_the_latest_observation() {
        let store = HealthStore::new(4);
        assert!(store.service_health(&service()).is_none());
        assert!(store.is_empty());

        store.record(
            workspace(),
            service(),
            HealthState::Degraded,
            Vec::new(),
            Timestamp::now(),
        );
        let health = store.service_health(&service()).unwrap();
        assert_eq!(health.state, HealthState::Degraded);
        assert_eq!(store.services().len(), 1);

        store.clear();
        assert!(store.is_empty());
    }
}
