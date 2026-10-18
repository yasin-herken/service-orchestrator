//! End-to-end tests for the bulk import, repository discovery, and saved
//! operation preset use cases (`tasks/032.md` §7–§15, §36–§38).
//!
//! The tests exercise the real use cases against real validated configuration.
//! Two boundaries are replaced:
//!
//! - the **repository inspector** is a fixture returning prepared inspections,
//!   so no real repository is read;
//! - the **repository discovery** port is a recording fixture, so the walker
//!   itself is tested where it lives (the composition root) while the use-case
//!   contract is tested here.
//!
//! The configuration manager is the real `FileConfigManager` over a temporary
//! file, so atomicity, revision conflicts, and reloads are genuinely exercised.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use service_orchestrator_application::{
    Application, ApplicationError, BulkImportOverrides, BulkImportPlan, BulkImportProposal,
    BulkImportRequest, BulkImportRow, BulkImportRowAction, BulkImportSelection,
    DetectedBuildSystem, DetectedIntegration, DetectedRuntime, DiscoveredRepository,
    DiscoveryError, DiscoveryReport, DiscoveryRequest, DiscoveryScanRequest, EnvironmentPhaseKind,
    ExistingEntityPolicy, InMemoryRuntimeStateStore, MavenMetadata, NodeMetadata, NullLogService,
    ProviderAvailability, RepositoryDiscovery, RepositoryInspection, RepositoryInspectionError,
    RepositoryInspector, RuntimeStateStore, TechnologyDetection, TemplateSource,
};
use service_orchestrator_config::{
    load_from_str, ConfigManager, EntityKind, FileConfigManager, TemplateCategory,
    MAX_DISCOVERY_DEPTH,
};
use service_orchestrator_domain::{
    Branch, BuildOperation, MigrationPresetId, MigrationTargetEnvironment, OperationPresetId,
    Permission, Port, ServiceId, Task, TaskId, TemplateId, Workflow, WorkflowId, WorkspaceId,
};
use service_orchestrator_execution::{
    TaskDefinition, TaskManager, TaskManagerError, WorkflowDefinition,
};
use service_orchestrator_policy::Authorization;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A workspace with one configured service, one group, one profile, and two
/// saved operation presets.
const BASE: &str = r#"config_version = 1

[discovery]
max_depth = 3
exclude = ["vendor"]

[[workspaces]]
id = "acme"
name = "Acme"
root = "/projects/acme"

[[services]]
id = "auth-service"
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
preset = "update"

[services.liquibase.database]
environment = "local"

[[services.ports]]
port = 8080

[[services.health_checks]]
type = "http"
url = "http://localhost:8080/actuator/health"

[[groups]]
id = "backend"
name = "Backend"
services = ["auth-service"]

[[profiles]]
id = "bff"
name = "BFF"
groups = ["backend"]

[[operation_presets]]
id = "backend-local"
name = "Backend Local Development"
workspace = "acme"
default = true
groups = ["backend"]

[operation_presets.git]
enabled = true
branch = "develop"

[operation_presets.libraries]
enabled = true

[operation_presets.migration]
preset = "update"

[operation_presets.build]
enabled = true
operation = "test"

[operation_presets.runtime]
action = "restart"

[operation_presets.health]
enabled = true

[[operation_presets]]
id = "sync-all"
name = "Sync All"
workspace = "acme"
services = ["auth-service"]

[operation_presets.git]
enabled = true
"#;

/// The Spring Boot backend template used by most tests.
const SPRING_TEMPLATE: &str = r#"
[[templates]]
id = "spring-boot-backend"
name = "Spring Boot Backend"
category = "spring_boot_backend"
type = "backend"
build_system = "maven"

[templates.commands]
build = "./mvnw clean install"
start = "./mvnw spring-boot:run -Dserver.port=${port}"

[[templates.health_checks]]
type = "http"
url = "http://localhost:${port}/actuator/health"
"#;

/// A second template sharing the Spring Boot category, for ambiguity tests.
const SPRING_TEMPLATE_TWO: &str = r#"
[[templates]]
id = "spring-boot-backend-strict"
name = "Spring Boot Backend (strict)"
category = "spring_boot_backend"
type = "backend"
"#;

/// The shared-library template, for the library import path.
const LIBRARY_TEMPLATE: &str = r#"
[[templates]]
id = "maven-library"
name = "Shared Maven Library"
category = "shared_maven_library"
type = "library"
build_system = "maven"

[templates.commands]
build = "./mvnw clean install"
"#;

/// A React frontend template, with a health check that needs `${port}`.
const REACT_TEMPLATE: &str = r#"
[[templates]]
id = "react-frontend"
name = "React Frontend"
category = "react_frontend"
type = "frontend"
build_system = "npm"

[templates.commands]
build = "npm run build"
start = "npm run start"

[[templates.health_checks]]
type = "http"
url = "http://localhost:${port}"
"#;

