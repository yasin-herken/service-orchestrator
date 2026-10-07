//! The workspace aggregate.
//!
//! A [`Workspace`] is the root aggregate: it owns the services, libraries,
//! groups, and profiles that make up one developer environment. Owning the
//! whole set is what makes cross-cutting invariants checkable in one place:
//! unique identifiers, resolvable references, and an acyclic dependency graph.
//!
//! The workspace is assembled from configuration by the `config` infrastructure
//! crate. The domain does not read files; it validates the resulting object.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::dependency::DependencyTarget;
use crate::error::DomainError;
use crate::group::ServiceGroup;
use crate::ids::{GroupId, LibraryId, ProfileId, ServiceId, WorkspaceId};
use crate::library::Library;
use crate::paths::AbsolutePath;
use crate::profile::Profile;
use crate::service::Service;

/// A complete developer environment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    /// The stable identifier of the workspace.
    pub id: WorkspaceId,
    /// The human-readable name shown in interfaces.
    pub name: String,
    /// The absolute filesystem location of the workspace.
    pub root: AbsolutePath,
    /// The services in the workspace.
    pub services: Vec<Service>,
    /// The libraries in the workspace.
    pub libraries: Vec<Library>,
    /// The service groups in the workspace.
    pub groups: Vec<ServiceGroup>,
    /// The developer profiles in the workspace.
    pub profiles: Vec<Profile>,
}

