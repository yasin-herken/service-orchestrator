//! The build-provider registry.
//!
//! A deliberately small resolver: it maps a [`BuildSystem`] from validated
//! configuration to the [`BuildProvider`] that implements it. It is not a plugin
//! framework — there is no discovery, no lifecycle, and no dynamic loading.
//! Adding a build system means implementing a provider and adding one line here
//! (`tasks/008.md`).
//!
//! [`BuildProviderRegistry::standard`] registers the production set (Maven and
//! npm). A caller may build a registry from an explicit list of providers, which
//! is how tests exercise provider-independent orchestration.

use std::collections::BTreeMap;
use std::sync::Arc;

use service_orchestrator_application::{BuildCapabilities, BuildCapabilityIndex, BuildError};
use service_orchestrator_domain::BuildSystem;

use crate::maven::MavenBuildProvider;
use crate::npm::NpmBuildProvider;
use crate::provider::BuildProvider;

/// A set of build providers keyed by the build system they implement.
#[derive(Default)]
pub struct BuildProviderRegistry {
    providers: BTreeMap<BuildSystem, Arc<dyn BuildProvider>>,
}

impl BuildProviderRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates the production registry: Maven and npm.
    #[must_use]
    pub fn standard() -> Self {
        let mut registry = Self::new();
        registry.register(Arc::new(MavenBuildProvider::new()));
        registry.register(Arc::new(NpmBuildProvider::new()));
        registry
    }

    /// Registers `provider`, replacing any provider for the same system.
    pub fn register(&mut self, provider: Arc<dyn BuildProvider>) {
        self.providers.insert(provider.system(), provider);
    }

    /// Returns the provider for `system`, if registered.
    #[must_use]
    pub fn get(&self, system: BuildSystem) -> Option<&Arc<dyn BuildProvider>> {
        self.providers.get(&system)
    }

    /// Resolves the provider for `system`.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError::UnsupportedSystem`] when no provider is registered.
    pub fn resolve(&self, system: BuildSystem) -> Result<&Arc<dyn BuildProvider>, BuildError> {
        self.providers
            .get(&system)
            .ok_or_else(|| BuildError::UnsupportedSystem {
                system: system.label().to_owned(),
            })
    }

    /// Returns the registered build systems, in a stable order.
    pub fn systems(&self) -> impl Iterator<Item = BuildSystem> + '_ {
        self.providers.keys().copied()
    }
}

impl std::fmt::Debug for BuildProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuildProviderRegistry")
            .field("systems", &self.systems().collect::<Vec<_>>())
            .finish()
    }
}

/// Projects the registry as the application core's read-only capability index.
///
/// This is how an interface learns which operations a build system supports
/// without reaching the provider directly: the registry is the single source of
/// truth for provider capabilities (`tasks/020.md` §13).
impl BuildCapabilityIndex for BuildProviderRegistry {
    fn capabilities(&self, system: BuildSystem) -> Option<BuildCapabilities> {
        self.providers
            .get(&system)
            .map(|provider| BuildCapabilities {
                build_system: system,
                operations: provider.capabilities().to_vec(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standard_registry_holds_maven_and_npm() {
        let registry = BuildProviderRegistry::standard();
        assert_eq!(
            registry.get(BuildSystem::Maven).map(|p| p.system()),
            Some(BuildSystem::Maven)
        );
        assert_eq!(
            registry.get(BuildSystem::Npm).map(|p| p.system()),
            Some(BuildSystem::Npm)
        );
    }

    #[test]
    fn an_empty_registry_rejects_every_system() {
        let registry = BuildProviderRegistry::new();
        let error = registry
            .resolve(BuildSystem::Maven)
            .err()
            .expect("an empty registry has no providers");
        assert!(matches!(error, BuildError::UnsupportedSystem { .. }));
        assert_eq!(error.code(), "BUILD_UNSUPPORTED_SYSTEM");
    }

    #[test]
    fn registering_replaces_a_provider_for_the_same_system() {
        let mut registry = BuildProviderRegistry::new();
        registry.register(Arc::new(MavenBuildProvider::new()));
        registry.register(Arc::new(MavenBuildProvider::new()));
        assert_eq!(registry.systems().count(), 1);
    }

    #[test]
    fn the_capability_index_projects_provider_operations() {
        use service_orchestrator_domain::BuildOperation;

        let registry = BuildProviderRegistry::standard();
        let maven = registry
            .capabilities(BuildSystem::Maven)
            .expect("maven is registered");
        assert_eq!(maven.build_system, BuildSystem::Maven);
        assert!(maven.supports(BuildOperation::Build));
        assert!(maven.supports(BuildOperation::PublishLocalArtifact));

        let npm = registry
            .capabilities(BuildSystem::Npm)
            .expect("npm is registered");
        assert_eq!(npm.build_system, BuildSystem::Npm);
        assert!(npm.supports(BuildOperation::InstallDependencies));
        assert!(!npm.supports(BuildOperation::PublishLocalArtifact));
    }

    #[test]
    fn an_empty_registry_reports_no_capabilities() {
        let registry = BuildProviderRegistry::new();
        assert!(registry.capabilities(BuildSystem::Maven).is_none());
    }
}
