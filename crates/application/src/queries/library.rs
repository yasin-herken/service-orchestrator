//! Read-only library queries.
//!
//! These queries inspect a library's repository, its artifact, and its
//! consumers. They never fetch, check out, build, or write: repository state is
//! read through the [`GitManager`], artifact presence and consumer versions
//! through the [`ArtifactInspector`]. The `async` shape reflects that both reach
//! the process manager through a port (`tasks/009.md` §26).
//!
//! [`GitManager`]: crate::ports::GitManager
//! [`ArtifactInspector`]: crate::ports::ArtifactInspector

use service_orchestrator_config::ConfiguredWorkspace;
use service_orchestrator_domain::{
    ArtifactCoordinate, BuildSystem, DependencyTarget, GitState, LibraryId, Permission,
    RepositoryStatus, ServiceId, WorkspaceId,
};

use crate::app::Application;
use crate::error::ApplicationError;
use crate::models::{
    AlignmentStatus, ConsumerAlignment, LibraryArtifactVerification, LibraryDependent,
    LibraryStatus, VersionAlignmentPlan,
};
use crate::ports::{
    ArtifactInspectionError, ArtifactPresence, DependencyVersionInspection, GitOptions,
    ResolvedDependency,
};

/// Maps a structured [`ArtifactInspectionError`] into an application error.
pub(crate) fn map_artifact_inspection_error(error: ArtifactInspectionError) -> ApplicationError {
    ApplicationError::ArtifactInspection {
        code: error.code().to_owned(),
        message: error.to_string(),
    }
}

/// Reduces a full repository status into the compact [`GitState`] the library
/// snapshot carries.
fn map_git_state(status: &RepositoryStatus) -> GitState {
    if !status.lifecycle.is_available() {
        return GitState::Unknown;
    }
    if status.has_local_changes() {
        return GitState::Modified;
    }
    if status.is_diverged() {
        return GitState::Diverged;
    }
    if status.is_ahead() {
        return GitState::Ahead;
    }
    if status.is_behind() {
        return GitState::Behind;
    }
    GitState::Clean
}

