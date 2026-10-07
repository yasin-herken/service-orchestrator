//! Runtime state storage.
//!
//! Persistent state (workspaces, services, profiles, groups) lives in the
//! validated configuration. **Runtime state** — whether a process is running,
//! the last observed health, the current Git state — is transient and changes
//! constantly, so it lives in a separate, focused store ([`RuntimeStateStore`]).
//!
//! The store is a port: the process, health, and Git adapters (built later)
//! publish observed state into it, and application queries read it. This task
//! ships a pure in-memory implementation, which is the documented MVP choice:
//! there is no database, and runtime state does not survive a restart.
//!
//! # Reads are side-effect free
//!
//! A query such as `get_service_status` reads the *last observed* state from
//! this store. It never probes a process, fetches a repository, or connects to
//! a port. Probing happens inside tasks.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use service_orchestrator_domain::{Branch, GitState, ProcessState, ServiceId};

/// The last observed health of a service.
///
/// This is the application's view of health, not a probe result. A service
/// whose process is [`ProcessState::Running`] is not necessarily
/// [`HealthState::Healthy`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    /// Health has not been observed.
    #[default]
    Unknown,
    /// The last health check succeeded.
    Healthy,
    /// The last health check failed.
    Unhealthy,
}

/// A snapshot of a service's transient runtime state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceRuntimeState {
    /// The logical state of the service's process.
    pub process: ProcessState,
    /// The last observed health.
    pub health: HealthState,
    /// The last observed Git state.
    pub git: GitState,
    /// The branch currently checked out, if known.
    pub branch: Option<Branch>,
}

impl Default for ServiceRuntimeState {
    fn default() -> Self {
        Self {
            process: ProcessState::NotRunning,
            health: HealthState::Unknown,
            git: GitState::Unknown,
            branch: None,
        }
    }
}

impl ServiceRuntimeState {
    /// Creates a runtime state snapshot with all values unknown/not running.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a copy with the process state replaced.
    #[must_use]
    pub fn with_process(mut self, process: ProcessState) -> Self {
        self.process = process;
        self
    }

    /// Returns a copy with the health state replaced.
    #[must_use]
    pub fn with_health(mut self, health: HealthState) -> Self {
        self.health = health;
        self
    }

    /// Returns a copy with the Git state replaced.
    #[must_use]
    pub fn with_git(mut self, git: GitState) -> Self {
        self.git = git;
        self
    }

    /// Returns a copy with the checked-out branch replaced.
    #[must_use]
    pub fn with_branch(mut self, branch: Branch) -> Self {
        self.branch = Some(branch);
        self
    }
}

/// Stores and retrieves transient runtime state for services.
///
/// Implementations must be safe to share across threads because a single
/// application instance is used by an interface while background task execution
/// updates state.
pub trait RuntimeStateStore: Send + Sync {
    /// Returns the last observed state for `service_id`, if any.
    fn service_state(&self, service_id: &ServiceId) -> Option<ServiceRuntimeState>;

    /// Records the observed state for `service_id`.
    fn set_service_state(&self, service_id: &ServiceId, state: ServiceRuntimeState);
}

/// An in-memory [`RuntimeStateStore`].
///
/// This is the MVP implementation: it is cheap, dependency-free, and does not
/// survive a process restart. A later task may replace it with a store backed
/// by the process manager; the port keeps that change local.
#[derive(Debug, Default)]
pub struct InMemoryRuntimeStateStore {
    states: Mutex<BTreeMap<ServiceId, ServiceRuntimeState>>,
}

impl InMemoryRuntimeStateStore {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<ServiceId, ServiceRuntimeState>> {
        self.states.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl RuntimeStateStore for InMemoryRuntimeStateStore {
    fn service_state(&self, service_id: &ServiceId) -> Option<ServiceRuntimeState> {
        self.lock().get(service_id).cloned()
    }

    fn set_service_state(&self, service_id: &ServiceId, state: ServiceRuntimeState) {
        self.lock().insert(service_id.clone(), state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_not_running_and_unknown() {
        let state = ServiceRuntimeState::default();
        assert_eq!(state.process, ProcessState::NotRunning);
        assert_eq!(state.health, HealthState::Unknown);
        assert_eq!(state.git, GitState::Unknown);
        assert!(state.branch.is_none());
    }

    #[test]
    fn in_memory_store_round_trips_state() {
        let store = InMemoryRuntimeStateStore::new();
        let service = ServiceId::new("auth-service").unwrap();
        assert!(store.service_state(&service).is_none());

        store.set_service_state(
            &service,
            ServiceRuntimeState::new()
                .with_process(ProcessState::Running)
                .with_health(HealthState::Healthy)
                .with_branch(Branch::new("main").unwrap()),
        );

        let state = store.service_state(&service).unwrap();
        assert_eq!(state.process, ProcessState::Running);
        assert_eq!(state.health, HealthState::Healthy);
        assert_eq!(state.branch.unwrap().as_str(), "main");
    }

    #[test]
    fn builder_methods_are_chainable() {
        let state = ServiceRuntimeState::new()
            .with_process(ProcessState::Stopped)
            .with_git(GitState::Clean);
        assert_eq!(state.process, ProcessState::Stopped);
        assert_eq!(state.git, GitState::Clean);
    }
}
