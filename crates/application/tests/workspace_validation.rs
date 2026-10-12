//! Integration tests for the read-only workspace validation use case.
//!
//! These drive the real [`Application::validate_workspace`] against real
//! validated configuration with a **controllable** [`WorkspaceProbe`], so every
//! branch — present, missing, occupied — is exercised deterministically without
//! touching the machine the tests run on (`tasks/017.md` §7, §8).

use std::collections::BTreeSet;
use std::sync::Arc;

use service_orchestrator_application::{
    Application, InMemoryRuntimeStateStore, NullLogService, ValidationCode, ValidationScope,
    ValidationSeverity, WorkspaceProbe,
};
use service_orchestrator_config::{load_from_str, Config};
use service_orchestrator_domain::{
    Dependency, DependencyKind, DependencyTarget, LibraryId, Permission, ServiceId, WorkspaceId,
};
use service_orchestrator_execution::TaskManager;

/// A fixture workspace with a library, two services, a duplicate-free port
/// layout, and a dependency chain.
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

[services.dependencies]
libraries = ["common-core"]

[[services.ports]]
port = 8081
protocol = "tcp"

[[services.health_checks]]
type = "port"
port = 8081

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

[execution]
max_parallel_tasks = 4
default_timeout_seconds = 1800
"#;

/// A probe whose answers are scripted by the test.
///
/// Anything not explicitly listed is reported as present and unused, so a test
/// only states the deviation it cares about.
#[derive(Default)]
struct ScriptedProbe {
    /// Paths reported as missing.
    missing: BTreeSet<String>,
    /// Paths reported as present but not directories.
    not_directories: BTreeSet<String>,
    /// Directories that lack a `.git` entry.
    without_git: BTreeSet<String>,
    /// Programs reported as absent from `PATH`.
    absent_programs: BTreeSet<String>,
    /// Ports reported as accepting connections.
    occupied_ports: BTreeSet<u16>,
}

impl ScriptedProbe {
    fn missing_paths(paths: &[&str]) -> Self {
        Self {
            missing: paths.iter().map(|p| (*p).to_owned()).collect(),
            ..Self::default()
        }
    }

    fn with_absent(mut self, programs: &[&str]) -> Self {
        self.absent_programs = programs.iter().map(|p| (*p).to_owned()).collect();
        self
    }

    fn with_occupied(mut self, ports: &[u16]) -> Self {
        self.occupied_ports = ports.iter().copied().collect();
        self
    }

    fn without_git(mut self, paths: &[&str]) -> Self {
        self.without_git = paths.iter().map(|p| (*p).to_owned()).collect();
        self
    }
}

impl WorkspaceProbe for ScriptedProbe {
    fn exists(&self, path: &str) -> bool {
        !self.missing.contains(path)
    }

    fn is_dir(&self, path: &str) -> bool {
        self.exists(path) && !self.not_directories.contains(path)
    }

    fn contains_entry(&self, path: &str, entry: &str) -> bool {
        if entry == ".git" {
            return !self.without_git.contains(path);
        }
        true
    }

    fn has_program(&self, program: &str) -> bool {
        !self.absent_programs.contains(program)
    }

    fn is_port_in_use(&self, port: u16) -> bool {
        self.occupied_ports.contains(&port)
    }
}

/// A [`TaskManager`] that rejects everything: validation must never submit.
#[derive(Default)]
struct RejectingTaskManager;

impl TaskManager for RejectingTaskManager {
    fn submit(
        &self,
        _definition: service_orchestrator_execution::TaskDefinition,
    ) -> Result<service_orchestrator_domain::TaskId, service_orchestrator_execution::TaskManagerError>
    {
        panic!("workspace validation must never submit a task")
    }

    fn submit_workflow(
        &self,
        _definition: service_orchestrator_execution::WorkflowDefinition,
    ) -> Result<
        service_orchestrator_domain::WorkflowId,
        service_orchestrator_execution::TaskManagerError,
    > {
        panic!("workspace validation must never submit a workflow")
    }

    fn task(
        &self,
        id: &service_orchestrator_domain::TaskId,
    ) -> Result<service_orchestrator_domain::Task, service_orchestrator_execution::TaskManagerError>
    {
        Err(service_orchestrator_execution::TaskManagerError::NotFound {
            entity: "task",
            id: id.to_string(),
        })
    }