/// Creates an isolated temporary directory for one test.
struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(tag: &str) -> Self {
        let mut path = std::env::temp_dir();
        let unique = format!(
            "so-bulk-import-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        path.push(unique);
        std::fs::create_dir_all(&path).expect("the temporary directory is created");
        Self { path }
    }

    fn config_path(&self) -> PathBuf {
        self.path.join("config.toml")
    }

    fn write_config(&self, text: &str) -> PathBuf {
        let path = self.config_path();
        std::fs::write(&path, text).expect("the fixture config is written");
        path
    }

    fn read_config(&self) -> String {
        std::fs::read_to_string(self.config_path()).expect("the config file is readable")
    }

    fn config_revision(&self) -> String {
        service_orchestrator_config::revision_of(&self.read_config())
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A repository inspector over prepared inspections.
#[derive(Default)]
struct FixtureInspector {
    inspections: Mutex<std::collections::BTreeMap<String, RepositoryInspection>>,
}

impl FixtureInspector {
    fn with(self, inspection: RepositoryInspection) -> Self {
        self.inspections
            .lock()
            .unwrap()
            .insert(inspection.path.clone(), inspection);
        self
    }
}

impl RepositoryInspector for FixtureInspector {
    fn inspect(&self, path: &str) -> Result<RepositoryInspection, RepositoryInspectionError> {
        Ok(self
            .inspections
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .unwrap_or_else(|| RepositoryInspection::missing(path)))
    }
}

/// A repository discovery port that records its request and returns a prepared
/// report.
#[derive(Default)]
struct FixtureDiscovery {
    requests: Mutex<Vec<DiscoveryRequest>>,
    report: Mutex<Option<DiscoveryReport>>,
}

impl FixtureDiscovery {
    fn returning(report: DiscoveryReport) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            report: Mutex::new(Some(report)),
        }
    }

    fn requests(&self) -> Vec<DiscoveryRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl RepositoryDiscovery for FixtureDiscovery {
    fn discover(&self, request: &DiscoveryRequest) -> Result<DiscoveryReport, DiscoveryError> {
        self.requests.lock().unwrap().push(request.clone());
        self.report
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| DiscoveryError::new("FIXTURE", "no prepared report"))
    }
}

/// A task engine that records submissions and fabricates identifiers.
#[derive(Default)]
struct FakeTasks {
    submitted: Mutex<Vec<TaskDefinition>>,
    workflows: Mutex<Vec<WorkflowDefinition>>,
    counter: AtomicUsize,
}

impl FakeTasks {
    fn workflows(&self) -> Vec<WorkflowDefinition> {
        self.workflows.lock().unwrap().clone()
    }
}

impl TaskManager for FakeTasks {
    fn submit(&self, definition: TaskDefinition) -> Result<TaskId, TaskManagerError> {
        self.submitted.lock().unwrap().push(definition);
        let index = self.counter.fetch_add(1, Ordering::SeqCst);
        Ok(TaskId::new(format!("task-{index}")).unwrap())
    }

    fn submit_workflow(
        &self,
        definition: WorkflowDefinition,
    ) -> Result<WorkflowId, TaskManagerError> {
        let index = self.counter.fetch_add(1, Ordering::SeqCst);
        let id = WorkflowId::new(format!("workflow-{index}")).unwrap();
        self.workflows.lock().unwrap().push(definition);
        Ok(id)
    }

    fn task(&self, id: &TaskId) -> Result<Task, TaskManagerError> {
        Err(TaskManagerError::NotFound {
            entity: "task",
            id: id.to_string(),
        })
    }

    fn workflow(&self, id: &WorkflowId) -> Result<Workflow, TaskManagerError> {
        Ok(Workflow::new(
            id.clone(),
            service_orchestrator_domain::WorkflowKind::PrepareEnvironment,
        ))
    }

    fn workflow_tasks(&self, _id: &WorkflowId) -> Result<Vec<Task>, TaskManagerError> {
        Ok(Vec::new())
    }

    fn cancel(&self, _id: &TaskId) -> Result<(), TaskManagerError> {
        Ok(())
    }
}

struct Harness {
    app: Application,
    tasks: Arc<FakeTasks>,
    dir: TestDir,
}

impl Harness {
    fn build(tag: &str, text: &str, inspector: FixtureInspector) -> Self {
        Self::build_with_authorization(tag, text, inspector, Authorization::standard())
    }

    fn build_with_authorization(
        tag: &str,
        text: &str,
        inspector: FixtureInspector,
        authorization: Authorization,
    ) -> Self {
        let dir = TestDir::new(tag);
        let path = dir.write_config(text);
        let config = load_from_str(text, "<test>").expect("fixture configuration is valid");
        let manager: Arc<dyn ConfigManager> =
            Arc::new(FileConfigManager::new(path, config.clone()));
        let tasks = Arc::new(FakeTasks::default());
        let app = Application::new(
            config,
            tasks.clone(),
            Arc::new(InMemoryRuntimeStateStore::new()) as Arc<dyn RuntimeStateStore>,
            Arc::new(NullLogService),
        )
        .with_configuration(manager)
        .with_repository_inspector(Arc::new(inspector))
        .with_authorization(authorization);
        Self { app, tasks, dir }
    }
}

