//! Artifact inspection boundary.
//!
//! The Library Manager does not resolve artifacts or read Maven POMs itself.
//! Those are build-ecosystem concerns: only the `build` adapter knows how Maven
//! lays out a repository or where a consumer declares a dependency version. The
//! application therefore describes *what* to inspect — a [`ResolvedArtifact`] —
//! and an [`ArtifactInspector`] answers with structured, credential-free data.
//!
//! # Why a port
//!
//! Every interface drives the same use cases, and artifact verification and
//! version alignment must behave identically for the TUI and MCP. Fixing the
//! boundary once keeps the application free of Maven and POM knowledge; a
//! [`NullArtifactInspector`] keeps the application constructible (and its
//! existing tests unchanged) when no inspector is wired.
//!
//! # Identity and safety
//!
//! A [`ResolvedArtifact`] pairs the owning workspace and library with the
//! module's absolute directory and the artifact coordinate, so an inspection
//! task submitted to the engine can run later without re-reading configuration.
//! It round-trips through task parameters. Only coordinates and paths are
//! carried — never credentials or Maven settings values (`AGENTS.md` §31).

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use service_orchestrator_domain::{
    AbsolutePath, ArtifactCoordinate, LibraryId, Version, WorkspaceId,
};
use thiserror::Error;

/// Parameter keys used to carry a [`ResolvedArtifact`] through the task engine.
pub mod parameter {
    /// The owning workspace identifier.
    pub const WORKSPACE: &str = "artifact.workspace";
    /// The library identifier.
    pub const LIBRARY: &str = "artifact.library";
    /// The library module's absolute directory.
    pub const PATH: &str = "artifact.path";
    /// The coordinate's group identifier.
    pub const GROUP: &str = "artifact.group";
    /// The coordinate's artifact identifier.
    pub const ARTIFACT: &str = "artifact.artifact";
    /// The coordinate's version.
    pub const VERSION: &str = "artifact.version";
    /// The coordinate's packaging, when set.
    pub const PACKAGING: &str = "artifact.packaging";
    /// The coordinate's classifier, when set.
    pub const CLASSIFIER: &str = "artifact.classifier";
}

/// An artifact resolved against a validated configuration.
///
/// This is the unit of work an [`ArtifactInspector`] takes. It is
/// workspace-aware: two workspaces may legitimately contain a library with the
/// same id, and their module directories are distinct.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedArtifact {
    /// The workspace that owns the library.
    pub workspace_id: WorkspaceId,
    /// The library identifier.
    pub library: LibraryId,
    /// The library module's absolute directory.
    pub path: AbsolutePath,
    /// The artifact coordinate to inspect.
    pub coordinate: ArtifactCoordinate,
}

impl ResolvedArtifact {
    /// Encodes the artifact as task parameters.
    #[must_use]
    pub fn to_parameters(&self) -> BTreeMap<String, String> {
        let mut parameters = BTreeMap::new();
        parameters.insert(
            parameter::WORKSPACE.to_owned(),
            self.workspace_id.to_string(),
        );
        parameters.insert(parameter::LIBRARY.to_owned(), self.library.to_string());
        parameters.insert(parameter::PATH.to_owned(), self.path.to_string());
        parameters.insert(
            parameter::GROUP.to_owned(),
            self.coordinate.group_id.clone(),
        );
        parameters.insert(
            parameter::ARTIFACT.to_owned(),
            self.coordinate.artifact_id.clone(),
        );
        parameters.insert(
            parameter::VERSION.to_owned(),
            self.coordinate.version.to_string(),
        );
        if let Some(packaging) = &self.coordinate.packaging {
            parameters.insert(parameter::PACKAGING.to_owned(), packaging.clone());
        }
        if let Some(classifier) = &self.coordinate.classifier {
            parameters.insert(parameter::CLASSIFIER.to_owned(), classifier.clone());
        }
        parameters
    }