impl Workspace {
    /// Creates an empty workspace.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the name is empty.
    pub fn new(
        id: WorkspaceId,
        name: impl Into<String>,
        root: AbsolutePath,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(DomainError::validation(
                "workspace",
                "name must not be empty",
            ));
        }
        Ok(Self {
            id,
            name: name.trim().to_owned(),
            root,
            services: Vec::new(),
            libraries: Vec::new(),
            groups: Vec::new(),
            profiles: Vec::new(),
        })
    }

    /// Adds a service.
    #[must_use]
    pub fn with_service(mut self, service: Service) -> Self {
        self.services.push(service);
        self
    }

    /// Adds a library.
    #[must_use]
    pub fn with_library(mut self, library: Library) -> Self {
        self.libraries.push(library);
        self
    }

    /// Adds a service group.
    #[must_use]
    pub fn with_group(mut self, group: ServiceGroup) -> Self {
        self.groups.push(group);
        self
    }

    /// Adds a profile.
    #[must_use]
    pub fn with_profile(mut self, profile: Profile) -> Self {
        self.profiles.push(profile);
        self
    }

    /// Returns the service with the given id, if present.
    #[must_use]
    pub fn service(&self, id: &ServiceId) -> Option<&Service> {
        self.services.iter().find(|service| &service.id == id)
    }

    /// Returns the library with the given id, if present.
    #[must_use]
    pub fn library(&self, id: &LibraryId) -> Option<&Library> {
        self.libraries.iter().find(|library| &library.id == id)
    }

    /// Returns the group with the given id, if present.
    #[must_use]
    pub fn group(&self, id: &GroupId) -> Option<&ServiceGroup> {
        self.groups.iter().find(|group| &group.id == id)
    }

    /// Returns the profile with the given id, if present.
    #[must_use]
    pub fn profile(&self, id: &ProfileId) -> Option<&Profile> {
        self.profiles.iter().find(|profile| &profile.id == id)
    }

    /// Validates the workspace and all of its contents.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError`] if an identifier is duplicated, a child entity
    /// is invalid, a reference cannot be resolved, or the service and library
    /// dependency graph contains a cycle.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.name.trim().is_empty() {
            return Err(DomainError::validation(
                "workspace",
                "name must not be empty",
            ));
        }

        ensure_unique(
            self.services.iter().map(|service| service.id.as_str()),
            "service",
        )?;
        ensure_unique(
            self.libraries.iter().map(|library| library.id.as_str()),
            "library",
        )?;
        ensure_unique(
            self.groups.iter().map(|group| group.id.as_str()),
            "service group",
        )?;
        ensure_unique(
            self.profiles.iter().map(|profile| profile.id.as_str()),
            "profile",
        )?;

        for service in &self.services {
            service.validate()?;
        }
        for library in &self.libraries {
            library.validate()?;
        }
        for group in &self.groups {
            group.validate()?;
        }
        for profile in &self.profiles {
            profile.validate()?;
        }

        self.validate_references()?;
        self.validate_acyclic()?;

        Ok(())
    }

    fn validate_references(&self) -> Result<(), DomainError> {
        for service in &self.services {
            for dependency in &service.dependencies {
                match &dependency.target {
                    DependencyTarget::Service(target) => {
                        if self.service(target).is_none() {
                            return Err(DomainError::not_found(
                                "service dependency",
                                target.as_str(),
                            ));
                        }
                    }
                    DependencyTarget::Library(target) => {
                        if self.library(target).is_none() {
                            return Err(DomainError::not_found(
                                "library dependency",
                                target.as_str(),
                            ));
                        }
                    }
                }
            }
        }

        for library in &self.libraries {
            for dependency in &library.dependencies {
                if let DependencyTarget::Library(target) = &dependency.target {
                    if self.library(target).is_none() {
                        return Err(DomainError::not_found(
                            "library dependency",
                            target.as_str(),
                        ));
                    }
                }
            }
        }

        for group in &self.groups {
            for service in &group.services {
                if self.service(service).is_none() {
                    return Err(DomainError::not_found("group service", service.as_str()));
                }
            }
        }

        for profile in &self.profiles {
            for service in &profile.services {
                if self.service(service).is_none() {
                    return Err(DomainError::not_found("profile service", service.as_str()));
                }
            }
            for group in &profile.groups {
                if self.group(group).is_none() {
                    return Err(DomainError::not_found("profile group", group.as_str()));
                }
            }
        }

        Ok(())
    }

    fn validate_acyclic(&self) -> Result<(), DomainError> {
        let mut nodes = Vec::with_capacity(self.services.len() + self.libraries.len());
        let mut edges: BTreeMap<String, Vec<String>> = BTreeMap::new();

        for service in &self.services {
            let node = format!("service:{}", service.id);
            nodes.push(node.clone());
            let targets = service
                .dependencies
                .iter()
                .map(|dependency| match &dependency.target {
                    DependencyTarget::Service(id) => format!("service:{id}"),
                    DependencyTarget::Library(id) => format!("library:{id}"),
                })
                .collect();
            edges.insert(node, targets);
        }

        for library in &self.libraries {
            let node = format!("library:{}", library.id);
            nodes.push(node.clone());
            let targets = library
                .dependencies
                .iter()
                .filter_map(|dependency| match &dependency.target {
                    DependencyTarget::Library(id) => Some(format!("library:{id}")),
                    DependencyTarget::Service(_) => None,
                })
                .collect();
            edges.insert(node, targets);
        }

        if let Some(cycle) = crate::graph::find_cycle(&nodes, &edges) {
            return Err(DomainError::DependencyCycle {
                nodes: cycle.join(", "),
            });
        }

        Ok(())
    }
}

