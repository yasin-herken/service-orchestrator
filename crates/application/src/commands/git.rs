//! Git commands.
//!
//! Every mutating Git operation is expressed as a task and submitted to the
//! engine — the application never runs `git` itself. Each task carries the
//! workspace-aware [`ResolvedRepository`] as executor parameters, so the engine
//! can run it later without re-reading configuration.
//!
//! Destructive operations ([`Application::reset_repository_hard`],
//! [`Application::clean_repository`], [`Application::discard_local_changes`])
//! pass through [`Application::authorize`] with
//! [`Permission::Destructive`](service_orchestrator_domain::Permission::Destructive)
//! at submission, so an interface cannot schedule them without an explicit
//! confirmation.

use std::collections::BTreeMap;

use service_orchestrator_config::ConfiguredWorkspace;
use service_orchestrator_domain::{
    Branch, LibraryId, Permission, ServiceId, TaskKind, WorkflowKind, WorkspaceId,
};
use service_orchestrator_execution::TaskDefinition;

use crate::app::Application;
use crate::commands::WorkflowBuilder;
use crate::error::ApplicationError;
use crate::models::OperationResult;
use crate::ports::{parameter, ResolvedRepository};

impl Application {
    /// Builds a Git task definition carrying a resolved repository.
    fn git_task(&self, kind: TaskKind, repository: &ResolvedRepository) -> TaskDefinition {
        TaskDefinition::new(kind)
            .with_parameters(repository.to_parameters())
            .with_timeout(self.default_timeout_seconds())
    }

    /// Synchronizes a service's repository: fetch, checkout, fast-forward.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, or [`ApplicationError::InvalidOperation`] if it has no repository.
    pub fn sync_repository(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let (workspace, service) = self.locate_service(service_id)?;
        self.check_dependencies(workspace, service)?;
        let repository = self.resolve_service_repository(service_id)?;
        self.schedule(self.git_task(TaskKind::GitSync, &repository))
    }

    /// Synchronizes a library's repository: fetch, checkout, fast-forward.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::LibraryNotFound`] if the library does not
    /// exist, or [`ApplicationError::InvalidOperation`] if it has no repository.
    pub fn sync_library(
        &self,
        library_id: &LibraryId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let repository = self.resolve_library_repository(library_id)?;
        self.schedule(
            TaskDefinition::new(TaskKind::GitSync)
                .for_library(library_id.clone())
                .with_parameters(repository.to_parameters())
                .with_timeout(self.default_timeout_seconds()),
        )
    }

    /// Fetches a service's repository without touching the working tree.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, or [`ApplicationError::InvalidOperation`] if it has no repository.
    pub fn fetch_repository(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let repository = self.resolve_service_repository(service_id)?;
        self.schedule(self.git_task(TaskKind::GitFetch, &repository))
    }

    /// Pulls a service's repository, fast-forward only.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, or [`ApplicationError::InvalidOperation`] if it has no repository.
    pub fn pull_repository(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let repository = self.resolve_service_repository(service_id)?;
        self.schedule(self.git_task(TaskKind::GitPull, &repository))
    }

    /// Checks out `branch` in a service's repository.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, or [`ApplicationError::InvalidOperation`] if it has no repository.
    pub fn checkout_repository(
        &self,
        service_id: &ServiceId,
        branch: &Branch,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let repository = self.resolve_service_repository(service_id)?;
        let mut parameters = repository.to_parameters();
        parameters.insert(parameter::BRANCH.to_owned(), branch.to_string());
        self.schedule(
            TaskDefinition::new(TaskKind::GitCheckout)
                .with_parameters(parameters)
                .with_timeout(self.default_timeout_seconds()),
        )
    }

    /// Clones a service's repository into its configured path.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, or [`ApplicationError::InvalidOperation`] if it has no repository.
    pub fn clone_repository(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let repository = self.resolve_service_repository(service_id)?;
        self.schedule(self.git_task(TaskKind::GitClone, &repository))
    }

