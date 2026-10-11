//! Per-artifact build coordination.
//!
//! Two operations that build the *same* module must not run at once: a build and
//! an install both write into the module's `target` directory and the local
//! repository, and interleaving them can corrupt shared artifact state
//! (`tasks/009.md` §23). [`ArtifactCoordinator`] serializes exactly those
//! operations — and nothing else.
//!
//! # Scope, not a global lock
//!
//! The coordinator is a keyed write lock: the key is the module's absolute
//! directory, so unrelated libraries and services still build concurrently. It
//! is intentionally not a scheduler — the task engine remains the only
//! concurrency authority (`AGENTS.md` §41). The coordinator only removes the
//! specific hazard of two writers racing over one module.
//!
//! # Why per module
//!
//! An artifact's identity is a Maven coordinate, but a coordinate maps to a
//! module directory during a build. Keying on the directory also covers reactor
//! projects, where several coordinates share one module directory, without
//! teaching the build adapter about dependency versions.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// Serializes build operations that write to the same module.
///
/// Cloning shares the same lock table, so every [`BuildManager`] clone (and
/// therefore every task executor) coordinates on the same keys.
///
/// [`BuildManager`]: crate::BuildManager
#[derive(Clone, Default)]
pub struct ArtifactCoordinator {
    locks: Arc<Mutex<BTreeMap<String, Arc<AsyncMutex<()>>>>>,
}

impl ArtifactCoordinator {
    /// Creates an empty coordinator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquires the write lock for `key`, waiting until it is free.
    ///
    /// The returned guard releases the lock when it is dropped. Distinct keys
    /// never contend.
    pub async fn acquire(&self, key: &str) -> OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self
                .locks
                .lock()
                .expect("the coordinator lock table is never poisoned");
            locks
                .entry(key.to_owned())
                .or_insert_with(|| Arc::new(AsyncMutex::new(())))
                .clone()
        };
        lock.lock_owned().await
    }
}

impl std::fmt::Debug for ArtifactCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tracked = self
            .locks
            .lock()
            .map(|locks| locks.len())
            .unwrap_or_default();
        f.debug_struct("ArtifactCoordinator")
            .field("tracked_keys", &tracked)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[tokio::test]
    async fn distinct_keys_do_not_contend() {
        let coordinator = ArtifactCoordinator::new();
        let _first = coordinator.acquire("/w/a").await;
        // Acquiring a different key must not block.
        let _second = coordinator.acquire("/w/b").await;
    }

    #[tokio::test]
    async fn the_same_key_serializes_writers() {
        let coordinator = ArtifactCoordinator::new();
        let active = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));

        let guard = coordinator.acquire("/w/a").await;

        let other = {
            let coordinator = coordinator.clone();
            let active = active.clone();
            let max = max.clone();
            tokio::spawn(async move {
                let _guard = coordinator.acquire("/w/a").await;
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                max.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
                active.fetch_sub(1, Ordering::SeqCst);
            })
        };

        // Give the spawned task time to reach the lock, then confirm it has not
        // entered its critical section while we hold the guard.
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(max.load(Ordering::SeqCst), 0);
        drop(guard);
        other.await.unwrap();
        assert_eq!(max.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cloning_shares_the_lock_table() {
        let coordinator = ArtifactCoordinator::new();
        let clone = coordinator.clone();
        assert_eq!(
            Arc::strong_count(&coordinator.locks),
            Arc::strong_count(&clone.locks)
        );
    }
}
