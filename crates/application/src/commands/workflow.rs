//! Workspace, group, profile, and environment commands.
//!
//! These commands compose other operations into workflows. They never execute
//! steps themselves: they construct a [`WorkflowDefinition`](service_orchestrator_execution::WorkflowDefinition)
//! and submit it to the task engine. Dependency information is preserved as
//! step prerequisites, but full dependency-aware scheduling remains the
//! engine's responsibility.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use service_orchestrator_config::ConfiguredWorkspace;
use service_orchestrator_domain::{
    Branch, BuildOperation, CommandKind, DependencyTarget, GroupId, LibraryId, MigrationOperation,
    Permission, ProcessState, ProfileId, ServiceCapability, ServiceId, StepId, TaskKind,
    TaskStatus, WorkflowId, WorkflowKind, WorkflowState, WorkspaceId,
};

use crate::app::Application;
use crate::commands::WorkflowBuilder;
use crate::error::ApplicationError;
use crate::models::{
    EnvironmentPhaseKind, EnvironmentPlan, EnvironmentPlanPhase, EnvironmentPlanStep,
    OperationResult, PrepareEnvironmentRequest, WorkflowCancellation, WorkflowPhaseProgress,
    WorkflowStatus, WorkflowTaskFailure,
};
use crate::ports::parameter;

/// The services, libraries, and ordering resolved for an environment request.
///
/// This is the single resolution both [`Application::plan_prepare_environment`]
/// and [`Application::prepare_environment`] consume, so a plan can never drift
/// from what execution schedules.
struct ResolvedEnvironment {
    /// The workspace the environment belongs to.
    workspace_id: WorkspaceId,
    /// The services the caller explicitly selected, in resolved order.
    selected_services: Vec<ServiceId>,
    /// Every service that participates (selection plus dependency closure), in
    /// dependency-first order.
    resolved_services: Vec<ServiceId>,
    /// The services added automatically as required dependencies.
    included_dependencies: Vec<ServiceId>,
    /// The required libraries, in dependency-first order.
    libraries: Vec<LibraryId>,
}

/// Accumulates a workflow builder and the equivalent read-only plan together.
///
/// Recording both at the same call site is what guarantees the plan describes
/// exactly the steps that would be scheduled.
struct EnvironmentComposer {
    builder: WorkflowBuilder,
    steps: Vec<EnvironmentPlanStep>,
}

impl EnvironmentComposer {
    fn new() -> Self {
        Self {
            builder: WorkflowBuilder::new(WorkflowKind::PrepareEnvironment),
            steps: Vec::new(),
        }
    }

    /// Adds a step, returning its identifier; also records it in the plan.
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        task: TaskKind,
        depends_on: Vec<StepId>,
        description: Option<String>,
        parameters: BTreeMap<String, String>,
        service: Option<ServiceId>,
        library: Option<LibraryId>,
    ) -> StepId {
        if let Some(phase) = EnvironmentPhaseKind::from_task_kind(&task) {
            self.steps.push(EnvironmentPlanStep {
                phase,
                task: task.clone(),
                service,
                library,
                description: description.clone().unwrap_or_default(),
            });
        }
        self.builder
            .add_with_parameters(task, depends_on, description, parameters)
    }

    /// Adds a prerequisite to a step already recorded in the plan.
    fn depend_on(&mut self, step: &StepId, prerequisite: StepId) {
        self.builder.depend_on(step, prerequisite);
    }

    /// Groups the recorded steps into canonical phase order.
    fn phases(&self) -> Vec<EnvironmentPlanPhase> {
        const ORDER: [EnvironmentPhaseKind; 7] = [
            EnvironmentPhaseKind::Validate,
            EnvironmentPhaseKind::GitSync,
            EnvironmentPhaseKind::LibraryPrepare,
            EnvironmentPhaseKind::Migration,
            EnvironmentPhaseKind::ServiceBuild,
            EnvironmentPhaseKind::ServiceStart,
            EnvironmentPhaseKind::HealthCheck,
        ];
        ORDER
            .into_iter()
            .filter_map(|kind| {
                let steps: Vec<EnvironmentPlanStep> = self
                    .steps
                    .iter()
                    .filter(|step| step.phase == kind)
                    .cloned()
                    .collect();
                if steps.is_empty() {
                    None
                } else {
                    Some(EnvironmentPlanPhase { kind, steps })
                }
            })
            .collect()
    }
}

