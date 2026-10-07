//! The service model.
//!
//! A [`Service`] is the central entity of the platform: a buildable and/or
//! runnable unit of software with an optional repository, runtime requirements,
//! commands, dependencies, health checks, and ports.
//!
//! Behaviour is never derived from a service's name. It is derived from its
//! [`ServiceType`] and its declared [`ServiceCapability`] set. This is what
//! allows a new service to be added purely through configuration, and what
//! prevents patterns such as `if service.name == "auth-service"`.
//!
//! Libraries are modelled separately as [`Library`](crate::library::Library).
//! A [`ServiceType::Library`] exists only so that a service-shaped
//! configuration can be rejected with a clear message; it does not make a
//! service a library.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::command::{CommandKind, CommandSet};
use crate::dependency::{Dependency, DependencyTarget};
use crate::environment::Environment;
use crate::error::DomainError;
use crate::health::{HealthCheck, HealthCheckKind};
use crate::ids::ServiceId;
use crate::paths::RelativePath;
use crate::port::Port;
use crate::repository::Repository;
use crate::runtime::{Runtime, RuntimeKind};

/// The high-level category of a service.
///
/// The `Other` variant keeps the type open to future categories without
/// changing `Service` or any of its consumers.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceType {
    /// A server-side application.
    Backend,
    /// A browser application.
    Frontend,
    /// A shared library. Kept only to reject service-shaped library entries.
    Library,
    /// A future or project-specific category.
    Other(String),
}

impl ServiceType {
    /// Returns a short, human-readable label.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Backend => "backend",
            Self::Frontend => "frontend",
            Self::Library => "library",
            Self::Other(name) => name,
        }
    }

    /// Returns the capabilities implied by this type alone.
    ///
    /// Configuration may add further capabilities; these are only defaults.
    #[must_use]
    pub fn default_capabilities(&self) -> BTreeSet<ServiceCapability> {
        let mut capabilities = BTreeSet::new();
        match self {
            Self::Backend => {
                capabilities.insert(ServiceCapability::Build);
                capabilities.insert(ServiceCapability::Run);
                capabilities.insert(ServiceCapability::Migrate);
            }
            Self::Frontend => {
                capabilities.insert(ServiceCapability::Build);
                capabilities.insert(ServiceCapability::Run);
            }
            Self::Library => {
                capabilities.insert(ServiceCapability::Build);
                capabilities.insert(ServiceCapability::ProvideArtifact);
            }
            Self::Other(_) => {}
        }
        capabilities
    }
}

impl fmt::Display for ServiceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A capability a service declares.
///
/// Capabilities, not names, drive behaviour: a service can be built if it has
/// [`Build`](ServiceCapability::Build), started if it has
/// [`Run`](ServiceCapability::Run), and so on.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceCapability {
    /// The service can be built.
    Build,
    /// The service can be started and stopped.
    Run,
    /// The service needs database migrations.
    Migrate,
    /// The service provides an artifact consumed by others.
    ProvideArtifact,
    /// The service exposes an HTTP endpoint.
    ServeHttp,
}

impl fmt::Display for ServiceCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Build => "build",
            Self::Run => "run",
            Self::Migrate => "migrate",
            Self::ProvideArtifact => "provide_artifact",
            Self::ServeHttp => "serve_http",
        };
        f.write_str(label)
    }
}

/// A buildable and/or runnable unit of software.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    /// The stable identifier of the service.
    pub id: ServiceId,
    /// The human-readable name shown in interfaces.
    pub display_name: String,
    /// The high-level category of the service.
    pub kind: ServiceType,
    /// The source repository, if the service has one.
    pub repository: Option<Repository>,
    /// The service's directory relative to the workspace root.
    pub path: Option<RelativePath>,
    /// The runtimes the service requires.
    pub runtimes: Vec<Runtime>,
    /// The configured commands for the service.
    pub commands: CommandSet,
    /// The declared dependencies of the service.
    pub dependencies: Vec<Dependency>,
    /// The configured health checks for the service.
    pub health_checks: Vec<HealthCheck>,
    /// The network ports the service uses.
    pub ports: Vec<Port>,
    /// The environment variables passed to the service.
    pub environment: Environment,
    /// The capabilities the service declares.
    pub capabilities: BTreeSet<ServiceCapability>,
}