impl Application {
    /// Returns a structured status snapshot of a library.
    ///
    /// The snapshot combines validated configuration with the last observed Git
    /// state, inspected best-effort through the [`GitManager`]. A repository that
    /// cannot be inspected yields [`GitState::Unknown`] and a structured
    /// `repository_error` rather than failing the whole query. Local artifact
    /// presence is deliberately **not** guessed here; the definitive answer is
    /// [`Application::verify_library_artifact`].
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::LibraryNotFound`] if the library does not
    /// exist.
    ///
    /// [`GitManager`]: crate::ports::GitManager
    pub async fn get_library_status(
        &self,
        library_id: &LibraryId,
    ) -> Result<LibraryStatus, ApplicationError> {
        self.authorize(Permission::Read)?;
        let (workspace, library) = self.locate_library(library_id)?;

        let mut branch = None;
        let mut commit = None;
        let mut git_state = GitState::Unknown;
        let mut repository_error = None;
        if library.repository.is_some() {
            match self.resolve_library_repository(library_id) {
                Ok(repository) => {
                    let options = GitOptions::new().with_timeout(self.git_read_timeout());
                    match self.git().status(&repository, &options).await {
                        Ok(status) => {
                            branch = status.branch.clone();
                            commit = status.commit.clone();
                            git_state = map_git_state(&status);
                        }
                        Err(error) => repository_error = Some(error.to_string()),
                    }
                }
                Err(error) => repository_error = Some(error.to_string()),
            }
        }

        let artifacts: Vec<ArtifactCoordinate> =
            library.produced_artifacts().into_iter().cloned().collect();
        Ok(LibraryStatus {
            library_id: library.id.clone(),
            workspace_id: workspace.workspace.id.clone(),
            display_name: library.display_name.clone(),
            build_system: library.build_system,
            artifact: library.artifact.clone(),
            artifacts,
            repository_url: library.repository.as_ref().map(|repo| repo.url.clone()),
            branch,
            commit,
            git_state,
            repository_error,
            artifact_present: None,
        })
    }

    /// Lists the services that depend on a library.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::LibraryNotFound`] if the library does not
    /// exist.
    pub fn find_library_dependents(
        &self,
        library_id: &LibraryId,
    ) -> Result<Vec<LibraryDependent>, ApplicationError> {
        self.authorize(Permission::Read)?;
        let (workspace, _library) = self.locate_library(library_id)?;
        Ok(dependents_in_workspace(workspace, library_id))
    }

    /// Verifies that a library's primary artifact is present in the effective
    /// local repository.
    ///
    /// Verification respects the effective Maven configuration: the
    /// [`ArtifactInspector`] resolves the local repository the way Maven does,
    /// rather than assuming `~/.m2/repository` (`tasks/009.md` §7).
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::LibraryNotFound`] if the library does not
    /// exist, [`ApplicationError::LibraryConfigurationInvalid`] if it has no path
    /// or artifact, or [`ApplicationError::ArtifactInspection`] if the
    /// repository cannot be determined or read. A missing artifact is **not** an
    /// error: it is reported with `present == false`.
    ///
    /// [`ArtifactInspector`]: crate::ports::ArtifactInspector
    pub async fn verify_library_artifact(
        &self,
        library_id: &LibraryId,
    ) -> Result<LibraryArtifactVerification, ApplicationError> {
        self.authorize(Permission::Read)?;
        let (workspace, library) = self.locate_library(library_id)?;
        let artifact = self.resolve_library_artifact(workspace, library)?;
        let presence = self
            .artifacts()
            .locate(&artifact)
            .await
            .map_err(map_artifact_inspection_error)?;
        let coordinate = artifact.coordinate.clone();
        let (present, path, message) = match presence {
            ArtifactPresence::Found { path, .. } => (
                true,
                Some(path),
                format!("artifact '{coordinate}' is present in the local repository"),
            ),
            ArtifactPresence::Missing { expected_path } => (
                false,
                expected_path,
                format!("artifact '{coordinate}' is not present in the local repository"),
            ),
        };
        Ok(LibraryArtifactVerification {
            library_id: library.id.clone(),
            workspace_id: workspace.workspace.id.clone(),
            artifact: coordinate,
            present,
            path,
            message,
        })
    }

    /// Produces a structured version-alignment plan for a library and its
    /// consumers.
    ///
    /// Every consumer is inspected read-only. A version that cannot be resolved
    /// reliably is reported as [`AlignmentStatus::Unknown`] rather than guessed,
    /// and a consumer that does not declare the artifact is
    /// [`AlignmentStatus::Unresolved`] (`tasks/009.md` §14).
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::LibraryNotFound`] if the library does not
    /// exist, [`ApplicationError::LibraryConfigurationInvalid`] if it has no path
    /// or artifact, or [`ApplicationError::VersionInspectionUnsupported`] when no
    /// artifact inspector is available to resolve consumer versions.
    pub async fn inspect_library_version_alignment(
        &self,
        library_id: &LibraryId,
    ) -> Result<VersionAlignmentPlan, ApplicationError> {
        self.authorize(Permission::Read)?;
        let (workspace, library) = self.locate_library(library_id)?;
        let target = self
            .resolve_library_artifact(workspace, library)?
            .coordinate;

        let mut consumers = Vec::new();
        for dependent in dependents_in_workspace(workspace, library_id) {
            consumers.push(
                self.inspect_consumer(workspace, &dependent.service_id, &target)
                    .await?,
            );
        }
        Ok(VersionAlignmentPlan {
            library_id: library.id.clone(),
            workspace_id: workspace.workspace.id.clone(),
            target,
            consumers,
        })
    }

    /// Inspects one consumer's declared version of `target`.
    ///
    /// A consumer that is not a Maven module, or that has no workspace path, is
    /// reported [`AlignmentStatus::Unknown`] without an inspection attempt. An
    /// inspection that reports the inspector itself unavailable fails the whole
    /// query; any other inspection failure degrades that one consumer to
    /// [`AlignmentStatus::Unknown`].
    async fn inspect_consumer(
        &self,
        workspace: &ConfiguredWorkspace,
        service_id: &ServiceId,
        target: &ArtifactCoordinate,
    ) -> Result<ConsumerAlignment, ApplicationError> {
        let workspace_id = workspace.workspace.id.clone();
        let Some(service) = workspace.workspace.service(service_id) else {
            return Ok(unknown_consumer(
                service_id.clone(),
                workspace_id,
                target,
                "the dependent service is not configured in this workspace",
            ));
        };
        if service.build_system != Some(BuildSystem::Maven) {
            return Ok(unknown_consumer(
                service_id.clone(),
                workspace_id,
                target,
                "the consumer does not use a Maven build system",
            ));
        }
        let Some(relative) = service.path.as_ref() else {
            return Ok(unknown_consumer(
                service_id.clone(),
                workspace_id,
                target,
                "the consumer has no workspace path",
            ));
        };
        let path = match self.absolute_entity_path(workspace, relative) {
            Ok(path) => path,
            Err(error) => {
                return Ok(unknown_consumer(
                    service_id.clone(),
                    workspace_id,
                    target,
                    error.to_string(),
                ));
            }
        };

        let dependency = ResolvedDependency {
            path: path.clone(),
            coordinate: target.clone(),
        };
        match self.artifacts().dependency_version(&dependency).await {
            Ok(DependencyVersionInspection::Declared { version }) => {
                let status = if version == target.version {
                    AlignmentStatus::Aligned
                } else {
                    AlignmentStatus::Mismatch
                };
                Ok(ConsumerAlignment {
                    service_id: service_id.clone(),
                    workspace_id,
                    path: Some(path),
                    current_version: Some(version),
                    status,
                    detail: None,
                })
            }
            Ok(DependencyVersionInspection::NotDeclared) => Ok(ConsumerAlignment {
                service_id: service_id.clone(),
                workspace_id,
                path: Some(path),
                current_version: None,
                status: AlignmentStatus::Unresolved,
                detail: Some("the consumer does not declare the artifact".to_owned()),
            }),
            Ok(DependencyVersionInspection::Unknown { reason }) => Ok(ConsumerAlignment {
                service_id: service_id.clone(),
                workspace_id,
                path: Some(path),
                current_version: None,
                status: AlignmentStatus::Unknown,
                detail: Some(reason),
            }),
            Err(error) if error.code() == "ARTIFACT_INSPECTION_UNAVAILABLE" => {
                Err(ApplicationError::VersionInspectionUnsupported {
                    library: target.artifact_id.clone(),
                    message: error.to_string(),
                })
            }
            Err(error) => Ok(ConsumerAlignment {
                service_id: service_id.clone(),
                workspace_id,
                path: Some(path),
                current_version: None,
                status: AlignmentStatus::Unknown,
                detail: Some(error.to_string()),
            }),
        }
    }
}