// -- inspection builders ------------------------------------------------------

fn maven_build_system() -> DetectedBuildSystem {
    DetectedBuildSystem {
        system: "maven".to_owned(),
        label: "Maven".to_owned(),
        availability: ProviderAvailability::Available,
        evidence: vec!["pom.xml".to_owned()],
        note: None,
    }
}

fn git_repository(path: &str, remote: &str, branch: &str) -> RepositoryInspection {
    let mut inspection = RepositoryInspection::missing(path);
    inspection.exists = true;
    inspection.is_directory = true;
    inspection.is_git_repository = true;
    inspection.branch = Some(branch.to_owned());
    inspection.remote = Some(remote.to_owned());
    inspection
}

fn spring_boot_service(path: &str, remote: &str) -> RepositoryInspection {
    let mut inspection = git_repository(path, remote, "main");
    inspection.build_systems.push(maven_build_system());
    inspection.maven = Some(MavenMetadata {
        group_id: Some("com.acme".to_owned()),
        artifact_id: Some("service".to_owned()),
        version: Some("1.0.0-SNAPSHOT".to_owned()),
        wrapper_available: true,
        spring_boot: true,
        ..MavenMetadata::default()
    });
    inspection.runtimes.push(DetectedRuntime {
        kind: "java".to_owned(),
        label: "Java".to_owned(),
        required_version: Some("21".to_owned()),
        evidence: vec!["pom.xml".to_owned()],
    });
    inspection.integrations.push(DetectedIntegration {
        kind: "spring_boot".to_owned(),
        label: "Spring Boot".to_owned(),
        evidence: vec!["pom.xml".to_owned()],
    });
    inspection
}

fn plain_maven_library(path: &str, remote: &str) -> RepositoryInspection {
    let mut inspection = git_repository(path, remote, "main");
    inspection.build_systems.push(maven_build_system());
    inspection.maven = Some(MavenMetadata {
        group_id: Some("com.acme".to_owned()),
        artifact_id: Some("common-utils".to_owned()),
        version: Some("2.1.0".to_owned()),
        wrapper_available: false,
        spring_boot: false,
        ..MavenMetadata::default()
    });
    inspection
}

fn react_frontend(path: &str, remote: &str) -> RepositoryInspection {
    let mut inspection = git_repository(path, remote, "main");
    inspection.build_systems.push(DetectedBuildSystem {
        system: "npm".to_owned(),
        label: "npm".to_owned(),
        availability: ProviderAvailability::Available,
        evidence: vec!["package.json".to_owned()],
        note: None,
    });
    inspection.node = Some(NodeMetadata {
        name: Some("storefront".to_owned()),
        version: Some("1.0.0".to_owned()),
        required_node_version: Some("20.x".to_owned()),
        package_manager: Some("npm".to_owned()),
        scripts: vec!["build".to_owned(), "start".to_owned()],
        react: true,
    });
    inspection.runtimes.push(DetectedRuntime {
        kind: "node".to_owned(),
        label: "Node".to_owned(),
        required_version: Some("20.x".to_owned()),
        evidence: vec!["package.json".to_owned()],
    });
    inspection.integrations.push(DetectedIntegration {
        kind: "react".to_owned(),
        label: "React".to_owned(),
        evidence: vec!["package.json".to_owned()],
    });
    inspection
}

fn workspace(id: &str) -> WorkspaceId {
    WorkspaceId::new(id).unwrap()
}

fn selection(path: &str) -> BulkImportSelection {
    BulkImportSelection {
        relative_path: path.to_owned(),
        template: None,
        existing: ExistingEntityPolicy::default(),
        overrides: BulkImportOverrides::default(),
    }
}

fn selection_with_template(path: &str, template: &str) -> BulkImportSelection {
    BulkImportSelection {
        template: Some(TemplateId::new(template).unwrap()),
        ..selection(path)
    }
}

fn request(selections: Vec<BulkImportSelection>) -> BulkImportRequest {
    BulkImportRequest {
        workspace_id: workspace("acme"),
        repositories: selections,
        bulk: BulkImportOverrides::default(),
        port_base: None,
    }
}

fn plan_import(app: &Application, request: BulkImportRequest) -> BulkImportPlan {
    app.plan_bulk_import(request).expect("planning succeeds")
}

fn row_for<'a>(plan: &'a BulkImportPlan, path: &str) -> &'a BulkImportRow {
    plan.rows
        .iter()
        .find(|row| row.relative_path == path)
        .expect("the row exists")
}

fn issue_codes(row: &BulkImportRow) -> Vec<&str> {
    row.errors.iter().map(|issue| issue.code.as_str()).collect()
}

// ---------------------------------------------------------------------------
// Discovery (§7–§9)
// ---------------------------------------------------------------------------