fn ensure_unique<'a>(
    values: impl Iterator<Item = &'a str>,
    entity: &'static str,
) -> Result<(), DomainError> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(DomainError::duplicate(entity, value));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dependency::Dependency;
    use crate::service::{ServiceCapability, ServiceType};

    fn workspace() -> Workspace {
        Workspace::new(
            WorkspaceId::new("project-x").unwrap(),
            "Project X",
            AbsolutePath::new("/Users/dev/projects/project-x").unwrap(),
        )
        .unwrap()
    }

    fn service(id: &str) -> Service {
        Service::new(ServiceId::new(id).unwrap(), id, ServiceType::Backend).unwrap()
    }

    fn library(id: &str) -> Library {
        Library::new(LibraryId::new(id).unwrap(), id).unwrap()
    }

    #[test]
    fn validates_a_consistent_workspace() {
        let common = LibraryId::new("common-core").unwrap();
        let workspace = workspace()
            .with_library(library("common-core"))
            .with_service(service("user-service"))
            .with_service(
                service("auth-service").with_dependency(Dependency::build_library(common.clone())),
            )
            .with_service(
                service("payment-service").with_dependency(Dependency::runtime_service(
                    ServiceId::new("user-service").unwrap(),
                )),
            )
            .with_group(
                ServiceGroup::new(GroupId::new("customer").unwrap(), "Customer")
                    .unwrap()
                    .with_service(ServiceId::new("auth-service").unwrap()),
            )
            .with_profile(
                Profile::new(ProfileId::new("backend").unwrap(), "Backend")
                    .unwrap()
                    .with_group(GroupId::new("customer").unwrap()),
            );
        assert!(workspace.validate().is_ok());
    }

    #[test]
    fn rejects_duplicate_service_ids() {
        let workspace = workspace()
            .with_service(service("auth-service"))
            .with_service(service("auth-service"));
        let error = workspace.validate().unwrap_err();
        assert!(matches!(error, DomainError::Duplicate { .. }));
    }

    #[test]
    fn rejects_unresolved_service_dependency() {
        let workspace = workspace().with_service(service("auth-service").with_dependency(
            Dependency::runtime_service(ServiceId::new("does-not-exist").unwrap()),
        ));
        let error = workspace.validate().unwrap_err();
        assert!(matches!(error, DomainError::NotFound { .. }));
    }

    #[test]
    fn rejects_unresolved_group_reference() {
        let workspace = workspace().with_group(
            ServiceGroup::new(GroupId::new("customer").unwrap(), "Customer")
                .unwrap()
                .with_service(ServiceId::new("missing").unwrap()),
        );
        assert!(workspace.validate().is_err());
    }

    #[test]
    fn rejects_unresolved_profile_group() {
        let workspace = workspace().with_profile(
            Profile::new(ProfileId::new("backend").unwrap(), "Backend")
                .unwrap()
                .with_group(GroupId::new("missing").unwrap()),
        );
        assert!(workspace.validate().is_err());
    }

    #[test]
    fn detects_dependency_cycles_across_services() {
        let workspace = workspace()
            .with_service(
                service("a")
                    .with_dependency(Dependency::runtime_service(ServiceId::new("b").unwrap())),
            )
            .with_service(
                service("b")
                    .with_dependency(Dependency::runtime_service(ServiceId::new("a").unwrap())),
            );
        let error = workspace.validate().unwrap_err();
        assert!(matches!(error, DomainError::DependencyCycle { .. }));
    }

    #[test]
    fn detects_dependency_cycles_between_services_and_libraries() {
        let workspace = workspace()
            .with_library(
                library("common-core").with_dependency(Dependency::build_library(
                    LibraryId::new("common-utils").unwrap(),
                )),
            )
            .with_library(
                library("common-utils").with_dependency(Dependency::build_library(
                    LibraryId::new("common-core").unwrap(),
                )),
            );
        let error = workspace.validate().unwrap_err();
        assert!(matches!(error, DomainError::DependencyCycle { .. }));
    }

    #[test]
    fn looks_up_children_by_id() {
        let workspace = workspace()
            .with_service(service("auth-service"))
            .with_library(library("common-core"));
        assert!(workspace
            .service(&ServiceId::new("auth-service").unwrap())
            .is_some());
        assert!(workspace
            .library(&LibraryId::new("common-core").unwrap())
            .is_some());
        assert!(workspace
            .service(&ServiceId::new("nope").unwrap())
            .is_none());
    }

    #[test]
    fn services_can_carry_capabilities() {
        let service = service("worker")
            .with_capability(ServiceCapability::Migrate)
            .with_capability(ServiceCapability::Run);
        let workspace = workspace().with_service(service);
        assert!(workspace.validate().is_ok());
    }
}