    /// Decodes an artifact from task parameters.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactInspectionError::InvalidParameter`] if a required
    /// parameter is missing or does not parse as its domain type.
    pub fn from_parameters(
        parameters: &BTreeMap<String, String>,
    ) -> Result<Self, ArtifactInspectionError> {
        let workspace_id =
            WorkspaceId::new(required(parameters, parameter::WORKSPACE)?).map_err(|error| {
                ArtifactInspectionError::invalid_parameter(parameter::WORKSPACE, error.to_string())
            })?;
        let library =
            LibraryId::new(required(parameters, parameter::LIBRARY)?).map_err(|error| {
                ArtifactInspectionError::invalid_parameter(parameter::LIBRARY, error.to_string())
            })?;
        let path = AbsolutePath::new(required(parameters, parameter::PATH)?).map_err(|error| {
            ArtifactInspectionError::invalid_parameter(parameter::PATH, error.to_string())
        })?;
        let version = Version::new(required(parameters, parameter::VERSION)?).map_err(|error| {
            ArtifactInspectionError::invalid_parameter(parameter::VERSION, error.to_string())
        })?;
        let coordinate = ArtifactCoordinate::new(
            required(parameters, parameter::GROUP)?,
            required(parameters, parameter::ARTIFACT)?,
            version,
        )
        .map_err(|error| {
            ArtifactInspectionError::invalid_parameter(parameter::ARTIFACT, error.to_string())
        })?;
        let coordinate = match parameters.get(parameter::PACKAGING) {
            Some(packaging) => coordinate.with_packaging(packaging.clone()),
            None => coordinate,
        };
        let coordinate = match parameters.get(parameter::CLASSIFIER) {
            Some(classifier) => coordinate.with_classifier(classifier.clone()),
            None => coordinate,
        };
        coordinate.validate().map_err(|error| {
            ArtifactInspectionError::invalid_parameter(parameter::CLASSIFIER, error.to_string())
        })?;
        Ok(Self {
            workspace_id,
            library,
            path,
            coordinate,
        })
    }
}

/// A consumer dependency to inspect.
///
/// The consumer is addressed by the absolute directory that contains its build
/// descriptor (for example a Maven `pom.xml`), plus the coordinate the library
/// produces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedDependency {
    /// The consumer module's absolute directory.
    pub path: AbsolutePath,
    /// The artifact coordinate the consumer is expected to reference.
    pub coordinate: ArtifactCoordinate,
}

/// Whether an artifact is present in the effective local repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum ArtifactPresence {
    /// The artifact was found.
    Found {
        /// The absolute path of the artifact file.
        path: AbsolutePath,
        /// The artifact file's size in bytes, when reported by the platform.
        size_bytes: Option<u64>,
    },
    /// The artifact was not found in the effective local repository.
    Missing {
        /// The path that was expected, when the repository location is known.
        expected_path: Option<AbsolutePath>,
    },
}

impl ArtifactPresence {
    /// Returns `true` when the artifact is present.
    #[must_use]
    pub const fn is_found(&self) -> bool {
        matches!(self, Self::Found { .. })
    }
}

/// The result of inspecting a consumer's declared dependency version.
///
/// The three outcomes are deliberate: a version that could not be resolved
/// reliably is reported as [`Unknown`](Self::Unknown) rather than guessed, and a
/// consumer that does not declare the artifact at all is
/// [`NotDeclared`](Self::NotDeclared) (`tasks/009.md` §14).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum DependencyVersionInspection {
    /// The consumer references a concrete, resolvable version.
    Declared {
        /// The version the consumer resolves to.
        version: Version,
    },
    /// The consumer does not declare a dependency on the artifact.
    NotDeclared,
    /// A version exists but could not be resolved reliably (for example it is
    /// supplied by a parent POM, a BOM, or a property the inspector cannot
    /// resolve).
    Unknown {
        /// Why the version could not be resolved.
        reason: String,
    },
}

/// A structured artifact-inspection failure.
///
/// Every variant carries enough context to explain what happened without
/// exposing credentials or Maven settings values.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ArtifactInspectionError {
    /// No inspector is wired; inspection cannot be performed.
    #[error("artifact inspection is unavailable: {message}")]
    Unavailable {
        /// A human-readable explanation.
        message: String,
    },

    /// The effective local repository location could not be determined.
    #[error("could not determine the effective local repository: {message}")]
    LocalRepositoryUnknown {
        /// A human-readable explanation.
        message: String,
    },

    /// A filesystem read failed.
    #[error("could not read '{path}': {message}")]
    Io {
        /// The path that could not be read.
        path: String,
        /// A human-readable explanation.
        message: String,
    },

    /// An inspection parameter was missing from a task.
    #[error("missing artifact parameter '{key}'")]
    MissingParameter {
        /// The missing parameter key.
        key: String,
    },

    /// An inspection parameter was invalid.
    #[error("invalid artifact parameter '{key}': {message}")]
    InvalidParameter {
        /// The offending parameter key.
        key: String,
        /// Why it is invalid.
        message: String,
    },

    /// The inspector itself failed.
    #[error("artifact inspector error: {message}")]
    Internal {
        /// A human-readable description of the failure.
        message: String,
    },
}

impl ArtifactInspectionError {
    /// Builds an [`ArtifactInspectionError::InvalidParameter`].
    #[must_use]
    pub fn invalid_parameter(key: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidParameter {
            key: key.into(),
            message: message.into(),
        }
    }