#[test]
fn discovery_merges_bounds_and_annotates_suggestions() {
    let spring = spring_boot_service(
        "/projects/acme/services/catalog-service",
        "git@github.com:acme/catalog-service.git",
    );
    let mut report = DiscoveryReport {
        workspace_root: "/projects/acme".to_owned(),
        repositories: Vec::new(),
        truncated: false,
        warnings: Vec::new(),
    };
    report.repositories.push(DiscoveredRepository {
        name: "catalog-service".to_owned(),
        relative_path: "services/catalog-service".to_owned(),
        detections: TechnologyDetection::detect(&spring),
        inspection: spring,
    });
    let unconfigured = spring_boot_service(
        "/projects/acme/services/other-service",
        "git@github.com:acme/other-service.git",
    );
    report.repositories.push(DiscoveredRepository {
        name: "other-service".to_owned(),
        relative_path: "services/other-service".to_owned(),
        detections: TechnologyDetection::detect(&unconfigured),
        inspection: unconfigured,
    });
    let configured = spring_boot_service(
        "/projects/acme/backend/auth-service",
        "git@github.com:acme/auth-service.git",
    );
    report.repositories.push(DiscoveredRepository {
        name: "auth-service".to_owned(),
        relative_path: "backend/auth-service".to_owned(),
        detections: TechnologyDetection::detect(&configured),
        inspection: configured,
    });

    let text = format!("{BASE}{SPRING_TEMPLATE}");
    let dir = TestDir::new("discovery");
    dir.write_config(&text);
    let config = load_from_str(&text, "<test>").unwrap();
    let discovery = Arc::new(FixtureDiscovery::returning(report));
    let app = Application::new(
        config,
        Arc::new(FakeTasks::default()) as Arc<dyn TaskManager>,
        Arc::new(InMemoryRuntimeStateStore::new()) as Arc<dyn RuntimeStateStore>,
        Arc::new(NullLogService),
    )
    .with_repository_discovery(discovery.clone());

    let mut scan = DiscoveryScanRequest::new(workspace("acme"));
    scan.max_depth = Some(1_000);
    scan.exclude = vec!["vendor".to_owned(), "generated".to_owned()];
    let annotated = app.discover_repositories(scan).expect("the scan succeeds");

    // The effective bounds: configured depth 3, requested 1000, hard cap 64.
    let recorded = &discovery.requests()[0];
    assert_eq!(recorded.workspace_root, "/projects/acme");
    assert_eq!(recorded.max_depth, MAX_DISCOVERY_DEPTH);
    assert!(recorded.exclude.contains(&".git".to_owned()));
    assert!(recorded.exclude.contains(&"node_modules".to_owned()));
    assert!(recorded.exclude.contains(&"vendor".to_owned()));
    assert!(recorded.exclude.contains(&"generated".to_owned()));

    assert_eq!(annotated.repositories.len(), 3);
    let catalog = &annotated.repositories[0];
    assert_eq!(
        catalog.suggested_category,
        Some(TemplateCategory::SpringBootBackend)
    );
    assert_eq!(
        catalog.suggested_template,
        Some(TemplateId::new("spring-boot-backend").unwrap())
    );
    assert!(catalog.existing.is_empty());

    let auth = &annotated.repositories[2];
    assert!(auth.is_configured());
    assert_eq!(auth.existing[0].id, "auth-service");
    assert_eq!(auth.existing[0].kind, EntityKind::Service);
    assert_eq!(annotated.workspace_id, workspace("acme"));
}

#[test]
fn discovery_without_a_walker_is_reported_as_unavailable() {
    let harness = Harness::build("discovery-null", BASE, FixtureInspector::default());
    let error = harness
        .app
        .discover_repositories(DiscoveryScanRequest::new(workspace("acme")))
        .expect_err("no walker is wired");
    assert!(matches!(
        error,
        ApplicationError::Discovery { ref code, .. } if code == "DISCOVERY_UNAVAILABLE"
    ));
}

// ---------------------------------------------------------------------------
// Planning (§10–§14)
// ---------------------------------------------------------------------------

