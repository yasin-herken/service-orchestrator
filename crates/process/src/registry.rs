//! The in-memory process registry.
//!
//! The registry is the manager's authoritative index of every process it has
//! spawned, keyed by [`ProcessId`]. It supports lookup by process id and by
//! service id and can list all processes or only the active ones. Completed
//! processes remain queryable until they are explicitly removed, so recent
//! exit information is available for inspection — mirroring the task engine's
//! "recent tasks" behaviour.
//!
//! A single `std::sync::Mutex` guards the map. Critical sections are short and
//! the lock is never held across an `.await`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use service_orchestrator_domain::ServiceId;

use crate::entry::ProcessEntry;
use crate::handle::ProcessHandle;
use crate::id::ProcessId;
use crate::snapshot::ProcessSnapshot;

/// An in-memory index of managed processes.
#[derive(Default)]
pub struct ProcessRegistry {
    entries: Mutex<BTreeMap<ProcessId, Arc<ProcessEntry>>>,
}

impl ProcessRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts an entry, replacing any existing process with the same id.
    pub(crate) fn insert(&self, entry: Arc<ProcessEntry>) {
        self.entries
            .lock()
            .expect("process registry mutex poisoned")
            .insert(entry.id().clone(), entry);
    }

    /// Returns a handle for a process, if it exists.
    #[must_use]
    pub fn get(&self, id: &ProcessId) -> Option<ProcessHandle> {
        self.entry(id).map(ProcessHandle::new)
    }

    /// Returns `true` if a process with `id` is registered.
    #[must_use]
    pub fn contains(&self, id: &ProcessId) -> bool {
        self.entries
            .lock()
            .expect("process registry mutex poisoned")
            .contains_key(id)
    }

    /// Removes a process from the registry, returning whether it existed.
    pub fn remove(&self, id: &ProcessId) -> bool {
        self.entries
            .lock()
            .expect("process registry mutex poisoned")
            .remove(id)
            .is_some()
    }

    /// Returns a snapshot of a process, if it exists.
    #[must_use]
    pub fn snapshot(&self, id: &ProcessId) -> Option<ProcessSnapshot> {
        self.entry(id).map(|entry| entry.snapshot())
    }

    /// Lists snapshots of every registered process, oldest id first.
    #[must_use]
    pub fn list(&self) -> Vec<ProcessSnapshot> {
        self.entries
            .lock()
            .expect("process registry mutex poisoned")
            .values()
            .map(|entry| entry.snapshot())
            .collect()
    }

    /// Lists snapshots of every active (non-terminal) process.
    #[must_use]
    pub fn list_active(&self) -> Vec<ProcessSnapshot> {
        self.entries
            .lock()
            .expect("process registry mutex poisoned")
            .values()
            .filter(|entry| !entry.status().is_terminal())
            .map(|entry| entry.snapshot())
            .collect()
    }

    /// Returns handles for every process (active or finished).
    #[must_use]
    pub fn handles(&self) -> Vec<ProcessHandle> {
        self.entries
            .lock()
            .expect("process registry mutex poisoned")
            .values()
            .cloned()
            .map(ProcessHandle::new)
            .collect()
    }

    /// Returns handles for every active process.
    #[must_use]
    pub fn active_handles(&self) -> Vec<ProcessHandle> {
        self.entries
            .lock()
            .expect("process registry mutex poisoned")
            .values()
            .filter(|entry| !entry.status().is_terminal())
            .cloned()
            .map(ProcessHandle::new)
            .collect()
    }

    /// Returns handles for every process associated with `service`.
    #[must_use]
    pub fn find_by_service(&self, service: &ServiceId) -> Vec<ProcessHandle> {
        self.entries
            .lock()
            .expect("process registry mutex poisoned")
            .values()
            .filter(|entry| entry.service() == Some(service))
            .cloned()
            .map(ProcessHandle::new)
            .collect()
    }

    /// Returns the number of registered processes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .expect("process registry mutex poisoned")
            .len()
    }

    /// Returns `true` if no processes are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn entry(&self, id: &ProcessId) -> Option<Arc<ProcessEntry>> {
        self.entries
            .lock()
            .expect("process registry mutex poisoned")
            .get(id)
            .cloned()
    }
}

impl std::fmt::Debug for ProcessRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessRegistry")
            .field("processes", &self.len())
            .finish()
    }
}
