//! Dependency relationships between services and libraries.
//!
//! Dependencies are declared, not inferred. They let the application and the
//! execution engine determine a safe execution order (for example: build a
//! library before the services that consume it, or start a service before the
//! services that call it). The domain only *models* the relationship; building
//! the dependency graph into a schedule is the execution engine's job.
//!
//! Dependencies are always expressed as references to other domain entities by
//! identifier. Definitions are never duplicated.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ids::{LibraryId, ServiceId};

/// What a dependency points at.
///
/// A service may depend on another service or on a library; a library may
/// depend on another library.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyTarget {
    /// A dependency on another service.
    Service(ServiceId),
    /// A dependency on a library.
    Library(LibraryId),
}

impl DependencyTarget {
    /// Returns the target as a string identifier, regardless of kind.
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Service(id) => id.as_str(),
            Self::Library(id) => id.as_str(),
        }
    }
}

impl fmt::Display for DependencyTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Service(id) => write!(f, "service:{id}"),
            Self::Library(id) => write!(f, "library:{id}"),
        }
    }
}

/// The nature of a dependency.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    /// The target must be built before this entity can be built.
    Build,
    /// The target must be running before this entity can start.
    Runtime,
}

impl fmt::Display for DependencyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Build => "build",
            Self::Runtime => "runtime",
        })
    }
}

/// A declared dependency on another service or library.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Dependency {
    /// The entity that must exist before this one can proceed.
    pub target: DependencyTarget,
    /// Whether the dependency is needed for building or for running.
    pub kind: DependencyKind,
    /// Whether the dependency is mandatory. Optional dependencies are hints
    /// for ordering but do not block execution.
    pub required: bool,
}

impl Dependency {
    /// Creates a required dependency.
    #[must_use]
    pub fn new(target: DependencyTarget, kind: DependencyKind) -> Self {
        Self {
            target,
            kind,
            required: true,
        }
    }

    /// Creates an optional dependency.
    #[must_use]
    pub fn optional(target: DependencyTarget, kind: DependencyKind) -> Self {
        Self {
            target,
            kind,
            required: false,
        }
    }

    /// Creates a required build dependency on a service.
    #[must_use]
    pub fn build_service(id: ServiceId) -> Self {
        Self::new(DependencyTarget::Service(id), DependencyKind::Build)
    }

    /// Creates a required runtime dependency on a service.
    #[must_use]
    pub fn runtime_service(id: ServiceId) -> Self {
        Self::new(DependencyTarget::Service(id), DependencyKind::Runtime)
    }

    /// Creates a required build dependency on a library.
    #[must_use]
    pub fn build_library(id: LibraryId) -> Self {
        Self::new(DependencyTarget::Library(id), DependencyKind::Build)
    }
}

impl fmt::Display for Dependency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let marker = if self.required { "" } else { "optional " };
        write!(f, "{}{}->{}", marker, self.kind, self.target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_dependency_defaults_to_required() {
        let dependency = Dependency::build_service(ServiceId::new("user-service").unwrap());
        assert!(dependency.required);
        assert_eq!(dependency.kind, DependencyKind::Build);
        assert_eq!(dependency.target.id(), "user-service");
    }

    #[test]
    fn optional_dependency_is_not_required() {
        let dependency = Dependency::optional(
            DependencyTarget::Library(LibraryId::new("common-core").unwrap()),
            DependencyKind::Build,
        );
        assert!(!dependency.required);
        assert!(dependency.to_string().starts_with("optional "));
    }

    #[test]
    fn targets_are_distinguishable_by_kind() {
        let service = DependencyTarget::Service(ServiceId::new("auth").unwrap());
        let library = DependencyTarget::Library(LibraryId::new("auth").unwrap());
        assert_ne!(service, library);
        assert_eq!(service.to_string(), "service:auth");
        assert_eq!(library.to_string(), "library:auth");
    }
}
