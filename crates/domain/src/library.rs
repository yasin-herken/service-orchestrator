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

use crate::build::BuildSystem;
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
///
/// The coordinate is the artifact's *identity*: `group_id`, `artifact_id`, and
/// `version`. Packaging and classifier are optional attributes that affect the
/// artifact's *file name* in a repository but not its identity — two coordinates
/// that differ only by packaging describe the same logical artifact. They are
/// carried here so an artifact's location can be computed without a second
/// lookup, and so multi-artifact (reactor) libraries can be modelled.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ArtifactCoordinate {
    /// The group identifier.
    pub group_id: String,
    /// The artifact identifier.
    pub artifact_id: String,
    /// The artifact version.
    pub version: Version,
    /// The packaging (for example `jar` or `pom`). Defaults to `jar` when
    /// omitted, which is Maven's default packaging.
    #[serde(default)]
    pub packaging: Option<String>,
    /// The classifier (for example `sources`), when the artifact has one.
    #[serde(default)]
    pub classifier: Option<String>,
}

impl ArtifactCoordinate {
    /// Constructs an artifact coordinate with default packaging.
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
            packaging: None,
            classifier: None,
        })
    }

    /// Sets the artifact packaging (for example `jar` or `pom`).
    #[must_use]
    pub fn with_packaging(mut self, packaging: impl Into<String>) -> Self {
        self.packaging = Some(packaging.into());
        self
    }

    /// Sets the artifact classifier.
    #[must_use]
    pub fn with_classifier(mut self, classifier: impl Into<String>) -> Self {
        self.classifier = Some(classifier.into());
        self
    }

    /// The packaging, defaulting to Maven's `jar`.
    #[must_use]
    pub fn packaging_or_default(&self) -> &str {
        self.packaging.as_deref().unwrap_or("jar")
    }

    /// The artifact's directory group path (`com.company` → `com/company`).
    ///
    /// This is the standard Maven repository layout and is deliberately kept in
    /// the domain as a pure, total function: it computes a *location* from an
    /// identity and performs no repository access.
    #[must_use]
    pub fn group_path(&self) -> String {
        self.group_id.replace('.', "/")
    }

    /// The artifact's file name within a version directory, for example
    /// `common-core-1.8.0-SNAPSHOT.jar` or `common-core-1.8.0-SNAPSHOT-sources.jar`.
    #[must_use]
    pub fn file_name(&self) -> String {
        let mut name = format!("{}-{}", self.artifact_id, self.version);
        if let Some(classifier) = &self.classifier {
            name.push('-');
            name.push_str(classifier);
        }
        name.push('.');
        name.push_str(self.packaging_or_default());
        name
    }

    /// The artifact's path relative to a repository root, using the standard
    /// Maven layout, for example
    /// `com/company/common-core/1.8.0-SNAPSHOT/common-core-1.8.0-SNAPSHOT.jar`.
    #[must_use]
    pub fn repository_path(&self) -> String {
        format!(
            "{}/{}/{}/{}",
            self.group_path(),
            self.artifact_id,
            self.version,
            self.file_name()
        )
    }

    /// Validates the coordinate's optional attributes.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if packaging or classifier is
    /// present but empty or contains whitespace or a path separator.
    pub fn validate(&self) -> Result<(), DomainError> {
        for (label, value) in [
            ("packaging", &self.packaging),
            ("classifier", &self.classifier),
        ] {
            if let Some(value) = value {
                if value.trim().is_empty() || value.contains(char::is_whitespace) {
                    return Err(DomainError::validation(
                        "library",
                        format!("artifact {label} must be non-empty and contain no whitespace"),
                    ));
                }
                if value.contains(['/', '\\']) {
                    return Err(DomainError::validation(
                        "library",
                        format!("artifact {label} must not contain a path separator"),
                    ));
                }
            }
        }
        Ok(())
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
    /// The library's primary published artifact coordinate, if known.
    ///
    /// A single-module Maven library produces exactly this artifact. A
    /// multi-module (reactor) repository produces several; the additional
    /// artifacts are listed in [`Library::artifacts`]. The primary coordinate is
    /// the one interfaces display and the one a consumer most commonly depends
    /// on.
    pub artifact: Option<ArtifactCoordinate>,
    /// Additional artifacts the library produces, beyond [`Library::artifact`].
    ///
    /// This exists so a reactor project can be modelled without pretending it
    /// has a single output. It is empty for the common single-artifact case.
    #[serde(default)]
    pub artifacts: Vec<ArtifactCoordinate>,
    /// The runtimes the library requires to build.
    pub runtimes: Vec<Runtime>,
    /// The configured commands for the library.
    pub commands: CommandSet,
    /// The build system used to build the library, if known.
    ///
    /// Selection is configuration-driven; when omitted, configuration
    /// normalization infers it from the library's runtimes and commands. The
    /// domain only names the system: how a build runs belongs to the build
    /// adapter.
    #[serde(default)]
    pub build_system: Option<BuildSystem>,
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
            artifacts: Vec::new(),
            runtimes: Vec::new(),
            commands: CommandSet::new(),
            build_system: None,
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

    /// Adds an additional artifact the library produces.
    #[must_use]
    pub fn with_additional_artifact(mut self, artifact: ArtifactCoordinate) -> Self {
        self.artifacts.push(artifact);
        self
    }

    /// Returns every artifact the library produces, primary first.
    ///
    /// A single-module library returns just its primary coordinate; a
    /// multi-module one returns the primary followed by its additional
    /// artifacts, with duplicates removed.
    #[must_use]
    pub fn produced_artifacts(&self) -> Vec<&ArtifactCoordinate> {
        let mut artifacts: Vec<&ArtifactCoordinate> = Vec::new();
        if let Some(primary) = &self.artifact {
            artifacts.push(primary);
        }
        for artifact in &self.artifacts {
            if !artifacts.contains(&artifact) {
                artifacts.push(artifact);
            }
        }
        artifacts
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

    /// Sets the build system.
    #[must_use]
    pub fn with_build_system(mut self, build_system: BuildSystem) -> Self {
        self.build_system = Some(build_system);
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

        for artifact in self.produced_artifacts() {
            artifact.validate()?;
        }

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

    #[test]
    fn computes_artifacts_in_maven_repository_layout() {
        let artifact = ArtifactCoordinate::new(
            "com.company",
            "common-core",
            Version::new("1.8.0-SNAPSHOT").unwrap(),
        )
        .unwrap();
        assert_eq!(artifact.group_path(), "com/company");
        assert_eq!(artifact.file_name(), "common-core-1.8.0-SNAPSHOT.jar");
        assert_eq!(
            artifact.repository_path(),
            "com/company/common-core/1.8.0-SNAPSHOT/common-core-1.8.0-SNAPSHOT.jar"
        );
    }

    #[test]
    fn honours_packaging_and_classifier_in_the_file_name() {
        let artifact =
            ArtifactCoordinate::new("com.company", "common-core", Version::new("1.8.0").unwrap())
                .unwrap()
                .with_packaging("pom");
        assert_eq!(artifact.file_name(), "common-core-1.8.0.pom");

        let classified = artifact.clone().with_classifier("sources");
        assert_eq!(classified.file_name(), "common-core-1.8.0-sources.pom");
    }

    #[test]
    fn rejects_invalid_packaging_or_classifier() {
        let artifact =
            ArtifactCoordinate::new("com.company", "common-core", Version::new("1.8.0").unwrap())
                .unwrap()
                .with_classifier("bad/value");
        assert!(artifact.validate().is_err());
    }

    #[test]
    fn produced_artifacts_prefer_primary_then_deduplicate() {
        let primary =
            ArtifactCoordinate::new("com.company", "common-core", Version::new("1.8.0").unwrap())
                .unwrap();
        let extra =
            ArtifactCoordinate::new("com.company", "common-api", Version::new("1.8.0").unwrap())
                .unwrap();
        let library = library("common-core")
            .with_artifact(primary.clone())
            .with_additional_artifact(extra.clone())
            .with_additional_artifact(primary.clone());
        let produced = library.produced_artifacts();
        assert_eq!(produced, vec![&primary, &extra]);
        assert!(library.validate().is_ok());
    }
}
