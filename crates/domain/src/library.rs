//! The library model.
//!
//! A [`Library`] is a shared, buildable artifact consumed by one or more
//! services. It is a first-class entity: libraries are synchronized, built, and
//! installed before the services that depend on them.
//!
//! Like services, libraries carry no name-based behaviour. Their dependencies
//! are explicit and must target other libraries.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::command::CommandSet;
use crate::dependency::{Dependency, DependencyTarget};
use crate::environment::Environment;
use crate::error::DomainError;
use crate::ids::LibraryId;
use crate::paths::RelativePath;
use crate::repository::Repository;
use crate::runtime::Runtime;
use crate::runtime::Version;

/// A Maven-style artifact coordinate, for example
/// `com.company:common-core:1.8.0-SNAPSHOT`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ArtifactCoordinate {
    /// The group identifier.
    pub group_id: String,
    /// The artifact identifier.
    pub artifact_id: String,
    /// The artifact version.
    pub version: Version,
}

impl ArtifactCoordinate {
    /// Constructs an artifact coordinate.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the group or artifact id is
    /// empty or contains whitespace.
    pub fn new(
        group_id: impl Into<String>,
        artifact_id: impl Into<String>,
        version: Version,
    ) -> Result<Self, DomainError> {
        let group_id = group_id.into();
        let artifact_id = artifact_id.into();
        if group_id.trim().is_empty() || group_id.contains(char::is_whitespace) {
            return Err(DomainError::validation(
                "library",
                "artifact group id must be non-empty and contain no whitespace",
            ));
        }
        if artifact_id.trim().is_empty() || artifact_id.contains(char::is_whitespace) {
            return Err(DomainError::validation(
                "library",
                "artifact id must be non-empty and contain no whitespace",
            ));
        }
        Ok(Self {
            group_id,
            artifact_id,
            version,
        })
    }
}

impl fmt::Display for ArtifactCoordinate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.group_id, self.artifact_id, self.version)
    }
}

/// A shared, buildable artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Library {
    /// The stable identifier of the library.
    pub id: LibraryId,
    /// The human-readable name shown in interfaces.
    pub display_name: String,
    /// The source repository, if the library has one.
    pub repository: Option<Repository>,
    /// The library's directory relative to the workspace root.
    pub path: Option<RelativePath>,
    /// The published artifact coordinate, if known.
    pub artifact: Option<ArtifactCoordinate>,
    /// The runtimes the library requires to build.
    pub runtimes: Vec<Runtime>,
    /// The configured commands for the library.
    pub commands: CommandSet,
    /// The library's dependencies. These must target other libraries.
    pub dependencies: Vec<Dependency>,
    /// The environment variables passed to the library's commands.
    pub environment: Environment,
}

impl Library {
    /// Creates a library.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the display name is empty.
    pub fn new(id: LibraryId, display_name: impl Into<String>) -> Result<Self, DomainError> {
        let display_name = display_name.into();
        if display_name.trim().is_empty() {
            return Err(DomainError::validation(
                "library",
                "display name must not be empty",
            ));
        }
        Ok(Self {
            id,
            display_name: display_name.trim().to_owned(),
            repository: None,
            path: None,
            artifact: None,
            runtimes: Vec::new(),
            commands: CommandSet::new(),
            dependencies: Vec::new(),
            environment: Environment::new(),
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

    /// Sets the artifact coordinate.
    #[must_use]
    pub fn with_artifact(mut self, artifact: ArtifactCoordinate) -> Self {
        self.artifact = Some(artifact);
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

    /// Replaces the environment.
    #[must_use]
    pub fn with_environment(mut self, environment: Environment) -> Self {
        self.environment = environment;
        self
    }

    /// Validates the library's invariants.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError`] if the name is empty, a command is malformed,
    /// a dependency is duplicated or targets the library itself, or a
    /// dependency targets a service rather than another library.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.display_name.trim().is_empty() {
            return Err(DomainError::validation(
                "library",
                "display name must not be empty",
            ));
        }

        self.commands.validate()?;

        let mut seen_targets = BTreeSet::new();
        for dependency in &self.dependencies {
            match &dependency.target {
                DependencyTarget::Library(target) if target == &self.id => {
                    return Err(DomainError::validation(
                        "library",
                        format!("library '{}' cannot depend on itself", self.id),
                    ));
                }
                DependencyTarget::Service(target) => {
                    return Err(DomainError::validation(
                        "library",
                        format!(
                            "library '{}' cannot depend on service '{target}'; libraries may only depend on libraries",
                            self.id
                        ),
                    ));
                }
                DependencyTarget::Library(_) => {}
            }
            if !seen_targets.insert(dependency.target.clone()) {
                return Err(DomainError::validation(
                    "library",
                    format!(
                        "library '{}' declares '{}' more than once",
                        self.id, dependency.target
                    ),
                ));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(id: &str) -> Library {
        Library::new(LibraryId::new(id).unwrap(), id).unwrap()
    }

    #[test]
    fn builds_artifact_coordinates() {
        let artifact = ArtifactCoordinate::new(
            "com.company",
            "common-core",
            Version::new("1.8.0-SNAPSHOT").unwrap(),
        )
        .unwrap();
        assert_eq!(
            artifact.to_string(),
            "com.company:common-core:1.8.0-SNAPSHOT"
        );
    }

    #[test]
    fn rejects_empty_display_name() {
        let id = LibraryId::new("common-core").unwrap();
        assert!(Library::new(id, "  ").is_err());
    }

    #[test]
    fn rejects_service_dependencies() {
        let library =
            library("common-core").with_dependency(crate::dependency::Dependency::runtime_service(
                crate::ids::ServiceId::new("auth").unwrap(),
            ));
        let error = library.validate().unwrap_err();
        assert!(error.to_string().contains("may only depend on libraries"));
    }

    #[test]
    fn rejects_self_dependency() {
        let library = library("common-core").with_dependency(Dependency::build_library(
            LibraryId::new("common-core").unwrap(),
        ));
        let error = library.validate().unwrap_err();
        assert!(error.to_string().contains("depend on itself"));
    }

    #[test]
    fn accepts_library_dependencies() {
        let library = library("common-core").with_dependency(Dependency::build_library(
            LibraryId::new("common-utils").unwrap(),
        ));
        assert!(library.validate().is_ok());
    }

    #[test]
    fn rejects_duplicate_dependencies() {
        let library = library("common-core")
            .with_dependency(Dependency::build_library(
                LibraryId::new("common-utils").unwrap(),
            ))
            .with_dependency(Dependency::build_library(
                LibraryId::new("common-utils").unwrap(),
            ));
        assert!(library.validate().is_err());
    }
}