    fn workflow(
        &self,
        id: &service_orchestrator_domain::WorkflowId,
    ) -> Result<
        service_orchestrator_domain::Workflow,
        service_orchestrator_execution::TaskManagerError,
    > {
        Err(service_orchestrator_execution::TaskManagerError::NotFound {
            entity: "workflow",
            id: id.to_string(),
        })
    }

    fn workflow_tasks(
        &self,
        _id: &service_orchestrator_domain::WorkflowId,
    ) -> Result<
        Vec<service_orchestrator_domain::Task>,
        service_orchestrator_execution::TaskManagerError,
    > {
        Ok(Vec::new())
    }

    fn cancel(
        &self,
        _id: &service_orchestrator_domain::TaskId,
    ) -> Result<(), service_orchestrator_execution::TaskManagerError> {
        panic!("workspace validation must never cancel a task")
    }
}

fn config() -> Config {
    load_from_str(CONFIG, "<test>").expect("fixture configuration is valid")
}

/// The loaded workspace root.
///
/// Configuration normalisation expands `~` into an absolute path, so tests
/// must probe the root the validator actually sees rather than the literal
/// `~` written in the fixture.
fn root_of(config: &Config) -> String {
    config
        .workspaces
        .first()
        .map(|workspace| {
            workspace
                .workspace
                .root
                .as_str()
                .trim_end_matches('/')
                .to_owned()
        })
        .expect("the fixture declares a workspace")
}

fn workspace() -> WorkspaceId {
    WorkspaceId::new("acme").unwrap()
}

fn harness(probe: Arc<dyn WorkspaceProbe>) -> Application {
    Application::new(
        config(),
        Arc::new(RejectingTaskManager),
        Arc::new(InMemoryRuntimeStateStore::new()),
        Arc::new(NullLogService),
    )
    .with_workspace_probe(probe)
}

fn path(relative: &str) -> String {
    format!("{}/{relative}", root_of(&config()))
}

// -- a healthy workspace -----------------------------------------------------

#[test]
fn a_healthy_workspace_reports_totals_and_no_errors() {
    let app = harness(Arc::new(ScriptedProbe::default()));
    let report = app.validate_workspace(&workspace()).unwrap();

    assert_eq!(report.workspace_id, workspace());
    assert_eq!(report.name, "Acme Platform");
    assert_eq!(report.totals.services, 2);
    assert_eq!(report.totals.backend_services, 1);
    assert_eq!(report.totals.frontend_services, 1);
    assert_eq!(report.totals.libraries, 1);
    assert_eq!(report.totals.repositories_valid, 3);
    assert_eq!(report.totals.repositories_missing, 0);
    assert_eq!(report.totals.repositories_invalid, 0);
    assert_eq!(
        report.totals.errors, 0,
        "no errors: {:#?}",
        report.diagnostics
    );
    assert_eq!(report.totals.port_conflicts, 0);
    assert!(report.port_conflicts.is_empty());
    assert!(report.occupied_ports.is_empty());
    assert!(!report.has_errors());
    assert_eq!(report.repositories.len(), 3);
}

#[test]
fn a_stopped_service_is_never_reported_as_broken() {
    let app = harness(Arc::new(ScriptedProbe::default()));
    let report = app.validate_workspace(&workspace()).unwrap();

    for diagnostic in &report.diagnostics {
        assert!(
            !diagnostic.message.contains("stopped"),
            "validation must not judge a service's running state: {diagnostic:?}"
        );
    }
    assert_eq!(
        report.totals.errors, 0,
        "an otherwise healthy workspace has no errors: {:#?}",
        report.diagnostics
    );
}

// -- repositories ------------------------------------------------------------

#[test]
fn a_missing_checkout_is_an_error_and_counted() {
    let probe = ScriptedProbe::missing_paths(&[
        &path("backend/auth-service/.git"),
        &path("frontend/web-ui/.git"),
        &path("libraries/common-core/.git"),
    ]);
    // `contains_entry` is what decides `.git`; emulate "checkout missing" by
    // also removing the directories themselves.
    let probe = ScriptedProbe {
        missing: [
            path("backend/auth-service"),
            path("frontend/web-ui"),
            path("libraries/common-core"),
        ]
        .into_iter()
        .collect(),
        ..probe
    };
    let app = harness(Arc::new(probe));
    let report = app.validate_workspace(&workspace()).unwrap();

    assert_eq!(report.totals.repositories_missing, 3);
    assert_eq!(report.totals.repositories_valid, 0);
    assert_eq!(report.totals.repositories_invalid, 0);
    assert!(report.has_errors());
    let missing = report
        .diagnostics
        .iter()
        .filter(|d| d.code == ValidationCode::MissingRepository)
        .count();
    assert_eq!(missing, 3);
    assert!(report.totals.invalid_configurations >= 3);
}

