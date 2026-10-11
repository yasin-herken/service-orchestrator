//! The managed-process registry.
//!
//! This is the runtime adapter's memory of which service processes *it* owns.
//! Ownership is the point: a started process must outlive the task that started
//! it (`tasks/010.md` §19, §20), so the adapter keeps a cloneable
//! [`ProcessHandle`] for every service it started, keyed by the workspace-aware
//! service id. The registry records the process id, pid, runtime, redacted
//! startup command, and start time, and derives live state from the handle's
//! snapshot so it is never stale.
//!
//! The registry never spawns, signals, or reaps: those belong to the process
//! manager. It only *tracks*.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use service_orchestrator_application::ServiceProcessInfo;
use service_orchestrator_domain::{ProcessState, ServiceId, WorkspaceId};
use service_orchestrator_process::{ProcessHandle, ProcessId};

use crate::runtime::RuntimeType;

/// One tracked service process.
#[derive(Clone, Debug)]
pub(crate) struct ManagedService {
    /// The owning workspace.
    pub workspace_id: WorkspaceId,
    /// The tracked service.
    pub service_id: ServiceId,
    /// The managed process identifier.
    pub process_id: ProcessId,
    /// The runtime that started it.
    pub runtime: RuntimeType,
    /// The redacted startup command.
    pub command: String,
    /// The live process handle.
    pub handle: ProcessHandle,
}

impl ManagedService {
    /// Builds an interface-neutral snapshot of this process.
    pub(crate) fn info(&self) -> ServiceProcessInfo {
        let snapshot = self.handle.snapshot();
        ServiceProcessInfo {
            workspace_id: self.workspace_id.clone(),
            service_id: self.service_id.clone(),
            process_id: Some(self.process_id.as_str().to_owned()),
            pid: snapshot.pid,
            state: snapshot.domain_state(),
            runtime: Some(self.runtime.label().to_owned()),
            command: Some(self.command.clone()),
            started_at: snapshot.started_at,
            exit_code: snapshot.exit.as_ref().and_then(|exit| exit.code),
            exit_reason: snapshot
                .exit
                .as_ref()
                .map(|exit| exit.reason.label().to_owned()),
            last_error: snapshot.last_error,
        }
    }

    /// Returns the logical state derived from the live handle.
    pub(crate) fn state(&self) -> ProcessState {
        self.handle.status().domain_state()
    }
}

/// An in-memory, shared registry of managed service processes.
#[derive(Clone, Default)]
pub(crate) struct ManagedProcessRegistry {
    services: Arc<Mutex<BTreeMap<ServiceId, ManagedService>>>,
}

impl ManagedProcessRegistry {
    /// Inserts or replaces the record for a service.
    pub(crate) fn insert(&self, record: ManagedService) {
        let mut services = self.services.lock().unwrap_or_else(PoisonError::into_inner);
        services.insert(record.service_id.clone(), record);
    }

    /// Returns a clone of the record for a service, if tracked.
    pub(crate) fn get(&self, service_id: &ServiceId) -> Option<ManagedService> {
        let services = self.services.lock().unwrap_or_else(PoisonError::into_inner);
        services.get(service_id).cloned()
    }

    /// Removes and returns a record.
    pub(crate) fn remove(&self, service_id: &ServiceId) -> Option<ManagedService> {
        let mut services = self.services.lock().unwrap_or_else(PoisonError::into_inner);
        services.remove(service_id)
    }

    /// Returns the live snapshot for a service, if tracked.
    pub(crate) fn info(&self, service_id: &ServiceId) -> Option<ServiceProcessInfo> {
        self.get(service_id).map(|record| record.info())
    }

    /// Returns live snapshots for every tracked service, in service-id order.
    pub(crate) fn infos(&self) -> Vec<ServiceProcessInfo> {
        let services = self.services.lock().unwrap_or_else(PoisonError::into_inner);
        services.values().map(ManagedService::info).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The registry is exercised end-to-end through the manager's process tests;
    // here we only pin the empty behaviour so the type is never a silent no-op.
    #[test]
    fn an_empty_registry_tracks_nothing() {
        let registry = ManagedProcessRegistry::default();
        let service = ServiceId::new("auth-service").unwrap();
        assert!(registry.get(&service).is_none());
        assert!(registry.info(&service).is_none());
        assert!(registry.infos().is_empty());
    }
}