impl Service {
    /// Creates a service with the default capabilities for its type.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the display name is empty.
    pub fn new(
        id: ServiceId,
        display_name: impl Into<String>,
        kind: ServiceType,
    ) -> Result<Self, DomainError> {
        let display_name = display_name.into();
        if display_name.trim().is_empty() {
            return Err(DomainError::validation(
                "service",
                "display name must not be empty",
            ));
        }
        let capabilities = kind.default_capabilities();
        Ok(Self {
            id,
            display_name: display_name.trim().to_owned(),
            kind,
            repository: None,
            path: None,
            runtimes: Vec::new(),
            commands: CommandSet::new(),
            dependencies: Vec::new(),
            health_checks: Vec::new(),
            ports: Vec::new(),
            environment: Environment::new(),
            capabilities,
        })
    }

    /// Sets the repository.
    #[must_use]
    pub fn with_repository(mut self, repository: Repository) -> Self {
        self.repository = Some(repository);
        self
    }

    /// Sets the workspace-relative path.
    #[must_use]
    pub fn with_path(mut self, path: RelativePath) -> Self {
        self.path = Some(path);
        self
    }

    /// Adds a runtime requirement.
    #[must_use]
    pub fn with_runtime(mut self, runtime: Runtime) -> Self {
        self.runtimes.push(runtime);
        self
    }

    /// Replaces the command set.
    #[must_use]
    pub fn with_commands(mut self, commands: CommandSet) -> Self {
        self.commands = commands;
        self
    }

    /// Adds a dependency.
    #[must_use]
    pub fn with_dependency(mut self, dependency: Dependency) -> Self {
        self.dependencies.push(dependency);
        self
    }

    /// Adds a health check.
    #[must_use]
    pub fn with_health_check(mut self, health_check: HealthCheck) -> Self {
        self.health_checks.push(health_check);
        self
    }

    /// Adds a port.
    #[must_use]
    pub fn with_port(mut self, port: Port) -> Self {
        self.ports.push(port);
        self
    }

    /// Replaces the environment.
    #[must_use]
    pub fn with_environment(mut self, environment: Environment) -> Self {
        self.environment = environment;
        self
    }

    /// Adds a capability beyond the type defaults.
    #[must_use]
    pub fn with_capability(mut self, capability: ServiceCapability) -> Self {
        self.capabilities.insert(capability);
        self
    }

    /// Returns `true` if the service declares the given capability.
    #[must_use]
    pub fn supports(&self, capability: ServiceCapability) -> bool {
        self.capabilities.contains(&capability)
    }

    /// Returns `true` if the service requires the given runtime kind.
    #[must_use]
    pub fn requires_runtime(&self, kind: &RuntimeKind) -> bool {
        self.runtimes.iter().any(|runtime| runtime.is_kind(kind))
    }

