//! End-to-end tests for the application core.
//!
//! These tests exercise the real use cases against real validated configuration
//! (loaded from an in-memory TOML fixture) and replace only the two outward
//! boundaries the task forbids us to implement here:
//!
//! - the task engine is a [`FakeTaskManager`] that records what was submitted
//!   instead of scheduling it;
//! - the log service is [`NullLogService`].
//!
//! The runtime-state store is the real in-memory implementation, so the
//! tests also verify that commands optimistically transition runtime state and
//! that queries read it back.
//!
//! Nothing here runs Maven, npm, Git, or a process.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use service_orchestrator_application::{
    Application, ApplicationError, HealthState, InMemoryRuntimeStateStore, LogRequest,
    NullLogService, OperationResult, PrepareEnvironmentRequest, RuntimeStateStore,
    ServiceRuntimeState,
};
use service_orchestrator_config::{load_from_str, Config};
use service_orchestrator_domain::{
    Branch, GroupId, LibraryId, Permission, ProcessState, ProfileId, ServiceId, TaskId, TaskKind,
    WorkflowId, WorkflowKind, WorkspaceId,
};
use service_orchestrator_execution::{
    TaskDefinition, TaskManager, TaskManagerError, TaskTarget, WorkflowDefinition,
};
use service_orchestrator_policy::Authorization;

const CONFIG: &str = r#"
config_version = 1

[[workspaces]]
id = "acme"
name = "Acme Platform"
root = "~/Projects/acme"

[[libraries]]
id = "common-core"
name = "common-core"
workspace = "acme"
path = "libraries/common-core"
type = "maven"

[libraries.repository]
url = "git@github.com:acme/common-core.git"
default_branch = "main"

[libraries.commands]
build = "./mvnw clean install"

[libraries.artifact]
group_id = "com.acme"
artifact_id = "common-core"
version = "1.0.0-SNAPSHOT"

[[services]]
id = "auth-service"
name = "auth-service"
type = "backend"
workspace = "acme"
path = "backend/auth-service"
default_branch = "main"

[services.repository]
url = "git@github.com:acme/auth-service.git"

[services.commands]
build = "./mvnw clean install"
start = "./mvnw spring-boot:run"

[services.liquibase]
enabled = true
command = "./mvnw liquibase:update"
timeout_seconds = 300

[services.dependencies]
libraries = ["common-core"]

[[services.ports]]
port = 8081
protocol = "tcp"

[[services.health_checks]]
type = "http"
url = "http://localhost:8081/actuator/health"

[[services]]
id = "web-ui"
name = "web-ui"
type = "frontend"
workspace = "acme"
path = "frontend/web-ui"
default_branch = "main"

[services.repository]
url = "git@github.com:acme/web-ui.git"

[services.runtime]
type = "node"
version = "20.19.4"

[services.commands]
build = "npm run build"
start = "npm start"

[services.dependencies]
services = ["auth-service"]

[[services.ports]]
port = 3000
protocol = "tcp"

[[services.health_checks]]
type = "port"
port = 3000

# A service with no capabilities, no repository, no commands, and no health
# checks. It exists to exercise the "cannot do this" preconditions.
[[services]]
id = "worker-svc"
name = "worker-svc"
type = "other:worker"
workspace = "acme"
path = "backend/worker-svc"

[[groups]]
id = "customer"
name = "Customer Management"
services = ["auth-service", "web-ui"]

[[profiles]]
id = "full-stack"
name = "Full Stack"
services = ["auth-service"]
groups = ["customer"]

[execution]
max_parallel_tasks = 4
default_timeout_seconds = 1800
"#;

/// A [`TaskManager`] that records submissions instead of executing them.
#[derive(Default)]
struct FakeTaskManager {
    submitted: Mutex<Vec<TaskDefinition>>,
    workflows: Mutex<Vec<WorkflowDefinition>>,
    counter: AtomicUsize,
    reject_next: AtomicBool,
}

