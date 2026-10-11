//! The runtime registry.
//!
//! A deliberately small resolver: it maps a service to the [`ServiceRuntime`]
//! that can run it. It is not a plugin framework — there is no discovery, no
//! lifecycle, and no dynamic loading. Adding a runtime means implementing it and
//! adding one line here (`tasks/010.md` §6).
//!
//! [`RuntimeRegistry::standard`] registers the production set (Java and Node). A
//! caller may build a registry from an explicit list of runtimes, which is how
//! tests exercise orchestration without a real toolchain.

use std::collections::BTreeMap;
use std::sync::Arc;

use service_orchestrator_domain::{BuildSystem, CommandKind, Service};

use crate::java::JavaRuntime;
use crate::node::NodeRuntime;
use crate::runtime::{basename, RuntimeType, ServiceRuntime};

/// A set of runtimes keyed by the type they implement.
#[derive(Default)]
pub struct RuntimeRegistry {
    runtimes: BTreeMap<RuntimeType, Arc<dyn ServiceRuntime>>,
}

impl RuntimeRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            runtimes: BTreeMap::new(),
        }
    }

    /// Creates a registry with the production runtimes (Java and Node).
    #[must_use]
    pub fn standard() -> Self {
        let mut registry = Self::new();
        registry.register(Arc::new(JavaRuntime::new()));
        registry.register(Arc::new(NodeRuntime::new()));
        registry
    }

    /// Registers a runtime, replacing any runtime of the same type.
    pub fn register(&mut self, runtime: Arc<dyn ServiceRuntime>) {
        self.runtimes.insert(runtime.runtime_type(), runtime);
    }

    /// Returns the runtime for `runtime_type`, if registered.
    #[must_use]
    pub fn get(&self, runtime_type: RuntimeType) -> Option<Arc<dyn ServiceRuntime>> {
        self.runtimes.get(&runtime_type).cloned()
    }

    /// Returns the registered runtime types, in a stable order.
    #[must_use]
    pub fn types(&self) -> Vec<RuntimeType> {
        self.runtimes.keys().copied().collect()
    }

    /// Returns `true` when no runtime is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.runtimes.is_empty()
    }

    /// Selects a runtime for a service.
    ///
    /// Selection is configuration-driven and ordered:
    ///
    /// 1. an explicitly declared runtime requirement (`java`/`maven` imply Java,
    ///    `node`/`npm` imply Node);
    /// 2. the service's build system (`maven` implies Java, `npm` implies Node);
    /// 3. the start command's program (`java`/`mvn` imply Java;
    ///    `node`/`npm`/`npx`/`yarn`/`pnpm` imply Node).
    ///
    /// Returns `None` when no registered runtime matches.
    #[must_use]
    pub fn resolve(&self, service: &Service) -> Option<Arc<dyn ServiceRuntime>> {
        for requirement in &service.runtimes {
            if let Some(runtime_type) = RuntimeType::from_kind(&requirement.kind) {
                if let Some(runtime) = self.get(runtime_type) {
                    return Some(runtime);
                }
            }
        }

        if let Some(system) = service.build_system {
            let runtime_type = match system {
                BuildSystem::Maven => RuntimeType::Java,
                BuildSystem::Npm => RuntimeType::Node,
            };
            if let Some(runtime) = self.get(runtime_type) {
                return Some(runtime);
            }
        }

        let command = service.commands.get(&CommandKind::Start)?;
        let runtime_type = match basename(&command.program) {
            "java" | "mvn" | "mvnw" => RuntimeType::Java,
            "node" | "npm" | "npx" | "yarn" | "pnpm" => RuntimeType::Node,
            _ => return None,
        };
        self.get(runtime_type)
    }
}

impl std::fmt::Debug for RuntimeRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeRegistry")
            .field("types", &self.types())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_domain::{
        CommandSet, CommandSpec, Runtime, RuntimeKind, Service, ServiceType,
    };

    fn java_service() -> Service {
        let mut commands = CommandSet::new();
        commands.set(
            CommandKind::Start,
            CommandSpec::new("./mvnw", ["spring-boot:run"]).unwrap(),
        );
        Service::new(
            service_orchestrator_domain::ServiceId::new("auth-service").unwrap(),
            "Auth service",
            ServiceType::Backend,
        )
        .unwrap()
        .with_runtime(Runtime::any(RuntimeKind::Java))
        .with_commands(commands)
    }

    #[test]
    fn standard_registers_java_and_node() {
        let registry = RuntimeRegistry::standard();
        assert_eq!(registry.types(), vec![RuntimeType::Java, RuntimeType::Node]);
        assert!(!registry.is_empty());
    }

    #[test]
    fn resolves_by_declared_runtime() {
        let registry = RuntimeRegistry::standard();
        let runtime = registry.resolve(&java_service()).unwrap();
        assert_eq!(runtime.runtime_type(), RuntimeType::Java);
    }

    #[test]
    fn resolves_by_start_command_when_no_runtime_declared() {
        let registry = RuntimeRegistry::standard();
        let mut commands = CommandSet::new();
        commands.set(
            CommandKind::Start,
            CommandSpec::new("npm", ["run", "start"]).unwrap(),
        );
        let service = Service::new(
            service_orchestrator_domain::ServiceId::new("web-ui").unwrap(),
            "Web UI",
            ServiceType::Frontend,
        )
        .unwrap()
        .with_commands(commands);
        let runtime = registry.resolve(&service).unwrap();
        assert_eq!(runtime.runtime_type(), RuntimeType::Node);
    }

    #[test]
    fn an_unknown_runtime_is_not_resolved() {
        let registry = RuntimeRegistry::standard();
        let mut commands = CommandSet::new();
        commands.set(
            CommandKind::Start,
            CommandSpec::new("python", ["app.py"]).unwrap(),
        );
        let service = Service::new(
            service_orchestrator_domain::ServiceId::new("worker").unwrap(),
            "Worker",
            ServiceType::Other("worker".to_owned()),
        )
        .unwrap()
        .with_commands(commands);
        assert!(registry.resolve(&service).is_none());
    }
}
