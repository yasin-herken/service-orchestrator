//! End-to-end tests for the Library Manager use cases.
//!
//! These tests drive the real application core against real validated
//! configuration. Only two boundaries are replaced:
//!
//! - the task engine is a [`FakeTaskManager`] that records submissions instead of
//!   scheduling them (the engine is not under test here);
//! - the artifact inspector is a fake whose presence and version answers the
//!   test controls, so no Maven, no local repository, and no filesystem is
//!   touched.
//!
//! Git is left as [`NullGitManager`] on purpose: a library status query must
//! degrade gracefully when Git is unavailable, and that is asserted directly.
//!
//! The same fixture also exercises library-to-library ordering, dependency-cycle
//! avoidance, and the structured version-alignment outcomes.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use service_orchestrator_application::{
    artifact_parameter, build_parameter, parameter, AlignmentStatus, Application, ApplicationError,
    ArtifactInspectionError, ArtifactInspector, ArtifactPresence, DependencyVersionInspection,
    InMemoryRuntimeStateStore, LibraryPrepareOptions, NullLogService, OperationResult,
    PrepareSelectedLibrariesRequest, ResolvedArtifact, ResolvedDependency,
};
use service_orchestrator_config::{load_from_str, Config};
use service_orchestrator_domain::{
    Branch, LibraryId, Task, TaskId, TaskKind, WorkflowId, WorkspaceId,
};
use service_orchestrator_execution::{
    TaskDefinition, TaskManager, TaskManagerError, TaskTarget, WorkflowDefinition,
};

const CONFIG: &str = r#"
config_version = 1

[[workspaces]]
id = "acme"
name = "Acme Platform"
root = "~/Projects/acme"

# A library that consumes nothing.
[[libraries]]
id = "common-utils"
name = "common-utils"
workspace = "acme"
path = "libraries/common-utils"
type = "maven"

[libraries.artifact]
group_id = "com.acme"
artifact_id = "common-utils"
version = "1.8.0-SNAPSHOT"

[libraries.commands]
build = "./mvnw clean install"

# A library that consumes common-utils, with a repository.
[[libraries]]
id = "common-core"
name = "common-core"
workspace = "acme"
path = "libraries/common-core"
type = "maven"

[libraries.repository]
url = "git@github.com:acme/common-core.git"
default_branch = "main"

[libraries.artifact]
group_id = "com.acme"
artifact_id = "common-core"
version = "1.8.0-SNAPSHOT"

[libraries.commands]
build = "./mvnw clean install"

[libraries.dependencies]
libraries = ["common-utils"]

# A library that consumes common-core.
[[libraries]]
id = "security-lib"
name = "security-lib"
workspace = "acme"
path = "libraries/security-lib"
type = "maven"

[libraries.artifact]
group_id = "com.acme"
artifact_id = "security-lib"
version = "1.8.0-SNAPSHOT"

[libraries.commands]
build = "./mvnw clean install"

[libraries.dependencies]
libraries = ["common-core"]

# A non-Maven library with no artifact coordinate, to exercise the "cannot
# install" precondition and the missing-coordinate path.
[[libraries]]
id = "ui-kit"
name = "ui-kit"
workspace = "acme"
path = "libraries/ui-kit"
type = "npm"

[libraries.commands]
build = "npm run build"

# A library with no recognized build system at all.
[[libraries]]
id = "docs"
name = "docs"
workspace = "acme"
path = "libraries/docs"
type = "other:docs"

[libraries.repository]
url = "git@github.com:acme/docs.git"
default_branch = "main"

[libraries.commands]
build = "/usr/bin/make docs"

# A Maven consumer of common-core.
[[services]]
id = "auth-service"
name = "auth-service"
type = "backend"
workspace = "acme"
path = "backend/auth-service"

[services.commands]
build = "./mvnw clean install"
start = "./mvnw spring-boot:run"

[services.dependencies]
libraries = ["common-core"]

# A non-Maven consumer of common-core.
[[services]]
id = "legacy-svc"
name = "legacy-svc"
type = "other:legacy"
workspace = "acme"
path = "backend/legacy-svc"