#[test]
fn planning_composes_a_service_from_a_template_and_detected_facts() {
    let text = format!("{BASE}{SPRING_TEMPLATE}");
    let inspector = FixtureInspector::default().with(spring_boot_service(
        "/projects/acme/services/catalog-service",
        "git@github.com:acme/catalog-service.git",
    ));
    let harness = Harness::build("plan-compose", &text, inspector);

    let mut import = request(vec![selection_with_template(
        "services/catalog-service",
        "spring-boot-backend",
    )]);
    import.port_base = Some(Port::new(8085).unwrap());
    let plan = plan_import(&harness.app, import);

    assert_eq!(plan.creates, 1);
    assert_eq!(plan.rejects, 0);
    assert!(
        plan.valid,
        "plan issues: {:?}",
        plan.preview.as_ref().map(|preview| &preview.issues)
    );
    let row = row_for(&plan, "services/catalog-service");
    assert_eq!(row.action, BulkImportRowAction::Create);
    assert_eq!(row.template_source, TemplateSource::Explicit);
    assert_eq!(row.entity_id.as_deref(), Some("catalog-service"));
    assert_eq!(row.ports, vec![Port::new(8085).unwrap()]);

    let Some(BulkImportProposal::Service { service }) = &row.proposal else {
        panic!("a service proposal");
    };
    assert_eq!(service.id, "catalog-service");
    assert_eq!(service.service_type, "backend");
    assert_eq!(service.workspace, "acme");
    assert_eq!(service.path.as_deref(), Some("services/catalog-service"));
    assert_eq!(service.default_branch.as_deref(), Some("main"));
    assert_eq!(service.build_system.as_deref(), Some("maven"));
    assert_eq!(
        service
            .repository
            .as_ref()
            .map(|repository| repository.url.as_str()),
        Some("git@github.com:acme/catalog-service.git")
    );
    assert_eq!(
        service
            .ports
            .iter()
            .map(|port| port.port)
            .collect::<Vec<_>>(),
        vec![8085]
    );
    assert_eq!(service.health_checks.len(), 1);
    assert_eq!(
        service.health_checks[0].url.as_deref(),
        Some("http://localhost:8085/actuator/health")
    );
    let start = service.commands.start.as_ref().expect("a start command");
    let service_orchestrator_config::raw::RawCommand::Structured(start) = start else {
        panic!("a structured command");
    };
    assert_eq!(start.program, "./mvnw");
    assert_eq!(
        start.args,
        vec![
            "spring-boot:run".to_owned(),
            "-Dserver.port=8085".to_owned()
        ]
    );
    assert!(service.liquibase.is_none(), "no migration was configured");
    assert_eq!(plan.revision, harness.dir.config_revision());
}

#[test]
fn planning_matches_unique_templates_but_never_silently_chooses() {
    // One template per detected category: each match is a suggestion.
    let text = format!("{BASE}{SPRING_TEMPLATE}{REACT_TEMPLATE}");
    let inspector = FixtureInspector::default()
        .with(spring_boot_service(
            "/projects/acme/services/catalog-service",
            "git@github.com:acme/catalog-service.git",
        ))
        .with(react_frontend(
            "/projects/acme/frontend/storefront",
            "git@github.com:acme/storefront.git",
        ));
    let harness = Harness::build("plan-match", &text, inspector);
    let mut import = request(vec![
        selection("services/catalog-service"),
        selection("frontend/storefront"),
    ]);
    import.port_base = Some(Port::new(8085).unwrap());
    let plan = plan_import(&harness.app, import);
    assert!(
        plan.valid,
        "plan issues: {:?}",
        plan.preview.as_ref().map(|preview| &preview.issues)
    );
    let catalog = row_for(&plan, "services/catalog-service");
    assert_eq!(catalog.template_source, TemplateSource::Matched);
    assert_eq!(catalog.entity_id.as_deref(), Some("catalog-service"));
    let storefront = row_for(&plan, "frontend/storefront");
    assert_eq!(storefront.template_source, TemplateSource::Matched);
    assert_eq!(storefront.entity_id.as_deref(), Some("storefront"));
    assert_eq!(storefront.entity_kind, Some(EntityKind::Service));

    // Two templates share the category: the planner refuses to choose.
    let text = format!("{BASE}{SPRING_TEMPLATE}{SPRING_TEMPLATE_TWO}");
    let inspector = FixtureInspector::default().with(spring_boot_service(
        "/projects/acme/services/catalog-service",
        "git@github.com:acme/catalog-service.git",
    ));
    let harness = Harness::build("plan-ambiguous", &text, inspector);
    let plan = plan_import(
        &harness.app,
        request(vec![selection("services/catalog-service")]),
    );
    let row = row_for(&plan, "services/catalog-service");
    assert_eq!(row.template_source, TemplateSource::None);
    assert!(!row.is_acceptable());
    assert!(issue_codes(row).contains(&"TEMPLATE_AMBIGUOUS"));
    assert!(row
        .errors
        .iter()
        .any(|issue| issue.message.contains("2 templates")));
    assert!(!plan.valid);
}

#[test]
fn planning_imports_libraries_and_requires_a_type_for_plain_maven() {
    let text = format!("{BASE}{LIBRARY_TEMPLATE}");
    let inspector = FixtureInspector::default().with(plain_maven_library(
        "/projects/acme/libraries/common-utils",
        "git@github.com:acme/common-utils.git",
    ));
    let harness = Harness::build("plan-library", &text, inspector);

    // Auto-matching suggests the library category and imports a library.
    let plan = plan_import(
        &harness.app,
        request(vec![selection("libraries/common-utils")]),
    );
    let row = row_for(&plan, "libraries/common-utils");
    assert_eq!(row.template_source, TemplateSource::Matched);
    assert_eq!(row.entity_kind, Some(EntityKind::Library));
    let Some(BulkImportProposal::Library { library }) = &row.proposal else {
        panic!("a library proposal");
    };
    assert_eq!(library.id, "common-utils");
    assert_eq!(library.library_type.as_deref(), Some("maven"));
    assert_eq!(
        library
            .artifact
            .as_ref()
            .map(|artifact| artifact.artifact_id.as_str()),
        Some("common-utils")
    );
    assert!(
        plan.valid,
        "plan issues: {:?}",
        plan.preview.as_ref().map(|preview| &preview.issues)
    );

    // The same repository without the library template cannot be classified.
    let text = format!("{BASE}{SPRING_TEMPLATE}");
    let inspector = FixtureInspector::default().with(plain_maven_library(
        "/projects/acme/libraries/common-utils",
        "git@github.com:acme/common-utils.git",
    ));
    let harness = Harness::build("plan-library-unclassified", &text, inspector);
    let plan = plan_import(
        &harness.app,
        request(vec![selection("libraries/common-utils")]),
    );
    let row = row_for(&plan, "libraries/common-utils");
    assert!(issue_codes(row).contains(&"SERVICE_TYPE_REQUIRED"));
}