#[test]
fn a_checkout_without_git_metadata_is_invalid_not_missing() {
    let probe = ScriptedProbe::default().without_git(&[&path("backend/auth-service")]);
    let app = harness(Arc::new(probe));
    let report = app.validate_workspace(&workspace()).unwrap();

    assert_eq!(report.totals.repositories_invalid, 1);
    assert_eq!(report.totals.repositories_valid, 2);
    assert!(report
        .diagnostics
        .iter()
        .any(|d| d.code == ValidationCode::MissingGitMetadata));
}

#[test]
fn a_missing_working_directory_is_reported_per_entity() {
    let app = harness(Arc::new(ScriptedProbe::missing_paths(&[&path(
        "frontend/web-ui",
    )])));
    let report = app.validate_workspace(&workspace()).unwrap();

    let diagnostic = report
        .diagnostics
        .iter()
        .find(|d| d.code == ValidationCode::MissingServicePath)
        .expect("the missing service path is reported");
    assert!(matches!(
        &diagnostic.scope,
        ValidationScope::Service(id) if id.as_str() == "web-ui"
    ));
    assert_eq!(diagnostic.severity, ValidationSeverity::Error);
}

// -- runtimes and build tools ------------------------------------------------

#[test]
fn a_missing_runtime_is_a_warning_not_an_error() {
    let app = harness(Arc::new(ScriptedProbe::default().with_absent(&["node"])));
    let report = app.validate_workspace(&workspace()).unwrap();

    let diagnostic = report
        .diagnostics
        .iter()
        .find(|d| d.code == ValidationCode::MissingRuntime)
        .expect("the missing runtime is reported");
    assert_eq!(diagnostic.severity, ValidationSeverity::Warning);
    assert!(diagnostic.message.contains("node"));
    assert_eq!(report.totals.missing_runtimes, 1);
    assert_eq!(
        report.totals.errors, 0,
        "a missing local runtime is not a configuration error: {:#?}",
        report.diagnostics
    );
    assert!(!report.has_errors());
}

#[test]
fn a_workspace_root_that_does_not_exist_is_an_error() {
    let app = harness(Arc::new(ScriptedProbe::missing_paths(&[
        root_of(&config()).as_str(),
    ])));
    let report = app.validate_workspace(&workspace()).unwrap();
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == ValidationCode::MissingWorkspaceRoot),
        "an absent root is reported: {:#?}",
        report.diagnostics
    );
    assert!(report.has_errors());
}

// -- dependencies ------------------------------------------------------------

#[test]
fn an_unconfigured_dependency_is_an_error() {
    // The configuration loader already rejects an unknown dependency, so the
    // validator's own check is a second line of defence behind a boundary that
    // may be bypassed (a future config source, a programmatic edit). Inject the
    // state directly to reach it.
    let mut config = config();
    let workspace_config = config
        .workspaces
        .iter_mut()
        .find(|w| w.workspace.id == workspace())
        .expect("the fixture declares the workspace");
    let auth = workspace_config
        .workspace
        .services
        .iter_mut()
        .find(|service| service.id.as_str() == "auth-service")
        .expect("the fixture declares auth-service");
    auth.dependencies.push(Dependency::new(
        DependencyTarget::Library(LibraryId::new("ghost-library").unwrap()),
        DependencyKind::Runtime,
    ));

    let app = Application::new(
        config,
        Arc::new(RejectingTaskManager),
        Arc::new(InMemoryRuntimeStateStore::new()),
        Arc::new(NullLogService),
    )
    .with_workspace_probe(Arc::new(ScriptedProbe::default()));

    let report = app.validate_workspace(&workspace()).unwrap();
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|d| d.code == ValidationCode::UnknownLibraryDependency)
        .expect("the unresolvable dependency is reported");
    assert_eq!(diagnostic.severity, ValidationSeverity::Error);
    assert!(diagnostic.message.contains("ghost-library"));
    assert_eq!(report.totals.dependency_errors, 1);
    assert!(report.has_errors());
}