/// Collects the services in `workspace` that depend on `library_id`.
fn dependents_in_workspace(
    workspace: &ConfiguredWorkspace,
    library_id: &LibraryId,
) -> Vec<LibraryDependent> {
    let workspace_id: &WorkspaceId = &workspace.workspace.id;
    let mut dependents = Vec::new();
    for service in &workspace.workspace.services {
        for dependency in &service.dependencies {
            if let DependencyTarget::Library(target) = &dependency.target {
                if target == library_id {
                    dependents.push(LibraryDependent {
                        service_id: service.id.clone(),
                        workspace_id: workspace_id.clone(),
                        required: dependency.required,
                        kind: dependency.kind,
                    });
                }
            }
        }
    }
    dependents
}

/// Builds an [`AlignmentStatus::Unknown`] consumer entry with no resolved path.
fn unknown_consumer(
    service_id: ServiceId,
    workspace_id: WorkspaceId,
    target: &ArtifactCoordinate,
    detail: impl Into<String>,
) -> ConsumerAlignment {
    ConsumerAlignment {
        service_id,
        workspace_id,
        path: None,
        current_version: None,
        status: AlignmentStatus::Unknown,
        detail: Some(match detail.into() {
            detail if detail.is_empty() => format!("cannot inspect {target}"),
            detail => detail,
        }),
    }
}