    /// Validates the service's invariants.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError`] if the name is empty, a command is malformed,
    /// a dependency is duplicated or targets the service itself, two ports
    /// collide, a health check probes an undeclared port, or a service is
    /// declared to be a library while also claiming to be runnable.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.display_name.trim().is_empty() {
            return Err(DomainError::validation(
                "service",
                "display name must not be empty",
            ));
        }

        self.commands.validate()?;

        let mut seen_targets = BTreeSet::new();
        for dependency in &self.dependencies {
            if let DependencyTarget::Service(target) = &dependency.target {
                if target == &self.id {
                    return Err(DomainError::validation(
                        "service",
                        format!("service '{}' cannot depend on itself", self.id),
                    ));
                }
            }
            if !seen_targets.insert(dependency.target.clone()) {
                return Err(DomainError::validation(
                    "service",
                    format!(
                        "service '{}' declares '{}' more than once",
                        self.id, dependency.target
                    ),
                ));
            }
        }

        let mut seen_ports = BTreeSet::new();
        for port in &self.ports {
            if !seen_ports.insert(*port) {
                return Err(DomainError::validation(
                    "service",
                    format!("service '{}' declares port {port} more than once", self.id),
                ));
            }
        }

        let mut seen_checks = BTreeSet::new();
        for health_check in &self.health_checks {
            if let Some(port) = health_check.kind.port() {
                if !seen_ports.contains(&port) {
                    return Err(DomainError::validation(
                        "service",
                        format!(
                            "service '{}' has a health check for port {port} that is not declared",
                            self.id
                        ),
                    ));
                }
            }
            if !seen_checks.insert(health_check.kind.label().to_owned()) {
                return Err(DomainError::validation(
                    "service",
                    format!(
                        "service '{}' declares health check '{}' more than once",
                        self.id,
                        health_check.kind.label()
                    ),
                ));
            }
        }

        if self.kind == ServiceType::Library {
            return Err(DomainError::validation(
                "service",
                format!(
                    "service '{}' has type 'library'; libraries must be declared as libraries",
                    self.id
                ),
            ));
        }

        Ok(())
    }

    /// Returns `true` if the service declares a start command.
    #[must_use]
    pub fn has_start_command(&self) -> bool {
        self.commands.contains(&CommandKind::Start)
    }

    /// Returns `true` if the service declares a build command.
    #[must_use]
    pub fn has_build_command(&self) -> bool {
        self.commands.contains(&CommandKind::Build)
    }

    /// Returns `true` if any health check is a port probe.
    #[must_use]
    pub fn has_port_health_check(&self) -> bool {
        self.health_checks
            .iter()
            .any(|check| matches!(check.kind, HealthCheckKind::Port(_)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(id: &str, kind: ServiceType) -> Service {
        Service::new(ServiceId::new(id).unwrap(), id, kind).unwrap()
    }

    #[test]
    fn backend_defaults_to_build_run_and_migrate() {
        let service = service("auth-service", ServiceType::Backend);
        assert!(service.supports(ServiceCapability::Build));
        assert!(service.supports(ServiceCapability::Run));
        assert!(service.supports(ServiceCapability::Migrate));
        assert!(!service.supports(ServiceCapability::ProvideArtifact));
    }

    #[test]
    fn library_defaults_to_building_artifacts() {
        let service = service("common-core", ServiceType::Library);
        assert!(service.supports(ServiceCapability::ProvideArtifact));
        assert!(!service.supports(ServiceCapability::Run));
    }

    #[test]
    fn rejects_empty_display_name() {
        let id = ServiceId::new("auth-service").unwrap();
        assert!(Service::new(id, "   ", ServiceType::Backend).is_err());
    }

    #[test]
    fn rejects_self_dependency() {
        let service = service("auth-service", ServiceType::Backend).with_dependency(
            Dependency::runtime_service(ServiceId::new("auth-service").unwrap()),
        );
        let error = service.validate().unwrap_err();
        assert!(error.to_string().contains("depend on itself"));
    }

    #[test]
    fn rejects_duplicate_dependencies() {
        let service = service("auth-service", ServiceType::Backend)
            .with_dependency(Dependency::build_library(
                crate::ids::LibraryId::new("common-core").unwrap(),
            ))
            .with_dependency(Dependency::runtime_service(
                ServiceId::new("user-service").unwrap(),
            ))
            .with_dependency(Dependency::runtime_service(
                ServiceId::new("user-service").unwrap(),
            ));
        let error = service.validate().unwrap_err();
        assert!(error.to_string().contains("more than once"));
    }

    #[test]
    fn rejects_duplicate_ports() {
        let service = service("auth-service", ServiceType::Backend)
            .with_port(Port::new(8081).unwrap())
            .with_port(Port::new(8081).unwrap());
        assert!(service.validate().is_err());
    }

    #[test]
    fn rejects_health_check_for_undeclared_port() {
        let service = service("auth-service", ServiceType::Backend).with_health_check(
            HealthCheck::new(HealthCheckKind::Port(Port::new(9999).unwrap())),
        );
        let error = service.validate().unwrap_err();
        assert!(error.to_string().contains("not declared"));
    }

    #[test]
    fn accepts_health_check_for_declared_port() {
        let service = service("auth-service", ServiceType::Backend)
            .with_port(Port::new(8081).unwrap())
            .with_health_check(HealthCheck::new(HealthCheckKind::Port(
                Port::new(8081).unwrap(),
            )));
        assert!(service.validate().is_ok());
    }

    #[test]
    fn rejects_a_service_typed_as_a_library() {
        let service = service("common-core", ServiceType::Library);
        assert!(service.validate().is_err());
    }

    #[test]
    fn capabilities_drive_behaviour_not_names() {
        let service = service("something-unusual", ServiceType::Other("worker".to_owned()))
            .with_capability(ServiceCapability::Run);
        assert!(service.supports(ServiceCapability::Run));
        assert!(!service.supports(ServiceCapability::Build));
    }

    #[test]
    fn detects_declared_command_capabilities() {
        let mut commands = CommandSet::new();
        commands.set(
            CommandKind::Start,
            crate::command::CommandSpec::new("npm", ["run", "start"]).unwrap(),
        );
        let service = service("web-ui", ServiceType::Frontend).with_commands(commands);
        assert!(service.has_start_command());
        assert!(!service.has_build_command());
    }

    #[test]
    fn requires_runtime_matches_kind() {
        let service =
            service("auth-service", ServiceType::Backend).with_runtime(Runtime::versioned(
                RuntimeKind::Java,
                crate::runtime::Version::new("21").unwrap(),
            ));
        assert!(service.requires_runtime(&RuntimeKind::Java));
        assert!(!service.requires_runtime(&RuntimeKind::Node));
    }
}