#[test]
fn a_dependency_cycle_is_detected() {
    // Configuration loading already rejects cycles, so the validator's own
    // cycle detector is defence in depth behind a boundary that may be
    // bypassed. Build the graph directly to reach it: `web-ui` depends on
    // `auth-service` and `auth-service` depends back on `web-ui`.
    let mut config = config();
    let workspace_config = config
        .workspaces
        .iter_mut()
        .find(|w| w.workspace.id == workspace())
        .expect("the fixture declares the workspace");
    let auth = workspace_config
        .workspace
        .services
        .iter_mut()
        .find(|service| service.id.as_str() == "auth-service")
        .expect("the fixture declares auth-service");
    auth.dependencies.push(Dependency::new(
        DependencyTarget::Service(ServiceId::new("web-ui").unwrap()),
        DependencyKind::Runtime,
    ));

    let app = Application::new(
        config,
        Arc::new(RejectingTaskManager),
        Arc::new(InMemoryRuntimeStateStore::new()),
        Arc::new(NullLogService),
    )
    .with_workspace_probe(Arc::new(ScriptedProbe::default()));

    let report = app.validate_workspace(&workspace()).unwrap();
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|d| d.code == ValidationCode::DependencyCycle)
        .expect("the cycle is reported");
    assert_eq!(diagnostic.severity, ValidationSeverity::Error);
    assert!(diagnostic.message.contains("auth-service"));
    assert!(diagnostic.message.contains("web-ui"));
    assert!(report.has_errors());
}

// -- ports -------------------------------------------------------------------

#[test]
fn port_configuration_conflicts_and_occupancy_are_distinguished() {
    // The loader keeps two services from sharing a health-checked port only by
    // accident here; make the conflict explicit by declaring web-ui's port a
    // second time on auth-service.
    let mut config = config();
    let workspace_config = config
        .workspaces
        .iter_mut()
        .find(|w| w.workspace.id == workspace())
        .expect("the fixture declares the workspace");
    let duplicate = workspace_config
        .workspace
        .services
        .iter()
        .find(|service| service.id.as_str() == "web-ui")
        .and_then(|service| service.ports.first().copied())
        .expect("web-ui declares a port");
    let auth = workspace_config
        .workspace
        .services
        .iter_mut()
        .find(|service| service.id.as_str() == "auth-service")
        .expect("the fixture declares auth-service");
    auth.ports.push(duplicate);

    // Nothing is actually listening: a conflict must be reported even though no
    // port is occupied, and occupancy must not be reported at all.
    let app = Application::new(
        config,
        Arc::new(RejectingTaskManager),
        Arc::new(InMemoryRuntimeStateStore::new()),
        Arc::new(NullLogService),
    )
    .with_workspace_probe(Arc::new(ScriptedProbe::default()));

    let report = app.validate_workspace(&workspace()).unwrap();
    assert_eq!(report.port_conflicts.len(), 1);
    assert_eq!(report.port_conflicts[0].port.value(), duplicate.value());
    let mut services: Vec<&str> = report.port_conflicts[0]
        .services
        .iter()
        .map(|id| id.as_str())
        .collect();
    services.sort_unstable();
    assert_eq!(services, vec!["auth-service", "web-ui"]);
    assert!(report.occupied_ports.is_empty());
    assert!(report
        .diagnostics
        .iter()
        .any(|d| d.code == ValidationCode::DuplicateConfiguredPort));
    assert_eq!(report.totals.port_conflicts, 1);
}

#[test]
fn an_occupied_port_is_a_warning_and_never_a_conflict() {
    let app = harness(Arc::new(ScriptedProbe::default().with_occupied(&[3000])));
    let report = app.validate_workspace(&workspace()).unwrap();

    assert!(report.port_conflicts.is_empty());
    assert_eq!(report.occupied_ports.len(), 1);
    assert_eq!(report.occupied_ports[0].port.value(), 3000);
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|d| d.code == ValidationCode::PortCurrentlyOccupied)
        .expect("the occupied port is reported");
    assert_eq!(diagnostic.severity, ValidationSeverity::Warning);
    assert!(!report.has_errors());
    assert_eq!(
        report.totals.errors, 0,
        "occupancy is not a configuration error: {:#?}",
        report.diagnostics
    );
}

// -- read-only guarantees ----------------------------------------------------

#[test]
fn validation_requires_only_read_permission() {
    let app = harness(Arc::new(ScriptedProbe::default()));
    // The application default authorization is standard (read allowed).
    let report = app.validate_workspace(&workspace());
    assert!(report.is_ok());
}