impl FakeTaskManager {
    fn new() -> Self {
        Self::default()
    }

    fn submitted(&self) -> Vec<TaskDefinition> {
        self.submitted.lock().unwrap().clone()
    }

    fn workflows(&self) -> Vec<WorkflowDefinition> {
        self.workflows.lock().unwrap().clone()
    }

    fn reject_next(&self) {
        self.reject_next.store(true, Ordering::SeqCst);
    }
}

impl TaskManager for FakeTaskManager {
    fn submit(&self, definition: TaskDefinition) -> Result<TaskId, TaskManagerError> {
        if self.reject_next.swap(false, Ordering::SeqCst) {
            return Err(TaskManagerError::Rejected {
                reason: "fake rejection".to_owned(),
            });
        }
        let index = self.counter.fetch_add(1, Ordering::SeqCst);
        self.submitted.lock().unwrap().push(definition);
        Ok(TaskId::new(format!("task-{index}")).unwrap())
    }

    fn submit_workflow(
        &self,
        definition: WorkflowDefinition,
    ) -> Result<WorkflowId, TaskManagerError> {
        let index = self.counter.fetch_add(1, Ordering::SeqCst);
        self.workflows.lock().unwrap().push(definition);
        Ok(WorkflowId::new(format!("workflow-{index}")).unwrap())
    }

    fn task(&self, id: &TaskId) -> Result<service_orchestrator_domain::Task, TaskManagerError> {
        Err(TaskManagerError::NotFound {
            entity: "task",
            id: id.to_string(),
        })
    }

    fn workflow(
        &self,
        id: &WorkflowId,
    ) -> Result<service_orchestrator_domain::Workflow, TaskManagerError> {
        Err(TaskManagerError::NotFound {
            entity: "workflow",
            id: id.to_string(),
        })
    }

    fn cancel(&self, _id: &TaskId) -> Result<(), TaskManagerError> {
        Ok(())
    }
}

struct Harness {
    app: Application,
    tasks: Arc<FakeTaskManager>,
    runtime: Arc<InMemoryRuntimeStateStore>,
}

fn config() -> Config {
    load_from_str(CONFIG, "<test>").expect("fixture configuration is valid")
}

fn harness() -> Harness {
    let tasks = Arc::new(FakeTaskManager::new());
    let runtime = Arc::new(InMemoryRuntimeStateStore::new());
    let app = Application::new(
        config(),
        tasks.clone(),
        runtime.clone(),
        Arc::new(NullLogService),
    );
    Harness {
        app,
        tasks,
        runtime,
    }
}

fn service(id: &str) -> ServiceId {
    ServiceId::new(id).unwrap()
}

fn workspace() -> WorkspaceId {
    WorkspaceId::new("acme").unwrap()
}

// -- queries -----------------------------------------------------------------

#[test]
fn lists_configured_services() {
    let h = harness();
    let services = h.app.list_services(&workspace()).unwrap();
    let ids: Vec<&str> = services.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(services.len(), 3);
    assert!(ids.contains(&"auth-service"));
    assert!(ids.contains(&"web-ui"));
    assert!(ids.contains(&"worker-svc"));
}

#[test]
fn unknown_workspace_is_reported() {
    let h = harness();
    let error = h
        .app
        .list_services(&WorkspaceId::new("missing").unwrap())
        .unwrap_err();
    assert!(matches!(error, ApplicationError::WorkspaceNotFound(_)));
}

#[test]
fn service_status_reflects_runtime_state() {
    let h = harness();
    h.runtime.set_service_state(
        &service("auth-service"),
        ServiceRuntimeState::new()
            .with_process(ProcessState::Running)
            .with_health(HealthState::Healthy)
            .with_branch(Branch::new("feature/x").unwrap()),
    );

    let status = h.app.get_service_status(&service("auth-service")).unwrap();
    assert_eq!(status.workspace_id, workspace());
    assert_eq!(status.process_state, ProcessState::Running);
    assert_eq!(status.health_state, HealthState::Healthy);
    assert_eq!(status.branch.unwrap().as_str(), "feature/x");
    assert_eq!(status.ports.len(), 1);
}

