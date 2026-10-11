//! Repository-scoped operation coordination.
//!
//! Two Git mutations must never run against the same checkout at once: a
//! `checkout` racing a `pull` on one repository can corrupt the working tree.
//! Mutations of *independent* repositories, however, must still run in
//! parallel, and read-only inspection may run concurrently even with other
//! reads.
//!
//! [`RepositoryCoordinator`] provides exactly that shape: one `RwLock` per
//! repository path, acquired as a write lock by mutations and a read lock by
//! read-only operations. There is no global lock, so the engine's concurrency
//! limit — not this coordinator — remains the only bound on parallelism across
//! repositories.
//!
//! The map that holds the locks grows one entry per distinct repository path;
//! for a workspace of dozens to a few hundred repositories this is negligible
//! and keeps coordination free of a reaper.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

/// A registry of per-repository read/write locks.
#[derive(Clone, Default)]
pub struct RepositoryCoordinator {
    locks: Arc<Mutex<HashMap<String, Arc<RwLock<()>>>>>,
}

impl RepositoryCoordinator {
    /// Creates an empty coordinator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquires a shared (read-only) lock for `key`.
    ///
    /// Multiple readers may hold the lock at once; a writer excludes them.
    pub async fn read(&self, key: &str) -> OwnedRwLockReadGuard<()> {
        self.lock_for(key).read_owned().await
    }

    /// Acquires an exclusive (mutating) lock for `key`.
    ///
    /// Waits for any in-flight readers or writers on the same repository to
    /// finish; independent repositories are unaffected.
    pub async fn write(&self, key: &str) -> OwnedRwLockWriteGuard<()> {
        self.lock_for(key).write_owned().await
    }

    fn lock_for(&self, key: &str) -> Arc<RwLock<()>> {
        let mut locks = self
            .locks
            .lock()
            .expect("repository coordinator mutex poisoned");
        locks
            .entry(key.to_owned())
            .or_insert_with(|| Arc::new(RwLock::new(())))
            .clone()
    }

    /// Returns the number of repositories currently tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.locks
            .lock()
            .expect("repository coordinator mutex poisoned")
            .len()
    }

    /// Returns `true` if no repository has been locked yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl std::fmt::Debug for RepositoryCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepositoryCoordinator")
            .field("repositories", &self.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn independent_repositories_do_not_block_each_other() {
        let coordinator = RepositoryCoordinator::new();
        let first = coordinator.write("/repo/a").await;
        // Acquiring a write lock on a *different* repository must not block.
        let second = tokio::time::timeout(Duration::from_secs(1), coordinator.write("/repo/b"))
            .await
            .expect("independent repository lock should not block");
        drop(first);
        drop(second);
    }

    #[tokio::test]
    async fn read_locks_are_shared() {
        let coordinator = RepositoryCoordinator::new();
        let _first = coordinator.read("/repo").await;
        let _second = tokio::time::timeout(Duration::from_secs(1), coordinator.read("/repo"))
            .await
            .expect("read locks should be shared");
    }

    #[tokio::test]
    async fn a_writer_excludes_a_reader_on_the_same_repository() {
        let coordinator = RepositoryCoordinator::new();
        let write = coordinator.write("/repo").await;
        let blocked = tokio::time::timeout(Duration::from_millis(50), coordinator.read("/repo"));
        assert!(blocked.await.is_err(), "a reader must wait for the writer");
        drop(write);
        let read = tokio::time::timeout(Duration::from_secs(1), coordinator.read("/repo"))
            .await
            .expect("reader proceeds after the writer releases");
        drop(read);
        assert_eq!(coordinator.len(), 1);
    }
}