    /// Returns a stable, machine-readable code for the failure.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "ARTIFACT_INSPECTION_UNAVAILABLE",
            Self::LocalRepositoryUnknown { .. } => "ARTIFACT_LOCAL_REPOSITORY_UNKNOWN",
            Self::Io { .. } => "ARTIFACT_IO_ERROR",
            Self::MissingParameter { .. } => "ARTIFACT_MISSING_PARAMETER",
            Self::InvalidParameter { .. } => "ARTIFACT_INVALID_PARAMETER",
            Self::Internal { .. } => "ARTIFACT_INTERNAL_ERROR",
        }
    }
}

fn required<'a>(
    parameters: &'a BTreeMap<String, String>,
    key: &str,
) -> Result<&'a str, ArtifactInspectionError> {
    parameters.get(key).map(String::as_str).ok_or_else(|| {
        ArtifactInspectionError::MissingParameter {
            key: key.to_owned(),
        }
    })
}

/// Inspects artifacts and consumer dependency versions.
///
/// Implementations are provided by the build adapter. Read-only and safe to run
/// concurrently; the inspector never mutates a repository or a POM.
#[async_trait]
pub trait ArtifactInspector: Send + Sync {
    /// Locates `artifact` in the effective local repository.
    ///
    /// # Errors
    ///
    /// Returns an [`ArtifactInspectionError`] when the effective local
    /// repository cannot be determined or a filesystem read fails. A missing
    /// artifact is not an error: it is [`ArtifactPresence::Missing`].
    async fn locate(
        &self,
        artifact: &ResolvedArtifact,
    ) -> Result<ArtifactPresence, ArtifactInspectionError>;

    /// Inspects the version a consumer resolves for `dependency`, when it can be
    /// determined reliably.
    ///
    /// # Errors
    ///
    /// Returns an [`ArtifactInspectionError`] when the consumer descriptor
    /// cannot be read. An unresolvable version is
    /// [`DependencyVersionInspection::Unknown`], not an error.
    async fn dependency_version(
        &self,
        dependency: &ResolvedDependency,
    ) -> Result<DependencyVersionInspection, ArtifactInspectionError>;
}

/// An [`ArtifactInspector`] that reports every inspection as unavailable.
///
/// The composition root replaces it with the real build-adapter inspector once
/// one is wired; until then, artifact verification and version alignment report
/// [`ArtifactInspectionError::Unavailable`] rather than fabricating a result.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullArtifactInspector;

#[async_trait]
impl ArtifactInspector for NullArtifactInspector {
    async fn locate(
        &self,
        _artifact: &ResolvedArtifact,
    ) -> Result<ArtifactPresence, ArtifactInspectionError> {
        Err(ArtifactInspectionError::Unavailable {
            message: "no artifact inspector is wired".to_owned(),
        })
    }

    async fn dependency_version(
        &self,
        _dependency: &ResolvedDependency,
    ) -> Result<DependencyVersionInspection, ArtifactInspectionError> {
        Err(ArtifactInspectionError::Unavailable {
            message: "no artifact inspector is wired".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact() -> ResolvedArtifact {
        ResolvedArtifact {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            library: LibraryId::new("common-core").unwrap(),
            path: AbsolutePath::new("/w/acme/libraries/common-core").unwrap(),
            coordinate: ArtifactCoordinate::new(
                "com.acme",
                "common-core",
                Version::new("1.8.0-SNAPSHOT").unwrap(),
            )
            .unwrap(),
        }
    }

    #[test]
    fn parameters_round_trip() {
        let original = artifact();
        let decoded = ResolvedArtifact::from_parameters(&original.to_parameters()).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn parameters_round_trip_with_packaging_and_classifier() {
        let original = ResolvedArtifact {
            coordinate: artifact()
                .coordinate
                .with_packaging("pom")
                .with_classifier("sources"),
            ..artifact()
        };
        let decoded = ResolvedArtifact::from_parameters(&original.to_parameters()).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn missing_parameters_are_reported() {
        let mut parameters = artifact().to_parameters();
        parameters.remove(parameter::PATH);
        let error = ResolvedArtifact::from_parameters(&parameters).unwrap_err();
        assert_eq!(error.code(), "ARTIFACT_MISSING_PARAMETER");
    }

    #[tokio::test]
    async fn null_inspector_reports_unavailable() {
        let inspector = NullArtifactInspector;
        let error = inspector.locate(&artifact()).await.unwrap_err();
        assert_eq!(error.code(), "ARTIFACT_INSPECTION_UNAVAILABLE");
    }
}