#[test]
fn unknown_service_is_reported() {
    let h = harness();
    let error = h.app.get_service_status(&service("nope")).unwrap_err();
    assert!(matches!(error, ApplicationError::ServiceNotFound(_)));
}

#[test]
fn workspace_status_aggregates_services() {
    let h = harness();
    let status = h.app.get_workspace_status(&workspace()).unwrap();
    assert_eq!(status.name, "Acme Platform");
    assert_eq!(status.services.len(), 3);
}

#[test]
fn lists_and_fetches_libraries_groups_and_profiles() {
    let h = harness();

    assert_eq!(h.app.list_libraries(&workspace()).unwrap().len(), 1);
    assert_eq!(
        h.app
            .get_library(&LibraryId::new("common-core").unwrap())
            .unwrap()
            .id
            .as_str(),
        "common-core"
    );
    assert!(matches!(
        h.app
            .get_library(&LibraryId::new("missing").unwrap())
            .unwrap_err(),
        ApplicationError::LibraryNotFound(_)
    ));

    assert_eq!(h.app.list_groups(&workspace()).unwrap().len(), 1);
    assert_eq!(
        h.app
            .get_group(&GroupId::new("customer").unwrap())
            .unwrap()
            .id
            .as_str(),
        "customer"
    );
    assert!(matches!(
        h.app
            .get_group(&GroupId::new("missing").unwrap())
            .unwrap_err(),
        ApplicationError::GroupNotFound(_)
    ));

    assert_eq!(h.app.list_profiles(&workspace()).unwrap().len(), 1);
    assert!(matches!(
        h.app
            .get_profile(&ProfileId::new("missing").unwrap())
            .unwrap_err(),
        ApplicationError::ProfileNotFound(_)
    ));
}

#[test]
fn unknown_tasks_and_workflows_are_reported() {
    let h = harness();
    assert!(matches!(
        h.app.get_task(&TaskId::new("t1").unwrap()).unwrap_err(),
        ApplicationError::TaskNotFound(_)
    ));
    assert!(matches!(
        h.app
            .get_workflow(&WorkflowId::new("w1").unwrap())
            .unwrap_err(),
        ApplicationError::WorkflowNotFound(_)
    ));
}

#[test]
fn logs_validate_the_service_first() {
    let h = harness();
    assert!(matches!(
        h.app
            .get_logs(&LogRequest::new(service("missing")))
            .unwrap_err(),
        ApplicationError::ServiceNotFound(_)
    ));
    assert!(h
        .app
        .get_logs(&LogRequest::new(service("auth-service")))
        .unwrap()
        .is_empty());
}

// -- command preconditions ---------------------------------------------------

#[test]
fn sync_service_submits_a_git_sync_task() {
    let h = harness();
    let result = h.app.sync_service(&service("auth-service")).unwrap();
    assert!(result.is_scheduled());

    let submitted = h.tasks.submitted();
    assert_eq!(submitted.len(), 1);
    assert_eq!(submitted[0].kind, TaskKind::GitSync);
    assert_eq!(
        submitted[0].target,
        TaskTarget::Service(service("auth-service"))
    );
}

#[test]
fn sync_service_requires_a_repository() {
    let h = harness();
    let error = h.app.sync_service(&service("worker-svc")).unwrap_err();
    assert!(matches!(error, ApplicationError::InvalidOperation { .. }));
    assert!(h.tasks.submitted().is_empty());
}

#[test]
fn build_service_submits_a_build_task_with_a_timeout() {
    let h = harness();
    h.app.build_service(&service("auth-service")).unwrap();

    let submitted = h.tasks.submitted();
    assert_eq!(submitted[0].kind, TaskKind::BuildService);
    assert_eq!(submitted[0].timeout_seconds, Some(1800));
}