#[test]
fn validation_of_an_unknown_workspace_fails_without_probing() {
    let app = harness(Arc::new(ScriptedProbe::default()));
    let error = app
        .validate_workspace(&WorkspaceId::new("missing").unwrap())
        .unwrap_err();
    assert!(matches!(
        error,
        service_orchestrator_application::ApplicationError::WorkspaceNotFound(_)
    ));
}

// -- permission vocabulary stays read-only -----------------------------------

#[test]
fn the_permission_required_by_validation_is_read() {
    // Guards the classification itself: validation is a read, so the policy
    // engine can never require confirmation for it.
    assert!(!Permission::Read.allows_mutation());
    assert!(!Permission::Read.requires_confirmation());
}

// -- scale -------------------------------------------------------------------

/// Generates the synthetic workspace TOML text.
fn synthetic_config_text(services: usize, libraries: usize) -> String {
    let mut text = String::from(
        "config_version = 1\n\n[[workspaces]]\nid = \"scale\"\nname = \"Scale\"\nroot = \"/tmp/scale\"\n\n",
    );
    for index in 0..libraries {
        text.push_str(&format!(
            r#"
[[libraries]]
id = "lib-{index}"
name = "lib-{index}"
workspace = "scale"
path = "libraries/lib-{index}"
type = "maven"

[libraries.repository]
url = "git@example.invalid:demo/lib-{index}.git"
default_branch = "main"

[libraries.commands]
build = "./mvnw clean install"

[libraries.artifact]
group_id = "com.demo"
artifact_id = "lib-{index}"
version = "1.0.0-SNAPSHOT"
"#
        ));
    }
    for index in 0..services {
        let dependency = if index == 0 {
            String::new()
        } else if libraries > 0 && index % 2 == 0 {
            format!(
                "\n[services.dependencies]\nlibraries = [\"lib-{}\"]\n",
                index % libraries
            )
        } else {
            format!(
                "\n[services.dependencies]\nservices = [\"svc-{}\"]\n",
                index - 1
            )
        };
        text.push_str(&format!(
            r#"
[[services]]
id = "svc-{index}"
name = "svc-{index}"
type = "backend"
workspace = "scale"
path = "services/svc-{index}"
default_branch = "main"

[services.repository]
url = "git@example.invalid:demo/svc-{index}.git"

[services.runtime]
type = "java"
version = "21"

[services.commands]
build = "./mvnw clean install"
start = "./mvnw spring-boot:run"

[[services.ports]]
port = {}
protocol = "tcp"
{dependency}"#,
            10_000 + index,
        ));
    }
    text.push_str("\n[execution]\nmax_parallel_tasks = 4\ndefault_timeout_seconds = 1800\n");
    text
}

/// Parses the generated text into a validated [`Config`].
fn synthetic_config(services: usize, libraries: usize) -> Config {
    load_from_str(&synthetic_config_text(services, libraries), "<scale>")
        .expect("the synthetic configuration is valid")
}

fn scale_harness(services: usize, libraries: usize) -> Application {
    Application::new(
        synthetic_config(services, libraries),
        Arc::new(RejectingTaskManager),
        Arc::new(InMemoryRuntimeStateStore::new()),
        Arc::new(NullLogService),
    )
    .with_workspace_probe(Arc::new(ScriptedProbe::default()))
}

fn scale_workspace() -> WorkspaceId {
    WorkspaceId::new("scale").unwrap()
}

/// Every configured entity is validated exactly once, with no duplicates.
///
/// `#[ignore]`d because it is a scale/soak check, not a correctness gate; run
/// it explicitly with `--ignored`.
#[test]
#[ignore = "scale check; run with --ignored"]
fn a_large_workspace_validates_every_entity_exactly_once() {
    const SERVICES: usize = 120;
    const LIBRARIES: usize = 30;

    let app = scale_harness(SERVICES, LIBRARIES);
    let report = app.validate_workspace(&scale_workspace()).unwrap();

    assert_eq!(report.totals.services, SERVICES);
    assert_eq!(report.totals.libraries, LIBRARIES);
    assert_eq!(report.repositories.len(), SERVICES + LIBRARIES);
    assert_eq!(
        report.totals.repositories_valid,
        SERVICES + LIBRARIES,
        "every checkout is seen as present"
    );
    assert_eq!(report.totals.errors, 0, "{:#?}", report.diagnostics);

    let mut seen = BTreeSet::new();
    for repository in &report.repositories {
        assert!(
            seen.insert(repository.path.clone()),
            "no path is reported twice"
        );
    }

    let ports: BTreeSet<u16> = report
        .occupied_ports
        .iter()
        .map(|occupied| occupied.port.value())
        .collect();
    assert!(ports.is_empty(), "the scripted probe occupies nothing");
    assert!(report.port_conflicts.is_empty(), "every port is unique");
}

