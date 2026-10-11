//! Library commands.
//!
//! The Library Manager is application-layer orchestration: it never runs a build
//! tool, a Git command, or a filesystem write itself. It validates library
//! configuration, resolves an artifact against the effective build system,
//! composes the existing Git and build tasks into dependency-ordered workflows,
//! and submits them to the task engine (`AGENTS.md` §21, `tasks/009.md` §17).
//!
//! # Identity, not repository names
//!
//! A Maven artifact is identified by its coordinate
//! (`group:artifact:version`), never by the repository that happens to produce
//! it. [`Application::resolve_library_artifact`] pairs a library's module
//! directory with its primary coordinate, so an inspection task submitted to the
//! engine can run later without re-reading configuration.
//!
//! # Ordering
//!
//! Library dependencies are resolved to a dependency-first order before any task
//! is scheduled. Independent libraries are scheduled without prerequisites and
//! may build concurrently; a dependent library's steps wait for its
//! prerequisites. Ordering is expressed as workflow-step dependencies — the
//! engine remains the only scheduler (`tasks/009.md` §10).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use service_orchestrator_config::ConfiguredWorkspace;
use service_orchestrator_domain::{
    BuildOperation, CommandKind, DependencyTarget, Library, LibraryId, Permission, StepId,
    TaskKind, WorkflowKind,
};
use service_orchestrator_execution::TaskDefinition;

use crate::app::Application;
use crate::commands::WorkflowBuilder;
use crate::error::ApplicationError;
use crate::models::{LibraryPrepareOptions, OperationResult, PrepareSelectedLibrariesRequest};
use crate::ports::{parameter, ResolvedArtifact};

impl Application {
    /// Builds a library.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::LibraryNotFound`] if the library does not
    /// exist, or [`ApplicationError::InvalidOperation`] if it has no build
    /// command or no path.
    pub fn build_library(
        &self,
        library_id: &LibraryId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let (workspace, library) = self.locate_library(library_id)?;
        if !library.commands.contains(&CommandKind::Build) {
            return Err(ApplicationError::invalid_operation(format!(
                "library '{library_id}' has no build command configured"
            )));
        }
        let target = self.library_build_target(workspace, library, BuildOperation::Build)?;
        self.schedule(
            TaskDefinition::new(TaskKind::BuildLibrary)
                .for_library(library_id.clone())
                .with_parameters(target.to_parameters())
                .with_timeout(self.default_timeout_seconds()),
        )
    }

