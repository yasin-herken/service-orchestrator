//! Build-system vocabulary.
//!
//! The domain names *which* build system an entity uses and *which*
//! build-system-neutral operation is being performed. It deliberately says
//! nothing about *how* a build runs: executables, wrappers, argument vectors,
//! and process execution belong to the infrastructure `build` adapter.
//!
//! Keeping the vocabulary here (rather than in the adapter or in configuration)
//! is what lets the application, the build adapter, and the interfaces agree on
//! a closed set of systems and operations while remaining build-system
//! agnostic. Maven is the primary system used by the current backend services,
//! but nothing in the domain or the application is coupled to it: adding a new
//! system is a new enum variant plus a new provider, not a change to
//! orchestration.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::task::TaskKind;

/// A build system.
///
/// The set is intentionally closed. Each variant maps to exactly one
/// [`BuildProvider`](https://docs.rs) in the infrastructure adapter, selected
/// from validated configuration. Gradle is planned but not modelled yet: an
/// unimplemented variant would appear operational to users, so it is added only
/// when a provider can honour it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildSystem {
    /// Apache Maven.
    Maven,
    /// The npm package manager.
    Npm,
}

impl BuildSystem {
    /// Returns the canonical, machine-readable label.
    ///
    /// The label is the spelling used in configuration files and in task
    /// parameters, so it is stable and round-trips through [`BuildSystem::parse`].
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Maven => "maven",
            Self::Npm => "npm",
        }
    }

    /// Parses a build-system label.
    ///
    /// Returns `None` for an unknown value; the caller decides whether that is
    /// a configuration error (an explicit `build_system`) or simply "no system
    /// could be inferred".
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "maven" => Some(Self::Maven),
            "npm" => Some(Self::Npm),
            _ => None,
        }
    }
}

impl fmt::Display for BuildSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A build-system-neutral operation.
///
/// These operations have genuinely shared *semantics* across build systems, but
/// not every system supports every operation. In particular Maven `install`,
/// Gradle `publishToMavenLocal`, and npm dependency installation differ, so a
/// provider declares the operations it supports and an unsupported request is
/// rejected with a structured error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildOperation {
    /// Remove generated outputs.
    Clean,
    /// Compile and assemble the entity.
    Build,
    /// Run the entity's test suite.
    Test,
    /// Resolve and install dependencies.
    InstallDependencies,
    /// Publish the entity's artifact to the local repository.
    PublishLocalArtifact,
}

impl BuildOperation {
    /// Returns the canonical, machine-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Build => "build",
            Self::Test => "test",
            Self::InstallDependencies => "install_dependencies",
            Self::PublishLocalArtifact => "publish_local_artifact",
        }
    }

    /// Parses an operation label.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "clean" => Some(Self::Clean),
            "build" => Some(Self::Build),
            "test" => Some(Self::Test),
            "install_dependencies" => Some(Self::InstallDependencies),
            "publish_local_artifact" => Some(Self::PublishLocalArtifact),
            _ => None,
        }
    }

    /// Returns the operation implied by a task kind, if any.
    ///
    /// `BuildService`, `BuildLibrary`, and the primitive `MavenBuild`/`NpmBuild`
    /// kinds all mean "build"; `NpmInstall` means "install dependencies";
    /// `LibraryInstall` means "publish the module's artifact to the local
    /// repository". Other kinds are not build operations.
    #[must_use]
    pub fn from_task_kind(kind: &TaskKind) -> Option<Self> {
        match kind {
            TaskKind::BuildService
            | TaskKind::BuildLibrary
            | TaskKind::MavenBuild
            | TaskKind::NpmBuild => Some(Self::Build),
            TaskKind::LibraryInstall => Some(Self::PublishLocalArtifact),
            TaskKind::NpmInstall => Some(Self::InstallDependencies),
            _ => None,
        }
    }
}

impl fmt::Display for BuildOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systems_round_trip_through_their_label() {
        for system in [BuildSystem::Maven, BuildSystem::Npm] {
            assert_eq!(BuildSystem::parse(system.label()), Some(system));
        }
        assert_eq!(BuildSystem::parse("gradle"), None);
        assert_eq!(BuildSystem::parse("  maven "), Some(BuildSystem::Maven));
    }

    #[test]
    fn operations_round_trip_through_their_label() {
        for operation in [
            BuildOperation::Clean,
            BuildOperation::Build,
            BuildOperation::Test,
            BuildOperation::InstallDependencies,
            BuildOperation::PublishLocalArtifact,
        ] {
            assert_eq!(BuildOperation::parse(operation.label()), Some(operation));
        }
        assert_eq!(BuildOperation::parse("deploy"), None);
    }

    #[test]
    fn task_kinds_map_to_operations() {
        assert_eq!(
            BuildOperation::from_task_kind(&TaskKind::BuildService),
            Some(BuildOperation::Build)
        );
        assert_eq!(
            BuildOperation::from_task_kind(&TaskKind::BuildLibrary),
            Some(BuildOperation::Build)
        );
        assert_eq!(
            BuildOperation::from_task_kind(&TaskKind::MavenBuild),
            Some(BuildOperation::Build)
        );
        assert_eq!(
            BuildOperation::from_task_kind(&TaskKind::NpmBuild),
            Some(BuildOperation::Build)
        );
        assert_eq!(
            BuildOperation::from_task_kind(&TaskKind::NpmInstall),
            Some(BuildOperation::InstallDependencies)
        );
        assert_eq!(
            BuildOperation::from_task_kind(&TaskKind::LibraryInstall),
            Some(BuildOperation::PublishLocalArtifact)
        );
        assert_eq!(BuildOperation::from_task_kind(&TaskKind::GitSync), None);
    }

    #[test]
    fn labels_are_stable() {
        assert_eq!(BuildSystem::Maven.to_string(), "maven");
        assert_eq!(
            BuildOperation::InstallDependencies.to_string(),
            "install_dependencies"
        );
    }
}
