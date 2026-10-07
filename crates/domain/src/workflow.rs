//! The workflow model.
//!
//! A [`Workflow`] is a composition of [`TaskKind`]s with declared ordering
//! constraints. It describes *what* should run and *in what order*, but not the
//! mechanism that runs it. Scheduling, concurrency, retry, timeout, and failure
//! propagation belong to the execution engine.
//!
//! The domain does validate the workflow's *shape*: step identifiers must be
//! unique, every declared dependency must exist, and the graph must be acyclic.
//! Those are invariants of a well-formed workflow, not scheduling decisions.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::ids::{StepId, WorkflowId};
use crate::task::TaskKind;

/// The high-level purpose of a workflow.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowKind {
    /// Prepare a full development environment.
    PrepareEnvironment,
    /// Synchronize the workspace's repositories.
    SyncWorkspace,
    /// Build every selected service.
    BuildWorkspace,
    /// Start a developer profile.
    StartProfile,
    /// Stop a developer profile.
    StopProfile,
    /// A project-specific workflow.
    Custom(String),
}

impl WorkflowKind {
    /// Returns a short, human-readable label.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::PrepareEnvironment => "prepare_environment",
            Self::SyncWorkspace => "sync_workspace",
            Self::BuildWorkspace => "build_workspace",
            Self::StartProfile => "start_profile",
            Self::StopProfile => "stop_profile",
            Self::Custom(name) => name,
        }
    }
}

impl fmt::Display for WorkflowKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A single step in a workflow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// The identifier of the step, unique within its workflow.
    pub id: StepId,
    /// An optional human-readable description.
    pub description: Option<String>,
    /// The kind of task the step performs.
    pub task: TaskKind,
    /// The steps that must complete before this step may run.
    pub depends_on: Vec<StepId>,
}

impl WorkflowStep {
    /// Creates a step with no dependencies.
    #[must_use]
    pub fn new(id: StepId, task: TaskKind) -> Self {
        Self {
            id,
            description: None,
            task,
            depends_on: Vec::new(),
        }
    }

    /// Sets the step description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Adds a prerequisite step.
    #[must_use]
    pub fn after(mut self, step: StepId) -> Self {
        self.depends_on.push(step);
        self
    }
}

/// A composition of tasks with ordering constraints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    /// The stable identifier of the workflow.
    pub id: WorkflowId,
    /// The high-level purpose of the workflow.
    pub kind: WorkflowKind,
    /// The steps that make up the workflow.
    pub steps: Vec<WorkflowStep>,
}

impl Workflow {
    /// Creates an empty workflow.
    #[must_use]
    pub fn new(id: WorkflowId, kind: WorkflowKind) -> Self {
        Self {
            id,
            kind,
            steps: Vec::new(),
        }
    }

    /// Adds a step.
    #[must_use]
    pub fn with_step(mut self, step: WorkflowStep) -> Self {
        self.steps.push(step);
        self
    }

    /// Returns the number of steps.
    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Returns `true` if the workflow has no steps.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Returns the step with the given id, if present.
    #[must_use]
    pub fn step(&self, id: &StepId) -> Option<&WorkflowStep> {
        self.steps.iter().find(|step| &step.id == id)
    }

    /// Iterates over the steps in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = &WorkflowStep> {
        self.steps.iter()
    }

    /// Validates the workflow's structure.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the workflow is empty, has
    /// duplicate step ids, or references an unknown step, or a
    /// [`DomainError::DependencyCycle`] if the steps form a cycle.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.steps.is_empty() {
            return Err(DomainError::validation(
                "workflow",
                format!("workflow '{}' must have at least one step", self.id),
            ));
        }

        let mut ids = std::collections::BTreeSet::new();
        for step in &self.steps {
            if !ids.insert(step.id.clone()) {
                return Err(DomainError::validation(
                    "workflow",
                    format!("workflow '{}' has duplicate step '{}'", self.id, step.id),
                ));
            }
        }

        for step in &self.steps {
            for dependency in &step.depends_on {
                if !ids.contains(dependency) {
                    return Err(DomainError::validation(
                        "workflow",
                        format!("step '{}' depends on unknown step '{dependency}'", step.id),
                    ));
                }
            }
        }

        let nodes: Vec<String> = self.steps.iter().map(|step| step.id.to_string()).collect();
        let edges: BTreeMap<String, Vec<String>> = self
            .steps
            .iter()
            .map(|step| {
                (
                    step.id.to_string(),
                    step.depends_on.iter().map(ToString::to_string).collect(),
                )
            })
            .collect();
        if let Some(cycle) = crate::graph::find_cycle(&nodes, &edges) {
            return Err(DomainError::DependencyCycle {
                nodes: cycle.join(", "),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(id: &str) -> WorkflowStep {
        WorkflowStep::new(StepId::new(id).unwrap(), TaskKind::Custom(id.to_owned()))
    }

    fn workflow() -> Workflow {
        Workflow::new(
            WorkflowId::new("prepare-environment").unwrap(),
            WorkflowKind::PrepareEnvironment,
        )
    }

    #[test]
    fn accepts_a_linear_dag() {
        let workflow = workflow()
            .with_step(step("sync"))
            .with_step(step("build").after(StepId::new("sync").unwrap()))
            .with_step(step("migrate").after(StepId::new("build").unwrap()))
            .with_step(
                step("start")
                    .after(StepId::new("build").unwrap())
                    .after(StepId::new("migrate").unwrap()),
            );
        assert!(workflow.validate().is_ok());
        assert_eq!(workflow.len(), 4);
    }

    #[test]
    fn rejects_empty_workflows() {
        assert!(workflow().validate().is_err());
    }

    #[test]
    fn rejects_duplicate_step_ids() {
        let workflow = workflow().with_step(step("sync")).with_step(step("sync"));
        assert!(workflow.validate().is_err());
    }

    #[test]
    fn rejects_unknown_dependencies() {
        let workflow = workflow().with_step(step("build").after(StepId::new("missing").unwrap()));
        assert!(workflow.validate().is_err());
    }

    #[test]
    fn detects_dependency_cycles() {
        let workflow = workflow()
            .with_step(step("a").after(StepId::new("b").unwrap()))
            .with_step(step("b").after(StepId::new("a").unwrap()));
        let error = workflow.validate().unwrap_err();
        assert!(matches!(error, DomainError::DependencyCycle { .. }));
    }

    #[test]
    fn detects_self_dependency_as_a_cycle() {
        let workflow = workflow().with_step(step("a").after(StepId::new("a").unwrap()));
        let error = workflow.validate().unwrap_err();
        assert!(matches!(error, DomainError::DependencyCycle { .. }));
    }

    #[test]
    fn looks_up_steps_by_id() {
        let workflow = workflow().with_step(step("sync"));
        assert!(workflow.step(&StepId::new("sync").unwrap()).is_some());
        assert!(workflow.step(&StepId::new("missing").unwrap()).is_none());
    }
}
