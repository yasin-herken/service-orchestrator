//! The health-checker registry.
//!
//! A deliberately small resolver: it maps a [`CheckType`] to the
//! [`HealthChecker`] that performs it. It is not a plugin framework — there is
//! no discovery and no dynamic loading. Adding a provider means implementing it
//! and adding one line to [`HealthCheckerRegistry::standard`]
//! (`tasks/012.md` §5, §6).
//!
//! [`HealthCheckerRegistry::standard`] registers the three supported providers
//! (process, TCP, HTTP) and is what the manager uses by default. A caller may
//! build a registry from an explicit list, which is how tests exercise
//! aggregation without opening a socket.

use std::collections::BTreeMap;
use std::sync::Arc;

use service_orchestrator_application::ServiceRuntimeIndex;

use crate::checker::{CheckType, HealthChecker};
use crate::http::HttpHealthChecker;
use crate::process::ProcessHealthChecker;
use crate::tcp::TcpHealthChecker;

/// A set of health providers keyed by the check type they implement.
#[derive(Default)]
pub struct HealthCheckerRegistry {
    checkers: BTreeMap<CheckType, Arc<dyn HealthChecker>>,
}

impl HealthCheckerRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            checkers: BTreeMap::new(),
        }
    }

    /// Creates a registry with the production providers.
    ///
    /// The process provider reads managed-process state from `service_runtime`;
    /// the TCP and HTTP providers are stateless.
    #[must_use]
    pub fn standard(service_runtime: Arc<dyn ServiceRuntimeIndex>) -> Self {
        let mut registry = Self::new();
        registry.register(Arc::new(ProcessHealthChecker::new(service_runtime)));
        registry.register(Arc::new(TcpHealthChecker::new()));
        registry.register(Arc::new(HttpHealthChecker::new()));
        registry
    }

    /// Registers a provider, replacing any provider of the same type.
    pub fn register(&mut self, checker: Arc<dyn HealthChecker>) {
        self.checkers.insert(checker.check_type(), checker);
    }

    /// Returns the provider registered for `check_type`, if any.
    #[must_use]
    pub fn get(&self, check_type: CheckType) -> Option<Arc<dyn HealthChecker>> {
        self.checkers.get(&check_type).cloned()
    }

    /// Returns the registered check types, in a stable order.
    #[must_use]
    pub fn types(&self) -> Vec<CheckType> {
        self.checkers.keys().copied().collect()
    }

    /// Returns `true` when no provider is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.checkers.is_empty()
    }
}

impl std::fmt::Debug for HealthCheckerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HealthCheckerRegistry")
            .field("types", &self.types())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_application::NullServiceRuntimeIndex;

    #[test]
    fn standard_registers_process_tcp_and_http() {
        let registry = HealthCheckerRegistry::standard(Arc::new(NullServiceRuntimeIndex));
        assert_eq!(
            registry.types(),
            vec![CheckType::Process, CheckType::Tcp, CheckType::Http]
        );
        assert!(!registry.is_empty());
    }

    #[test]
    fn an_empty_registry_resolves_nothing() {
        let registry = HealthCheckerRegistry::new();
        assert!(registry.get(CheckType::Http).is_none());
        assert!(registry.is_empty());
    }
}
