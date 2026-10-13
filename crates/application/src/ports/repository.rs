//! Read-only repository inspection and technology detection.
//!
//! When a developer registers a workspace or a service they point the
//! application at a local directory. The application core never touches that
//! directory itself (`AGENTS.md` §1): it reaches the filesystem only through
//! [`RepositoryInspector`], which returns *observations* — never mutations
//! (`tasks/023.md` §17, §18).
//!
//! Inspection is deliberately conservative. It reads well-known project files
//! and `.git` metadata; it **never** executes a project script, runs a build, or
//! installs anything (`tasks/023.md` §17, §20). A detected technology is a
//! *suggestion*, not a fact, and a technology whose execution provider is not
//! installed (today, Gradle) is reported as such rather than presented as
//! usable (`tasks/023.md` §19).

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Whether a detected technology can actually be executed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAvailability {
    /// A registered provider can execute the technology (Maven, npm).
    Available,
    /// The technology was detected, but no execution provider is installed
    /// (currently Gradle). Interfaces must not present it as operational
    /// (`tasks/023.md` §19).
    ProviderNotInstalled,
    /// The technology is not a build system, so provider availability does not
    /// apply (Java or Node runtimes, Spring Boot, Liquibase).
    NotApplicable,
}

impl ProviderAvailability {
    /// Returns `true` when the technology can be executed right now.
    #[must_use]
    pub const fn is_operational(self) -> bool {
        matches!(self, Self::Available)
    }
}

/// A build system detected in a repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedBuildSystem {
    /// The stable machine-readable kind, for example `maven`, `npm`, `gradle`.
    pub system: String,
    /// A human-readable label, for example `Maven`.
    pub label: String,
    /// Whether an execution provider for this build system is installed.
    pub availability: ProviderAvailability,
    /// The files that led to the detection, for example `pom.xml`.
    pub evidence: Vec<String>,
    /// An optional explanation, for example the "provider not installed" note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A runtime requirement suggested by a repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedRuntime {
    /// The stable runtime kind, for example `java` or `node`.
    pub kind: String,
    /// A human-readable label, for example `Java`.
    pub label: String,
    /// A suggested or required version, when the repository declares one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_version: Option<String>,
    /// The files that led to the detection.
    pub evidence: Vec<String>,
}

/// A non-build integration detected in a repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedIntegration {
    /// The stable machine-readable kind, for example `spring_boot`,
    /// `liquibase`, or `react`.
    pub kind: String,
    /// A human-readable label.
    pub label: String,
    /// The files that led to the detection.
    pub evidence: Vec<String>,
}

/// A bounded, read-only summary of a Maven project.
///
/// When a value can only be known after effective-POM evaluation (an inherited
/// parent version, a property-substituted packaging) it is left `None` rather
/// than guessed (`tasks/023.md` §21).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MavenMetadata {
    /// The declared `groupId`, if present in this POM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    /// The declared `artifactId`, if present in this POM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    /// The declared `version`, if literal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The declared `packaging`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packaging: Option<String>,
    /// The suggested Java version from `maven.compiler.*` properties.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java_version: Option<String>,
    /// The module directories declared in `<modules>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<String>,
    /// Whether a Maven Wrapper (`mvnw`) is present in the directory.
    pub wrapper_available: bool,
    /// Whether a Spring Boot dependency or parent was detected.
    pub spring_boot: bool,
}

/// A bounded, read-only summary of a Node project.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeMetadata {
    /// The `name` field of `package.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The `version` field of `package.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The Node version requirement, from `.nvmrc`, `.node-version`, or
    /// `engines.node`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_node_version: Option<String>,
    /// A package-manager hint, for example `npm` or `yarn`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_manager: Option<String>,
    /// The declared npm script names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scripts: Vec<String>,
    /// Whether React is a dependency.
    pub react: bool,
}