#[test]
fn planning_checks_ports_and_reports_conflicts() {
    let text = format!("{BASE}{SPRING_TEMPLATE}");
    let inspector = FixtureInspector::default().with(spring_boot_service(
        "/projects/acme/services/catalog-service",
        "git@github.com:acme/catalog-service.git",
    ));
    let harness = Harness::build("plan-ports", &text, inspector);

    // The Spring Boot template needs `${port}`: without a port base the row is
    // rejected with an actionable error.
    let plan = plan_import(
        &harness.app,
        request(vec![selection_with_template(
            "services/catalog-service",
            "spring-boot-backend",
        )]),
    );
    let row = row_for(&plan, "services/catalog-service");
    assert!(issue_codes(row).contains(&"PORT_REQUIRED"));
    assert!(!row.is_acceptable());

    // An explicit port that an existing service already uses is a conflict.
    let mut conflicting = request(vec![selection_with_template(
        "services/catalog-service",
        "spring-boot-backend",
    )]);
    conflicting.bulk.ports = vec![Port::new(8080).unwrap()];
    let plan = plan_import(&harness.app, conflicting);
    let row = row_for(&plan, "services/catalog-service");
    assert!(issue_codes(row).contains(&"PORT_CONFLICT"));
    assert!(row
        .errors
        .iter()
        .any(|issue| issue.message.contains("auth-service")));
}

#[test]
fn planning_assigns_ports_in_path_order_skipping_configured_ports() {
    let text = format!("{BASE}{SPRING_TEMPLATE}");
    let inspector = FixtureInspector::default()
        .with(spring_boot_service(
            "/projects/acme/services/a-service",
            "git@github.com:acme/a-service.git",
        ))
        .with(spring_boot_service(
            "/projects/acme/services/b-service",
            "git@github.com:acme/b-service.git",
        ));
    let harness = Harness::build("plan-port-order", &text, inspector);
    let mut import = request(vec![
        selection_with_template("services/b-service", "spring-boot-backend"),
        selection_with_template("services/a-service", "spring-boot-backend"),
    ]);
    import.port_base = Some(Port::new(8080).unwrap());
    let plan = plan_import(&harness.app, import);

    assert_eq!(
        row_for(&plan, "services/a-service").ports,
        vec![Port::new(8081).unwrap()],
        "8080 is taken by auth-service"
    );
    assert_eq!(
        row_for(&plan, "services/b-service").ports,
        vec![Port::new(8082).unwrap()]
    );
    assert!(
        plan.valid,
        "plan issues: {:?}",
        plan.preview.as_ref().map(|preview| &preview.issues)
    );
}

#[test]
fn planning_skips_existing_entities_and_queues_updates_for_authorization() {
    let text = format!("{BASE}{SPRING_TEMPLATE}");
    let inspector = FixtureInspector::default().with(spring_boot_service(
        "/projects/acme/backend/auth-service",
        "git@github.com:acme/auth-service.git",
    ));
    let harness = Harness::build("plan-existing", &text, inspector);

    // Default policy: skip.
    let plan = plan_import(
        &harness.app,
        request(vec![selection_with_template(
            "backend/auth-service",
            "spring-boot-backend",
        )]),
    );
    let row = row_for(&plan, "backend/auth-service");
    assert_eq!(
        row.action,
        BulkImportRowAction::Skip {
            existing: "auth-service".to_owned(),
            kind: EntityKind::Service,
        }
    );
    assert_eq!(plan.skips, 1);
    assert!(row.proposal.is_none());

    // Explicit update: the row is planned but the commit needs authorization.
    let mut update = selection_with_template("backend/auth-service", "spring-boot-backend");
    update.existing = ExistingEntityPolicy::Update;
    let mut update_request = request(vec![update]);
    update_request.port_base = Some(Port::new(8081).unwrap());
    let plan = plan_import(&harness.app, update_request.clone());
    let row = row_for(&plan, "backend/auth-service");
    assert_eq!(
        row.action,
        BulkImportRowAction::Update {
            existing: "auth-service".to_owned(),
            kind: EntityKind::Service,
        }
    );
    assert_eq!(row.entity_id.as_deref(), Some("auth-service"));
    assert!(
        plan.valid,
        "plan issues: {:?}",
        plan.preview.as_ref().map(|preview| &preview.issues)
    );

    let error = harness
        .app
        .commit_bulk_import(update_request, &plan.revision)
        .expect_err("an update requires destructive authorization");
    assert!(matches!(
        error,
        ApplicationError::PermissionDenied {
            permission: Permission::Destructive,
            ..
        }
    ));
}