    /// Installs a library's artifact into the effective local repository.
    ///
    /// The install is a build-system operation (`PublishLocalArtifact`): Maven
    /// performs it with its `install` phase through the Maven build provider. The
    /// manager never copies a JAR into `~/.m2/repository` itself
    /// (`tasks/009.md` §6).
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::LibraryNotFound`] if the library does not
    /// exist, [`ApplicationError::BuildProviderUnsupported`] if it has no
    /// recognized build system, [`ApplicationError::LibraryConfigurationInvalid`]
    /// if it has no path or no artifact coordinate, or
    /// [`ApplicationError::InvalidOperation`] if the target is invalid.
    pub fn install_library(
        &self,
        library_id: &LibraryId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let (workspace, library) = self.locate_library(library_id)?;
        self.ensure_publish_supported(library)?;
        let artifact = self.resolve_library_artifact(workspace, library)?;
        let target =
            self.library_build_target(workspace, library, BuildOperation::PublishLocalArtifact)?;
        let mut parameters = target.to_parameters();
        // Carry the artifact identity alongside the build target so the recorded
        // task names the exact coordinate that will be verified.
        for (key, value) in artifact.to_parameters() {
            parameters.entry(key).or_insert(value);
        }
        self.schedule(
            TaskDefinition::new(TaskKind::LibraryInstall)
                .for_library(library_id.clone())
                .with_parameters(parameters)
                .with_timeout(self.default_timeout_seconds()),
        )
    }

    /// Prepares one library: optional Git synchronization, build, local install,
    /// and artifact verification, in dependency order.
    ///
    /// Synchronization is optional so a developer can build the current local
    /// branch with uncommitted changes; the configured branch is never forced
    /// (`tasks/009.md` §20).
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::LibraryNotFound`] if the library does not
    /// exist, or a structured configuration, provider, or dependency error when a
    /// requested phase is not valid for the library.
    pub fn prepare_library(
        &self,
        library_id: &LibraryId,
        options: &LibraryPrepareOptions,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let (workspace, library) = self.locate_library(library_id)?;
        let selected = vec![library.id.clone()];
        self.prepare_libraries_workflow(workspace, &selected, options, WorkflowKind::PrepareLibrary)
    }

    /// Prepares several libraries in dependency order.
    ///
    /// An empty selection means "every library in the workspace". Transitive
    /// library dependencies are included so a dependent library is never built
    /// before the library it consumes.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] or
    /// [`ApplicationError::LibraryNotFound`] for unknown entities,
    /// [`ApplicationError::DependencyCycle`] if the selection is cyclic, or a
    /// structured configuration or provider error for an invalid phase.
    pub fn prepare_selected_libraries(
        &self,
        request: PrepareSelectedLibrariesRequest,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let workspace = self.require_workspace(&request.workspace_id)?;
        self.prepare_libraries_workflow(
            workspace,
            &request.libraries,
            &request.options,
            WorkflowKind::PrepareLibraries,
        )
    }

    // -- helpers --------------------------------------------------------------

    /// Rejects a library that declares no build system, since no provider can
    /// publish its artifact.
    fn ensure_publish_supported(&self, library: &Library) -> Result<(), ApplicationError> {
        if library.build_system.is_none() {
            return Err(ApplicationError::BuildProviderUnsupported {
                library: library.id.to_string(),
                operation: BuildOperation::PublishLocalArtifact.label().to_owned(),
            });
        }
        Ok(())
    }

    /// Builds the prepare workflow for the resolved libraries.
    fn prepare_libraries_workflow(
        &self,
        workspace: &ConfiguredWorkspace,
        requested: &[LibraryId],
        options: &LibraryPrepareOptions,
        kind: WorkflowKind,
    ) -> Result<OperationResult, ApplicationError> {
        let selected = self.selected_library_ids(workspace, requested)?;
        if selected.is_empty() {
            return Err(ApplicationError::invalid_operation(
                "no libraries were selected to prepare",
            ));
        }

        for id in &selected {
            let library = workspace
                .workspace
                .library(id)
                .expect("selected libraries are validated to exist");
            if options.build && !library.commands.contains(&CommandKind::Build) {
                return Err(ApplicationError::LibraryConfigurationInvalid {
                    library: id.to_string(),
                    message: "no build command is configured".to_owned(),
                });
            }
            if options.install {
                self.ensure_publish_supported(library)?;
                // Resolves the module path and primary coordinate, failing early
                // if either is missing.
                let _ = self.resolve_library_artifact(workspace, library)?;
            }
        }

        let order = self.library_build_order(workspace, &selected)?;
        let mut builder = WorkflowBuilder::new(kind);
        let mut last_steps: BTreeMap<LibraryId, StepId> = BTreeMap::new();

        for id in &order {
            let library = workspace
                .workspace
                .library(id)
                .expect("ordered libraries are configured");

            // A library's steps wait for every selected dependency's final step.
            let mut cursor: Vec<StepId> = Vec::new();
            for dependency in &library.dependencies {
                if let DependencyTarget::Library(dependency_id) = &dependency.target {
                    if let Some(step) = last_steps.get(dependency_id) {
                        cursor.push(step.clone());
                    }
                }
            }

            if options.sync {
                if let Some(parameters) = self.library_repository_parameters(workspace, id)? {
                    let sync = builder.add_with_parameters(
                        TaskKind::GitSync,
                        cursor.clone(),
                        Some(format!("sync {id}")),
                        parameters.clone(),
                    );
                    cursor = vec![sync.clone()];
                    if let Some(branch) = &options.branch {
                        let mut checkout_parameters = parameters;
                        checkout_parameters
                            .insert(parameter::BRANCH.to_owned(), branch.to_string());
                        let checkout = builder.add_with_parameters(
                            TaskKind::GitCheckout,
                            vec![sync],
                            Some(format!("checkout {branch} for {id}")),
                            checkout_parameters,
                        );
                        cursor = vec![checkout];
                    }
                }
            }

            if options.build {
                let target =
                    self.library_build_target(workspace, library, BuildOperation::Build)?;
                let build = builder.add_with_parameters(
                    TaskKind::BuildLibrary,
                    cursor.clone(),
                    Some(format!("build {id}")),
                    target.to_parameters(),
                );
                cursor = vec![build];
            }

            if options.install {
                let target = self.library_build_target(
                    workspace,
                    library,
                    BuildOperation::PublishLocalArtifact,
                )?;
                let install = builder.add_with_parameters(
                    TaskKind::LibraryInstall,
                    cursor.clone(),
                    Some(format!("install {id}")),
                    target.to_parameters(),
                );
                cursor = vec![install];
            }

            if options.verify {
                let artifact = self.resolve_library_artifact(workspace, library)?;
                let verify = builder.add_with_parameters(
                    TaskKind::ArtifactVerification,
                    cursor.clone(),
                    Some(format!("verify {id}")),
                    artifact.to_parameters(),
                );
                cursor = vec![verify];
            }

            if let Some(step) = cursor.last() {
                last_steps.insert(id.clone(), step.clone());
            }
        }

        self.finish_workflow(builder, TaskKind::BuildLibrary)
    }

    /// Resolves the libraries selected by a request.
    ///
    /// An empty selection means "every library in the workspace"; a non-empty
    /// selection is validated up front so a typo fails before any work is
    /// scheduled. Duplicates are removed while preserving order.
    pub(crate) fn selected_library_ids(
        &self,
        workspace: &ConfiguredWorkspace,
        requested: &[LibraryId],
    ) -> Result<Vec<LibraryId>, ApplicationError> {
        if requested.is_empty() {
            return Ok(workspace
                .workspace
                .libraries
                .iter()
                .map(|library| library.id.clone())
                .collect());
        }
        let mut seen = Vec::new();
        for id in requested {
            if workspace.workspace.library(id).is_none() {
                return Err(ApplicationError::LibraryNotFound(id.clone()));
            }
            if !seen.contains(id) {
                seen.push(id.clone());
            }
        }
        Ok(seen)
    }

    /// Returns the selected libraries and their transitive library dependencies
    /// in dependency-first order.
    ///
    /// Uses a simple Kahn topological sort over the declared library
    /// dependencies. The domain exposes only crate-private cycle detection, so
    /// the application performs the ordering itself rather than duplicating the
    /// graph model (`tasks/009.md` §9). A cycle is reported as
    /// [`ApplicationError::DependencyCycle`].
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::DependencyResolution`] if a declared
    /// dependency is not configured, or [`ApplicationError::DependencyCycle`] if
    /// the closure is cyclic.
    pub(crate) fn library_build_order(
        &self,
        workspace: &ConfiguredWorkspace,
        requested: &[LibraryId],
    ) -> Result<Vec<LibraryId>, ApplicationError> {
        // Transitive closure of the requested libraries.
        let mut closure: BTreeSet<LibraryId> = BTreeSet::new();
        let mut stack: Vec<LibraryId> = requested.to_vec();
        while let Some(id) = stack.pop() {
            if !closure.insert(id.clone()) {
                continue;
            }
            let library = workspace.workspace.library(&id).ok_or_else(|| {
                ApplicationError::DependencyResolution {
                    library: id.to_string(),
                    message: "the library is not configured in this workspace".to_owned(),
                }
            })?;
            for dependency in &library.dependencies {
                if let DependencyTarget::Library(target) = &dependency.target {
                    if workspace.workspace.library(target).is_none() {
                        return Err(ApplicationError::DependencyResolution {
                            library: id.to_string(),
                            message: format!("dependency '{target}' is not configured"),
                        });
                    }
                    stack.push(target.clone());
                }
            }
        }

        // Kahn's algorithm: a library depends on its library dependencies, so
        // edges point from a dependency to its dependents.
        let mut in_degree: BTreeMap<LibraryId, usize> =
            closure.iter().map(|id| (id.clone(), 0usize)).collect();
        let mut dependents: BTreeMap<LibraryId, Vec<LibraryId>> = BTreeMap::new();
        for id in &closure {
            let library = workspace
                .workspace
                .library(id)
                .expect("the closure contains only configured libraries");
            for dependency in &library.dependencies {
                if let DependencyTarget::Library(target) = &dependency.target {
                    if closure.contains(target) {
                        *in_degree.get_mut(id).expect("in-degree is seeded") += 1;
                        dependents
                            .entry(target.clone())
                            .or_default()
                            .push(id.clone());
                    }
                }
            }
        }

        let mut ready: VecDeque<LibraryId> = in_degree
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(id, _)| id.clone())
            .collect();
        let mut order: Vec<LibraryId> = Vec::with_capacity(closure.len());
        while let Some(id) = ready.pop_front() {
            order.push(id.clone());
            if let Some(children) = dependents.get(&id) {
                for child in children {
                    let degree = in_degree.get_mut(child).expect("a known dependent");
                    *degree -= 1;
                    if *degree == 0 {
                        ready.push_back(child.clone());
                    }
                }
            }
        }

        if order.len() != closure.len() {
            let ordered: BTreeSet<&LibraryId> = order.iter().collect();
            let cyclic: Vec<String> = closure
                .iter()
                .filter(|id| !ordered.contains(id))
                .map(ToString::to_string)
                .collect();
            return Err(ApplicationError::DependencyCycle {
                libraries: cyclic.join(", "),
            });
        }

        Ok(order)
    }

    /// Returns a [`ResolvedArtifact`] for a library's primary artifact.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::LibraryConfigurationInvalid`] if the library
    /// has no workspace path or no artifact coordinate, or
    /// [`ApplicationError::InvalidOperation`] if the joined path cannot be
    /// represented.
    pub(crate) fn resolve_library_artifact(
        &self,
        workspace: &ConfiguredWorkspace,
        library: &Library,
    ) -> Result<ResolvedArtifact, ApplicationError> {
        let relative =
            library
                .path
                .as_ref()
                .ok_or_else(|| ApplicationError::LibraryConfigurationInvalid {
                    library: library.id.to_string(),
                    message: "no workspace path is configured".to_owned(),
                })?;
        let coordinate = library
            .produced_artifacts()
            .into_iter()
            .next()
            .cloned()
            .ok_or_else(|| ApplicationError::LibraryConfigurationInvalid {
                library: library.id.to_string(),
                message: "no artifact coordinate is configured".to_owned(),
            })?;
        let path = self.absolute_entity_path(workspace, relative)?;
        Ok(ResolvedArtifact {
            workspace_id: workspace.workspace.id.clone(),
            library: library.id.clone(),
            path,
            coordinate,
        })
    }
}