#[test]
fn build_service_requires_a_build_command() {
    let h = harness();
    assert!(matches!(
        h.app.build_service(&service("worker-svc")).unwrap_err(),
        ApplicationError::InvalidOperation { .. }
    ));
}

#[test]
fn start_service_marks_state_and_rejects_double_start() {
    let h = harness();
    let id = service("auth-service");

    let result = h.app.start_service(&id).unwrap();
    assert!(result.is_scheduled());
    assert_eq!(h.tasks.submitted()[0].kind, TaskKind::StartService);
    assert_eq!(
        h.runtime.service_state(&id).unwrap().process,
        ProcessState::Starting
    );

    // A second start while transitioning is a precondition failure, not a
    // silent duplicate.
    assert!(matches!(
        h.app.start_service(&id).unwrap_err(),
        ApplicationError::InvalidOperation { .. }
    ));
}

#[test]
fn start_service_is_a_no_op_when_already_running() {
    let h = harness();
    let id = service("auth-service");
    h.runtime.set_service_state(
        &id,
        ServiceRuntimeState::new().with_process(ProcessState::Running),
    );

    let result = h.app.start_service(&id).unwrap();
    assert_eq!(
        result,
        OperationResult::AlreadyInDesiredState {
            kind: TaskKind::StartService
        }
    );
    assert!(h.tasks.submitted().is_empty());
}

#[test]
fn start_service_requires_a_start_command() {
    let h = harness();
    assert!(matches!(
        h.app.start_service(&service("worker-svc")).unwrap_err(),
        ApplicationError::InvalidOperation { .. }
    ));
}

#[test]
fn stop_service_submits_and_marks_state_when_running() {
    let h = harness();
    let id = service("auth-service");
    h.runtime.set_service_state(
        &id,
        ServiceRuntimeState::new().with_process(ProcessState::Running),
    );

    h.app.stop_service(&id).unwrap();
    assert_eq!(h.tasks.submitted()[0].kind, TaskKind::StopService);
    assert_eq!(
        h.runtime.service_state(&id).unwrap().process,
        ProcessState::Stopping
    );
}

#[test]
fn stop_service_is_a_no_op_when_not_running() {
    let h = harness();
    let result = h.app.stop_service(&service("auth-service")).unwrap();
    assert_eq!(
        result,
        OperationResult::AlreadyInDesiredState {
            kind: TaskKind::StopService
        }
    );
    assert!(h.tasks.submitted().is_empty());
}

#[test]
fn restart_service_is_unconditional() {
    let h = harness();
    h.app.restart_service(&service("auth-service")).unwrap();
    assert_eq!(h.tasks.submitted()[0].kind, TaskKind::RestartService);
}

#[test]
fn run_liquibase_uses_the_configured_timeout() {
    let h = harness();
    h.app.run_liquibase(&service("auth-service")).unwrap();
    let submitted = h.tasks.submitted();
    assert_eq!(submitted[0].kind, TaskKind::Liquibase);
    assert_eq!(submitted[0].timeout_seconds, Some(300));
}

#[test]
fn run_liquibase_requires_configuration() {
    let h = harness();
    assert!(matches!(
        h.app.run_liquibase(&service("worker-svc")).unwrap_err(),
        ApplicationError::InvalidOperation { .. }
    ));
}

#[test]
fn health_check_requires_configured_checks() {
    let h = harness();
    h.app.health_check(&service("auth-service")).unwrap();
    assert_eq!(h.tasks.submitted()[0].kind, TaskKind::HealthCheck);

    assert!(matches!(
        h.app.health_check(&service("worker-svc")).unwrap_err(),
        ApplicationError::InvalidOperation { .. }
    ));
}

#[test]
fn build_library_targets_the_library() {
    let h = harness();
    h.app
        .build_library(&LibraryId::new("common-core").unwrap())
        .unwrap();
    let submitted = h.tasks.submitted();
    assert_eq!(submitted[0].kind, TaskKind::BuildLibrary);
    assert_eq!(
        submitted[0].target,
        TaskTarget::Library(LibraryId::new("common-core").unwrap())
    );
}

