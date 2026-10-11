//! Per-database-target migration coordination.
//!
//! Two migrations aimed at the *same* configured database must not run at once:
//! Liquibase's own change-log lock is a safety net, not a scheduler, and two
//! Service Orchestrator tasks racing over one database can still interleave
//! before the lock is taken (`tasks/011.md` §15). [`DatabaseCoordinator`]
//! serializes exactly those operations — and nothing else.
//!
//! # Scope, not a global lock
//!
//! The coordinator is a keyed write lock: the key is the target database's
//! stable identity (type, host, name, schema), so unrelated databases still
//! migrate concurrently. It is intentionally not a scheduler — the task engine
//! remains the only concurrency authority (`AGENTS.md` §41). The coordinator
//! only removes the specific hazard of two writers racing over one database.
//!
//! # Limitation: in-process only
//!
//! The coordinator coordinates migrations *within one running Service
//! Orchestrator instance*. It does **not** coordinate separate instances, a
//! developer's hand-run Liquibase, or CI. Liquibase's database lock is the only
//! cross-process mechanism, and it is why the coordinator is a defense-in-depth
//! improvement rather than a replacement (`tasks/011.md` §15).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// Serializes migrations that write to the same configured database target.
///
/// Cloning shares the same lock table, so every [`MigrationManager`] clone (and
/// therefore every task executor) coordinates on the same keys.
///
/// [`MigrationManager`]: crate::MigrationManager
#[derive(Clone, Default)]
pub struct DatabaseCoordinator {
    locks: Arc<Mutex<BTreeMap<String, Arc<AsyncMutex<()>>>>>,
}

impl DatabaseCoordinator {
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

impl std::fmt::Debug for DatabaseCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tracked = self
            .locks
            .lock()
            .map(|locks| locks.len())
            .unwrap_or_default();
        f.debug_struct("DatabaseCoordinator")
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
        let coordinator = DatabaseCoordinator::new();
        let _first = coordinator.acquire("type=postgresql;host=a").await;
        // Acquiring a different key must not block.
        let _second = coordinator.acquire("type=postgresql;host=b").await;
    }

    #[tokio::test]
    async fn the_same_key_serializes_writers() {
        let coordinator = DatabaseCoordinator::new();
        let active = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));

        let guard = coordinator.acquire("shared").await;

        let other = {
            let coordinator = coordinator.clone();
            let active = active.clone();
            let max = max.clone();
            tokio::spawn(async move {
                let _guard = coordinator.acquire("shared").await;
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
        let coordinator = DatabaseCoordinator::new();
        let clone = coordinator.clone();
        assert_eq!(
            Arc::strong_count(&coordinator.locks),
            Arc::strong_count(&clone.locks)
        );
    }
}
