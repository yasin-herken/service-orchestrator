//! Workspace, group, profile, and environment commands.
//!
//! These commands compose other operations into workflows. They never execute
//! steps themselves: they construct a [`WorkflowDefinition`](service_orchestrator_execution::WorkflowDefinition)
//! and submit it to the task engine. Dependency information is preserved as
//! step prerequisites, but full dependency-aware scheduling remains the
//! engine's responsibility.

use std::collections::BTreeMap;

use service_orchestrator_config::ConfiguredWorkspace;
use service_orchestrator_domain::{
    CommandKind, DependencyTarget, GroupId, Permission, ProcessState, ProfileId, ServiceCapability,
    ServiceId, StepId, TaskKind, WorkflowKind, WorkspaceId,
};

use crate::app::Application;
use crate::commands::WorkflowBuilder;
use crate::error::ApplicationError;
use crate::models::{OperationResult, PrepareEnvironmentRequest};

impl Application {
    /// Synchronizes every repository in a workspace.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] if the workspace does
    /// not exist.
    pub fn sync_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let workspace = self.require_workspace(workspace_id)?;
        let mut builder = WorkflowBuilder::new(WorkflowKind::SyncWorkspace);
        for service in &workspace.workspace.services {
            if service.repository.is_some() {
                builder.add(
                    TaskKind::GitSync,
                    Vec::new(),
                    Some(format!("sync {}", service.id)),
                );
            }
        }
        for library in &workspace.workspace.libraries {
            if library.repository.is_some() {
                builder.add(
                    TaskKind::GitSync,
                    Vec::new(),
                    Some(format!("sync {}", library.id)),
                );
            }
        }
        self.finish_workflow(builder, TaskKind::GitSync)
    }

    /// Builds every library and then every buildable service in a workspace.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] if the workspace does
    /// not exist.
    pub fn build_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let workspace = self.require_workspace(workspace_id)?;
        let mut builder = WorkflowBuilder::new(WorkflowKind::BuildWorkspace);

        let mut library_steps: Vec<StepId> = Vec::new();
        for library in &workspace.workspace.libraries {
            if library.commands.contains(&CommandKind::Build) {
                library_steps.push(builder.add(
                    TaskKind::BuildLibrary,
                    Vec::new(),
                    Some(format!("build {}", library.id)),
                ));
            }
        }
        for service in &workspace.workspace.services {
            if service.supports(ServiceCapability::Build) && service.has_build_command() {
                builder.add(
                    TaskKind::BuildService,
                    library_steps.clone(),
                    Some(format!("build {}", service.id)),
                );
            }
        }
        self.finish_workflow(builder, TaskKind::BuildService)
    }

    /// Prepares a development environment according to a structured request.
    ///
    /// The request selects the services and the phases to run: synchronization,
    /// an optional branch checkout, library/service builds, Liquibase, start,
    /// and health checks. Phases are wired into a single dependency-ordered
    /// workflow; the engine executes it.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] or
    /// [`ApplicationError::ServiceNotFound`] for unknown entities, and
    /// [`ApplicationError::InvalidOperation`] when a requested phase is not
    /// valid for a selected service.
    pub fn prepare_environment(
        &self,
        request: PrepareEnvironmentRequest,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let workspace = self.require_workspace(&request.workspace_id)?;
        let selected = self.selected_service_ids(workspace, &request.services)?;
        if selected.is_empty() {
            return Err(ApplicationError::invalid_operation(
                "no services were selected to prepare",
            ));
        }

        for id in &selected {
            let service = workspace
                .workspace
                .service(id)
                .expect("selected services are validated to exist");
            self.check_dependencies(workspace, service)?;
            if request.start
                && !(service.supports(ServiceCapability::Run) && service.has_start_command())
            {
                return Err(ApplicationError::invalid_operation(format!(
                    "service '{id}' cannot be started"
                )));
            }
            if request.build && !service.supports(ServiceCapability::Build) {
                return Err(ApplicationError::invalid_operation(format!(
                    "service '{id}' does not declare the build capability"
                )));
            }
        }

        let mut builder = WorkflowBuilder::new(WorkflowKind::PrepareEnvironment);

        // Synchronize repositories.
        let mut sync_steps: BTreeMap<ServiceId, StepId> = BTreeMap::new();
        for id in &selected {
            let service = workspace
                .workspace
                .service(id)
                .expect("selected services are validated to exist");
            if service.repository.is_some() {
                let step = builder.add(TaskKind::GitSync, Vec::new(), Some(format!("sync {id}")));
                sync_steps.insert(id.clone(), step);
            }
        }

        // Optionally check out a branch after synchronizing.
        if let Some(branch) = &request.branch {
            for id in &selected {
                let prerequisites = sync_steps.get(id).cloned().into_iter().collect();
                builder.add(
                    TaskKind::GitCheckout,
                    prerequisites,
                    Some(format!("checkout {branch} for {id}")),
                );
            }
        }

        // Build libraries.
        let mut library_steps: Vec<StepId> = Vec::new();
        if request.build {
            for library in &workspace.workspace.libraries {
                if library.commands.contains(&CommandKind::Build) {
                    library_steps.push(builder.add(
                        TaskKind::BuildLibrary,
                        Vec::new(),
                        Some(format!("build {}", library.id)),
                    ));
                }
            }
        }

        // Build services.
        let mut build_steps: BTreeMap<ServiceId, StepId> = BTreeMap::new();
        if request.build {
            for id in &selected {
                let service = workspace
                    .workspace
                    .service(id)
                    .expect("selected services are validated to exist");
                if service.has_build_command() {
                    let step = builder.add(
                        TaskKind::BuildService,
                        library_steps.clone(),
                        Some(format!("build {id}")),
                    );
                    build_steps.insert(id.clone(), step);
                }
            }
        }

        // Run Liquibase.
        let mut migrate_steps: BTreeMap<ServiceId, StepId> = BTreeMap::new();
        if request.run_liquibase {
            for id in &selected {
                let enabled = self
                    .liquibase_config(workspace, id)
                    .is_some_and(|config| config.enabled);
                if enabled {
                    let prerequisites = build_steps.get(id).cloned().into_iter().collect();
                    let step = builder.add(
                        TaskKind::Liquibase,
                        prerequisites,
                        Some(format!("liquibase {id}")),
                    );
                    migrate_steps.insert(id.clone(), step);
                }
            }
        }

        // Start services, then probe their health.
        if request.start {
            let mut start_steps: BTreeMap<ServiceId, StepId> = BTreeMap::new();
            for id in &selected {
                let prerequisites = Self::start_prerequisites(id, &build_steps, &migrate_steps);
                let step = builder.add(
                    TaskKind::StartService,
                    prerequisites,
                    Some(format!("start {id}")),
                );
                start_steps.insert(id.clone(), step);
            }
            for id in &selected {
                let service = workspace
                    .workspace
                    .service(id)
                    .expect("selected services are validated to exist");
                for dependency in &service.dependencies {
                    if !dependency.required {
                        continue;
                    }
                    if let DependencyTarget::Service(target) = &dependency.target {
                        if let Some(prerequisite) = start_steps.get(target) {
                            builder.depend_on(&start_steps[id], prerequisite.clone());
                        }
                    }
                }
            }
            for id in &selected {
                let service = workspace
                    .workspace
                    .service(id)
                    .expect("selected services are validated to exist");
                if !service.health_checks.is_empty() {
                    builder.add(
                        TaskKind::HealthCheck,
                        vec![start_steps[id].clone()],
                        Some(format!("health {id}")),
                    );
                }
            }
        }

        self.finish_workflow(builder, TaskKind::StartService)
    }

    /// Starts every service in a group.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] or
    /// [`ApplicationError::GroupNotFound`], or
    /// [`ApplicationError::InvalidOperation`] if a member cannot be started.
    pub fn start_group(
        &self,
        workspace_id: &WorkspaceId,
        group_id: &GroupId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let workspace = self.require_workspace(workspace_id)?;
        let group = workspace
            .workspace
            .group(group_id)
            .ok_or_else(|| ApplicationError::GroupNotFound(group_id.clone()))?;
        let kind = WorkflowKind::Custom("start_group".to_owned());
        self.start_services_workflow(workspace, &group.services, kind)
    }

    /// Stops every service in a group.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] or
    /// [`ApplicationError::GroupNotFound`].
    pub fn stop_group(
        &self,
        workspace_id: &WorkspaceId,
        group_id: &GroupId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let workspace = self.require_workspace(workspace_id)?;
        let group = workspace
            .workspace
            .group(group_id)
            .ok_or_else(|| ApplicationError::GroupNotFound(group_id.clone()))?;
        let kind = WorkflowKind::Custom("stop_group".to_owned());
        self.stop_services_workflow(workspace, &group.services, kind)
    }

    /// Starts every service selected by a profile.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] or
    /// [`ApplicationError::ProfileNotFound`], or
    /// [`ApplicationError::InvalidOperation`] if a selected service cannot be
    /// started.
    pub fn start_profile(
        &self,
        workspace_id: &WorkspaceId,
        profile_id: &ProfileId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let workspace = self.require_workspace(workspace_id)?;
        let profile = workspace
            .workspace
            .profile(profile_id)
            .ok_or_else(|| ApplicationError::ProfileNotFound(profile_id.clone()))?;
        let services = self.profile_service_ids(workspace, profile);
        self.start_services_workflow(workspace, &services, WorkflowKind::StartProfile)
    }

    /// Stops every service selected by a profile.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] or
    /// [`ApplicationError::ProfileNotFound`].
    pub fn stop_profile(
        &self,
        workspace_id: &WorkspaceId,
        profile_id: &ProfileId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let workspace = self.require_workspace(workspace_id)?;
        let profile = workspace
            .workspace
            .profile(profile_id)
            .ok_or_else(|| ApplicationError::ProfileNotFound(profile_id.clone()))?;
        let services = self.profile_service_ids(workspace, profile);
        self.stop_services_workflow(workspace, &services, WorkflowKind::StopProfile)
    }

    /// Builds and submits a start workflow for the given services.
    fn start_services_workflow(
        &self,
        workspace: &ConfiguredWorkspace,
        service_ids: &[ServiceId],
        kind: WorkflowKind,
    ) -> Result<OperationResult, ApplicationError> {
        let mut to_start: Vec<ServiceId> = Vec::new();
        for id in service_ids {
            let service = workspace
                .workspace
                .service(id)
                .ok_or_else(|| ApplicationError::ServiceNotFound(id.clone()))?;
            self.check_dependencies(workspace, service)?;
            if !(service.supports(ServiceCapability::Run) && service.has_start_command()) {
                return Err(ApplicationError::invalid_operation(format!(
                    "service '{id}' cannot be started"
                )));
            }
            let current = self.runtime().service_state(id).unwrap_or_default();
            if current.process.is_running() {
                continue;
            }
            if current.process.is_active() {
                return Err(ApplicationError::invalid_operation(format!(
                    "service '{id}' is currently {} and cannot be started",
                    current.process
                )));
            }
            to_start.push(id.clone());
        }

        if to_start.is_empty() {
            return Ok(OperationResult::AlreadyInDesiredState {
                kind: TaskKind::StartService,
            });
        }

        let mut builder = WorkflowBuilder::new(kind.clone());
        let mut steps: BTreeMap<ServiceId, StepId> = BTreeMap::new();
        for id in &to_start {
            steps.insert(
                id.clone(),
                builder.add(
                    TaskKind::StartService,
                    Vec::new(),
                    Some(format!("start {id}")),
                ),
            );
        }
        for id in &to_start {
            let service = workspace
                .workspace
                .service(id)
                .expect("services were validated above");
            for dependency in &service.dependencies {
                if !dependency.required {
                    continue;
                }
                if let DependencyTarget::Service(target) = &dependency.target {
                    if let Some(prerequisite) = steps.get(target) {
                        builder.depend_on(&steps[id], prerequisite.clone());
                    }
                }
            }
        }

        let workflow_id = self
            .tasks()
            .submit_workflow(builder.build())
            .map_err(super::map_submit_error)?;
        for id in &to_start {
            let current = self.runtime().service_state(id).unwrap_or_default();
            self.runtime()
                .set_service_state(id, current.with_process(ProcessState::Starting));
        }
        Ok(OperationResult::WorkflowScheduled { workflow_id, kind })
    }

    /// Builds and submits a stop workflow for the given services.
    fn stop_services_workflow(
        &self,
        workspace: &ConfiguredWorkspace,
        service_ids: &[ServiceId],
        kind: WorkflowKind,
    ) -> Result<OperationResult, ApplicationError> {
        let mut to_stop: Vec<ServiceId> = Vec::new();
        for id in service_ids {
            let service = workspace
                .workspace
                .service(id)
                .ok_or_else(|| ApplicationError::ServiceNotFound(id.clone()))?;
            if !service.supports(ServiceCapability::Run) {
                continue;
            }
            let current = self.runtime().service_state(id).unwrap_or_default();
            match current.process {
                ProcessState::Running | ProcessState::Starting | ProcessState::Unknown => {
                    to_stop.push(id.clone());
                }
                ProcessState::Stopping
                | ProcessState::Stopped
                | ProcessState::Failed
                | ProcessState::NotRunning => {}
            }
        }

        if to_stop.is_empty() {
            return Ok(OperationResult::AlreadyInDesiredState {
                kind: TaskKind::StopService,
            });
        }

        let mut builder = WorkflowBuilder::new(kind.clone());
        let mut steps: BTreeMap<ServiceId, StepId> = BTreeMap::new();
        for id in &to_stop {
            steps.insert(
                id.clone(),
                builder.add(
                    TaskKind::StopService,
                    Vec::new(),
                    Some(format!("stop {id}")),
                ),
            );
        }
        // A service that depends on another must stop before it, so the
        // dependency's stop step depends on the dependent's stop step.
        for id in &to_stop {
            let service = workspace
                .workspace
                .service(id)
                .expect("services were validated above");
            for dependency in &service.dependencies {
                if !dependency.required {
                    continue;
                }
                if let DependencyTarget::Service(target) = &dependency.target {
                    if let Some(dependency_step) = steps.get(target) {
                        builder.depend_on(dependency_step, steps[id].clone());
                    }
                }
            }
        }

        let workflow_id = self
            .tasks()
            .submit_workflow(builder.build())
            .map_err(super::map_submit_error)?;
        for id in &to_stop {
            let current = self.runtime().service_state(id).unwrap_or_default();
            self.runtime()
                .set_service_state(id, current.with_process(ProcessState::Stopping));
        }
        Ok(OperationResult::WorkflowScheduled { workflow_id, kind })
    }
}
