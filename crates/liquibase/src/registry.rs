//! The migration-provider registry.
//!
//! A deliberately small resolver: it maps a [`MigrationProviderType`] from
//! validated configuration to the [`MigrationProvider`] that implements it. It
//! is not a plugin framework — there is no discovery, no lifecycle, and no
//! dynamic loading. Adding a provider means implementing [`MigrationProvider`]
//! and adding one line here (`tasks/011.md` §5).
//!
//! [`MigrationProviderRegistry::standard`] registers the production set
//! (Liquibase). A caller may build a registry from an explicit list of providers,
//! which is how tests exercise provider-independent orchestration and how an
//! unregistered provider is exercised.

use std::collections::BTreeMap;
use std::sync::Arc;

use service_orchestrator_application::MigrationError;
use service_orchestrator_domain::MigrationProviderType;

use crate::liquibase::LiquibaseProvider;
use crate::provider::MigrationProvider;

/// A set of migration providers keyed by the provider they implement.
#[derive(Default)]
pub struct MigrationProviderRegistry {
    providers: BTreeMap<MigrationProviderType, Arc<dyn MigrationProvider>>,
}

impl MigrationProviderRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates the production registry: Liquibase.
    #[must_use]
    pub fn standard() -> Self {
        let mut registry = Self::new();
        registry.register(Arc::new(LiquibaseProvider::new()));
        registry
    }

    /// Registers `provider`, replacing any provider for the same type.
    pub fn register(&mut self, provider: Arc<dyn MigrationProvider>) {
        self.providers.insert(provider.provider(), provider);
    }

    /// Returns the provider for `provider`, if registered.
    #[must_use]
    pub fn get(&self, provider: &MigrationProviderType) -> Option<&Arc<dyn MigrationProvider>> {
        self.providers.get(provider)
    }

    /// Resolves the provider for `provider`.
    ///
    /// # Errors
    ///
    /// Returns [`MigrationError::ProviderNotFound`] when no provider is
    /// registered.
    pub fn resolve(
        &self,
        provider: &MigrationProviderType,
    ) -> Result<&Arc<dyn MigrationProvider>, MigrationError> {
        self.providers
            .get(provider)
            .ok_or_else(|| MigrationError::ProviderNotFound {
                provider: provider.label().to_owned(),
            })
    }

    /// Returns the registered providers, in a stable order.
    pub fn providers(&self) -> impl Iterator<Item = &MigrationProviderType> + '_ {
        self.providers.keys()
    }
}

impl std::fmt::Debug for MigrationProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MigrationProviderRegistry")
            .field("providers", &self.providers().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standard_registry_holds_liquibase() {
        let registry = MigrationProviderRegistry::standard();
        assert_eq!(
            registry
                .get(&MigrationProviderType::Liquibase)
                .map(|provider| provider.provider()),
            Some(MigrationProviderType::Liquibase)
        );
    }

    #[test]
    fn an_empty_registry_rejects_every_provider() {
        let registry = MigrationProviderRegistry::new();
        let error = registry
            .resolve(&MigrationProviderType::Liquibase)
            .err()
            .expect("an empty registry has no providers");
        assert_eq!(error.code(), "MIGRATION_PROVIDER_NOT_FOUND");
    }

    #[test]
    fn an_unregistered_custom_provider_is_rejected() {
        let registry = MigrationProviderRegistry::standard();
        let error = registry
            .resolve(&MigrationProviderType::Custom("dbmate".to_owned()))
            .err()
            .expect("dbmate is not registered");
        match error {
            MigrationError::ProviderNotFound { provider } => assert_eq!(provider, "dbmate"),
            other => panic!("expected a provider-not-found error, got {other:?}"),
        }
    }
}