/// A read-only inspection of a local repository directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInspection {
    /// The inspected absolute path.
    pub path: String,
    /// Whether the path exists.
    pub exists: bool,
    /// Whether the path exists and is a directory.
    pub is_directory: bool,
    /// Whether a `.git` directory or file is present.
    pub is_git_repository: bool,
    /// The current branch, from `.git/HEAD`, when it is a symbolic ref.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The `origin` remote URL, from `.git/config`, when readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    /// The directly relevant project files that were found.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub project_files: Vec<String>,
    /// The detected build systems.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub build_systems: Vec<DetectedBuildSystem>,
    /// The suggested runtimes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtimes: Vec<DetectedRuntime>,
    /// The detected non-build integrations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub integrations: Vec<DetectedIntegration>,
    /// Maven metadata, when the directory is a Maven project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maven: Option<MavenMetadata>,
    /// Node metadata, when the directory is a Node project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeMetadata>,
    /// Human-readable notes, including detection caveats.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl RepositoryInspection {
    /// An inspection of a path that does not exist.
    #[must_use]
    pub fn missing(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            exists: false,
            is_directory: false,
            is_git_repository: false,
            branch: None,
            remote: None,
            project_files: Vec::new(),
            build_systems: Vec::new(),
            runtimes: Vec::new(),
            integrations: Vec::new(),
            maven: None,
            node: None,
            notes: Vec::new(),
        }
    }

    /// Returns the first build system with a registered provider, if any.
    #[must_use]
    pub fn operational_build_system(&self) -> Option<&DetectedBuildSystem> {
        self.build_systems
            .iter()
            .find(|system| system.availability.is_operational())
    }
}

/// An error raised when a repository cannot be inspected.
#[derive(Clone, Debug, PartialEq, Eq, Error, Serialize, Deserialize)]
#[error("repository inspection error ({code}): {message}")]
pub struct RepositoryInspectionError {
    /// A stable, machine-readable code such as `INSPECTION_UNAVAILABLE`.
    pub code: String,
    /// A human-readable explanation.
    pub message: String,
}

impl RepositoryInspectionError {
    /// Builds an error with `code` and `message`.
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// A read-only inspection boundary over a local directory.
///
/// Implementations must be side-effect free: reads only, no script execution,
/// no build, no install, no checkout (`tasks/023.md` §17).
pub trait RepositoryInspector: Send + Sync {
    /// Inspects the directory at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`RepositoryInspectionError`] only when inspection could not be
    /// attempted at all. A path that does not exist is reported through
    /// [`RepositoryInspection::exists`], not as an error.
    fn inspect(&self, path: &str) -> Result<RepositoryInspection, RepositoryInspectionError>;
}

/// A [`RepositoryInspector`] that refuses to inspect anything.
///
/// Used when the application core is built without a filesystem inspector.
/// Every call fails with `INSPECTION_UNAVAILABLE` rather than fabricating a
/// result.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullRepositoryInspector;

impl RepositoryInspector for NullRepositoryInspector {
    fn inspect(&self, _path: &str) -> Result<RepositoryInspection, RepositoryInspectionError> {
        Err(RepositoryInspectionError::new(
            "INSPECTION_UNAVAILABLE",
            "repository inspection is not available without a filesystem inspector",
        ))
    }
}

/// A structured command suggestion: a program plus its argument vector.
///
/// It is never a shell line (`AGENTS.md` §32); the wizard confirms it before it
/// becomes configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestedCommand {
    /// The executable to run.
    pub program: String,
    /// The arguments passed to the executable, in order.
    pub args: Vec<String>,
}

/// A suggested service configuration derived from a repository inspection.
///
/// It is a *starting point* for the service wizard, not a committed change; the
/// user reviews and edits it before it is validated and saved
/// (`tasks/023.md` §16, §23).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectConfigurationSuggestion {
    /// The inspected path.
    pub path: String,
    /// The suggested service type: `backend` or `frontend`, when inferable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_type: Option<String>,
    /// The suggested build system: `maven` or `npm`, when inferable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_system: Option<String>,
    /// The suggested runtimes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtimes: Vec<DetectedRuntime>,
    /// The suggested build command, when inferable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_command: Option<SuggestedCommand>,
    /// The suggested start command, when inferable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_command: Option<SuggestedCommand>,
    /// The detected integrations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub integrations: Vec<DetectedIntegration>,
    /// Warnings the user must see before saving, for example an uninstalled
    /// Gradle provider or a missing start script.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// The underlying inspection, embedded so an interface can render every
    /// detection detail without a second call.
    pub inspection: RepositoryInspection,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_inspector_never_fabricates_a_result() {
        let error = NullRepositoryInspector
            .inspect("/tmp/nothing")
            .expect_err("the null inspector refuses");
        assert_eq!(error.code, "INSPECTION_UNAVAILABLE");
    }

    #[test]
    fn provider_availability_reports_operational_only_when_available() {
        assert!(ProviderAvailability::Available.is_operational());
        assert!(!ProviderAvailability::ProviderNotInstalled.is_operational());
        assert!(!ProviderAvailability::NotApplicable.is_operational());
    }

    #[test]
    fn a_missing_inspection_reports_absence() {
        let inspection = RepositoryInspection::missing("/tmp/ghost");
        assert!(!inspection.exists);
        assert!(inspection.operational_build_system().is_none());
    }
}