/// Whether an environment composition is a read-only plan or an execution.
///
/// The mode changes only *policy* handling: planning resolves a database target
/// without authorizing it and reports the requirement, while execution enforces
/// the shared policy for every step (`tasks/016.md` §35).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EnvironmentMode {
    /// Read-only planning: only [`Permission::Read`] is required.
    Plan,
    /// Execution: every step's policy is enforced during resolution.
    Execute,
}

/// A composed environment workflow together with the resolution it came from.
///
/// The same composition produces both the read-only [`EnvironmentPlan`] and the
/// [`WorkflowDefinition`](service_orchestrator_execution::WorkflowDefinition)
/// that is submitted, so a plan can never describe different work than
/// execution schedules (`tasks/016.md` §10, §36).
struct EnvironmentComposition {
    composer: EnvironmentComposer,
    resolved: ResolvedEnvironment,
    target_branch: Option<Branch>,
    requires_destructive_authorization: bool,
    warnings: Vec<String>,
}

impl EnvironmentComposition {
    /// Builds the read-only plan from the composed workflow.
    fn plan(&self) -> EnvironmentPlan {
        EnvironmentPlan {
            workspace_id: self.resolved.workspace_id.clone(),
            target_branch: self.target_branch.clone(),
            selected_services: self.resolved.selected_services.clone(),
            resolved_services: self.resolved.resolved_services.clone(),
            included_dependencies: self.resolved.included_dependencies.clone(),
            required_libraries: self.resolved.libraries.clone(),
            phases: self.composer.phases(),
            step_count: self.composer.steps.len() as u32,
            requires_destructive_authorization: self.requires_destructive_authorization,
            warnings: self.warnings.clone(),
        }
    }
}

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
            if let Some(parameters) = self.service_repository_parameters(workspace, &service.id)? {
                builder.add_with_parameters(
                    TaskKind::GitSync,
                    Vec::new(),
                    Some(format!("sync {}", service.id)),
                    parameters,
                );
            }
        }
        for library in &workspace.workspace.libraries {
            if let Some(parameters) = self.library_repository_parameters(workspace, &library.id)? {
                builder.add_with_parameters(
                    TaskKind::GitSync,
                    Vec::new(),
                    Some(format!("sync {}", library.id)),
                    parameters,
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
                let target =
                    self.library_build_target(workspace, library, BuildOperation::Build)?;
                library_steps.push(builder.add_with_parameters(
                    TaskKind::BuildLibrary,
                    Vec::new(),
                    Some(format!("build {}", library.id)),
                    target.to_parameters(),
                ));
            }
        }
        for service in &workspace.workspace.services {
            if service.supports(ServiceCapability::Build) && service.has_build_command() {
                let target =
                    self.service_build_target(workspace, service, BuildOperation::Build)?;
                builder.add_with_parameters(
                    TaskKind::BuildService,
                    library_steps.clone(),
                    Some(format!("build {}", service.id)),
                    target.to_parameters(),
                );
            }
        }
        self.finish_workflow(builder, TaskKind::BuildService)
    }

    /// Prepares a development environment according to a structured request.
    ///
    /// The request selects the services — explicitly, by profile, by group, or
    /// the whole workspace — resolves their required dependencies into a
    /// dependency-ordered plan, and schedules one workflow whose phases are:
    /// repository synchronization, library preparation, migrations, service
    /// builds, startup, and health checks. The engine executes it; the
    /// application never runs a Git, build, migration, or process command.
    ///
    /// Execution revalidates every precondition and enforces the shared policy:
    /// a plan is not authorization to execute it. A migration against a
    /// non-local database target therefore still requires
    /// [`Permission::Destructive`] here even if
    /// [`Application::plan_prepare_environment`] ran first.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`],
    /// [`ApplicationError::ServiceNotFound`], [`ApplicationError::ProfileNotFound`],
    /// or [`ApplicationError::GroupNotFound`] for unknown entities;
    /// [`ApplicationError::ServiceDependencyCycle`] for a cyclic selection;
    /// [`ApplicationError::InvalidOperation`] when a requested phase is not valid
    /// for a selected service; or [`ApplicationError::PermissionDenied`] when a
    /// required authorization was not granted.
    pub fn prepare_environment(
        &self,
        request: PrepareEnvironmentRequest,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let composition = self.build_environment_workflow(&request, EnvironmentMode::Execute)?;
        self.finish_workflow(composition.composer.builder, TaskKind::StartService)
    }

    /// Plans a development environment preparation without changing anything.
    ///
    /// The plan describes exactly the steps [`Application::prepare_environment`]
    /// would schedule for the same request — both are produced by the same
    /// composer — including resolved dependencies, required libraries, the
    /// per-phase step order, and whether any step requires
    /// [`Permission::Destructive`]. Planning mutates no repository, database, or
    /// process and requires only [`Permission::Read`].
    ///
    /// # Errors
    ///
    /// Returns the same entity and precondition errors as
    /// [`Application::prepare_environment`], but never
    /// [`ApplicationError::PermissionDenied`] for a database-mutating step: it
    /// reports the requirement instead of enforcing it.
    pub fn plan_prepare_environment(
        &self,
        request: &PrepareEnvironmentRequest,
    ) -> Result<EnvironmentPlan, ApplicationError> {
        self.authorize(Permission::Read)?;
        let composition = self.build_environment_workflow(request, EnvironmentMode::Plan)?;
        Ok(composition.plan())
    }

    /// Returns an aggregated, structured status for a workflow.
    ///
    /// Every underlying task is correlated by the shared workflow id, so the TUI
    /// and MCP observe the same authoritative state through the daemon. The
    /// status carries counts, per-phase progress, and structured failures —
    /// never unbounded logs (`tasks/016.md` §28, §29).
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkflowNotFound`] for an unknown workflow.
    pub fn get_workflow_status(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<WorkflowStatus, ApplicationError> {
        self.authorize(Permission::Read)?;
        let workflow = self
            .tasks()
            .workflow(workflow_id)
            .map_err(|_| ApplicationError::WorkflowNotFound(workflow_id.clone()))?;
        let tasks = self
            .tasks()
            .workflow_tasks(workflow_id)
            .map_err(|_| ApplicationError::WorkflowNotFound(workflow_id.clone()))?;

        let mut status = WorkflowStatus {
            workflow_id: workflow_id.clone(),
            kind: workflow.kind,
            state: WorkflowState::Queued,
            total: u32::try_from(tasks.len()).unwrap_or(u32::MAX),
            succeeded: 0,
            failed: 0,
            cancelled: 0,
            skipped: 0,
            pending: 0,
            started_at: None,
            finished_at: None,
            phases: Vec::new(),
            failures: Vec::new(),
        };

        let mut phases: BTreeMap<EnvironmentPhaseKind, WorkflowPhaseProgress> = BTreeMap::new();
        for task in &tasks {
            match task.status {
                TaskStatus::Succeeded => status.succeeded += 1,
                TaskStatus::Failed => {
                    status.failed += 1;
                    let (code, message) = task.error.as_ref().map_or_else(
                        || ("TASK_FAILED".to_owned(), "task failed".to_owned()),
                        |failure| (failure.code.clone(), failure.message.clone()),
                    );
                    status.failures.push(WorkflowTaskFailure {
                        task_id: task.id.clone(),
                        kind: task.kind.clone(),
                        service: task.service.clone(),
                        library: task.library.clone(),
                        code,
                        message,
                    });
                }
                TaskStatus::Cancelled => status.cancelled += 1,
                TaskStatus::Skipped => status.skipped += 1,
                TaskStatus::Queued | TaskStatus::Running | TaskStatus::Blocked => {
                    status.pending += 1
                }
            }

            if let Some(started) = task.started_at {
                status.started_at = Some(match status.started_at {
                    Some(current) if current <= started => current,
                    _ => started,
                });
            }
            if let Some(finished) = task.finished_at {
                status.finished_at = Some(match status.finished_at {
                    Some(current) if current >= finished => current,
                    _ => finished,
                });
            }

            if let Some(phase) = EnvironmentPhaseKind::from_task_kind(&task.kind) {
                let entry = phases.entry(phase).or_insert(WorkflowPhaseProgress {
                    phase,
                    total: 0,
                    succeeded: 0,
                    failed: 0,
                    pending: 0,
                });
                entry.total += 1;
                match task.status {
                    TaskStatus::Succeeded => entry.succeeded += 1,
                    TaskStatus::Failed => entry.failed += 1,
                    TaskStatus::Queued | TaskStatus::Running | TaskStatus::Blocked => {
                        entry.pending += 1;
                    }
                    TaskStatus::Cancelled | TaskStatus::Skipped => {}
                }
            }
        }
        status.phases = phases.into_values().collect();
        let started = tasks.iter().any(|task| task.started_at.is_some());
        status.state = WorkflowState::classify(
            status.pending,
            started,
            status.succeeded,
            status.failed,
            status.cancelled,
        );
        Ok(status)
    }

    /// Requests cancellation of every still-active task in a workflow.
    ///
    /// Cancellation stops scheduling pending tasks, asks running tasks to cancel
    /// cooperatively, and lets completed results stand. It never stops a service
    /// the workflow did not start and performs no destructive cleanup
    /// (`tasks/016.md` §25).
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkflowNotFound`] for an unknown workflow.
    pub fn cancel_workflow(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<WorkflowCancellation, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let tasks = self
            .tasks()
            .workflow_tasks(workflow_id)
            .map_err(|_| ApplicationError::WorkflowNotFound(workflow_id.clone()))?;
        let mut cancelled_tasks = 0;
        let mut already_complete = true;
        for task in &tasks {
            if task.status.is_terminal() {
                continue;
            }
            already_complete = false;
            self.tasks()
                .cancel(&task.id)
                .map_err(|_| ApplicationError::TaskNotFound(task.id.clone()))?;
            cancelled_tasks += 1;
        }
        Ok(WorkflowCancellation {
            cancelled_tasks,
            already_complete,
        })
    }

    // -- environment composition ----------------------------------------------

    /// Resolves and composes an environment workflow and its plan.
    ///
    /// This is the single source of truth for both planning and execution: the
    /// returned composition holds the [`WorkflowBuilder`] that will be submitted
    /// and the plan that describes it. In [`EnvironmentMode::Plan`] no policy is
    /// enforced beyond [`Permission::Read`]; in [`EnvironmentMode::Execute`] each
    /// migration target is authorized as it is resolved.
    fn build_environment_workflow(
        &self,
        request: &PrepareEnvironmentRequest,
        mode: EnvironmentMode,
    ) -> Result<EnvironmentComposition, ApplicationError> {
        let workspace = self.require_workspace(&request.workspace_id)?;
        let resolved = self.environment_resolution(workspace, request)?;
        self.validate_environment(workspace, request, &resolved)?;

        let mut composer = EnvironmentComposer::new();
        let mut warnings: Vec<String> = Vec::new();
        let mut requires_destructive_authorization = false;

        // -- Git synchronization ----------------------------------------------
        let mut service_sync: BTreeMap<ServiceId, StepId> = BTreeMap::new();
        let mut library_sync: BTreeMap<LibraryId, StepId> = BTreeMap::new();
        let mut service_checkout: BTreeMap<ServiceId, StepId> = BTreeMap::new();
        let mut library_checkout: BTreeMap<LibraryId, StepId> = BTreeMap::new();

        if request.sync {
            for id in &resolved.resolved_services {
                if let Some(parameters) = self.service_repository_parameters(workspace, id)? {
                    let step = composer.add(
                        TaskKind::GitSync,
                        Vec::new(),
                        Some(format!("sync {id}")),
                        parameters,
                        Some(id.clone()),
                        None,
                    );
                    service_sync.insert(id.clone(), step);
                }
            }
            for library in &resolved.libraries {
                if let Some(parameters) = self.library_repository_parameters(workspace, library)? {
                    let step = composer.add(
                        TaskKind::GitSync,
                        Vec::new(),
                        Some(format!("sync {library}")),
                        parameters,
                        None,
                        Some(library.clone()),
                    );
                    library_sync.insert(library.clone(), step);
                }
            }
            if let Some(branch) = &request.branch {
                for id in &resolved.resolved_services {
                    let Some(sync) = service_sync.get(id).cloned() else {
                        continue;
                    };
                    let Some(mut parameters) = self.service_repository_parameters(workspace, id)?
                    else {
                        continue;
                    };
                    parameters.insert(parameter::BRANCH.to_owned(), branch.to_string());
                    let step = composer.add(
                        TaskKind::GitCheckout,
                        vec![sync],
                        Some(format!("checkout {branch} for {id}")),
                        parameters,
                        Some(id.clone()),
                        None,
                    );
                    service_checkout.insert(id.clone(), step);
                }
                for library in &resolved.libraries {
                    let Some(sync) = library_sync.get(library).cloned() else {
                        continue;
                    };
                    let Some(mut parameters) =
                        self.library_repository_parameters(workspace, library)?
                    else {
                        continue;
                    };
                    parameters.insert(parameter::BRANCH.to_owned(), branch.to_string());
                    let step = composer.add(
                        TaskKind::GitCheckout,
                        vec![sync],
                        Some(format!("checkout {branch} for {library}")),
                        parameters,
                        None,
                        Some(library.clone()),
                    );
                    library_checkout.insert(library.clone(), step);
                }
            }
        }

        // -- Library preparation ----------------------------------------------
        let mut library_build: BTreeMap<LibraryId, StepId> = BTreeMap::new();
        if request.build_libraries {
            for library_id in &resolved.libraries {
                let library = workspace
                    .workspace
                    .library(library_id)
                    .expect("resolved libraries are configured");
                if !library.commands.contains(&CommandKind::Build) {
                    warnings.push(format!(
                        "library '{library_id}' has no build command and was not built"
                    ));
                    continue;
                }
                let mut prerequisites = Vec::new();
                if let Some(step) = library_checkout
                    .get(library_id)
                    .or_else(|| library_sync.get(library_id))
                {
                    prerequisites.push(step.clone());
                }
                for dependency in &library.dependencies {
                    if let DependencyTarget::Library(target) = &dependency.target {
                        if let Some(step) = library_build.get(target) {
                            prerequisites.push(step.clone());
                        }
                    }
                }
                let target =
                    self.library_build_target(workspace, library, BuildOperation::Build)?;
                let step = composer.add(
                    TaskKind::BuildLibrary,
                    prerequisites,
                    Some(format!("build {library_id}")),
                    target.to_parameters(),
                    None,
                    Some(library_id.clone()),
                );
                library_build.insert(library_id.clone(), step);
            }
        }

        // -- Migrations --------------------------------------------------------
        let mut migrate: BTreeMap<ServiceId, StepId> = BTreeMap::new();
        if request.run_liquibase {
            for id in &resolved.resolved_services {
                let config = self.liquibase_config(workspace, id);
                let enabled = config.is_some_and(|config| config.enabled);
                if !enabled {
                    warnings.push(format!(
                        "migrations are disabled for '{id}' and were skipped"
                    ));
                    continue;
                }
                if config.is_some_and(|config| config.database.environment.requires_authorization())
                {
                    requires_destructive_authorization = true;
                }
                let (target, _timeout) = match mode {
                    EnvironmentMode::Execute => {
                        self.resolve_migration(id, MigrationOperation::Apply)?
                    }
                    EnvironmentMode::Plan => {
                        self.resolve_migration_target(id, MigrationOperation::Apply)?
                    }
                };
                let service = workspace
                    .workspace
                    .service(id)
                    .expect("resolved services are configured");
                let mut prerequisites = Vec::new();
                if let Some(step) = service_checkout.get(id).or_else(|| service_sync.get(id)) {
                    prerequisites.push(step.clone());
                }
                for dependency in &service.dependencies {
                    if let DependencyTarget::Library(target) = &dependency.target {
                        if let Some(step) = library_build.get(target) {
                            prerequisites.push(step.clone());
                        }
                    }
                }
                let step = composer.add(
                    TaskKind::Liquibase,
                    prerequisites,
                    Some(format!("liquibase {id}")),
                    target.to_parameters(),
                    Some(id.clone()),
                    None,
                );
                migrate.insert(id.clone(), step);
            }
        }

        // -- Service builds ----------------------------------------------------
        let mut service_build: BTreeMap<ServiceId, StepId> = BTreeMap::new();
        if request.build {
            for id in &resolved.resolved_services {
                let service = workspace
                    .workspace
                    .service(id)
                    .expect("resolved services are configured");
                if !service.has_build_command() {
                    continue;
                }
                let mut prerequisites = Vec::new();
                if let Some(step) = service_checkout.get(id).or_else(|| service_sync.get(id)) {
                    prerequisites.push(step.clone());
                }
                if let Some(step) = migrate.get(id) {
                    prerequisites.push(step.clone());
                }
                for dependency in &service.dependencies {
                    match &dependency.target {
                        DependencyTarget::Library(target) => {
                            if let Some(step) = library_build.get(target) {
                                prerequisites.push(step.clone());
                            }
                        }
                        DependencyTarget::Service(target) => {
                            if let Some(step) = service_build.get(target) {
                                prerequisites.push(step.clone());
                            }
                        }
                    }
                }
                let target =
                    self.service_build_target(workspace, service, BuildOperation::Build)?;
                let step = composer.add(
                    TaskKind::BuildService,
                    prerequisites,
                    Some(format!("build {id}")),
                    target.to_parameters(),
                    Some(id.clone()),
                    None,
                );
                service_build.insert(id.clone(), step);
            }
        }

        // -- Startup and health ------------------------------------------------
        if request.start {
            let mut start: BTreeMap<ServiceId, StepId> = BTreeMap::new();
            for id in &resolved.resolved_services {
                let service = workspace
                    .workspace
                    .service(id)
                    .expect("resolved services are configured");
                if !(service.supports(ServiceCapability::Run) && service.has_start_command()) {
                    warnings.push(format!("service '{id}' cannot be started and was skipped"));
                    continue;
                }
                let current = self.runtime().service_state(id).unwrap_or_default();
                if current.process.is_running() {
                    warnings.push(format!(
                        "service '{id}' is already running and was not restarted"
                    ));
                    continue;
                }
                if current.process.is_active() {
                    return Err(ApplicationError::invalid_operation(format!(
                        "service '{id}' is currently {} and cannot be started",
                        current.process
                    )));
                }
                let mut prerequisites = Vec::new();
                if let Some(step) = service_build.get(id) {
                    prerequisites.push(step.clone());
                }
                if let Some(step) = migrate.get(id) {
                    prerequisites.push(step.clone());
                }
                let target = self.service_runtime_target(workspace, service)?;
                let step = composer.add(
                    TaskKind::StartService,
                    prerequisites,
                    Some(format!("start {id}")),
                    target.to_parameters(),
                    Some(id.clone()),
                    None,
                );
                start.insert(id.clone(), step);
            }
            // Runtime-dependency ordering between the services being started.
            for id in &resolved.resolved_services {
                let Some(step) = start.get(id).cloned() else {
                    continue;
                };
                let service = workspace
                    .workspace
                    .service(id)
                    .expect("resolved services are configured");
                for dependency in &service.dependencies {
                    if !dependency.required {
                        continue;
                    }
                    if let DependencyTarget::Service(target) = &dependency.target {
                        if let Some(prerequisite) = start.get(target) {
                            composer.depend_on(&step, prerequisite.clone());
                        }
                    }
                }
            }
            if request.wait_for_health {
                for id in &resolved.resolved_services {
                    let Some(step) = start.get(id).cloned() else {
                        continue;
                    };
                    let service = workspace
                        .workspace
                        .service(id)
                        .expect("resolved services are configured");
                    if service.health_checks.is_empty() {
                        continue;
                    }
                    composer.add(
                        TaskKind::HealthCheck,
                        vec![step],
                        Some(format!("health {id}")),
                        BTreeMap::new(),
                        Some(id.clone()),
                        None,
                    );
                }
            }
        }

        Ok(EnvironmentComposition {
            composer,
            resolved,
            target_branch: if request.sync {
                request.branch.clone()
            } else {
                None
            },
            requires_destructive_authorization,
            warnings,
        })
    }

    /// Resolves the services selected by an environment request.
    ///
    /// Selection precedence is explicit services, then profile, then group, then
    /// the whole workspace (`tasks/016.md` §7, §9). A profile or group that
    /// selects nothing yields an empty selection rather than implicitly
    /// selecting the entire workspace.
    fn environment_selection(
        &self,
        workspace: &ConfiguredWorkspace,
        request: &PrepareEnvironmentRequest,
    ) -> Result<Vec<ServiceId>, ApplicationError> {
        if !request.services.is_empty() {
            return self.selected_service_ids(workspace, &request.services);
        }
        if let Some(profile_id) = &request.profile {
            let profile = workspace
                .workspace
                .profile(profile_id)
                .ok_or_else(|| ApplicationError::ProfileNotFound(profile_id.clone()))?;
            let services = self.profile_service_ids(workspace, profile);
            if services.is_empty() {
                return Ok(Vec::new());
            }
            return self.selected_service_ids(workspace, &services);
        }
        if let Some(group_id) = &request.group {
            let group = workspace
                .workspace
                .group(group_id)
                .ok_or_else(|| ApplicationError::GroupNotFound(group_id.clone()))?;
            if group.services.is_empty() {
                return Ok(Vec::new());
            }
            return self.selected_service_ids(workspace, &group.services);
        }
        Ok(workspace
            .workspace
            .services
            .iter()
            .map(|service| service.id.clone())
            .collect())
    }

    /// Resolves the selection into a service closure, library set, and ordering.
    ///
    /// The closure includes the transitive required service dependencies when
    /// [`include_dependencies`](PrepareEnvironmentRequest::include_dependencies)
    /// is set; required libraries are always resolved because every participating
    /// service needs them. Services are ordered dependency-first and libraries
    /// follow [`Application::library_build_order`], so the resolution can never
    /// build or start a dependent before its prerequisite.
    fn environment_resolution(
        &self,
        workspace: &ConfiguredWorkspace,
        request: &PrepareEnvironmentRequest,
    ) -> Result<ResolvedEnvironment, ApplicationError> {
        let selected = self.environment_selection(workspace, request)?;
        if selected.is_empty() {
            return Err(ApplicationError::invalid_operation(
                "no services were selected to prepare",
            ));
        }

        let mut closure: BTreeSet<ServiceId> = BTreeSet::new();
        let mut libraries: BTreeSet<LibraryId> = BTreeSet::new();
        let mut stack: Vec<ServiceId> = selected.clone();
        while let Some(id) = stack.pop() {
            if !closure.insert(id.clone()) {
                continue;
            }
            let service = workspace
                .workspace
                .service(&id)
                .ok_or_else(|| ApplicationError::ServiceNotFound(id.clone()))?;
            for dependency in &service.dependencies {
                match &dependency.target {
                    DependencyTarget::Service(target) => {
                        workspace.workspace.service(target).ok_or_else(|| {
                            ApplicationError::DependencyNotFound {
                                owner: id.to_string(),
                                target: target.to_string(),
                            }
                        })?;
                        if request.include_dependencies && dependency.required {
                            stack.push(target.clone());
                        }
                    }
                    DependencyTarget::Library(target) => {
                        workspace.workspace.library(target).ok_or_else(|| {
                            ApplicationError::DependencyNotFound {
                                owner: id.to_string(),
                                target: target.to_string(),
                            }
                        })?;
                        libraries.insert(target.clone());
                    }
                }
            }
        }

        let resolved_services = self.service_order(workspace, &closure)?;
        let included_dependencies = resolved_services
            .iter()
            .filter(|id| !selected.contains(id))
            .cloned()
            .collect();

        let libraries: Vec<LibraryId> = libraries.into_iter().collect();
        let ordered_libraries = if libraries.is_empty() {
            Vec::new()
        } else {
            self.library_build_order(workspace, &libraries)?
        };

        Ok(ResolvedEnvironment {
            workspace_id: workspace.workspace.id.clone(),
            selected_services: selected,
            resolved_services,
            included_dependencies,
            libraries: ordered_libraries,
        })
    }

    /// Orders a service closure dependency-first.
    ///
    /// A Kahn topological sort over the declared required service dependencies:
    /// edges point from a dependency to its dependents, so every dependency
    /// appears before the service that needs it. A cycle is reported as
    /// [`ApplicationError::ServiceDependencyCycle`] rather than producing an
    /// arbitrary order (`tasks/016.md` §8).
    fn service_order(
        &self,
        workspace: &ConfiguredWorkspace,
        closure: &BTreeSet<ServiceId>,
    ) -> Result<Vec<ServiceId>, ApplicationError> {
        let mut in_degree: BTreeMap<ServiceId, usize> =
            closure.iter().map(|id| (id.clone(), 0usize)).collect();
        let mut dependents: BTreeMap<ServiceId, Vec<ServiceId>> = BTreeMap::new();
        for id in closure {
            let service = workspace
                .workspace
                .service(id)
                .expect("the closure contains only configured services");
            for dependency in &service.dependencies {
                if let DependencyTarget::Service(target) = &dependency.target {
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

        let mut ready: VecDeque<ServiceId> = in_degree
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(id, _)| id.clone())
            .collect();
        let mut order: Vec<ServiceId> = Vec::with_capacity(closure.len());
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
            let ordered: BTreeSet<&ServiceId> = order.iter().collect();
            let cyclic: Vec<String> = closure
                .iter()
                .filter(|id| !ordered.contains(id))
                .map(ToString::to_string)
                .collect();
            return Err(ApplicationError::ServiceDependencyCycle {
                services: cyclic.join(", "),
            });
        }
        Ok(order)
    }

    /// Validates the request against the selected services before scheduling.
    ///
    /// Only explicitly selected services are held to a strict capability
    /// contract; an automatically included dependency participates in the phases
    /// it supports and is reported as skipped otherwise. This keeps a typo in the
    /// selection a hard error while letting the closure stay minimal
    /// (`tasks/016.md` §9).
    fn validate_environment(
        &self,
        workspace: &ConfiguredWorkspace,
        request: &PrepareEnvironmentRequest,
        resolved: &ResolvedEnvironment,
    ) -> Result<(), ApplicationError> {
        for id in &resolved.selected_services {
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
        Ok(())
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

    /// Starts explicitly selected services as one workflow.
    ///
    /// This is the bulk form of [`Application::start_service`]. It reuses the
    /// task engine for concurrency, preserves runtime-dependency ordering where
    /// relationships are explicitly modelled, and returns a single workflow id
    /// whose per-step results are independent (`tasks/010.md` §30).
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] if the workspace does not
    /// exist, [`ApplicationError::ServiceNotFound`] for an unknown member, or
    /// [`ApplicationError::InvalidOperation`] if a member cannot be started.
    pub fn start_services(
        &self,
        workspace_id: &WorkspaceId,
        service_ids: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let workspace = self.require_workspace(workspace_id)?;
        let kind = WorkflowKind::Custom("start_services".to_owned());
        self.start_services_workflow(workspace, service_ids, kind)
    }

    /// Stops explicitly selected services as one workflow.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] or
    /// [`ApplicationError::ServiceNotFound`].
    pub fn stop_services(
        &self,
        workspace_id: &WorkspaceId,
        service_ids: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let workspace = self.require_workspace(workspace_id)?;
        let kind = WorkflowKind::Custom("stop_services".to_owned());
        self.stop_services_workflow(workspace, service_ids, kind)
    }

    /// Restarts explicitly selected services as one workflow.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] or
    /// [`ApplicationError::ServiceNotFound`].
    pub fn restart_services(
        &self,
        workspace_id: &WorkspaceId,
        service_ids: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let workspace = self.require_workspace(workspace_id)?;
        let kind = WorkflowKind::Custom("restart_services".to_owned());
        self.restart_services_workflow(workspace, service_ids, kind)
    }

    /// Builds and submits a restart workflow for the given services.
    ///
    /// Restart steps are independent: each step stops its service and confirms
    /// the exit before starting the replacement, and the task engine bounds
    /// concurrency. A restart must not be reordered against its own start.
    fn restart_services_workflow(
        &self,
        workspace: &ConfiguredWorkspace,
        service_ids: &[ServiceId],
        kind: WorkflowKind,
    ) -> Result<OperationResult, ApplicationError> {
        let mut to_restart: Vec<ServiceId> = Vec::new();
        for id in service_ids {
            let service = workspace
                .workspace
                .service(id)
                .ok_or_else(|| ApplicationError::ServiceNotFound(id.clone()))?;
            self.check_dependencies(workspace, service)?;
            if !(service.supports(ServiceCapability::Run) && service.has_start_command()) {
                return Err(ApplicationError::invalid_operation(format!(
                    "service '{id}' cannot be restarted"
                )));
            }
            to_restart.push(id.clone());
        }

        if to_restart.is_empty() {
            return Ok(OperationResult::AlreadyInDesiredState {
                kind: TaskKind::RestartService,
            });
        }

        let mut builder = WorkflowBuilder::new(kind.clone());
        for id in &to_restart {
            let service = workspace
                .workspace
                .service(id)
                .expect("services were validated above");
            let target = self.service_runtime_target(workspace, service)?;
            builder.add_with_parameters(
                TaskKind::RestartService,
                Vec::new(),
                Some(format!("restart {id}")),
                target.to_parameters(),
            );
        }

        let workflow_id = self
            .tasks()
            .submit_workflow(builder.build())
            .map_err(super::map_submit_error)?;
        for id in &to_restart {
            let current = self.runtime().service_state(id).unwrap_or_default();
            self.runtime()
                .set_service_state(id, current.with_process(ProcessState::Starting));
        }
        Ok(OperationResult::WorkflowScheduled { workflow_id, kind })
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
            let service = workspace
                .workspace
                .service(id)
                .expect("services were validated above");
            let target = self.service_runtime_target(workspace, service)?;
            steps.insert(
                id.clone(),
                builder.add_with_parameters(
                    TaskKind::StartService,
                    Vec::new(),
                    Some(format!("start {id}")),
                    target.to_parameters(),
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
            let service = workspace
                .workspace
                .service(id)
                .expect("services were validated above");
            let target = self.service_runtime_target(workspace, service)?;
            steps.insert(
                id.clone(),
                builder.add_with_parameters(
                    TaskKind::StopService,
                    Vec::new(),
                    Some(format!("stop {id}")),
                    target.to_parameters(),
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