/// Measures validation latency at the documented target scale.
///
/// `#[ignore]`d so the normal suite stays fast; run it with `--ignored` to
/// reproduce the numbers in `docs/performance-report.md`.
#[test]
#[ignore = "measurement; run with --ignored"]
fn validation_latency_at_scale_is_bounded() {
    const SERVICES: usize = 100;
    const LIBRARIES: usize = 25;
    const BUDGET: std::time::Duration = std::time::Duration::from_millis(500);

    let app = scale_harness(SERVICES, LIBRARIES);

    // Warm-up so the measurement excludes first-call allocations.
    app.validate_workspace(&scale_workspace())
        .expect("the warm-up validation succeeds");

    let samples = 10u32;
    let started = std::time::Instant::now();
    for _ in 0..samples {
        app.validate_workspace(&scale_workspace())
            .expect("the validation succeeds");
    }
    let per_call = started.elapsed() / samples;
    eprintln!(
        "validate_workspace: {samples} calls over {SERVICES} services + {LIBRARIES} libraries \
         took {:?} total, {per_call:?} per call",
        started.elapsed()
    );
    assert!(
        per_call < BUDGET,
        "validation of {SERVICES} services took {per_call:?}, over the {BUDGET:?} budget"
    );
}

/// Measures the read-path operations at the documented workspace scales.
///
/// `tasks/017.md` §24 asks for 10/40/100/200-service fixtures. Every workload
/// here is in-process and read-only: no subprocess, socket, or filesystem
/// access. `#[ignore]`d; run with `--ignored --nocapture` to reproduce the
/// numbers in `docs/performance-report.md`.
#[test]
#[ignore = "measurement; run with --ignored"]
fn read_path_latency_across_documented_scales() {
    for services in [10usize, 40, 100, 200] {
        let libraries = services / 4;
        let app = scale_harness(services, libraries);
        let ws = scale_workspace();

        // Warm-up call for each workload.
        let _ = app.list_services(&ws);
        let _ = app.validate_workspace(&ws);

        const SAMPLES: u32 = 20;

        let started = std::time::Instant::now();
        for _ in 0..SAMPLES {
            let listed = app.list_services(&ws).expect("services are listed");
            assert_eq!(listed.len(), services);
        }
        let list = started.elapsed() / SAMPLES;

        let started = std::time::Instant::now();
        for _ in 0..SAMPLES {
            let status = app.get_workspace_status(&ws).expect("status resolves");
            assert_eq!(status.services.len(), services);
        }
        let status = started.elapsed() / SAMPLES;

        let started = std::time::Instant::now();
        for _ in 0..SAMPLES {
            let report = app.validate_workspace(&ws).expect("validation succeeds");
            assert_eq!(report.totals.services, services);
        }
        let validate = started.elapsed() / SAMPLES;

        eprintln!(
            "scale={services:>3} services + {libraries:>2} libraries: \
             list_services={list:?}  get_workspace_status={status:?}  \
             validate_workspace={validate:?}"
        );
    }
}

/// Measures raw configuration loading at each documented scale.
///
/// Parsing and normalisation happen once per process start, so this is the
/// startup-side number. `#[ignore]`d; run with `--ignored --nocapture`.
#[test]
#[ignore = "measurement; run with --ignored"]
fn configuration_loading_latency_across_documented_scales() {
    use std::time::Duration;

    for services in [10usize, 40, 100, 200] {
        let text = synthetic_config_text(services, services / 4);

        // Warm-up.
        load_from_str(&text, "<scale>").expect("the synthetic configuration is valid");

        const SAMPLES: u32 = 10;
        let started = std::time::Instant::now();
        for _ in 0..SAMPLES {
            load_from_str(&text, "<scale>").expect("the synthetic configuration is valid");
        }
        let per_call: Duration = started.elapsed() / SAMPLES;
        eprintln!(
            "config load: {services:>3} services ({B} bytes): {per_call:?} per load",
            B = text.len(),
        );
    }
}