// ---------------------------------------------------------------------------
// Commit (§15)
// ---------------------------------------------------------------------------

#[test]
fn commit_persists_the_whole_batch_atomically_and_reports_it() {
    let text = format!("{BASE}{SPRING_TEMPLATE}{LIBRARY_TEMPLATE}");
    let inspector = FixtureInspector::default()
        .with(spring_boot_service(
            "/projects/acme/services/catalog-service",
            "git@github.com:acme/catalog-service.git",
        ))
        .with(plain_maven_library(
            "/projects/acme/libraries/common-utils",
            "git@github.com:acme/common-utils.git",
        ));
    let harness = Harness::build("commit", &text, inspector);

    let before = harness.dir.config_revision();
    let mut import = request(vec![
        selection_with_template("services/catalog-service", "spring-boot-backend"),
        selection_with_template("libraries/common-utils", "maven-library"),
    ]);
    import.port_base = Some(Port::new(8085).unwrap());
    let plan = plan_import(&harness.app, import.clone());
    assert!(plan.valid);
    assert_eq!(plan.revision, before);

    let commit = harness
        .app
        .commit_bulk_import(import, &plan.revision)
        .expect("the commit succeeds");
    assert_eq!(commit.created_services, vec!["catalog-service".to_owned()]);
    assert_eq!(commit.created_libraries, vec!["common-utils".to_owned()]);
    assert!(commit.updated.is_empty());
    assert_eq!(commit.counts.services, 2, "auth-service plus the new one");
    assert_eq!(commit.counts.libraries, 1);

    let after = harness.dir.read_config();
    assert!(after.contains("id = \"catalog-service\""));
    assert!(after.contains("id = \"common-utils\""));
    assert_ne!(before, harness.dir.config_revision());
    assert_eq!(commit.revision, harness.dir.config_revision());
    // The hand-written service is untouched.
    assert!(after.contains("id = \"auth-service\""));
}

#[test]
fn commit_refuses_a_plan_whose_revision_is_stale() {
    let text = format!("{BASE}{SPRING_TEMPLATE}");
    let inspector = FixtureInspector::default().with(spring_boot_service(
        "/projects/acme/services/catalog-service",
        "git@github.com:acme/catalog-service.git",
    ));
    let harness = Harness::build("commit-stale", &text, inspector);

    let mut import = request(vec![selection_with_template(
        "services/catalog-service",
        "spring-boot-backend",
    )]);
    import.port_base = Some(Port::new(8085).unwrap());
    let plan = plan_import(&harness.app, import.clone());
    assert!(plan.valid);

    // Another writer edits the file between planning and committing.
    let edited = format!("{text}\n# a concurrent edit\n");
    std::fs::write(harness.dir.config_path(), &edited).expect("the concurrent edit lands");

    let error = harness
        .app
        .commit_bulk_import(import, &plan.revision)
        .expect_err("the commit must detect the concurrent edit");
    assert!(
        matches!(error, ApplicationError::Configuration { ref code, .. } if code == "CONFIG_CONFLICT"),
        "unexpected error: {error:?}"
    );
    // Nothing was partially written: the concurrent edit is intact and the
    // import never landed.
    let after = harness.dir.read_config();
    assert!(after.contains("a concurrent edit"));
    assert!(!after.contains("catalog-service"));
}