[services.dependencies]
libraries = ["common-core"]

[execution]
max_parallel_tasks = 4
default_timeout_seconds = 1800
"#;

/// A [`TaskManager`] that records submissions instead of executing them.
#[derive(Default)]
struct FakeTaskManager {
    submitted: Mutex<Vec<TaskDefinition>>,
    workflows: Mutex<Vec<WorkflowDefinition>>,
    workflow_tasks: Mutex<BTreeMap<WorkflowId, Vec<Task>>>,
    counter: AtomicUsize,
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
}

impl TaskManager for FakeTaskManager {
    fn submit(&self, definition: TaskDefinition) -> Result<TaskId, TaskManagerError> {
        let index = self.counter.fetch_add(1, Ordering::SeqCst);
        self.submitted.lock().unwrap().push(definition);
        Ok(TaskId::new(format!("task-{index}")).unwrap())
    }

    fn submit_workflow(
        &self,
        definition: WorkflowDefinition,
    ) -> Result<WorkflowId, TaskManagerError> {
        let index = self.counter.fetch_add(1, Ordering::SeqCst);
        let workflow_id = WorkflowId::new(format!("workflow-{index}")).unwrap();
        let tasks = definition
            .steps
            .iter()
            .enumerate()
            .map(|(position, step)| {
                let task_id = TaskId::new(format!("task-{index}-{position}")).unwrap();
                Task::new(task_id, step.task.clone())
            })
            .collect();
        self.workflow_tasks
            .lock()
            .unwrap()
            .insert(workflow_id.clone(), tasks);
        self.workflows.lock().unwrap().push(definition);
        Ok(workflow_id)
    }

    fn task(&self, id: &TaskId) -> Result<Task, TaskManagerError> {
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

    fn workflow_tasks(&self, id: &WorkflowId) -> Result<Vec<Task>, TaskManagerError> {
        self.workflow_tasks
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| TaskManagerError::NotFound {
                entity: "workflow",
                id: id.to_string(),
            })
    }

    fn cancel(&self, _id: &TaskId) -> Result<(), TaskManagerError> {
        Ok(())
    }
}

/// An inspector whose answers the test controls.
struct FakeInspector {
    presence: ArtifactPresence,
    version: DependencyVersionInspection,
    locate_error: Option<ArtifactInspectionError>,
    version_error: Option<ArtifactInspectionError>,
    locate_calls: AtomicUsize,
    version_calls: AtomicUsize,
}

impl FakeInspector {
    fn present(version: DependencyVersionInspection) -> Self {
        Self {
            presence: ArtifactPresence::Found {
                path: service_orchestrator_domain::AbsolutePath::new(
                    "/repo/com/acme/common-core/1.8.0-SNAPSHOT/common-core-1.8.0-SNAPSHOT.jar",
                )
                .unwrap(),
                size_bytes: Some(1024),
            },
            version,
            locate_error: None,
            version_error: None,
            locate_calls: AtomicUsize::new(0),
            version_calls: AtomicUsize::new(0),
        }
    }

    fn missing() -> Self {
        Self {
            presence: ArtifactPresence::Missing {
                expected_path: None,
            },
            version: DependencyVersionInspection::NotDeclared,
            locate_error: None,
            version_error: None,
            locate_calls: AtomicUsize::new(0),
            version_calls: AtomicUsize::new(0),
        }
    }

    fn unavailable() -> Self {
        let error = ArtifactInspectionError::Unavailable {
            message: "no inspector".to_owned(),
        };
        Self {
            presence: ArtifactPresence::Missing {
                expected_path: None,
            },
            version: DependencyVersionInspection::NotDeclared,
            locate_error: Some(error.clone()),
            version_error: Some(error),
            locate_calls: AtomicUsize::new(0),
            version_calls: AtomicUsize::new(0),
        }
    }

    fn locate_calls(&self) -> usize {
        self.locate_calls.load(Ordering::SeqCst)
    }

