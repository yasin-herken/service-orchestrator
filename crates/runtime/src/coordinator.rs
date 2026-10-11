//! Per-service lifecycle coordination.
//!
//! Two lifecycle operations on the same service must never interleave: starting
//! a service while a slow stop is still winding it down, or two starts racing,
//! would leave the managed-process registry describing a process that no longer
//! exists (`tasks/010.md` §14). The coordinator hands out one async lock per
//! service id, so operations on *different* services still run concurrently while
//! operations on the *same* service serialize.
//!
//! Locks are created on first use and retained; the map is bounded by the number
//! of distinct services a session touches, which is small and stable.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use service_orchestrator_domain::ServiceId;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// A registry of per-service async locks.
#[derive(Clone, Default)]
pub(crate) struct ServiceCoordinator {
    locks: Arc<Mutex<BTreeMap<ServiceId, Arc<AsyncMutex<()>>>>>,
}

impl ServiceCoordinator {
    /// Creates an empty coordinator.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Acquires the lock for `service_id`, waiting if another operation holds it.
    pub(crate) async fn acquire(&self, service_id: &ServiceId) -> OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
            Arc::clone(
                locks
                    .entry(service_id.clone())
                    .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
            )
        };
        lock.lock_owned().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn serializes_the_same_service() {
        let coordinator = ServiceCoordinator::new();
        let service = ServiceId::new("auth-service").unwrap();
        let guard = coordinator.acquire(&service).await;
        // A second acquire on the same service cannot complete while held.
        let nested = coordinator.acquire(&service);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), nested)
                .await
                .is_err()
        );
        drop(guard);
    }

    #[tokio::test]
    async fn different_services_do_not_block_each_other() {
        let coordinator = ServiceCoordinator::new();
        let first = ServiceId::new("auth-service").unwrap();
        let second = ServiceId::new("user-service").unwrap();
        let _first = coordinator.acquire(&first).await;
        let second_guard = coordinator.acquire(&second).await;
        drop(second_guard);
    }
}