#[test]
fn commit_rejects_an_invalid_plan_with_an_actionable_summary() {
    let text = format!("{BASE}{SPRING_TEMPLATE}");
    let inspector = FixtureInspector::default().with(spring_boot_service(
        "/projects/acme/services/catalog-service",
        "git@github.com:acme/catalog-service.git",
    ));
    let harness = Harness::build("commit-invalid", &text, inspector);

    // No port base: the row cannot be composed, so the plan is invalid.
    let error = harness
        .app
        .commit_bulk_import(
            request(vec![selection_with_template(
                "services/catalog-service",
                "spring-boot-backend",
            )]),
            &harness.dir.config_revision(),
        )
        .expect_err("an invalid plan cannot be committed");
    match error {
        ApplicationError::ConfigurationInvalid { message } => {
            assert!(message.contains("services/catalog-service"));
            assert!(message.contains("not valid"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
    assert!(!harness.dir.read_config().contains("catalog-service"));
}

// ---------------------------------------------------------------------------
// Saved operation presets and migration presets (§16, §26, §36–§38)
// ---------------------------------------------------------------------------

#[test]
fn presets_are_listed_and_loaded_as_freshly_composed_requests() {
    let text = format!("{BASE}{SPRING_TEMPLATE}");
    let harness = Harness::build("presets", &text, FixtureInspector::default());

    let presets = harness
        .app
        .list_operation_presets(Some(&workspace("acme")))
        .expect("the listing succeeds");
    assert_eq!(presets.len(), 2);
    let backend = presets
        .iter()
        .find(|preset| preset.id.as_str() == "backend-local")
        .expect("the backend preset");
    assert!(backend.default);
    assert_eq!(backend.workspace_id, workspace("acme"));
    // Loading the preset composes the request from its references.
    assert_eq!(
        backend.request.services,
        vec![ServiceId::new("auth-service").unwrap()]
    );
    assert!(backend.request.sync);
    assert_eq!(
        backend.request.branch,
        Some(Branch::new("develop").unwrap())
    );
    assert!(backend.request.build_libraries);
    assert!(backend.request.run_liquibase);
    assert_eq!(
        backend.request.migration_preset,
        Some(MigrationPresetId::new("update").unwrap())
    );
    assert!(backend.request.build);
    assert_eq!(backend.request.build_operation, Some(BuildOperation::Test));
    assert!(backend.request.restart);
    assert!(!backend.request.start && !backend.request.stop);
    assert!(backend.request.wait_for_health);

    // The single-preset query returns the same request.
    let request = harness
        .app
        .operation_preset_request(
            &workspace("acme"),
            &OperationPresetId::new("backend-local").unwrap(),
        )
        .expect("the preset resolves");
    assert_eq!(request, backend.request);

    // Loading never executes anything.
    assert!(harness.tasks.workflows().is_empty());

    // A preset that belongs to the workspace can be planned end to end.
    let plan = harness
        .app
        .plan_prepare_environment(&request)
        .expect("the composed request plans");
    let phases: Vec<EnvironmentPhaseKind> = plan.phases.iter().map(|phase| phase.kind).collect();
    assert!(phases.contains(&EnvironmentPhaseKind::GitSync));
    assert!(phases.contains(&EnvironmentPhaseKind::ServiceBuild));
    assert!(phases.contains(&EnvironmentPhaseKind::ServiceStart));
    assert!(phases.contains(&EnvironmentPhaseKind::HealthCheck));
    assert!(phases.contains(&EnvironmentPhaseKind::Migration));
    // The service is not running, so the stop half of the restart is skipped
    // with a warning rather than fabricated.
    assert!(!phases.contains(&EnvironmentPhaseKind::ServiceStop));
    assert!(plan
        .warnings
        .iter()
        .any(|warning| warning.contains("not running")));
    assert!(!plan.requires_destructive_authorization);

    // Unknown presets and unknown workspaces fail cleanly.
    let error = harness
        .app
        .operation_preset_request(
            &workspace("acme"),
            &OperationPresetId::new("missing").unwrap(),
        )
        .expect_err("the preset does not exist");
    assert!(matches!(error, ApplicationError::InvalidOperation { .. }));
    assert!(harness
        .app
        .list_operation_presets(Some(&workspace("nope")))
        .is_err());
}

#[test]
fn migration_presets_are_listed_with_their_safety_classification() {
    let text = format!("{BASE}{SPRING_TEMPLATE}");
    let harness = Harness::build("migration-presets", &text, FixtureInspector::default());
    let presets = harness
        .app
        .list_migration_presets()
        .expect("the listing succeeds");
    let ids: Vec<&str> = presets.iter().map(|preset| preset.id.as_str()).collect();
    assert_eq!(ids, vec!["status", "update", "reset_local"]);

    let status = &presets[0];
    assert!(status.read_only && !status.destructive && status.builtin);
    let reset = &presets[2];
    assert!(!reset.read_only && reset.destructive);
    assert_eq!(
        reset.allowed_environments,
        vec![MigrationTargetEnvironment::Local]
    );
}

#[test]
fn executing_a_composed_preset_runs_the_ordinary_authorization_path() {
    // A composed preset is only submitted; nothing stores an authorization and
    // nothing is replayed automatically (`tasks/032.md` §37).
    let text = format!("{BASE}{SPRING_TEMPLATE}");
    let harness = Harness::build("preset-execute", &text, FixtureInspector::default());
    let request = harness
        .app
        .operation_preset_request(
            &workspace("acme"),
            &OperationPresetId::new("backend-local").unwrap(),
        )
        .expect("the preset resolves");
    let result = harness
        .app
        .prepare_environment(request)
        .expect("the safe-write workflow is scheduled");
    assert!(result.is_scheduled());
    assert_eq!(harness.tasks.workflows().len(), 1);
}

#[test]
fn unknown_workspaces_and_missing_repositories_fail_cleanly() {
    let harness = Harness::build("errors", BASE, FixtureInspector::default());
    let mut import = request(vec![selection("services/missing-service")]);
    import.workspace_id = workspace("nope");
    assert!(matches!(
        harness.app.plan_bulk_import(import),
        Err(ApplicationError::WorkspaceNotFound(_))
    ));

    let plan = plan_import(
        &harness.app,
        request(vec![selection("services/missing-service")]),
    );
    let row = row_for(&plan, "services/missing-service");
    assert!(issue_codes(row).contains(&"REPOSITORY_MISSING"));
    assert_eq!(row.template_source, TemplateSource::None);
    assert!(!row.is_acceptable());
}