#[test]
fn task_manager_failures_are_mapped() {
    let h = harness();
    h.tasks.reject_next();
    let error = h.app.sync_service(&service("auth-service")).unwrap_err();
    assert!(matches!(error, ApplicationError::TaskManager { .. }));
}

// -- policy ------------------------------------------------------------------

#[test]
fn authorizes_read_and_safe_write_but_not_destructive_by_default() {
    let h = harness();
    assert!(h.app.authorize(Permission::Read).is_ok());
    assert!(h.app.authorize(Permission::SafeWrite).is_ok());
    assert!(matches!(
        h.app.authorize(Permission::Destructive).unwrap_err(),
        ApplicationError::PermissionDenied {
            permission: Permission::Destructive,
            ..
        }
    ));

    let privileged = h
        .app
        .clone()
        .with_authorization(Authorization::allow_destructive());
    assert!(privileged.authorize(Permission::Destructive).is_ok());
}

// -- workspace, group, profile, and environment workflows --------------------

#[test]
fn sync_workspace_builds_a_git_sync_workflow() {
    let h = harness();
    let result = h.app.sync_workspace(&workspace()).unwrap();
    assert_eq!(
        result,
        OperationResult::WorkflowScheduled {
            workflow_id: WorkflowId::new("workflow-0").unwrap(),
            kind: WorkflowKind::SyncWorkspace,
        }
    );

    let workflows = h.tasks.workflows();
    assert_eq!(workflows.len(), 1);
    // auth-service, web-ui, and common-core (the library) all have repositories.
    assert_eq!(workflows[0].steps.len(), 3);
    assert!(workflows[0]
        .steps
        .iter()
        .all(|s| s.task == TaskKind::GitSync));
}

#[test]
fn build_workspace_orders_libraries_before_services() {
    let h = harness();
    h.app.build_workspace(&workspace()).unwrap();
    let workflows = h.tasks.workflows();
    let steps = &workflows[0].steps;

    let library_step = steps
        .iter()
        .find(|s| s.task == TaskKind::BuildLibrary)
        .expect("library build step");
    let service_steps: Vec<_> = steps
        .iter()
        .filter(|s| s.task == TaskKind::BuildService)
        .collect();
    assert_eq!(service_steps.len(), 2);
    for step in service_steps {
        assert!(step.depends_on.contains(&library_step.id));
    }
}

#[test]
fn prepare_environment_wires_the_full_pipeline() {
    let h = harness();
    let request = PrepareEnvironmentRequest::new(workspace())
        .with_services([service("auth-service")])
        .with_branch(Branch::new("feature/x").unwrap())
        .build()
        .run_liquibase()
        .start();

    let result = h.app.prepare_environment(request).unwrap();
    assert!(matches!(
        result,
        OperationResult::WorkflowScheduled {
            kind: WorkflowKind::PrepareEnvironment,
            ..
        }
    ));

    let workflows = h.tasks.workflows();
    let steps = &workflows[0].steps;
    let kinds: BTreeSet<&TaskKind> = steps.iter().map(|s| &s.task).collect();
    for expected in [
        TaskKind::GitSync,
        TaskKind::GitCheckout,
        TaskKind::BuildLibrary,
        TaskKind::BuildService,
        TaskKind::Liquibase,
        TaskKind::StartService,
        TaskKind::HealthCheck,
    ] {
        assert!(kinds.contains(&expected), "missing {expected}");
    }

    // Every prerequisite names a step that actually exists: the workflow is
    // well formed before it reaches the engine.
    let ids: BTreeSet<_> = steps.iter().map(|s| s.id.clone()).collect();
    for step in steps {
        for dependency in &step.depends_on {
            assert!(ids.contains(dependency), "unknown prerequisite");
        }
    }
}