    fn version_calls(&self) -> usize {
        self.version_calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl ArtifactInspector for FakeInspector {
    async fn locate(
        &self,
        _artifact: &ResolvedArtifact,
    ) -> Result<ArtifactPresence, ArtifactInspectionError> {
        self.locate_calls.fetch_add(1, Ordering::SeqCst);
        match &self.locate_error {
            Some(error) => Err(error.clone()),
            None => Ok(self.presence.clone()),
        }
    }

    async fn dependency_version(
        &self,
        _dependency: &ResolvedDependency,
    ) -> Result<DependencyVersionInspection, ArtifactInspectionError> {
        self.version_calls.fetch_add(1, Ordering::SeqCst);
        match &self.version_error {
            Some(error) => Err(error.clone()),
            None => Ok(self.version.clone()),
        }
    }
}

struct Harness {
    app: Application,
    tasks: Arc<FakeTaskManager>,
}

fn config() -> Config {
    load_from_str(CONFIG, "<test>").expect("fixture configuration is valid")
}

fn harness_with(inspector: Arc<dyn ArtifactInspector>) -> Harness {
    let tasks = Arc::new(FakeTaskManager::new());
    let app = Application::new(
        config(),
        tasks.clone(),
        Arc::new(InMemoryRuntimeStateStore::new()),
        Arc::new(NullLogService),
    )
    .with_artifact_inspector(inspector);
    Harness { app, tasks }
}

fn harness() -> Harness {
    harness_with(Arc::new(FakeInspector::present(
        DependencyVersionInspection::Declared {
            version: service_orchestrator_domain::Version::new("1.8.0-SNAPSHOT").unwrap(),
        },
    )))
}

fn library(id: &str) -> LibraryId {
    LibraryId::new(id).unwrap()
}

fn workspace() -> WorkspaceId {
    WorkspaceId::new("acme").unwrap()
}

// -- listing ------------------------------------------------------------------

#[test]
fn lists_the_configured_libraries() {
    let h = harness();
    let ids: Vec<String> = h
        .app
        .list_libraries(&workspace())
        .unwrap()
        .into_iter()
        .map(|library| library.id.to_string())
        .collect();
    assert_eq!(
        ids,
        vec![
            "common-utils",
            "common-core",
            "security-lib",
            "ui-kit",
            "docs"
        ]
    );
}

#[test]
fn get_library_reports_a_missing_library() {
    let h = harness();
    assert!(matches!(
        h.app.get_library(&library("missing")).unwrap_err(),
        ApplicationError::LibraryNotFound(_)
    ));
}

// -- build --------------------------------------------------------------------

#[test]
fn build_library_schedules_a_library_build_with_target_parameters() {
    let h = harness();
    let result = h.app.build_library(&library("common-core")).unwrap();
    let task_id = result.task_id().expect("a task was scheduled");
    assert_eq!(task_id.as_str(), "task-0");

    let submitted = h.tasks.submitted();
    assert_eq!(submitted.len(), 1);
    assert_eq!(submitted[0].kind, TaskKind::BuildLibrary);
    assert_eq!(
        submitted[0].target,
        TaskTarget::Library(library("common-core"))
    );
    assert_eq!(
        submitted[0]
            .parameters
            .get(build_parameter::OPERATION)
            .map(String::as_str),
        Some("build")
    );
    assert!(submitted[0]
        .parameters
        .get(build_parameter::PATH)
        .is_some_and(|path| path.ends_with("/libraries/common-core")));
}

#[test]
fn install_library_schedules_a_install_task_with_the_artifact() {
    let h = harness();
    let result = h.app.install_library(&library("common-core")).unwrap();
    assert!(result.is_scheduled());

    let submitted = h.tasks.submitted();
    assert_eq!(submitted[0].kind, TaskKind::LibraryInstall);
    assert_eq!(
        submitted[0]
            .parameters
            .get(build_parameter::OPERATION)
            .map(String::as_str),
        Some("publish_local_artifact")
    );
    assert_eq!(
        submitted[0]
            .parameters
            .get("artifact.version")
            .map(String::as_str),
        Some("1.8.0-SNAPSHOT")
    );
    assert_eq!(
        submitted[0]
            .parameters
            .get(artifact_parameter::ARTIFACT)
            .map(String::as_str),
        Some("common-core")
    );
}

#[test]
fn install_library_rejects_a_library_with_no_build_system() {
    let h = harness();
    let error = h.app.install_library(&library("docs")).unwrap_err();
    assert!(matches!(
        error,
        ApplicationError::BuildProviderUnsupported { .. }
    ));
}

#[test]
fn install_library_rejects_a_library_with_no_artifact() {
    let h = harness();
    let error = h.app.install_library(&library("ui-kit")).unwrap_err();
    match error {
        ApplicationError::LibraryConfigurationInvalid { library, .. } => {
            assert_eq!(library, "ui-kit");
        }
        other => panic!("expected a configuration error, got {other:?}"),
    }
}

// -- prepare ------------------------------------------------------------------

#[test]
fn prepare_library_builds_a_full_workflow_in_order() {
    let h = harness();
    let options = LibraryPrepareOptions::new();
    let result = h
        .app
        .prepare_library(&library("common-core"), &options)
        .unwrap();
    assert!(matches!(
        result,
        OperationResult::WorkflowScheduled {
            kind: service_orchestrator_domain::WorkflowKind::PrepareLibrary,
            ..
        }
    ));

    let workflows = h.tasks.workflows();
    let steps = &workflows[0].steps;
    // The transitive dependency `common-utils` is prepared first; it has no
    // repository, so it contributes no Git step. `common-core` then syncs, builds,
    // installs, and verifies.
    let described: Vec<&str> = steps
        .iter()
        .map(|step| step.description.as_deref().unwrap_or(""))
        .collect();
    assert_eq!(
        described,
        vec![
            "build common-utils",
            "install common-utils",
            "verify common-utils",
            "sync common-core",
            "build common-core",
            "install common-core",
            "verify common-core",
        ]
    );
    // Each library's steps form a chain.
    let id_of = |description: &str| {
        steps
            .iter()
            .find(|step| step.description.as_deref() == Some(description))
            .map(|step| step.id.clone())
            .unwrap()
    };
    assert!(steps[1].depends_on.contains(&id_of("build common-utils")));
    assert!(steps[2].depends_on.contains(&id_of("install common-utils")));
    assert!(steps[3].depends_on.contains(&id_of("verify common-utils")));
    assert!(steps[4].depends_on.contains(&id_of("sync common-core")));
    assert!(steps[5].depends_on.contains(&id_of("build common-core")));
    assert!(steps[6].depends_on.contains(&id_of("install common-core")));
}

#[test]
fn prepare_library_can_skip_synchronization() {
    let h = harness();
    let options = LibraryPrepareOptions::new().with_sync(false);
    h.app
        .prepare_library(&library("common-core"), &options)
        .unwrap();
    let workflows = h.tasks.workflows();
    assert!(workflows[0]
        .steps
        .iter()
        .all(|step| step.task != TaskKind::GitSync));
}

#[test]
fn prepare_library_can_checkout_a_requested_branch() {
    let h = harness();
    let options = LibraryPrepareOptions::new().with_branch(Branch::new("feature/x").unwrap());
    h.app
        .prepare_library(&library("common-core"), &options)
        .unwrap();
    let workflows = h.tasks.workflows();
    let checkout = workflows[0]
        .steps
        .iter()
        .find(|step| step.task == TaskKind::GitCheckout)
        .expect("a checkout step was added");
    assert_eq!(
        checkout
            .parameters
            .get(parameter::BRANCH)
            .map(String::as_str),
        Some("feature/x")
    );
}

#[test]
fn prepare_selected_libraries_orders_dependencies_first() {
    let h = harness();
    let request =
        PrepareSelectedLibrariesRequest::new(workspace()).with_libraries([library("security-lib")]);
    h.app.prepare_selected_libraries(request).unwrap();

    let workflows = h.tasks.workflows();
    let builds: Vec<&str> = workflows[0]
        .steps
        .iter()
        .filter(|step| step.task == TaskKind::BuildLibrary)
        .map(|step| step.description.as_deref().unwrap_or(""))
        .collect();
    // security-lib depends on common-core, which depends on common-utils, so the
    // transitive closure is ordered dependency-first.
    assert_eq!(
        builds,
        vec![
            "build common-utils",
            "build common-core",
            "build security-lib"
        ]
    );

    // The first step of a dependent library waits for its dependency's last step.
    let step_id = |description: &str| {
        workflows[0]
            .steps
            .iter()
            .find(|step| step.description.as_deref() == Some(description))
            .map(|step| step.id.clone())
            .unwrap()
    };
    let utils_verify = step_id("verify common-utils");
    let core_sync = workflows[0]
        .steps
        .iter()
        .find(|step| step.description.as_deref() == Some("sync common-core"))
        .expect("common-core has a repository");
    assert!(core_sync.depends_on.contains(&utils_verify));
}

#[test]
fn prepare_selected_libraries_defaults_to_every_library() {
    let h = harness();
    // Synchronization only, so the request stays valid even for a library that
    // cannot be built (docs) or installed (ui-kit). Every library with a
    // repository contributes one sync step.
    let request = PrepareSelectedLibrariesRequest::new(workspace()).with_options(
        LibraryPrepareOptions::new()
            .with_sync(true)
            .with_build(false)
            .with_install(false)
            .with_verify(false),
    );
    h.app.prepare_selected_libraries(request).unwrap();
    let workflows = h.tasks.workflows();
    let synced: Vec<&str> = workflows[0]
        .steps
        .iter()
        .filter(|step| step.task == TaskKind::GitSync)
        .map(|step| step.description.as_deref().unwrap_or(""))
        .collect();
    assert_eq!(synced.len(), 2);
    assert!(synced.contains(&"sync common-core"));
    assert!(synced.contains(&"sync docs"));
}

#[test]
fn prepare_library_rejects_an_unknown_library() {
    let h = harness();
    let error = h
        .app
        .prepare_library(&library("missing"), &LibraryPrepareOptions::new())
        .unwrap_err();
    assert!(matches!(error, ApplicationError::LibraryNotFound(_)));
}

// -- status, dependents, and alignment ---------------------------------------

#[tokio::test]
async fn get_library_status_degrades_gracefully_without_git() {
    let h = harness();
    let status = h
        .app
        .get_library_status(&library("common-core"))
        .await
        .unwrap();
    assert_eq!(status.workspace_id, workspace());
    assert_eq!(
        status.artifact.as_ref().map(ToString::to_string),
        Some("com.acme:common-core:1.8.0-SNAPSHOT".to_owned())
    );
    assert_eq!(status.artifacts.len(), 1);
    assert!(status.repository_error.is_some());
    assert_eq!(
        status.git_state,
        service_orchestrator_domain::GitState::Unknown
    );
    assert_eq!(status.artifact_present, None);
}

#[test]
fn find_library_dependents_lists_consuming_services() {
    let h = harness();
    let dependents = h
        .app
        .find_library_dependents(&library("common-core"))
        .unwrap();
    let ids: Vec<String> = dependents
        .iter()
        .map(|dependent| dependent.service_id.to_string())
        .collect();
    assert_eq!(ids, vec!["auth-service", "legacy-svc"]);
}

#[tokio::test]
async fn verify_library_artifact_reports_presence() {
    let inspector = Arc::new(FakeInspector::present(
        DependencyVersionInspection::NotDeclared,
    ));
    let h = harness_with(inspector.clone());
    let verification = h
        .app
        .verify_library_artifact(&library("common-core"))
        .await
        .unwrap();
    assert!(verification.present);
    assert!(verification.path.is_some());
    assert_eq!(inspector.locate_calls(), 1);
}

#[tokio::test]
async fn verify_library_artifact_reports_a_missing_artifact() {
    let h = harness_with(Arc::new(FakeInspector::missing()));
    let verification = h
        .app
        .verify_library_artifact(&library("common-core"))
        .await
        .unwrap();
    assert!(!verification.present);
    assert!(verification.path.is_none());
}

#[tokio::test]
async fn verify_library_artifact_requires_an_inspector() {
    // The default application has no inspector wired.
    let h = harness_with(Arc::new(
        service_orchestrator_application::NullArtifactInspector,
    ));
    let error = h
        .app
        .verify_library_artifact(&library("common-core"))
        .await
        .unwrap_err();
    match error {
        ApplicationError::ArtifactInspection { code, .. } => {
            assert_eq!(code, "ARTIFACT_INSPECTION_UNAVAILABLE");
        }
        other => panic!("expected an inspection error, got {other:?}"),
    }
}

#[tokio::test]
async fn version_alignment_reports_aligned_and_unknown_consumers() {
    let inspector = Arc::new(FakeInspector::present(
        DependencyVersionInspection::Declared {
            version: service_orchestrator_domain::Version::new("1.8.0-SNAPSHOT").unwrap(),
        },
    ));
    let h = harness_with(inspector.clone());
    let plan = h
        .app
        .inspect_library_version_alignment(&library("common-core"))
        .await
        .unwrap();
    assert_eq!(
        plan.target.to_string(),
        "com.acme:common-core:1.8.0-SNAPSHOT"
    );
    assert_eq!(plan.consumers.len(), 2);

    let auth = plan
        .consumers
        .iter()
        .find(|consumer| consumer.service_id.as_str() == "auth-service")
        .unwrap();
    assert_eq!(auth.status, AlignmentStatus::Aligned);
    assert_eq!(
        auth.current_version.as_ref().map(ToString::to_string),
        Some("1.8.0-SNAPSHOT".to_owned())
    );

    // legacy-svc is not a Maven module, so it is reported Unknown without an
    // inspection attempt.
    let legacy = plan
        .consumers
        .iter()
        .find(|consumer| consumer.service_id.as_str() == "legacy-svc")
        .unwrap();
    assert_eq!(legacy.status, AlignmentStatus::Unknown);

    // Only the Maven consumer was inspected.
    assert_eq!(inspector.version_calls(), 1);
    assert!(!plan.is_fully_aligned());
    assert_eq!(plan.aligned_count(), 1);
}

#[tokio::test]
async fn version_alignment_reports_a_mismatch() {
    let inspector = Arc::new(FakeInspector::present(
        DependencyVersionInspection::Declared {
            version: service_orchestrator_domain::Version::new("1.7.0-SNAPSHOT").unwrap(),
        },
    ));
    let h = harness_with(inspector);
    let plan = h
        .app
        .inspect_library_version_alignment(&library("common-core"))
        .await
        .unwrap();
    let auth = plan
        .consumers
        .iter()
        .find(|consumer| consumer.service_id.as_str() == "auth-service")
        .unwrap();
    assert_eq!(auth.status, AlignmentStatus::Mismatch);
}

#[tokio::test]
async fn version_alignment_reports_an_unresolvable_version() {
    let inspector = Arc::new(FakeInspector::present(
        DependencyVersionInspection::Unknown {
            reason: "inherited from a parent POM".to_owned(),
        },
    ));
    let h = harness_with(inspector);
    let plan = h
        .app
        .inspect_library_version_alignment(&library("common-core"))
        .await
        .unwrap();
    let auth = plan
        .consumers
        .iter()
        .find(|consumer| consumer.service_id.as_str() == "auth-service")
        .unwrap();
    assert_eq!(auth.status, AlignmentStatus::Unknown);
    assert!(auth.current_version.is_none());
}

#[tokio::test]
async fn version_alignment_reports_an_undeclared_dependency() {
    let inspector = Arc::new(FakeInspector::present(
        DependencyVersionInspection::NotDeclared,
    ));
    let h = harness_with(inspector);
    let plan = h
        .app
        .inspect_library_version_alignment(&library("common-core"))
        .await
        .unwrap();
    let auth = plan
        .consumers
        .iter()
        .find(|consumer| consumer.service_id.as_str() == "auth-service")
        .unwrap();
    assert_eq!(auth.status, AlignmentStatus::Unresolved);
}

#[tokio::test]
async fn version_alignment_fails_when_inspection_is_unavailable() {
    let h = harness_with(Arc::new(FakeInspector::unavailable()));
    let error = h
        .app
        .inspect_library_version_alignment(&library("common-core"))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ApplicationError::VersionInspectionUnsupported { .. }
    ));
}