    /// Discards the working tree of a service's repository with a hard reset.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::PermissionDenied`] when destructive
    /// operations are not authorized, [`ApplicationError::ServiceNotFound`] if
    /// the service does not exist, or [`ApplicationError::InvalidOperation`] if
    /// it has no repository.
    pub fn reset_repository_hard(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::Destructive)?;
        let repository = self.resolve_service_repository(service_id)?;
        self.schedule(self.git_task(TaskKind::GitResetHard, &repository))
    }

    /// Removes untracked files from a service's repository.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::PermissionDenied`] when destructive
    /// operations are not authorized, [`ApplicationError::ServiceNotFound`] if
    /// the service does not exist, or [`ApplicationError::InvalidOperation`] if
    /// it has no repository.
    pub fn clean_repository(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::Destructive)?;
        let repository = self.resolve_service_repository(service_id)?;
        self.schedule(self.git_task(TaskKind::GitClean, &repository))
    }

    /// Discards every local change in a service's repository.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::PermissionDenied`] when destructive
    /// operations are not authorized, [`ApplicationError::ServiceNotFound`] if
    /// the service does not exist, or [`ApplicationError::InvalidOperation`] if
    /// it has no repository.
    pub fn discard_local_changes(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::Destructive)?;
        let repository = self.resolve_service_repository(service_id)?;
        self.schedule(self.git_task(TaskKind::GitDiscardChanges, &repository))
    }

    // -- multi-repository operations -----------------------------------------

    /// Synchronizes the selected repositories in a workspace.
    ///
    /// An empty selection means "every service with a repository". The
    /// repositories are submitted as independent workflow steps, so the engine
    /// runs them with the configured concurrency and one repository's failure
    /// never invalidates another's result.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] or
    /// [`ApplicationError::ServiceNotFound`] for unknown entities.
    pub fn sync_repositories(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        self.repositories_workflow(
            workspace_id,
            services,
            TaskKind::GitSync,
            None,
            "sync_repositories",
        )
    }

    /// Fetches the selected repositories in a workspace.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] or
    /// [`ApplicationError::ServiceNotFound`] for unknown entities.
    pub fn fetch_repositories(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        self.repositories_workflow(
            workspace_id,
            services,
            TaskKind::GitFetch,
            None,
            "fetch_repositories",
        )
    }

    /// Checks out `branch` in the selected repositories of a workspace.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] or
    /// [`ApplicationError::ServiceNotFound`] for unknown entities.
    pub fn checkout_repositories(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
        branch: &Branch,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        self.repositories_workflow(
            workspace_id,
            services,
            TaskKind::GitCheckout,
            Some(branch),
            "checkout_repositories",
        )
    }

    /// Builds a workflow of independent Git steps over the selected services.
    fn repositories_workflow(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
        kind: TaskKind,
        branch: Option<&Branch>,
        name: &str,
    ) -> Result<OperationResult, ApplicationError> {
        let workspace = self.require_workspace(workspace_id)?;
        let selected = self.selected_service_ids(workspace, services)?;
        let mut builder = WorkflowBuilder::new(WorkflowKind::Custom(name.to_owned()));
        for id in &selected {
            let parameters = match self.service_repository_parameters(workspace, id)? {
                Some(parameters) => parameters,
                None => continue,
            };
            let mut parameters = parameters;
            if let Some(branch) = branch {
                parameters.insert(parameter::BRANCH.to_owned(), branch.to_string());
            }
            builder.add_with_parameters(
                kind.clone(),
                Vec::new(),
                Some(format!("{} {}", kind.label(), id)),
                parameters,
            );
        }
        self.finish_workflow(builder, kind)
    }

    /// Returns the executor parameters for a service's repository, if it has one.
    ///
    /// This is used by the workspace and environment workflows to attach the
    /// resolved repository to each Git step.
    pub(crate) fn service_repository_parameters(
        &self,
        workspace: &ConfiguredWorkspace,
        id: &ServiceId,
    ) -> Result<Option<BTreeMap<String, String>>, ApplicationError> {
        let service = workspace
            .workspace
            .service(id)
            .ok_or_else(|| ApplicationError::ServiceNotFound(id.clone()))?;
        let Some(repository) = service.repository.as_ref() else {
            return Ok(None);
        };
        Ok(Some(
            self.resolve_repository(workspace, id.as_str(), repository)?
                .to_parameters(),
        ))
    }

    /// Returns the executor parameters for a library's repository, if it has one.
    pub(crate) fn library_repository_parameters(
        &self,
        workspace: &ConfiguredWorkspace,
        id: &LibraryId,
    ) -> Result<Option<BTreeMap<String, String>>, ApplicationError> {
        let library = workspace
            .workspace
            .library(id)
            .ok_or_else(|| ApplicationError::LibraryNotFound(id.clone()))?;
        let Some(repository) = library.repository.as_ref() else {
            return Ok(None);
        };
        Ok(Some(
            self.resolve_repository(workspace, id.as_str(), repository)?
                .to_parameters(),
        ))
    }
}
