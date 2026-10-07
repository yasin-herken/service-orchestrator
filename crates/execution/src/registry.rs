//! The executor registry.
//!
//! The scheduler never contains a `match task.kind { ... }` with one branch per
//! operation. Instead, concrete executors are registered against a
//! [`TaskKind`](service_orchestrator_domain::TaskKind) and looked up at run
//! time. Adding a new kind of work is a registration, not a change to the
//! scheduling core.
//!
//! The registry is populated by the composition root and is immutable once
//! the engine is built.

use std::collections::BTreeMap;
use std::sync::Arc;

use service_orchestrator_domain::TaskKind;

use crate::executor::TaskExecutor;

/// Maps task kinds to the executors that perform them.
#[derive(Default)]
pub struct ExecutorRegistry {
    executors: BTreeMap<TaskKind, Arc<dyn TaskExecutor>>,
}

impl ExecutorRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            executors: BTreeMap::new(),
        }
    }

    /// Registers an executor for a task kind, replacing any previous entry.
    pub fn register(&mut self, kind: TaskKind, executor: Arc<dyn TaskExecutor>) {
        self.executors.insert(kind, executor);
    }

    /// Returns the executor registered for `kind`, if any.
    #[must_use]
    pub fn get(&self, kind: &TaskKind) -> Option<Arc<dyn TaskExecutor>> {
        self.executors.get(kind).cloned()
    }

    /// Returns `true` if an executor is registered for `kind`.
    #[must_use]
    pub fn contains(&self, kind: &TaskKind) -> bool {
        self.executors.contains_key(kind)
    }

    /// Returns the registered task kinds in sorted order.
    #[must_use]
    pub fn kinds(&self) -> Vec<TaskKind> {
        self.executors.keys().cloned().collect()
    }
}

impl std::fmt::Debug for ExecutorRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutorRegistry")
            .field("kinds", &self.kinds())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::TaskContext;
    use crate::executor::{TaskExecutorError, TaskOutcome};
    use async_trait::async_trait;

    struct Noop;

    #[async_trait]
    impl TaskExecutor for Noop {
        async fn execute(&self, _context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
            Ok(TaskOutcome::succeeded(None))
        }
    }

    #[test]
    fn registers_and_looks_up_executors() {
        let mut registry = ExecutorRegistry::new();
        assert!(!registry.contains(&TaskKind::GitFetch));
        registry.register(TaskKind::GitFetch, Arc::new(Noop));
        assert!(registry.contains(&TaskKind::GitFetch));
        assert!(registry.get(&TaskKind::GitFetch).is_some());
        assert!(registry.get(&TaskKind::MavenBuild).is_none());
        assert_eq!(registry.kinds(), vec![TaskKind::GitFetch]);
    }

    #[test]
    fn registering_the_same_kind_replaces_the_executor() {
        let mut registry = ExecutorRegistry::new();
        registry.register(TaskKind::GitFetch, Arc::new(Noop));
        registry.register(TaskKind::GitFetch, Arc::new(Noop));
        assert_eq!(registry.kinds().len(), 1);
    }

    #[test]
    fn supports_custom_task_kinds() {
        let mut registry = ExecutorRegistry::new();
        registry.register(TaskKind::Custom("lint".to_owned()), Arc::new(Noop));
        assert!(registry.contains(&TaskKind::Custom("lint".to_owned())));
        assert!(!registry.contains(&TaskKind::Custom("format".to_owned())));
    }
}
