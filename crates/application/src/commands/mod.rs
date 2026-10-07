//! Application commands (write operations).
//!
//! Commands are the "C" half of the commands/queries split. Each command
//! validates its preconditions, applies the policy check, builds a task or
//! workflow definition, and submits it to the [`TaskManager`]. Commands never
//! execute infrastructure directly.
//!
//! [`TaskManager`]: service_orchestrator_execution::TaskManager

use std::collections::BTreeMap;

use service_orchestrator_config::ConfiguredWorkspace;
use service_orchestrator_domain::{
    DependencyTarget, Permission, Service, ServiceId, StepId, TaskId, TaskKind, WorkflowKind,
    WorkflowStep,
};
use service_orchestrator_execution::{TaskDefinition, TaskManagerError, WorkflowDefinition};

use crate::app::Application;
use crate::error::ApplicationError;
use crate::models::OperationResult;

mod library;
mod service;
mod workflow;

/// Maps a task-engine submission failure into an application error.
pub(crate) fn map_submit_error(error: TaskManagerError) -> ApplicationError {
    ApplicationError::TaskManager {
        message: error.to_string(),
    }
}

/// Incrementally assembles a [`WorkflowDefinition`].
///
/// Step identifiers are generated internally (`s0`, `s1`, ...), so a workflow
/// never depends on entity identifiers being unique across kinds or on their
/// length. Callers record the returned [`StepId`] to wire later steps to it.
pub(crate) struct WorkflowBuilder {
    kind: WorkflowKind,
    steps: Vec<WorkflowStep>,
    counter: usize,
}

impl WorkflowBuilder {
    /// Creates a builder for a workflow of `kind`.
    pub(crate) fn new(kind: WorkflowKind) -> Self {
        Self {
            kind,
            steps: Vec::new(),
            counter: 0,
        }
    }

    /// Adds a step with the given task and prerequisites, returning its id.
    pub(crate) fn add(
        &mut self,
        task: TaskKind,
        depends_on: Vec<StepId>,
        description: Option<String>,
    ) -> StepId {
        let id = StepId::new(format!("s{}", self.counter))
            .expect("generated step identifiers are always valid");
        self.counter += 1;
        self.steps.push(WorkflowStep {
            id: id.clone(),
            description,
            task,
            depends_on,
        });
        id
    }

    /// Adds a prerequisite to an existing step.
    pub(crate) fn depend_on(&mut self, step: &StepId, prerequisite: StepId) {
        if let Some(entry) = self.steps.iter_mut().find(|entry| &entry.id == step) {
            entry.depends_on.push(prerequisite);
        }
    }

    /// Returns `true` if no steps have been added.
    pub(crate) fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Consumes the builder and produces a workflow definition.
    pub(crate) fn build(self) -> WorkflowDefinition {
        WorkflowDefinition {
            kind: self.kind,
            steps: self.steps,
        }
    }
}

impl Application {
    /// Requests cancellation of a task.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::TaskNotFound`] if the task does not exist,
    /// or [`ApplicationError::TaskManager`] if the engine refuses cancellation.
    pub fn cancel_task(&self, task_id: &TaskId) -> Result<(), ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        self.tasks()
            .cancel(task_id)
            .map_err(|_| ApplicationError::TaskNotFound(task_id.clone()))
    }

    /// Submits a single-task definition and returns the scheduling result.
    pub(crate) fn schedule(
        &self,
        definition: TaskDefinition,
    ) -> Result<OperationResult, ApplicationError> {
        let kind = definition.kind.clone();
        let task_id = self.tasks().submit(definition).map_err(map_submit_error)?;
        Ok(OperationResult::TaskScheduled { task_id, kind })
    }

    /// Submits a workflow, or reports that there was nothing to do.
    pub(crate) fn finish_workflow(
        &self,
        builder: WorkflowBuilder,
        fallback: TaskKind,
    ) -> Result<OperationResult, ApplicationError> {
        if builder.is_empty() {
            return Ok(OperationResult::AlreadyInDesiredState { kind: fallback });
        }
        let kind = builder.kind.clone();
        let workflow_id = self
            .tasks()
            .submit_workflow(builder.build())
            .map_err(map_submit_error)?;
        Ok(OperationResult::WorkflowScheduled { workflow_id, kind })
    }

    /// Resolves the services selected by a request.
    ///
    /// An empty selection means "every service in the workspace". A non-empty
    /// selection is validated up front so a typo fails before any work is
    /// scheduled. Duplicates are removed while preserving order.
    pub(crate) fn selected_service_ids(
        &self,
        workspace: &ConfiguredWorkspace,
        requested: &[ServiceId],
    ) -> Result<Vec<ServiceId>, ApplicationError> {
        if requested.is_empty() {
            return Ok(workspace
                .workspace
                .services
                .iter()
                .map(|service| service.id.clone())
                .collect());
        }
        let mut seen = Vec::new();
        for id in requested {
            if workspace.workspace.service(id).is_none() {
                return Err(ApplicationError::ServiceNotFound(id.clone()));
            }
            if !seen.contains(id) {
                seen.push(id.clone());
            }
        }
        Ok(seen)
    }

    /// Expands a profile into the set of services it selects.
    pub(crate) fn profile_service_ids(
        &self,
        workspace: &ConfiguredWorkspace,
        profile: &service_orchestrator_domain::Profile,
    ) -> Vec<ServiceId> {
        let mut services: Vec<ServiceId> = profile.services.clone();
        for group_id in &profile.groups {
            if let Some(group) = workspace.workspace.group(group_id) {
                for service in &group.services {
                    if !services.contains(service) {
                        services.push(service.clone());
                    }
                }
            }
        }
        services
    }

    /// Verifies that every required dependency of `service` resolves.
    ///
    /// Configuration and domain validation already guarantee this for loaded
    /// configuration; the check keeps the use case honest and produces a
    /// precise application error if an invalid workspace is ever supplied.
    pub(crate) fn check_dependencies(
        &self,
        workspace: &ConfiguredWorkspace,
        service: &Service,
    ) -> Result<(), ApplicationError> {
        for dependency in &service.dependencies {
            if !dependency.required {
                continue;
            }
            let resolved = match &dependency.target {
                DependencyTarget::Service(target) => workspace.workspace.service(target).is_some(),
                DependencyTarget::Library(target) => workspace.workspace.library(target).is_some(),
            };
            if !resolved {
                return Err(ApplicationError::DependencyNotFound {
                    owner: service.id.to_string(),
                    target: dependency.target.to_string(),
                });
            }
        }
        Ok(())
    }

    /// Returns the start-step prerequisites for a service: its build and
    /// migration steps, if any.
    pub(crate) fn start_prerequisites(
        service_id: &ServiceId,
        build_steps: &BTreeMap<ServiceId, StepId>,
        migrate_steps: &BTreeMap<ServiceId, StepId>,
    ) -> Vec<StepId> {
        let mut prerequisites = Vec::new();
        if let Some(step) = build_steps.get(service_id) {
            prerequisites.push(step.clone());
        }
        if let Some(step) = migrate_steps.get(service_id) {
            prerequisites.push(step.clone());
        }
        prerequisites
    }
}