#[test]
fn prepare_environment_on_a_no_op_selection_reports_desired_state() {
    let h = harness();
    let request =
        PrepareEnvironmentRequest::new(workspace()).with_services([service("worker-svc")]);
    let result = h.app.prepare_environment(request).unwrap();
    assert_eq!(
        result,
        OperationResult::AlreadyInDesiredState {
            kind: TaskKind::StartService
        }
    );
    assert!(h.tasks.workflows().is_empty());
}

#[test]
fn prepare_environment_rejects_unknown_workspace_and_services() {
    let h = harness();
    let unknown_workspace = PrepareEnvironmentRequest::new(WorkspaceId::new("missing").unwrap());
    assert!(matches!(
        h.app.prepare_environment(unknown_workspace).unwrap_err(),
        ApplicationError::WorkspaceNotFound(_)
    ));

    let unknown_service = PrepareEnvironmentRequest::new(workspace())
        .with_services([service("missing")])
        .build();
    assert!(matches!(
        h.app.prepare_environment(unknown_service).unwrap_err(),
        ApplicationError::ServiceNotFound(_)
    ));
}

#[test]
fn start_group_orders_dependencies_and_marks_state() {
    let h = harness();
    let result = h
        .app
        .start_group(&workspace(), &GroupId::new("customer").unwrap())
        .unwrap();
    assert!(matches!(
        result,
        OperationResult::WorkflowScheduled {
            kind: WorkflowKind::Custom(_),
            ..
        }
    ));

    let workflows = h.tasks.workflows();
    let steps = &workflows[0].steps;
    assert_eq!(steps.len(), 2);
    // web-ui depends on auth-service, so its start step depends on auth's.
    let web = steps.iter().find(|s| s.id.as_str() == "s1").unwrap();
    assert_eq!(web.depends_on, vec![steps[0].id.clone()]);

    assert_eq!(
        h.runtime
            .service_state(&service("auth-service"))
            .unwrap()
            .process,
        ProcessState::Starting
    );
    assert_eq!(
        h.runtime.service_state(&service("web-ui")).unwrap().process,
        ProcessState::Starting
    );
}

#[test]
fn stop_group_reverses_dependencies() {
    let h = harness();
    h.runtime.set_service_state(
        &service("auth-service"),
        ServiceRuntimeState::new().with_process(ProcessState::Running),
    );
    h.runtime.set_service_state(
        &service("web-ui"),
        ServiceRuntimeState::new().with_process(ProcessState::Running),
    );

    h.app
        .stop_group(&workspace(), &GroupId::new("customer").unwrap())
        .unwrap();
    let workflows = h.tasks.workflows();
    let steps = &workflows[0].steps;
    // auth-service must stop after web-ui (its dependent).
    let auth = steps.iter().find(|s| s.id.as_str() == "s0").unwrap();
    assert_eq!(auth.depends_on, vec![steps[1].id.clone()]);
}

#[test]
fn group_and_profile_lookups_are_reported() {
    let h = harness();
    assert!(matches!(
        h.app
            .start_group(&workspace(), &GroupId::new("missing").unwrap())
            .unwrap_err(),
        ApplicationError::GroupNotFound(_)
    ));
    assert!(matches!(
        h.app
            .start_profile(&workspace(), &ProfileId::new("missing").unwrap())
            .unwrap_err(),
        ApplicationError::ProfileNotFound(_)
    ));
}

#[test]
fn start_profile_selects_services_and_groups() {
    let h = harness();
    h.app
        .start_profile(&workspace(), &ProfileId::new("full-stack").unwrap())
        .unwrap();
    let workflows = h.tasks.workflows();
    // The profile names auth-service and the customer group (auth + web-ui);
    // duplicates are removed.
    assert_eq!(workflows[0].steps.len(), 2);
    assert_eq!(workflows[0].kind, WorkflowKind::StartProfile);
}

#[test]
fn cancel_task_delegates_to_the_engine() {
    let h = harness();
    assert!(h.app.cancel_task(&TaskId::new("task-0").unwrap()).is_ok());
}
