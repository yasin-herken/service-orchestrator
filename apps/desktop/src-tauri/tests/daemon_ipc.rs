//! Integration tests for the desktop adapter (`tasks/018.md` §32;
//! `tasks/019.md` §38, §39).
//!
//! Each test owns a real [`Daemon`] running on a private Unix domain socket in
//! a throwaway temporary directory, and drives the adapter's own [`queries`]
//! and [`mutations`] — the exact functions behind the Tauri commands — through
//! a real [`RemoteClient`] over the versioned IPC protocol. Nothing in the
//! adapter is mocked here: command mapping, input validation, error shaping,
//! and the view models are exercised against the real shared core.
//!
//! The daemon is wired with **fake task executors**, an in-memory runtime-state
//! store, the real [`LogHub`], and the real [`HealthStore`]. The fake executors
//! update the runtime store and the health index exactly as a real
//! runtime/health adapter would (process → running/stopped, health → healthy,
//! with the configured check outcomes), and the task ingestor correlates task
//! events into the log hub, so `tasks/021.md` §42 can observe migration
//! progress, task logs, service log subscriptions, and health changes end-to-end
//! without ever starting an operating-system process. No Git command, Maven
//! build, Liquibase migration, database client, or company repository,
//! database, credential, or host is touched.
//!
//! # Why the tests are serialized
//!
//! The socket path is resolved from the `SERVICE_ORCHESTRATOR_SOCKET`
//! environment variable, which is process-global. The harness holds a
//! process-wide lock for its whole lifetime and restores the previous value on
//! drop, so tests in this binary run one at a time and stay isolated from the
//! daemon and MCP test binaries (separate processes).

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use service_orchestrator_application::{
    build_parameter, parameter, Application, ArtifactInspectionError, ArtifactInspector,
    ArtifactPresence, BuildCapabilities, BuildCapabilityIndex, BulkOperationSummary,
    DependencyVersionInspection, DetectedBuildSystem, DetectedIntegration, DetectedRuntime,
    DiffOptions, GitError, GitManager, GitOptions, HealthCheckOutcome, HealthState,
    InMemoryRuntimeStateStore, LogEntry, LogLevel, LogSource, MavenMetadata, NodeMetadata,
    ProviderAvailability, RepositoryInspection, RepositoryInspectionError, RepositoryInspector,
    ResolvedArtifact, ResolvedDependency, ResolvedMigrationTarget, ResolvedRepository,
    ResolvedServiceTarget, RuntimeStateStore, CHECK_HTTP, CHECK_PROCESS, CHECK_TCP,
};
use service_orchestrator_client::{
    socket_path, ClientKind, OrchestratorClient, RemoteClient, PROTOCOL_VERSION, SOCKET_ENV,
};
use service_orchestrator_config::{
    load_from_str, Config, ConfigChange, ConfigManager, EntityChange, FileConfigManager,
};
use service_orchestrator_daemon::{Daemon, DaemonError, DaemonOptions};
use service_orchestrator_desktop_lib::{
    build, configuration,
    dto::{LogEntryView, OperationOutcome, OperationStatus, WorkflowStatusView},
    git, health, library, logs, migration, mutations, queries, tasks, workflow,
};
use service_orchestrator_domain::{
    AbsolutePath, Branch, BranchInfo, BranchKind, BuildOperation, BuildSystem, GitOperation,
    GitOperationResult, HealthCheckKind, MigrationOperation, ProcessState, RepoDiff,
    RepositoryLifecycle, RepositoryStatus, ServiceId, Task, TaskKind, TaskStatus, Timestamp,
    Version, WorkflowId,
};
use service_orchestrator_execution::{
    TaskContext, TaskEngine, TaskExecutor, TaskExecutorError, TaskOutcome,
};
use service_orchestrator_health::HealthStore;
use service_orchestrator_logging::{spawn_task_ingestor, LogHub};

/// The fixture configuration, shared with the daemon and MCP adapter tests.
const FIXTURE: &str = include_str!("../../../../crates/mcp/tests/fixtures/config.toml");

/// Serializes the tests so the process-global socket override cannot race.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// How long a test waits for the daemon to become reachable or to stop.
const DEADLINE: Duration = Duration::from_secs(15);

/// How many health observations the index retains per service.
const HEALTH_HISTORY: usize = 8;

/// Everything a test can turn when building the shared core.
#[derive(Clone)]
struct CoreSpec {
    /// The configuration text the core is built from.
    config: String,
    /// Service ids whose lifecycle operation must fail.
    failing_starts: BTreeSet<String>,
    /// Entity ids whose build task must fail.
    failing_builds: BTreeSet<String>,
    /// Entity ids whose Git, build, or migration task blocks until cancelled.
    blocking: BTreeSet<String>,
}

impl Default for CoreSpec {
    fn default() -> Self {
        Self {
            config: FIXTURE.to_owned(),
            failing_starts: BTreeSet::new(),
            failing_builds: BTreeSet::new(),
            blocking: BTreeSet::new(),
        }
    }
}

impl CoreSpec {
    /// The fixture with its migration target moved to `environment`.
    ///
    /// Only the Liquibase database environment is rewritten, so the resulting
    /// configuration differs exactly in the declared environment the shared
    /// migration policy reads (`tasks/021.md` §8, §9).
    fn with_environment(environment: &str) -> Self {
        let config = FIXTURE.replace(
            r#"environment = "local""#,
            &format!(r#"environment = "{environment}""#),
        );
        assert_ne!(
            config, FIXTURE,
            "the fixture must declare a migration environment"
        );
        Self {
            config,
            ..Self::default()
        }
    }

    /// Adds the services whose lifecycle operations must fail.
    fn failing_starts(mut self, ids: &[&str]) -> Self {
        self.failing_starts = ids.iter().map(|id| (*id).to_owned()).collect();
        self
    }

    /// Adds the entities whose build tasks must fail.
    fn failing_builds(mut self, ids: &[&str]) -> Self {
        self.failing_builds = ids.iter().map(|id| (*id).to_owned()).collect();
        self
    }

    /// Adds the entities whose Git, build, or migration tasks block.
    fn blocking(mut self, ids: &[&str]) -> Self {
        self.blocking = ids.iter().map(|id| (*id).to_owned()).collect();
        self
    }

    /// The fixture with a second, independent workspace.
    ///
    /// The added workspace declares one service but neither the fixture profile
    /// nor the fixture group, so a workspace-scoped lookup under it must not
    /// resolve them (`tasks/022.md` §42 items 2, 34).
    fn with_second_workspace() -> Self {
        let config = format!(
            "{FIXTURE}\n\
             [[workspaces]]\n\
             id = \"other\"\n\
             name = \"Other Platform\"\n\
             root = \"~/Projects/other\"\n\
             \n\
             [[services]]\n\
             id = \"ledger-api\"\n\
             name = \"ledger-api\"\n\
             type = \"backend\"\n\
             workspace = \"other\"\n\
             path = \"backend/ledger-api\"\n\
             default_branch = \"main\"\n\
             build_system = \"maven\"\n\
             \n\
             [services.repository]\n\
             url = \"git@github.com:other/ledger-api.git\"\n\
             \n\
             [services.commands]\n\
             build = \"./mvnw clean install\"\n\
             start = \"./mvnw spring-boot:run\"\n"
        );
        Self {
            config,
            ..Self::default()
        }
    }
}

/// The shared application core one daemon serves.
struct Core {
    /// The application every interface reaches over IPC.
    application: Application,
    /// The single authoritative task engine.
    engine: Arc<TaskEngine>,
    /// Test-only fake infrastructure state.
    fakes: Arc<Mutex<FakeState>>,
    /// The real log hub behind task logs and service subscriptions.
    hub: Arc<LogHub>,
    /// The real health index behind the Health screen's observations.
    health: Arc<HealthStore>,
    /// The task-event ingestor correlating engine events into the hub.
    ingestor: tokio::task::JoinHandle<()>,
}

/// A daemon and the temporary directory it owns.
struct Harness {
    /// Held for the harness lifetime so only one test mutates the socket
    /// environment at a time.
    lock: MutexGuard<'static, ()>,
    /// The socket override value to restore when the harness is dropped.
    previous_socket: Option<OsString>,
    /// The runtime the daemon task runs on; kept alive for the whole test.
    runtime: tokio::runtime::Runtime,
    /// The daemon's own task, until it is shut down on drop.
    daemon: Option<tokio::task::JoinHandle<Result<(), DaemonError>>>,
    /// The configuration file the socket path is derived from.
    config_path: PathBuf,
    /// The socket the daemon binds.
    socket: PathBuf,
    /// The throwaway directory that holds the configuration, socket, and lock.
    root: PathBuf,
    /// Test-only port state shared with the fake Git manager, artifact
    /// inspector, and task executors, so a test can observe and steer what the
    /// "infrastructure" reports without any real Git, Maven, or npm execution.
    fakes: Arc<Mutex<FakeState>>,
    /// The spec the core was built from, so a restart rebuilds the same core.
    spec: CoreSpec,
    /// The core the running daemon serves.
    core: Core,
}

impl Harness {
    /// Starts an isolated daemon over a fresh temporary directory.
    fn start() -> Self {
        Self::start_with_spec(CoreSpec::default())
    }

    /// Starts an isolated daemon whose start executor fails for the named
    /// services, so bulk partial-failure handling can be exercised.
    fn start_with_failures(failing_starts: &[&str]) -> Self {
        Self::start_with(failing_starts, &[], &[])
    }

    /// Starts an isolated daemon with configurable fake-port behaviour.
    ///
    /// `failing_starts` fails service lifecycle operations, `failing_builds`
    /// fails service/library build tasks, and `blocking` makes a Git, build, or
    /// migration task on that entity wait until it is cancelled, so
    /// cancellation can be observed without any real long-running process.
    fn start_with(failing_starts: &[&str], failing_builds: &[&str], blocking: &[&str]) -> Self {
        Self::start_with_spec(
            CoreSpec::default()
                .failing_starts(failing_starts)
                .failing_builds(failing_builds)
                .blocking(blocking),
        )
    }

    /// Starts an isolated daemon whose migration target declares `environment`,
    /// so the destructive-authorization policy can be exercised
    /// (`tasks/021.md` §42 items 4–5).
    fn start_with_environment(environment: &str) -> Self {
        Self::start_with_spec(CoreSpec::with_environment(environment))
    }

    /// Starts an isolated daemon over an explicit core specification.
    fn start_with_spec(spec: CoreSpec) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());

        let root = unique_root();
        std::fs::create_dir_all(&root).expect("the temporary directory is created");
        let config_path = root.join("config.toml");
        std::fs::write(&config_path, &spec.config).expect("the configuration is written");

        let socket = root.join("run/daemon.sock");
        let previous_socket = std::env::var_os(SOCKET_ENV);
        std::env::set_var(SOCKET_ENV, &socket);
        assert_eq!(
            socket_path(&config_path),
            socket,
            "the harness pins the socket path exactly"
        );

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a Tokio runtime is built");
        let core = runtime.block_on(build_core(&spec, &config_path));
        let daemon = Daemon::new(
            core.application.clone(),
            Arc::clone(&core.engine),
            DaemonOptions {
                socket_path: socket.clone(),
            },
        );
        let handle = runtime.spawn(daemon.run());

        let harness = Self {
            lock,
            previous_socket,
            runtime,
            daemon: Some(handle),
            config_path,
            socket,
            root,
            fakes: Arc::clone(&core.fakes),
            spec,
            core,
        };
        harness.wait_ready();
        harness
    }

    /// Connects a fresh desktop (GUI) client, never starting a daemon of its
    /// own.
    ///
    /// This is also the transport the GUI adapter uses in production when no
    /// daemon binary could be resolved: attach to what is already running.
    fn client(&self) -> RemoteClient {
        self.client_of(ClientKind::Desktop)
    }

    /// Connects a fresh client presenting `kind`, so one test can act as the
    /// GUI, the TUI, and MCP against the same daemon.
    fn client_of(&self, kind: ClientKind) -> RemoteClient {
        RemoteClient::connect_existing(&self.config_path, kind).expect("the daemon is reachable")
    }

    /// Connects a fresh client standing in for the TUI.
    fn tui_client(&self) -> RemoteClient {
        self.client_of(ClientKind::Tui)
    }

    /// Connects a fresh client standing in for an MCP agent.
    fn mcp_client(&self) -> RemoteClient {
        self.client_of(ClientKind::Mcp)
    }

    /// Restarts the daemon over the same configuration, as a reconnecting GUI
    /// does (`tasks/021.md` §42 item 12).
    ///
    /// The running daemon shuts down gracefully — releasing the socket and the
    /// single-instance lock — its core is torn down, and a fresh core is built
    /// from the same spec. Configuration-backed state returns; in-memory task
    /// history, log history, and health observations do not, exactly as they do
    /// not across a real daemon restart.
    fn restart(&mut self) {
        let client = RemoteClient::connect_existing(&self.config_path, ClientKind::Cli)
            .expect("the daemon is reachable");
        client.shutdown_daemon().expect("shutdown is accepted");
        if let Some(handle) = self.daemon.take() {
            let stopped = self
                .runtime
                .block_on(async { tokio::time::timeout(Duration::from_secs(10), handle).await });
            assert!(
                stopped.is_ok_and(|inner| inner.is_ok_and(|outcome| outcome.is_ok())),
                "the daemon stopped cleanly"
            );
        }

        // Tear down the old core: the ingestor first so no further log entry is
        // produced, then the engine so no stale task survives into the rebuild.
        self.core.ingestor.abort();
        let previous = Arc::clone(&self.core.engine);
        let _ = self.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), previous.shutdown()).await
        });

        let spec = self.spec.clone();
        let core = self.runtime.block_on(build_core(&spec, &self.config_path));
        let daemon = Daemon::new(
            core.application.clone(),
            Arc::clone(&core.engine),
            DaemonOptions {
                socket_path: self.socket.clone(),
            },
        );
        let handle = self.runtime.spawn(daemon.run());
        self.fakes = Arc::clone(&core.fakes);
        self.core = core;
        self.daemon = Some(handle);
        self.wait_ready();
    }

    /// Polls until the daemon accepts a connection.
    fn wait_ready(&self) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            if RemoteClient::connect_existing(&self.config_path, ClientKind::Cli).is_ok() {
                return;
            }
            if let Some(handle) = &self.daemon {
                assert!(!handle.is_finished(), "the daemon exited during startup");
            }
            assert!(
                Instant::now() < deadline,
                "the daemon did not become reachable in time"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // Keep the lock held until the environment is restored.
        let _guard = &self.lock;

        if let Ok(client) = RemoteClient::connect_existing(&self.config_path, ClientKind::Cli) {
            let _ = client.shutdown_daemon();
        }
        if let Some(handle) = self.daemon.take() {
            let _ = self
                .runtime
                .block_on(async { tokio::time::timeout(Duration::from_secs(5), handle).await });
        }
        match &self.previous_socket {
            Some(value) => std::env::set_var(SOCKET_ENV, value),
            None => std::env::remove_var(SOCKET_ENV),
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The read path: every command-shaped query answered by a real daemon.
#[test]
fn the_adapter_reads_workspaces_services_and_status_over_ipc() {
    let harness = Harness::start();
    let client = harness.client();

    let workspaces = queries::list_workspaces(&client).expect("workspaces load");
    assert_eq!(workspaces.len(), 1);
    let demo = &workspaces[0];
    assert_eq!(demo.id, "demo");
    assert_eq!(demo.name, "Demo Platform");
    assert_eq!(demo.service_count, 2);
    assert!(!demo.root.is_empty());

    let services = queries::list_services(&client, "demo").expect("services load");
    assert_eq!(services.len(), 2);
    let orders = services
        .iter()
        .find(|service| service.id == "orders-api")
        .expect("orders-api is listed");
    let admin = services
        .iter()
        .find(|service| service.id == "admin-ui")
        .expect("admin-ui is listed");
    assert_eq!(orders.kind, "backend");
    assert_eq!(admin.kind, "frontend");
    assert_eq!(orders.port, Some(8081));
    assert_eq!(admin.port, Some(3000));

    // A fresh daemon has observed no process, health, or Git activity yet;
    // the strings are exactly what the frontend's union types declare.
    assert_eq!(orders.process_state, "not_running");
    assert_eq!(orders.health_state, "unknown");
    assert_eq!(orders.git_state, "unknown");
    assert_eq!(orders.branch, None);

    let status = queries::workspace_status(&client, "demo").expect("status loads");
    assert_eq!(status.workspace_id, "demo");
    assert_eq!(status.name, "Demo Platform");
    assert_eq!(status.services.len(), 2);

    let one = queries::service_status(&client, "demo", "orders-api").expect("service loads");
    assert_eq!(one, *orders);

    // The serialized shape is the frontend's `ServiceSummary` contract, field
    // for field.
    let json = serde_json::to_value(&one).expect("summaries serialize");
    assert_eq!(
        json,
        serde_json::json!({
            "id": "orders-api",
            "display_name": "orders-api",
            "kind": "backend",
            "workspace_id": "demo",
            "process_state": "not_running",
            "health_state": "unknown",
            "git_state": "unknown",
            "branch": null,
            "port": 8081,
        })
    );
}

/// Input validation and cross-workspace checks stay inside the adapter.
#[test]
fn the_adapter_rejects_invalid_mismatched_and_unknown_input() {
    let harness = Harness::start();
    let client = harness.client();

    let error = queries::list_services(&client, "").expect_err("an empty id is invalid");
    assert_eq!(error.code, "INVALID_INPUT");
    assert!(!error.message.is_empty());

    let error = queries::list_services(&client, "two words").expect_err("spaces are invalid");
    assert_eq!(error.code, "INVALID_INPUT");

    let error = queries::workspace_status(&client, "ghost-workspace")
        .expect_err("the workspace does not exist");
    assert_eq!(error.code, "WorkspaceNotFound");

    let error = queries::service_status(&client, "demo", "ghost-api")
        .expect_err("the service does not exist");
    assert_eq!(error.code, "ServiceNotFound");
    assert!(!error.message.is_empty());

    let error = queries::service_status(&client, "other-workspace", "orders-api")
        .expect_err("the service belongs to another workspace");
    assert_eq!(error.code, "NOT_FOUND");
    assert!(error.message.contains("orders-api"), "names the service");
}

/// One authoritative daemon answers every client, exactly as the GUI, TUI,
/// and MCP adapters share it (`tasks/018.md` §9; `tasks/015.md` §3).
#[test]
fn every_client_sees_the_same_authoritative_daemon() {
    let harness = Harness::start();
    let first = harness.client();
    let second = harness.client();

    let a = first.daemon_status().expect("status answers");
    let b = second.daemon_status().expect("status answers");
    assert_eq!(a.pid, b.pid, "both clients reach the same process");
    assert_eq!(
        a.pid,
        std::process::id(),
        "the harness runs the daemon in-process"
    );
    assert_eq!(a.socket_path, b.socket_path, "and the same endpoint");
    assert_eq!(
        a.protocol_version, PROTOCOL_VERSION,
        "same protocol version"
    );
    assert!(
        a.clients >= 2,
        "two attached clients share one daemon, got {}",
        a.clients
    );

    // Reads keep working from the second client while the first is attached.
    let workspaces = queries::list_workspaces(&second).expect("reads work concurrently");
    assert_eq!(workspaces.len(), 1);
}

/// The full lifecycle the GUI drives: start → running, health check, restart,
/// stop → stopped, all observed through the adapter's own query layer.
#[test]
fn the_adapter_drives_the_service_lifecycle_through_the_shared_core() {
    let harness = Harness::start();
    let client = harness.client();

    // A fresh daemon has observed no process activity yet.
    let before = queries::service_status(&client, "demo", "orders-api").expect("status loads");
    assert_eq!(before.process_state, "not_running");
    assert_eq!(before.health_state, "unknown");

    // Start schedules exactly one task.
    let started = mutations::start_service(&client, "orders-api").expect("start is scheduled");
    assert_eq!(started.status, OperationStatus::Task);
    assert_eq!(started.kind, "start_service");
    assert!(started.task_id.is_some());
    assert_eq!(started.workflow_id, None);
    assert_eq!(settle(&client, &started).status, TaskStatus::Succeeded);
    wait_for_process_state(&client, "orders-api", ProcessState::Running);

    // A health check is a scheduled task that refreshes observed health.
    let checked =
        mutations::check_service_health(&client, "orders-api").expect("health is scheduled");
    assert_eq!(checked.status, OperationStatus::Task);
    assert_eq!(settle(&client, &checked).status, TaskStatus::Succeeded);

    // The detail combines real configuration with real observed facts.
    let detail = queries::service_detail(&client, "demo", "orders-api").expect("detail loads");
    assert_eq!(detail.process_state, "running");
    assert_eq!(detail.health_state, "healthy");
    assert_eq!(detail.build_system.as_deref(), Some("maven"));
    assert_eq!(detail.ports, vec![8081]);
    assert!(
        detail.repository.is_some(),
        "the fixture declares a repository"
    );
    assert_eq!(
        detail.runtimes.first().map(|r| r.kind.as_str()),
        Some("java")
    );
    assert!(
        detail
            .health_checks
            .iter()
            .any(|check| check.kind.starts_with("http")),
        "the fixture configures an http health check"
    );

    // Restart is one coordinated task, never a GUI-side stop/start pair.
    let restarted = mutations::restart_service(&client, "orders-api").expect("restart scheduled");
    assert_eq!(restarted.status, OperationStatus::Task);
    assert_eq!(restarted.kind, "restart_service");
    assert_eq!(settle(&client, &restarted).status, TaskStatus::Succeeded);
    wait_for_process_state(&client, "orders-api", ProcessState::Running);

    // Stop.
    let stopped = mutations::stop_service(&client, "orders-api").expect("stop scheduled");
    assert_eq!(stopped.status, OperationStatus::Task);
    assert_eq!(stopped.kind, "stop_service");
    assert_eq!(settle(&client, &stopped).status, TaskStatus::Succeeded);
    wait_for_process_state(&client, "orders-api", ProcessState::Stopped);

    let after = queries::service_detail(&client, "demo", "orders-api").expect("detail reloads");
    assert_eq!(after.process_state, "stopped");
}

/// Bulk operations schedule exactly one workflow for the explicit selection.
#[test]
fn bulk_operations_schedule_one_workflow_for_the_selection() {
    let harness = Harness::start();
    let client = harness.client();
    let selection = vec!["orders-api".to_owned(), "admin-ui".to_owned()];

    let started = mutations::start_services(&client, "demo", &selection).expect("bulk start");
    assert_eq!(started.status, OperationStatus::Workflow);
    assert_eq!(started.kind, "start_services");
    assert!(started.task_id.is_none());
    let workflow = WorkflowId::new(started.workflow_id.clone().expect("a workflow id"))
        .expect("a valid workflow id");

    let summary = wait_for_workflow(&client, &workflow);
    assert_eq!(summary.total, 2);
    assert_eq!(summary.succeeded, 2);
    assert_eq!(summary.failed, 0);
    wait_for_process_state(&client, "orders-api", ProcessState::Running);
    wait_for_process_state(&client, "admin-ui", ProcessState::Running);

    let restarted = mutations::restart_services(&client, "demo", &selection).expect("bulk restart");
    assert_eq!(restarted.status, OperationStatus::Workflow);
    assert_eq!(restarted.kind, "restart_services");
    let workflow = WorkflowId::new(restarted.workflow_id.clone().expect("a workflow id"))
        .expect("a valid workflow id");
    assert_eq!(wait_for_workflow(&client, &workflow).succeeded, 2);

    let stopped = mutations::stop_services(&client, "demo", &selection).expect("bulk stop");
    assert_eq!(stopped.status, OperationStatus::Workflow);
    assert_eq!(stopped.kind, "stop_services");
    let workflow = WorkflowId::new(stopped.workflow_id.clone().expect("a workflow id"))
        .expect("a valid workflow id");
    assert_eq!(wait_for_workflow(&client, &workflow).succeeded, 2);
    wait_for_process_state(&client, "orders-api", ProcessState::Stopped);
    wait_for_process_state(&client, "admin-ui", ProcessState::Stopped);
}

/// A bulk start where one member fails settles the workflow per service; the
/// successful member still runs and the failure is structured.
#[test]
fn bulk_start_reports_partial_failure_per_service() {
    let harness = Harness::start_with_failures(&["admin-ui"]);
    let client = harness.client();
    let selection = vec!["orders-api".to_owned(), "admin-ui".to_owned()];

    let started = mutations::start_services(&client, "demo", &selection).expect("bulk start");
    let workflow = WorkflowId::new(started.workflow_id.clone().expect("a workflow id"))
        .expect("a valid workflow id");
    let summary = wait_for_workflow(&client, &workflow);
    assert_eq!(summary.total, 2);
    assert_eq!(summary.succeeded, 1);
    assert_eq!(summary.failed, 1);

    // The successful member runs; the failed one does not.
    wait_for_process_state(&client, "orders-api", ProcessState::Running);
    let admin = queries::service_status(&client, "demo", "admin-ui").expect("status loads");
    assert_ne!(admin.process_state, "running");

    // A single failed start carries the structured runtime failure code.
    let failed = mutations::start_service(&client, "admin-ui").expect("schedule is accepted");
    let task = settle(&client, &failed);
    assert_eq!(task.status, TaskStatus::Failed);
    let error = task.error.expect("a structured failure");
    assert_eq!(error.code, "RUNTIME_PORT_ALREADY_IN_USE");
    assert!(!error.message.is_empty());
}

/// Input validation and idempotent no-op states stay in the adapter.
#[test]
fn the_adapter_rejects_invalid_selections_and_reports_no_op_states() {
    let harness = Harness::start();
    let client = harness.client();

    let error = mutations::start_services(&client, "demo", &[]).expect_err("an empty selection");
    assert_eq!(error.code, "INVALID_INPUT");
    assert!(error.message.contains("at least one"));

    let error = mutations::start_service(&client, "bad id").expect_err("a rejected id");
    assert_eq!(error.code, "INVALID_INPUT");

    let error = mutations::start_service(&client, "ghost-api").expect_err("an unknown service");
    assert_eq!(error.code, "ServiceNotFound");

    // Stopping a service that is not running is an honest no-op.
    let outcome = mutations::stop_service(&client, "orders-api").expect("a no-op, not an error");
    assert_eq!(outcome.status, OperationStatus::AlreadyInDesiredState);
    assert_eq!(outcome.task_id, None);
    assert_eq!(outcome.workflow_id, None);

    // Starting an already-running service is also a no-op.
    let started = mutations::start_service(&client, "orders-api").expect("start is scheduled");
    assert_eq!(settle(&client, &started).status, TaskStatus::Succeeded);
    wait_for_process_state(&client, "orders-api", ProcessState::Running);
    let again = mutations::start_service(&client, "orders-api").expect("no-op");
    assert_eq!(again.status, OperationStatus::AlreadyInDesiredState);
}

/// The GUI, TUI, and MCP share one authoritative daemon: a start from one is
/// the same task and the same state the others observe.
#[test]
fn gui_and_other_interfaces_share_one_authoritative_state() {
    let harness = Harness::start();
    let gui = harness.client();
    // A second client stands in for the TUI/MCP: same daemon, same core.
    let other = harness.client();

    let started = mutations::start_service(&gui, "orders-api").expect("start is scheduled");
    let task_id = started.task_id.clone().expect("a task id");
    assert_eq!(settle(&gui, &started).status, TaskStatus::Succeeded);

    wait_for_process_state(&gui, "orders-api", ProcessState::Running);
    wait_for_process_state(&other, "orders-api", ProcessState::Running);
    let task = wait_for_task(&other, &task_id);
    assert_eq!(task.status, TaskStatus::Succeeded);

    // A stop from the other interface is visible to the GUI.
    let stopped = mutations::stop_service(&other, "orders-api").expect("stop is scheduled");
    assert_eq!(settle(&other, &stopped).status, TaskStatus::Succeeded);
    wait_for_process_state(&gui, "orders-api", ProcessState::Stopped);
}

/// A stopped daemon surfaces a structured `DAEMON_UNAVAILABLE` error rather than
/// a panic; the frontend disables mutating actions on that code.
#[test]
fn a_stopped_daemon_surfaces_a_structured_unavailable_error() {
    use service_orchestrator_client::codes;

    let harness = Harness::start();
    let client = harness.client();
    assert!(queries::list_workspaces(&client).is_ok());

    client.shutdown_daemon().expect("shutdown is accepted");

    let deadline = Instant::now() + DEADLINE;
    loop {
        match queries::list_workspaces(&client) {
            Ok(_) => {
                assert!(Instant::now() < deadline, "the daemon never stopped");
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => {
                assert_eq!(error.code, codes::DAEMON_UNAVAILABLE);
                assert!(!error.message.is_empty());
                break;
            }
        }
    }
}

/// The desktop Git screen loads configured repositories from configuration and
/// inspects real working-tree state on demand through the shared core
/// (`tasks/020.md` §4, §6, §35, §40 items 3–4).
#[test]
fn the_adapter_lists_and_inspects_git_repositories() {
    let harness = Harness::start();
    let client = harness.client();

    let repositories = git::list_repositories(&client, "demo").expect("repositories load");
    assert_eq!(
        repositories.len(),
        2,
        "both fixture services have repositories"
    );
    let orders = repositories
        .iter()
        .find(|repo| repo.service_id == "orders-api")
        .expect("orders-api is listed");
    let admin = repositories
        .iter()
        .find(|repo| repo.service_id == "admin-ui")
        .expect("admin-ui is listed");
    assert_eq!(orders.id, "demo:orders-api", "ids are workspace-aware");
    assert_eq!(admin.id, "demo:admin-ui");
    assert!(orders.repository.is_some());
    assert_eq!(orders.path.as_deref(), Some("backend/orders-api"));

    // Inspection observes the working copy on demand; it performs no Git write.
    let status = git::get_git_status(&client, "orders-api").expect("status loads");
    assert_eq!(status.lifecycle, "available");
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(status.upstream.as_deref(), Some("origin/main"));
    assert!(status.clean);
    assert!(!status.has_local_changes);
    assert_eq!(status.ahead, Some(0));
    assert_eq!(status.behind, Some(0));

    let branches = git::list_branches(&client, "orders-api").expect("branches load");
    assert_eq!(branches.len(), 2);
    assert!(branches
        .iter()
        .any(|branch| branch.name == "main" && branch.kind == "local" && branch.current));
    assert!(branches
        .iter()
        .any(|branch| branch.name == "origin/main" && branch.kind == "remote"));

    // A batch inspection reports each repository independently.
    let selection = vec!["orders-api".to_owned(), "admin-ui".to_owned()];
    let inspected = git::inspect_repositories(&client, "demo", &selection).expect("batch loads");
    assert_eq!(inspected.len(), 2);
    assert!(inspected
        .iter()
        .all(|entry| entry.status.is_some() && entry.error.is_none()));
    assert_eq!(inspected[0].repository_id, "demo:orders-api");
    assert_eq!(inspected[1].repository_id, "demo:admin-ui");
}

/// Safe Git fetch, checkout, and bulk synchronization run through the shared
/// core as real tasks and workflows, and the observed working copy reflects
/// them (`tasks/020.md` §5, §6, §9, §40 items 5–6).
#[test]
fn git_fetch_checkout_and_bulk_sync_run_through_the_shared_core() {
    let harness = Harness::start();
    let client = harness.client();
    let selection = vec!["orders-api".to_owned(), "admin-ui".to_owned()];

    // A read-only preview aggregates real status without mutating anything.
    let preview =
        git::preview_git_sync(&client, "demo", &selection, "main", "sync").expect("preview builds");
    assert_eq!(preview.total, 2);
    assert_eq!(preview.ready, 2);
    assert_eq!(preview.dirty, 0);
    assert_eq!(preview.missing_branch, 0);
    assert_eq!(preview.unavailable, 0);
    assert_eq!(preview.mode, "sync");
    assert!(preview.entries.iter().all(|entry| !entry.blocked
        && entry.target_branch_exists
        && entry.expected_operations == vec!["fetch", "checkout", "pull"]));

    // A missing target branch is reported as blocked, never silently attempted.
    let missing = git::preview_git_sync(&client, "demo", &selection, "release/1.0", "sync")
        .expect("preview builds");
    assert_eq!(missing.missing_branch, 2);
    assert!(missing.entries.iter().all(|entry| entry.blocked));

    // A single safe fetch is exactly one task.
    let fetched = git::fetch_repository(&client, "orders-api").expect("fetch scheduled");
    assert_eq!(fetched.status, OperationStatus::Task);
    assert_eq!(fetched.kind, "git_fetch");
    assert_eq!(settle(&client, &fetched).status, TaskStatus::Succeeded);

    // A single checkout is one task.
    let checked =
        git::checkout_repository(&client, "orders-api", "main").expect("checkout scheduled");
    assert_eq!(checked.kind, "git_checkout");
    assert_eq!(settle(&client, &checked).status, TaskStatus::Succeeded);

    // A bulk sync is exactly one workflow over the explicit selection.
    let synced = git::sync_repositories(&client, "demo", &selection).expect("bulk sync");
    assert_eq!(synced.status, OperationStatus::Workflow);
    assert_eq!(synced.kind, "sync_repositories");
    let summary = wait_for_workflow(&client, &workflow_of(&synced));
    assert_eq!(summary.total, 2);
    assert_eq!(summary.succeeded, 2);

    // Bulk fetch and bulk checkout use the same workflow path.
    let bulk_fetch = git::fetch_repositories(&client, "demo", &selection).expect("bulk fetch");
    assert_eq!(bulk_fetch.kind, "fetch_repositories");
    assert_eq!(
        wait_for_workflow(&client, &workflow_of(&bulk_fetch)).succeeded,
        2
    );
    let bulk_checkout =
        git::checkout_repositories(&client, "demo", &selection, "main").expect("bulk checkout");
    assert_eq!(bulk_checkout.kind, "checkout_repositories");
    assert_eq!(
        wait_for_workflow(&client, &workflow_of(&bulk_checkout)).succeeded,
        2
    );

    // The observed state reflects the operations the workflow ran.
    let status = git::get_git_status(&client, "orders-api").expect("status reloads");
    assert_eq!(status.branch.as_deref(), Some("main"));
}

/// The Builds screen renders the provider's real capabilities — never
/// hard-coding Maven or npm — and runs Maven and npm builds through the shared
/// core (`tasks/020.md` §12, §13, §16, §40 items 7–9).
#[test]
fn build_targets_carry_provider_capabilities_and_builds_run() {
    let harness = Harness::start();
    let client = harness.client();

    let targets = build::list_build_targets(&client, "demo").expect("targets load");
    assert_eq!(targets.len(), 2);
    let maven = targets
        .iter()
        .find(|target| target.service_id == "orders-api")
        .expect("orders-api is a build target");
    let npm = targets
        .iter()
        .find(|target| target.service_id == "admin-ui")
        .expect("admin-ui is a build target");
    assert_eq!(maven.build_system.as_deref(), Some("maven"));
    assert_eq!(npm.build_system.as_deref(), Some("npm"));

    let maven_caps = maven.capability.as_ref().expect("maven capabilities");
    assert_eq!(maven_caps.build_system, "maven");
    assert!(!maven_caps.empty);
    let build_op = maven_caps
        .operations
        .iter()
        .find(|operation| operation.operation == "build")
        .expect("the build operation is listed");
    assert!(build_op.supported && build_op.exposed && build_op.reason.is_none());
    let publish = maven_caps
        .operations
        .iter()
        .find(|operation| operation.operation == "publish_local_artifact")
        .expect("publish is listed");
    assert!(
        publish.supported && !publish.exposed && publish.reason.is_some(),
        "supported-but-not-exposed operations carry an explanation"
    );

    // The dedicated capability query matches the row projection, and npm is
    // resolved independently of Maven.
    let caps = build::get_build_capabilities(&client, "orders-api").expect("capabilities load");
    assert_eq!(caps.build_system, "maven");
    let npm_caps = build::get_build_capabilities(&client, "admin-ui").expect("capabilities load");
    assert_eq!(npm_caps.build_system, "npm");
    assert!(npm_caps
        .operations
        .iter()
        .any(|operation| operation.operation == "build" && operation.supported));

    // A single Maven build is one task; its progress and bounded logs are
    // readable through the task-engine queries.
    let orders = build::build_service(&client, "orders-api").expect("build scheduled");
    assert_eq!(orders.status, OperationStatus::Task);
    assert_eq!(orders.kind, "build_service");
    let task = settle(&client, &orders);
    assert_eq!(task.status, TaskStatus::Succeeded);
    let view = tasks::get_task(&client, task.id.as_str()).expect("the task loads");
    assert_eq!(view.kind, "build_service");
    assert_eq!(view.status, "succeeded");
    assert!(
        view.progress
            .message
            .as_deref()
            .is_some_and(|m| !m.is_empty()),
        "the executor recorded progress"
    );
    let logs = tasks::get_task_logs(&client, task.id.as_str(), &tasks::TaskLogsQuery::default())
        .expect("logs");
    assert!(!logs.entries.is_empty(), "the task ingestor recorded logs");
    assert!(
        logs.entries
            .iter()
            .any(|entry| entry.task_id.as_deref() == Some(task.id.as_str())),
        "the entries are correlated with the task"
    );

    // A single npm build is one task.
    let admin = build::build_service(&client, "admin-ui").expect("build scheduled");
    assert_eq!(admin.kind, "build_service");
    assert_eq!(settle(&client, &admin).status, TaskStatus::Succeeded);

    // A bulk build is exactly one workflow, never one request per service.
    let selection = vec!["orders-api".to_owned(), "admin-ui".to_owned()];
    let bulk = build::build_services(&client, "demo", &selection).expect("bulk build");
    assert_eq!(bulk.status, OperationStatus::Workflow);
    assert_eq!(bulk.kind, "prepare_environment");
    let summary = wait_for_workflow(&client, &workflow_of(&bulk));
    assert_eq!(summary.total, 2);
    assert_eq!(summary.succeeded, 2);
}

/// The Libraries screen lists a library with its dependents, verifies its
/// installed artifact, inspects version alignment, and prepares it through the
/// existing library workflows (`tasks/020.md` §20–§26, §40 items 10–12).
#[test]
fn library_operations_run_through_the_shared_core() {
    let harness = Harness::start();
    let client = harness.client();

    let libraries = library::list_libraries(&client, "demo").expect("libraries load");
    assert_eq!(libraries.len(), 1);
    let shared = &libraries[0];
    assert_eq!(shared.id, "shared-kit");
    assert_eq!(shared.build_system.as_deref(), Some("maven"));
    let artifact = shared.artifact.as_ref().expect("a primary artifact");
    assert_eq!(artifact.coordinate, "com.demo:shared-kit:1.8.0-SNAPSHOT");
    assert_eq!(artifact.packaging, "jar");
    assert_eq!(shared.configured_version.as_deref(), Some("1.8.0-SNAPSHOT"));
    assert_eq!(shared.branch.as_deref(), Some("main"));
    assert!(shared
        .dependents
        .iter()
        .any(|dependent| dependent.service_id == "orders-api"));

    let dependents =
        library::get_library_dependents(&client, "shared-kit").expect("dependents load");
    assert_eq!(dependents.len(), 1);
    assert_eq!(dependents[0].service_id, "orders-api");
    assert!(dependents[0].required);

    // Verification reports presence (and the path) without writing anything.
    let verification =
        library::verify_library_artifact(&client, "shared-kit").expect("verification");
    assert!(verification.present);
    assert!(verification.path.is_some());
    assert_eq!(
        verification.artifact.coordinate,
        "com.demo:shared-kit:1.8.0-SNAPSHOT"
    );

    // Version alignment is read-only: it reports the consumer's version and
    // never rewrites a POM.
    let alignment =
        library::inspect_version_alignment(&client, "shared-kit").expect("alignment loads");
    assert_eq!(
        alignment.target.coordinate,
        "com.demo:shared-kit:1.8.0-SNAPSHOT"
    );
    assert_eq!(alignment.total, 1);
    assert_eq!(alignment.aligned_count, 1);
    assert!(alignment.fully_aligned);
    assert_eq!(alignment.consumers[0].service_id, "orders-api");
    assert_eq!(alignment.consumers[0].status, "aligned");

    // Single library build, install, and sync are tasks.
    let built = library::build_library(&client, "shared-kit").expect("build scheduled");
    assert_eq!(built.kind, "build_library");
    assert_eq!(settle(&client, &built).status, TaskStatus::Succeeded);

    let installed = library::install_library(&client, "shared-kit").expect("install scheduled");
    assert_eq!(installed.kind, "library_install");
    assert_eq!(settle(&client, &installed).status, TaskStatus::Succeeded);

    let synced = library::sync_library(&client, "shared-kit").expect("sync scheduled");
    assert_eq!(synced.kind, "git_sync");
    assert_eq!(settle(&client, &synced).status, TaskStatus::Succeeded);

    // Prepare one library without forcing a branch (synchronization disabled).
    let prepared = library::prepare_library(&client, "shared-kit", false, None, true, true, true)
        .expect("prepare scheduled");
    assert_eq!(prepared.status, OperationStatus::Workflow);
    assert_eq!(prepared.kind, "prepare_library");
    let summary = wait_for_workflow(&client, &workflow_of(&prepared));
    assert_eq!(summary.total, 3, "build, install, and verify");
    assert_eq!(summary.succeeded, 3);

    // Prepare a selection as one dependency-ordered workflow.
    let selection = vec!["shared-kit".to_owned()];
    let bulk = library::prepare_libraries(
        &client,
        "demo",
        &selection,
        true,
        Some("main"),
        true,
        true,
        true,
    )
    .expect("bulk prepare");
    assert_eq!(bulk.kind, "prepare_libraries");
    assert_eq!(
        wait_for_workflow(&client, &workflow_of(&bulk)).succeeded,
        5,
        "sync, checkout, build, install, and verify"
    );
}

/// Version alignment reports a genuine mismatch without changing anything
/// (`tasks/020.md` §23, §24).
#[test]
fn library_version_alignment_reports_a_mismatch() {
    let harness = Harness::start();
    let client = harness.client();
    {
        let mut state = harness
            .fakes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.declared_version = Some(Version::new("1.9.0-SNAPSHOT").expect("a valid version"));
    }

    let alignment =
        library::inspect_version_alignment(&client, "shared-kit").expect("alignment loads");
    assert_eq!(alignment.aligned_count, 0);
    assert!(!alignment.fully_aligned);
    let consumer = &alignment.consumers[0];
    assert_eq!(consumer.service_id, "orders-api");
    assert_eq!(consumer.status, "mismatch");
    assert_eq!(consumer.current_version.as_deref(), Some("1.9.0-SNAPSHOT"));
}

/// A long-running operation can be cancelled through the existing task-engine
/// API, and cancellation is reported honestly (`tasks/020.md` §29, §40 item 13).
#[test]
fn a_running_operation_can_be_cancelled() {
    let harness = Harness::start_with(&[], &[], &["admin-ui"]);
    let client = harness.client();

    // A blocked fetch stays active until it is cancelled.
    let fetched = git::fetch_repository(&client, "admin-ui").expect("fetch scheduled");
    let task_id = fetched.task_id.clone().expect("a task id");
    let running = tasks::get_task(&client, &task_id).expect("the task loads");
    assert!(
        !matches!(
            running.status.as_str(),
            "succeeded" | "failed" | "cancelled"
        ),
        "the blocked task is still active, got '{}'",
        running.status
    );

    let cancelled = tasks::cancel_task(&client, &task_id).expect("cancel is accepted");
    assert_eq!(cancelled.kind, "git_fetch");
    let settled = wait_for_task(&client, &task_id);
    assert_eq!(settled.status, TaskStatus::Cancelled);

    // Cancelling a workflow cancels its still-active tasks; completed work is
    // not claimed to have been rolled back.
    let selection = vec!["orders-api".to_owned(), "admin-ui".to_owned()];
    let bulk = git::sync_repositories(&client, "demo", &selection).expect("bulk sync");
    let workflow = workflow_of(&bulk);
    let _ = tasks::cancel_workflow(&client, workflow.as_str()).expect("cancel workflow");
    let summary = wait_for_workflow(&client, &workflow);
    assert_eq!(summary.total, 2);
    assert_eq!(summary.pending, 0);
    assert!(
        summary.cancelled >= 1,
        "the blocked repository is cancelled"
    );
    assert_eq!(
        summary.succeeded + summary.cancelled,
        2,
        "each repository settles exactly once"
    );
}

/// A bulk operation where one member fails reports the partial failure per
/// target, and a single failure is structured (`tasks/020.md` §30, §40 item 14).
#[test]
fn bulk_builds_report_partial_failure_per_target() {
    let harness = Harness::start_with(&[], &["admin-ui"], &[]);
    let client = harness.client();
    let selection = vec!["orders-api".to_owned(), "admin-ui".to_owned()];

    let bulk = build::build_services(&client, "demo", &selection).expect("bulk build");
    let summary = wait_for_workflow(&client, &workflow_of(&bulk));
    assert_eq!(summary.total, 2);
    assert_eq!(summary.succeeded, 1);
    assert_eq!(summary.failed, 1);

    let failed = build::build_service(&client, "admin-ui").expect("schedule is accepted");
    let task = settle(&client, &failed);
    assert_eq!(task.status, TaskStatus::Failed);
    let error = task.error.expect("a structured failure");
    assert_eq!(error.code, "BUILD_COMMAND_FAILED");
    assert!(!error.message.is_empty());
}

/// The GUI, TUI, and MCP share one authoritative daemon: a Git or build task
/// scheduled from the GUI is the same task the other interfaces observe
/// (`tasks/020.md` §3, §40 item 15).
#[test]
fn other_interfaces_observe_the_same_git_and_build_tasks() {
    let harness = Harness::start();
    let gui = harness.client();
    let other = harness.client();

    let built = build::build_service(&gui, "orders-api").expect("build scheduled");
    let task_id = built.task_id.clone().expect("a task id");
    assert_eq!(settle(&gui, &built).status, TaskStatus::Succeeded);
    let observed = tasks::get_task(&other, &task_id).expect("the other interface sees the task");
    assert_eq!(observed.kind, "build_service");
    assert_eq!(observed.status, "succeeded");

    let selection = vec!["orders-api".to_owned(), "admin-ui".to_owned()];
    let synced = git::sync_repositories(&gui, "demo", &selection).expect("bulk sync");
    let workflow = workflow_of(&synced);
    assert_eq!(wait_for_workflow(&gui, &workflow).succeeded, 2);
    let summary = tasks::summarize_workflow(&other, workflow.as_str()).expect("summary");
    assert_eq!(summary.succeeded, 2);

    // The other interface observes the same working copy the GUI operated on.
    let status = git::get_git_status(&other, "admin-ui").expect("status loads");
    assert_eq!(status.branch.as_deref(), Some("main"));
}

/// Invalid and unknown inputs are rejected with structured codes, either by the
/// adapter or by the core (`tasks/020.md` §27, §30).
#[test]
fn git_build_and_library_inputs_are_validated() {
    let harness = Harness::start();
    let client = harness.client();

    assert_eq!(
        git::list_repositories(&client, "")
            .expect_err("an empty workspace is invalid")
            .code,
        "INVALID_INPUT"
    );
    assert_eq!(
        git::preview_git_sync(
            &client,
            "demo",
            &["orders-api".to_owned()],
            "main",
            "rebase"
        )
        .expect_err("an unknown mode is invalid")
        .code,
        "INVALID_INPUT"
    );
    assert_eq!(
        git::fetch_repositories(&client, "demo", &[])
            .expect_err("an empty selection is invalid")
            .code,
        "INVALID_INPUT"
    );
    assert_eq!(
        build::list_build_targets(&client, "bad id")
            .expect_err("spaces are invalid")
            .code,
        "INVALID_INPUT"
    );
    assert_eq!(
        build::get_build_capabilities(&client, "ghost-api")
            .expect_err("the service does not exist")
            .code,
        "ServiceNotFound"
    );
    assert_eq!(
        library::list_libraries(&client, "")
            .expect_err("an empty workspace is invalid")
            .code,
        "INVALID_INPUT"
    );
    assert_eq!(
        library::verify_library_artifact(&client, "ghost-lib")
            .expect_err("the library does not exist")
            .code,
        "LibraryNotFound"
    );
    assert_eq!(
        library::prepare_libraries(&client, "demo", &[], false, None, true, true, true)
            .expect_err("an empty selection is invalid")
            .code,
        "INVALID_INPUT"
    );

    let error = tasks::get_task_logs(
        &client,
        "task-1",
        &tasks::TaskLogsQuery {
            minimum_level: Some("verbose".to_owned()),
            ..tasks::TaskLogsQuery::default()
        },
    )
    .expect_err("an unknown severity is invalid");
    assert_eq!(error.code, "INVALID_INPUT");
}

// -- `tasks/021.md` §42: migration, task, health, and log integration --------

/// The Migration screen's read path against a real daemon: the desktop client
/// attaches (§42 item 1), migration-enabled services are listed (item 2), the
/// configuration snapshot is inspected (item 3), and a SQL preview runs as a
/// task whose SQL is readable from the task's own logs (items 4, 7).
#[test]
fn the_desktop_client_connects_and_the_migration_screen_reads_the_shared_core() {
    let harness = Harness::start();
    let client = harness.client();

    // Item 1: a desktop-kind client attaches to the already-running daemon.
    let status = client.daemon_status().expect("the daemon answers");
    assert_eq!(status.protocol_version, PROTOCOL_VERSION);
    assert_eq!(status.socket_path, harness.socket.display().to_string());
    assert!(status.clients >= 1);
    assert_eq!(
        queries::list_workspaces(&client)
            .expect("workspaces load")
            .len(),
        1
    );

    // Item 2: only services with migration configuration appear, and the row
    // reflects configuration — never an assumed database state.
    let rows = migration::list_migrations(&client, "demo").expect("migrations load");
    assert_eq!(rows.len(), 1, "only orders-api declares Liquibase");
    let row = &rows[0];
    assert_eq!(row.status.service_id, "orders-api");
    assert_eq!(row.status.workspace_id, "demo");
    assert!(row.status.enabled);
    assert_eq!(row.status.provider, "liquibase");
    assert_eq!(row.status.execution_mode, "maven");
    assert_eq!(row.status.environment, "local");
    assert!(
        !row.status.requires_authorization,
        "a local target needs no confirmation"
    );
    assert_eq!(row.status.pending, None, "live state is never fabricated");
    assert_eq!(row.status.locked, None, "a lock is never assumed");
    assert!(row.status.timeout_seconds.is_some());
    assert!(!row.status.database_identity.is_empty());
    assert!(row.latest_task.is_none(), "nothing has run yet");
    assert_eq!(
        migration::list_migrations(&client, "")
            .expect_err("an empty workspace is invalid")
            .code,
        "INVALID_INPUT"
    );

    // Item 3: a service without migration configuration is an explicit error,
    // not a silently missing row.
    let error = migration::get_migration_status(&client, "admin-ui")
        .expect_err("admin-ui declares no migrations");
    assert_eq!(error.code, "InvalidOperation");
    assert!(
        error.message.contains("migration configuration"),
        "{}",
        error.message
    );

    let snapshot = migration::get_migration_status(&client, "orders-api").expect("status loads");
    assert_eq!(snapshot.service_id, "orders-api");
    assert_eq!(snapshot.workspace_id, "demo");
    assert_eq!(snapshot.environment, "local");
    assert_eq!(snapshot.pending, None);
    assert!(
        !snapshot.database_identity.contains("super-secret"),
        "the database identity never carries a credential"
    );

    // Every migration operation the core defines is one task.
    let validated = migration::validate_migration(&client, "orders-api").expect("validate runs");
    assert_eq!(validated.status, OperationStatus::Task);
    assert_eq!(validated.kind, "migration_validate");
    assert_eq!(settle(&client, &validated).status, TaskStatus::Succeeded);

    let inspected = migration::inspect_status(&client, "orders-api").expect("status runs");
    assert_eq!(inspected.kind, "migration_status");
    assert_eq!(settle(&client, &inspected).status, TaskStatus::Succeeded);

    let pending = migration::inspect_pending(&client, "orders-api").expect("pending runs");
    assert_eq!(pending.kind, "migration_pending");
    assert_eq!(settle(&client, &pending).status, TaskStatus::Succeeded);

    // Items 4 and 7: the preview is a task, and its SQL is in the task logs.
    let preview = migration::preview(&client, "orders-api").expect("preview scheduled");
    assert_eq!(preview.status, OperationStatus::Task);
    assert_eq!(preview.kind, "migration_preview");
    let preview_id = preview.task_id.clone().expect("a task id");
    assert_eq!(settle(&client, &preview).status, TaskStatus::Succeeded);

    let entries = wait_for_task_logs(&client, &preview_id, |entries| {
        entries.iter().any(|entry| entry.source == "migration")
    });
    let sql = entries
        .iter()
        .find(|entry| entry.source == "migration")
        .expect("the preview SQL is recorded");
    assert!(sql.message.contains("CREATE TABLE"), "{}", sql.message);
    assert_eq!(sql.service_id.as_deref(), Some("orders-api"));
    assert_eq!(sql.task_id.as_deref(), Some(preview_id.as_str()));

    // The listing reports the newest migration task of the service.
    let rows = migration::list_migrations(&client, "demo").expect("migrations reload");
    let latest = rows[0].latest_task.as_ref().expect("a latest task");
    assert_eq!(latest.kind, "migration_preview");
    assert_eq!(latest.service_id.as_deref(), Some("orders-api"));
    assert_eq!(latest.workspace_id.as_deref(), Some("demo"));
}

/// Applying migrations is authorized by the daemon, never by the frontend
/// (`tasks/021.md` §8, §9, §39; §42 item 5 of the harness contract: no real
/// database is touched, so the authorization boundary itself is asserted).
#[test]
fn migration_apply_honors_only_the_daemon_verified_confirmation() {
    let harness = Harness::start_with_environment("staging");
    let gui = harness.client();
    let mcp = harness.mcp_client();
    let cli = harness.client_of(ClientKind::Cli);

    // The declared environment is what the shared policy reads.
    let status = migration::get_migration_status(&gui, "orders-api").expect("status loads");
    assert_eq!(status.environment, "staging");
    assert!(status.requires_authorization);

    // An unconfirmed request is refused with an actionable explanation.
    let error = migration::apply(&gui, "orders-api", false).expect_err("not confirmed");
    assert_eq!(error.code, "PermissionDenied");
    assert!(
        error.message.contains("explicit confirmation"),
        "{}",
        error.message
    );

    // A non-interactive interface can never self-confirm, however it asks.
    let error = migration::apply(&mcp, "orders-api", true).expect_err("MCP is not interactive");
    assert_eq!(error.code, "PermissionDenied");
    let error = migration::apply(&cli, "orders-api", true).expect_err("CLI is not interactive");
    assert_eq!(error.code, "PermissionDenied");

    // Bulk migration runs only through the coordinated backend workflow, and
    // only when the same policy has authorized it.
    let selection = vec!["orders-api".to_owned(), "admin-ui".to_owned()];
    let error =
        migration::apply_selected(&gui, "demo", &selection, false).expect_err("not confirmed");
    assert_eq!(error.code, "PermissionDenied");

    // The desktop's confirmation is honored for this same-user interactive peer
    // — the daemon decided that, not the frontend.
    let applied = migration::apply(&gui, "orders-api", true).expect("the confirmed apply runs");
    assert_eq!(applied.status, OperationStatus::Task);
    assert_eq!(applied.kind, "migration_apply");
    assert_eq!(settle(&gui, &applied).status, TaskStatus::Succeeded);

    let bulk = migration::apply_selected(&gui, "demo", &selection, true).expect("bulk apply");
    assert_eq!(bulk.status, OperationStatus::Workflow);
    assert_eq!(bulk.kind, "apply_migrations");
    let summary = wait_for_workflow(&gui, &workflow_of(&bulk));
    assert_eq!(
        summary.total, 1,
        "admin-ui declares no migrations and is skipped"
    );
    assert_eq!(summary.succeeded, 1);

    // Exactly two applies reached the migration executor; every refused request
    // scheduled nothing at all.
    let state = harness
        .fakes
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let applies = state
        .recorded
        .iter()
        .filter(|entry| entry.starts_with("apply "))
        .count();
    assert_eq!(applies, 2, "{:?}", state.recorded);
}

/// Migration progress, logs, and cancellation are reported honestly
/// (`tasks/021.md` §6, §7, §14; §42 items 6, 7, 9).
#[test]
fn migration_progress_logs_and_cancellation_are_reported_honestly() {
    let harness = Harness::start_with(&[], &[], &["orders-api"]);
    let client = harness.client();

    // A local target needs no confirmation, so this apply is scheduled at all.
    let outcome = migration::apply(&client, "orders-api", false).expect("the apply is scheduled");
    assert_eq!(outcome.kind, "migration_apply");
    let task_id = outcome.task_id.clone().expect("a task id");

    // Item 6: progress is observable while the migration runs.
    let deadline = Instant::now() + DEADLINE;
    let running_message = loop {
        let task = tasks::get_task(&client, &task_id).expect("the task loads");
        assert!(
            !matches!(task.status.as_str(), "succeeded" | "failed" | "cancelled"),
            "the blocked migration settled early ({})",
            task.status
        );
        if task.status == "running" {
            if let Some(message) = task.progress.message.clone() {
                break message;
            }
        }
        assert!(
            Instant::now() < deadline,
            "no running progress was ever observed"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(running_message.contains("orders-api"), "{running_message}");

    // Item 7: the migration's own log lines are readable while it runs, and
    // every line is correlated with the task.
    let during = wait_for_task_logs(&client, &task_id, |entries| {
        entries
            .iter()
            .any(|entry| entry.message.contains("is running"))
    });
    assert!(during
        .iter()
        .all(|entry| entry.task_id.as_deref() == Some(task_id.as_str())));

    // Item 9: cancellation settles the task, and nothing claims a rollback.
    let cancelled = tasks::cancel_task(&client, &task_id).expect("cancel is accepted");
    assert_eq!(cancelled.kind, "migration_apply");
    let settled = wait_for_task(&client, &task_id);
    assert_eq!(settled.status, TaskStatus::Cancelled);
    {
        let state = harness
            .fakes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert!(
            !state
                .recorded
                .iter()
                .any(|entry| entry == "apply orders-api"),
            "a cancelled apply never reports an applied migration"
        );
    }
    let after = wait_for_task_logs(&client, &task_id, |entries| {
        entries
            .iter()
            .any(|entry| entry.message.contains("cancelled"))
    });
    assert!(!after.is_empty());
}

/// The Tasks screen reads active and completed tasks from one bounded,
/// newest-first, workspace-scoped query (`tasks/021.md` §10, §12, §15, §38;
/// §42 item 8).
#[test]
fn task_listing_is_bounded_newest_first_and_workspace_scoped() {
    let harness = Harness::start();
    let client = harness.client();

    // A fresh daemon has no history to show.
    assert!(tasks::list_tasks(&client, None)
        .expect("an empty list")
        .is_empty());

    let first = build::build_service(&client, "orders-api").expect("build scheduled");
    assert_eq!(settle(&client, &first).status, TaskStatus::Succeeded);
    let second = build::build_service(&client, "admin-ui").expect("build scheduled");
    assert_eq!(settle(&client, &second).status, TaskStatus::Succeeded);
    let third = library::build_library(&client, "shared-kit").expect("build scheduled");
    assert_eq!(settle(&client, &third).status, TaskStatus::Succeeded);

    let all = tasks::list_tasks(&client, None).expect("tasks load");
    assert_eq!(all.len(), 3, "one row per observed task");
    let expected: BTreeSet<String> = [first, second, third]
        .map(|outcome| outcome.task_id.expect("a task id"))
        .into_iter()
        .collect();
    let actual: BTreeSet<String> = all.iter().map(|row| row.id.clone()).collect();
    assert_eq!(actual, expected);

    // Newest first, with the documented tie-break on the id.
    for pair in all.windows(2) {
        assert!(
            pair[0].created_at > pair[1].created_at
                || (pair[0].created_at == pair[1].created_at && pair[0].id >= pair[1].id),
            "rows are newest first"
        );
    }

    // Every row resolves its owning workspace from configuration.
    assert!(all
        .iter()
        .all(|row| row.workspace_id.as_deref() == Some("demo")));
    assert!(all.iter().all(|row| row.finished_at.is_some()));

    // The limit bounds the recent slice, and the hard cap is enforced.
    let limited = tasks::list_tasks(&client, Some(2)).expect("bounded");
    assert_eq!(limited.len(), 2);
    let single = tasks::list_tasks(&client, Some(1)).expect("bounded");
    assert_eq!(single.len(), 1);
    let capped = tasks::list_tasks(&client, Some(usize::MAX)).expect("clamped");
    assert_eq!(capped.len(), 3, "the cap never exceeds the retention");
}

/// The Health screen reflects real observations, keeps health distinct from
/// process state, and never invents a check
/// (`tasks/021.md` §17–§22; §42 item 10).
#[test]
fn health_observations_are_reflected_and_stay_distinct_from_process_state() {
    let harness = Harness::start();
    let client = harness.client();

    // Nothing has been checked yet: explicitly unknown, no fabricated checks.
    let rows = health::list_health(&client, "demo").expect("health loads");
    assert_eq!(rows.len(), 2);
    let orders = rows
        .iter()
        .find(|row| row.service_id == "orders-api")
        .expect("orders-api is listed");
    assert_eq!(orders.health_state, "unknown");
    assert!(orders.checked_at.is_none());
    assert!(orders.process_check.is_none());
    assert!(orders.tcp_check.is_none());
    assert!(orders.http_check.is_none());
    assert_eq!(orders.process_state, "not_running");

    // A health check is exactly one task that refreshes the observation.
    let outcome = mutations::check_service_health(&client, "orders-api").expect("check scheduled");
    assert_eq!(outcome.status, OperationStatus::Task);
    assert_eq!(settle(&client, &outcome).status, TaskStatus::Succeeded);

    let detail = health::get_health(&client, "orders-api", "demo").expect("detail loads");
    assert_eq!(detail.row.health_state, "healthy");
    assert!(detail.row.checked_at.is_some());
    assert_eq!(detail.checks.len(), 1, "the configured http check");
    let check = &detail.checks[0];
    assert_eq!(check.check_type, "http");
    assert_eq!(check.state, "healthy");
    assert!(check.required);
    assert_eq!(
        check.target.as_deref(),
        Some("http://localhost:8081/actuator/health")
    );
    assert!(detail.last_failure.is_none());

    // Each service reports its own configured probe type.
    let outcome = mutations::check_service_health(&client, "admin-ui").expect("check scheduled");
    assert_eq!(settle(&client, &outcome).status, TaskStatus::Succeeded);
    let admin = health::get_health(&client, "admin-ui", "demo").expect("detail loads");
    assert_eq!(admin.checks.len(), 1);
    assert_eq!(admin.checks[0].check_type, "tcp");
    assert_eq!(admin.checks[0].target.as_deref(), Some("localhost:3000"));

    // Process state and health remain distinct states (§18): a stopped process
    // does not rewrite the last observation.
    let started = mutations::start_service(&client, "orders-api").expect("start scheduled");
    assert_eq!(settle(&client, &started).status, TaskStatus::Succeeded);
    wait_for_process_state(&client, "orders-api", ProcessState::Running);
    let stopped = mutations::stop_service(&client, "orders-api").expect("stop scheduled");
    assert_eq!(settle(&client, &stopped).status, TaskStatus::Succeeded);
    wait_for_process_state(&client, "orders-api", ProcessState::Stopped);

    let detail = health::get_health(&client, "orders-api", "demo").expect("detail reloads");
    assert_eq!(detail.row.process_state, "stopped");
    assert_eq!(
        detail.row.health_state, "healthy",
        "the last observation is reported, not invented"
    );

    // The health index is the real store, and its history is bounded.
    let service = ServiceId::new("orders-api").expect("a valid service id");
    let retained = harness.core.health.history_len(&service);
    assert!(retained >= 1, "the observation was published");
    assert!(
        retained <= HEALTH_HISTORY,
        "health history stays bounded, got {retained}"
    );

    // A detail view may never cross a workspace boundary.
    let error = health::get_health(&client, "orders-api", "other-workspace")
        .expect_err("the service belongs to another workspace");
    assert_eq!(error.code, "NOT_FOUND");
}

/// Realtime subscriptions and historical queries answer from one bounded log
/// store, and secrets are scrubbed before storage — not by the GUI
/// (`tasks/021.md` §23–§29, §39; §42 item 11).
#[test]
fn realtime_and_historical_logs_are_served_from_one_bounded_store() {
    let harness = Harness::start();
    let client = harness.client();

    // Subscribe before producing anything, exactly as the Logs screen does.
    let service_id = ServiceId::new("orders-api").expect("a valid service id");
    let mut subscription = client
        .subscribe_service_logs(&service_id)
        .expect("the subscription is accepted");

    let preview = migration::preview(&client, "orders-api").expect("preview scheduled");
    let task_id = preview.task_id.clone().expect("a task id");
    assert_eq!(settle(&client, &preview).status, TaskStatus::Succeeded);

    // Item 11: the realtime stream delivers the correlated entry as it is
    // recorded; task lifecycle lines may arrive first, so follow until the SQL.
    let mut streamed_sql = None;
    let deadline = Instant::now() + DEADLINE;
    while Instant::now() < deadline && streamed_sql.is_none() {
        let received = harness.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), subscription.recv()).await
        });
        match received {
            Ok(Ok(entry)) if entry.message.contains("CREATE TABLE") => {
                streamed_sql = Some(entry);
            }
            Ok(Ok(_)) => {}
            _ => break,
        }
    }
    let streamed = streamed_sql.expect("the preview SQL reached the subscriber");
    assert_eq!(streamed.service_id, Some(service_id.clone()));
    assert_eq!(
        streamed.task_id.as_ref().map(ToString::to_string),
        Some(task_id.clone())
    );
    assert_eq!(streamed.source, LogSource::Migration);

    // Dropping the subscription unsubscribes (§34 cleanup).
    drop(subscription);

    // Task scope answers the task's own page, oldest first.
    let task_logs = logs::get_logs(
        &client,
        &logs::LogQuery {
            task_id: Some(task_id.clone()),
            ..logs::LogQuery::default()
        },
    )
    .expect("task logs");
    assert!(!task_logs.entries.is_empty());
    assert!(task_logs
        .entries
        .iter()
        .all(|entry| entry.task_id.as_deref() == Some(task_id.as_str())));

    // Service scope answers the service's page and pages forward by cursor.
    let service_logs = logs::get_logs(
        &client,
        &logs::LogQuery {
            service_id: Some("orders-api".to_owned()),
            limit: Some(1),
            ..logs::LogQuery::default()
        },
    )
    .expect("service logs");
    assert_eq!(service_logs.entries.len(), 1);
    assert!(service_logs.truncated, "the service has more than one line");
    let cursor = service_logs.next_cursor.expect("a continuation cursor");
    let next = logs::get_logs(
        &client,
        &logs::LogQuery {
            service_id: Some("orders-api".to_owned()),
            limit: Some(1),
            after: Some(cursor),
            ..logs::LogQuery::default()
        },
    )
    .expect("the next page");
    assert_eq!(next.entries.len(), 1);
    assert!(
        next.entries[0].sequence > cursor,
        "the cursor pages strictly forward"
    );

    // Workspace scope is bounded, oldest-first, and honest about truncation.
    let page = logs::get_logs(
        &client,
        &logs::LogQuery {
            workspace_id: Some("demo".to_owned()),
            limit: Some(3),
            ..logs::LogQuery::default()
        },
    )
    .expect("workspace logs");
    assert_eq!(page.entries.len(), 3);
    assert!(page.truncated, "more entries exist than the limit allows");
    let cursor = page.next_cursor.expect("a continuation cursor");
    assert_eq!(
        cursor,
        page.entries.last().expect("entries").sequence,
        "the cursor is the newest returned sequence"
    );
    for pair in page.entries.windows(2) {
        assert!(pair[0].sequence < pair[1].sequence, "oldest first");
    }

    // Severity filtering and the hard limit are enforced by the store.
    let capped = logs::get_logs(
        &client,
        &logs::LogQuery {
            limit: Some(usize::MAX),
            ..logs::LogQuery::default()
        },
    )
    .expect("logs");
    assert!(capped.entries.len() <= 1_000);
    let errors = logs::get_logs(
        &client,
        &logs::LogQuery {
            minimum_level: Some("error".to_owned()),
            limit: Some(50),
            ..logs::LogQuery::default()
        },
    )
    .expect("error logs");
    assert!(errors
        .entries
        .iter()
        .all(|entry| entry.level == "error" || entry.level == "fatal"));

    // §39: a secret recorded anywhere is scrubbed by the store before any
    // interface can read it — redaction is never a frontend concern.
    harness.core.hub.record(
        LogEntry::new(
            LogLevel::Error,
            "connection failed: DB_PASSWORD=super-secret-value",
        )
        .with_service(ServiceId::new("orders-api").expect("a valid service id")),
    );
    let observed = logs::get_logs(
        &client,
        &logs::LogQuery {
            limit: Some(1_000),
            ..logs::LogQuery::default()
        },
    )
    .expect("logs");
    assert!(observed
        .entries
        .iter()
        .any(|entry| entry.message.contains("DB_PASSWORD=***")));
    assert!(observed
        .entries
        .iter()
        .all(|entry| !entry.message.contains("super-secret-value")));
}

/// A restarted daemon restores configuration-backed state while in-memory
/// history stays gone — no stale task or health claim is replayed
/// (`tasks/021.md` §35, §36; §42 item 12).
#[test]
fn a_daemon_restart_restores_configuration_backed_state() {
    let mut harness = Harness::start();
    let client = harness.client();

    // Produce the in-memory history a restart must not replay.
    let built = build::build_service(&client, "orders-api").expect("build scheduled");
    assert_eq!(settle(&client, &built).status, TaskStatus::Succeeded);
    let checked = mutations::check_service_health(&client, "orders-api").expect("check scheduled");
    assert_eq!(settle(&client, &checked).status, TaskStatus::Succeeded);
    assert!(!tasks::list_tasks(&client, None)
        .expect("task history")
        .is_empty());
    assert!(health::list_health(&client, "demo")
        .expect("health")
        .iter()
        .any(|row| row.health_state == "healthy"));

    harness.restart();

    // A fresh client reattaches; configuration-backed state is restored.
    let client = harness.client();
    let workspaces = queries::list_workspaces(&client).expect("workspaces return");
    assert_eq!(workspaces.len(), 1);
    let services = queries::list_services(&client, "demo").expect("services return");
    assert_eq!(services.len(), 2);
    let migrations = migration::list_migrations(&client, "demo").expect("migration rows return");
    assert_eq!(migrations.len(), 1);
    assert!(
        migrations[0].latest_task.is_none(),
        "no stale task is replayed"
    );

    // In-memory history does not come back, and nothing is invented for it.
    assert!(tasks::list_tasks(&client, None)
        .expect("fresh task history")
        .is_empty());
    let health_rows = health::list_health(&client, "demo").expect("fresh health");
    assert!(health_rows
        .iter()
        .all(|row| row.health_state == "unknown" && row.checked_at.is_none()));

    // The restarted daemon serves new work normally.
    let rebuilt = build::build_service(&client, "admin-ui").expect("build scheduled");
    assert_eq!(settle(&client, &rebuilt).status, TaskStatus::Succeeded);
}

/// The GUI, the TUI, and MCP observe one authoritative daemon for tasks, health,
/// cancellation, and logs (`tasks/021.md` §43).
#[test]
fn gui_tui_and_mcp_observe_the_same_authoritative_state() {
    let harness = Harness::start_with(&[], &[], &["admin-ui"]);
    let gui = harness.client();
    let tui = harness.tui_client();
    let mcp = harness.mcp_client();

    // GUI migration task → the TUI observes the same task.
    let preview = migration::preview(&gui, "orders-api").expect("preview scheduled");
    let task_id = preview.task_id.clone().expect("a task id");
    assert_eq!(settle(&gui, &preview).status, TaskStatus::Succeeded);
    let observed = tasks::get_task(&tui, &task_id).expect("the TUI sees the task");
    assert_eq!(observed.kind, "migration_preview");
    assert_eq!(observed.status, "succeeded");

    // MCP task submission → the GUI observes the same task.
    let built = build::build_service(&mcp, "orders-api").expect("build scheduled");
    let build_id = built.task_id.clone().expect("a task id");
    assert_eq!(settle(&mcp, &built).status, TaskStatus::Succeeded);
    let listed = tasks::list_tasks(&gui, None).expect("tasks load");
    assert!(listed.iter().any(|row| row.id == build_id));

    // TUI service startup → the GUI's health view updates.
    let started = mutations::start_service(&tui, "orders-api").expect("start scheduled");
    assert_eq!(settle(&tui, &started).status, TaskStatus::Succeeded);
    wait_for_process_state(&gui, "orders-api", ProcessState::Running);
    let rows = health::list_health(&gui, "demo").expect("health loads");
    let orders = rows
        .iter()
        .find(|row| row.service_id == "orders-api")
        .expect("orders-api is listed");
    assert_eq!(orders.process_state, "running");
    assert_eq!(orders.health_state, "healthy");
    assert!(orders.checked_at.is_some());

    // GUI cancellation → MCP observes the cancellation.
    let blocked = git::fetch_repository(&gui, "admin-ui").expect("fetch scheduled");
    let blocked_id = blocked.task_id.clone().expect("a task id");
    let cancelled = tasks::cancel_task(&gui, &blocked_id).expect("cancel is accepted");
    assert_eq!(cancelled.kind, "git_fetch");
    assert_eq!(
        wait_for_task(&gui, &blocked_id).status,
        TaskStatus::Cancelled
    );
    let seen = tasks::get_task(&mcp, &blocked_id).expect("MCP sees the task");
    assert_eq!(seen.status, "cancelled");

    // Logs come from one store, so both interfaces read the same lines.
    let gui_logs = wait_for_task_logs(&gui, &task_id, |entries| !entries.is_empty());
    let mcp_logs = wait_for_task_logs(&mcp, &task_id, |entries| !entries.is_empty());
    assert_eq!(gui_logs[0].sequence, mcp_logs[0].sequence);
    assert_eq!(gui_logs[0].message, mcp_logs[0].message);
    assert!(gui_logs
        .iter()
        .all(|entry| entry.task_id.as_deref() == Some(task_id.as_str())));
}

/// Builds the real application core the daemon serves: fake task executors, an
/// in-memory runtime-state store, the real [`LogHub`], and the real
/// [`HealthStore`].
///
/// Everything a screen reads — workspaces, services, migration configuration,
/// task state, task logs, service log subscriptions, health observations — is
/// answered by that core over the real IPC protocol, so Git, build, library,
/// migration, health, and log flows settle without ever touching an
/// operating-system process, a real Git repository, a Maven/npm project, a
/// database, or the network (`tasks/020.md` §40, `tasks/021.md` §42).
///
/// Returns the [`Core`] the harness owns for the daemon's lifetime.
async fn build_core(spec: &CoreSpec, config_path: &std::path::Path) -> Core {
    let config = load_from_str(&spec.config, "<desktop-test>").expect("the fixture is valid");
    // The daemon owns the single configuration manager over the harness's file,
    // so the desktop adapter's configuration commands exercise the real
    // validate/preview/commit/reload path end to end (`tasks/023.md` §37, §41).
    let manager: Arc<dyn ConfigManager> = Arc::new(FileConfigManager::new(
        config_path.to_path_buf(),
        config.clone(),
    ));
    // Both maps are derived from configuration before it moves into the
    // application: the health probe reports the checks a real probe would run,
    // and the migration executor re-checks the declared environment exactly as
    // the real migration manager does.
    let checks = check_descriptors(&config);
    let migration_requires_authorization = migration_targets_requiring_authorization(&config);
    let store = Arc::new(InMemoryRuntimeStateStore::new());
    let fakes = Arc::new(Mutex::new(FakeState::seeded()));
    let hub = Arc::new(LogHub::new());
    let health_index = Arc::new(HealthStore::new(HEALTH_HISTORY));

    // Start/restart observe a running, healthy process; stop observes a stopped
    // process and leaves the last health observation untouched, exactly as the
    // real health index does. Health checks refresh the observation.
    let starting = Arc::new(FakeLifecycleExecutor {
        store: Arc::clone(&store),
        process: Some(ProcessState::Running),
        health: HealthState::Healthy,
        failing: spec.failing_starts.clone(),
        health_index: Arc::clone(&health_index),
        checks: Arc::clone(&checks),
        observes: true,
    });
    let stopping = Arc::new(FakeLifecycleExecutor {
        store: Arc::clone(&store),
        process: Some(ProcessState::Stopped),
        health: HealthState::Unknown,
        failing: BTreeSet::new(),
        health_index: Arc::clone(&health_index),
        checks: Arc::clone(&checks),
        observes: false,
    });
    let health_check = Arc::new(FakeLifecycleExecutor {
        store: Arc::clone(&store),
        process: None,
        health: HealthState::Healthy,
        failing: BTreeSet::new(),
        health_index: Arc::clone(&health_index),
        checks,
        observes: true,
    });

    let git_executor = Arc::new(FakeGitExecutor {
        state: Arc::clone(&fakes),
        blocking: spec.blocking.clone(),
    });
    let build_executor = Arc::new(FakeBuildExecutor {
        state: Arc::clone(&fakes),
        failing: spec.failing_builds.clone(),
        blocking: spec.blocking.clone(),
    });
    let verify_executor = Arc::new(FakeArtifactExecutor {
        state: Arc::clone(&fakes),
    });
    let migration_executor = Arc::new(FakeMigrationExecutor {
        state: Arc::clone(&fakes),
        hub: Arc::clone(&hub),
        blocking: spec.blocking.clone(),
        requires_authorization: migration_requires_authorization,
    });

    let engine = Arc::new(
        TaskEngine::builder()
            .max_parallel_tasks(4)
            .executor(TaskKind::StartService, starting.clone())
            .executor(TaskKind::RestartService, starting)
            .executor(TaskKind::StopService, stopping)
            .executor(TaskKind::HealthCheck, health_check.clone())
            .executor(TaskKind::ProcessHealthCheck, health_check.clone())
            .executor(TaskKind::TcpHealthCheck, health_check.clone())
            .executor(TaskKind::HttpHealthCheck, health_check.clone())
            .executor(TaskKind::ReadinessCheck, health_check)
            .executor(TaskKind::GitSync, git_executor.clone())
            .executor(TaskKind::GitFetch, git_executor.clone())
            .executor(TaskKind::GitPull, git_executor.clone())
            .executor(TaskKind::GitCheckout, git_executor)
            .executor(TaskKind::BuildService, build_executor.clone())
            .executor(TaskKind::BuildLibrary, build_executor.clone())
            .executor(TaskKind::LibraryInstall, build_executor)
            .executor(TaskKind::ArtifactVerification, verify_executor)
            .executor(TaskKind::MigrationValidate, migration_executor.clone())
            .executor(TaskKind::MigrationStatus, migration_executor.clone())
            .executor(TaskKind::MigrationPending, migration_executor.clone())
            .executor(TaskKind::MigrationPreview, migration_executor.clone())
            .executor(TaskKind::MigrationApply, migration_executor.clone())
            .executor(TaskKind::Liquibase, migration_executor)
            .build()
            .expect("a Tokio runtime is active"),
    );
    // Task events become correlated log entries in the hub, exactly as
    // `spawn_task_ingestor` does in production, so task logs and service log
    // subscriptions are populated over IPC. The spawn happens inside the
    // runtime context the harness drives `build_core` on.
    let ingestor = spawn_task_ingestor(Arc::clone(&hub), engine.subscribe());

    // The read-only Git/build/library/migration queries reach the fake ports or
    // validated configuration directly, exactly as they reach the real adapters
    // in production; the mutating operations go through the fake task executors
    // above.
    let application = Application::new(config, engine.clone(), store, hub.clone())
        .with_git_manager(Arc::new(FakeGitManager {
            state: Arc::clone(&fakes),
        }))
        .with_artifact_inspector(Arc::new(FakeArtifactInspector {
            state: Arc::clone(&fakes),
        }))
        .with_build_capabilities(Arc::new(FakeBuildCapabilityIndex))
        .with_health_index(health_index.clone())
        .with_configuration(manager)
        .with_repository_inspector(Arc::new(FakeRepositoryInspector));

    Core {
        application,
        engine,
        fakes,
        hub,
        health: health_index,
        ingestor,
    }
}

/// One configured health check, described the way a completed probe reports it.
struct CheckDescriptor {
    /// The probe type label (`process`, `tcp`, `http`, …).
    check_type: String,
    /// What the probe targets (`localhost:8081`, an HTTP URL), if anything.
    target: Option<String>,
    /// Whether the check is required for the service to be healthy.
    required: bool,
}

/// Reads every service's configured health checks from configuration.
fn check_descriptors(config: &Config) -> Arc<BTreeMap<String, Vec<CheckDescriptor>>> {
    let mut descriptors: BTreeMap<String, Vec<CheckDescriptor>> = BTreeMap::new();
    for workspace in &config.workspaces {
        for service in &workspace.workspace.services {
            let checks = service
                .health_checks
                .iter()
                .map(|check| {
                    let (check_type, target) = match &check.kind {
                        HealthCheckKind::Process => (CHECK_PROCESS.to_owned(), None),
                        HealthCheckKind::Port(port) => (
                            CHECK_TCP.to_owned(),
                            Some(format!("localhost:{}", port.value())),
                        ),
                        HealthCheckKind::Http(http) => {
                            (CHECK_HTTP.to_owned(), Some(http.url.clone()))
                        }
                        HealthCheckKind::Other(name) => (name.clone(), None),
                    };
                    CheckDescriptor {
                        check_type,
                        target,
                        required: check.required,
                    }
                })
                .collect();
            descriptors.insert(service.id.to_string(), checks);
        }
    }
    Arc::new(descriptors)
}

/// The services whose declared migration environment requires authorization.
///
/// A mutating operation against one of these targets reaches the database only
/// when the caller explicitly authorized it (`tasks/021.md` §8, §9).
fn migration_targets_requiring_authorization(config: &Config) -> BTreeSet<String> {
    config
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.liquibase.iter())
        .filter(|(_, liquibase)| {
            liquibase.enabled && liquibase.database.environment.requires_authorization()
        })
        .map(|(service_id, _)| service_id.to_string())
        .collect()
}

/// The test-only "infrastructure" state the fake ports and executors share.
///
/// It stands in for the real Git manager, the real artifact inspector, and the
/// real build adapter, so the desktop adapter's IPC flow can be exercised
/// end-to-end without any real Git, Maven, npm, filesystem, or network access.
struct FakeState {
    /// Observed Git status per repository (keyed by service or library id).
    git: BTreeMap<String, RepositoryStatus>,
    /// Branches per repository.
    branches: BTreeMap<String, Vec<BranchInfo>>,
    /// The version the inspector reports a consumer declares, or `None` when
    /// the consumer does not declare the artifact.
    declared_version: Option<Version>,
    /// Whether the inspector reports the artifact present.
    artifact_present: bool,
    /// The operations the fake executors observed, in order, for assertions.
    recorded: Vec<String>,
}

impl FakeState {
    /// A state where both fixture services and the library are checked out on
    /// `main`, clean, and up to date with `origin/main`, and the library's
    /// artifact is installed and consumed at the aligned version.
    fn seeded() -> Self {
        let mut git = BTreeMap::new();
        let mut branches = BTreeMap::new();
        for name in ["orders-api", "admin-ui", "shared-kit"] {
            git.insert(name.to_owned(), available_status("main"));
            branches.insert(
                name.to_owned(),
                vec![
                    BranchInfo::new("main", BranchKind::Local, true),
                    BranchInfo::new("origin/main", BranchKind::Remote, false),
                ],
            );
        }
        Self {
            git,
            branches,
            declared_version: Some(Version::new("1.8.0-SNAPSHOT").expect("a valid version")),
            artifact_present: true,
            recorded: Vec::new(),
        }
    }

    /// Returns the observed status of `name`, defaulting to a clean `main`.
    fn status(&self, name: &str) -> RepositoryStatus {
        self.git
            .get(name)
            .cloned()
            .unwrap_or_else(|| available_status("main"))
    }
}

/// A clean, available working copy on `branch`, tracking `origin/<branch>`.
fn available_status(branch: &str) -> RepositoryStatus {
    RepositoryStatus {
        lifecycle: RepositoryLifecycle::Available,
        branch: Some(Branch::new(branch).expect("a valid branch")),
        commit: Some("0123456789abcdef".to_owned()),
        upstream: Some(format!("origin/{branch}")),
        ahead: Some(0),
        behind: Some(0),
        ..RepositoryStatus::default()
    }
}

/// A [`GitManager`] over the shared [`FakeState`].
///
/// Read-only queries (`status`, `branches`) answer from the state exactly as
/// the real manager answers from a working copy. Mutating methods are never
/// reached by the engine in this harness (the fake task executors run instead),
/// but are implemented so the application can be constructed with a real
/// manager.
struct FakeGitManager {
    /// The shared fake state.
    state: Arc<Mutex<FakeState>>,
}

#[async_trait]
impl GitManager for FakeGitManager {
    async fn status(
        &self,
        repository: &ResolvedRepository,
        _options: &GitOptions,
    ) -> Result<RepositoryStatus, GitError> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        Ok(state.status(&repository.name))
    }

    async fn branches(
        &self,
        repository: &ResolvedRepository,
        _options: &GitOptions,
    ) -> Result<Vec<BranchInfo>, GitError> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        Ok(state
            .branches
            .get(&repository.name)
            .cloned()
            .unwrap_or_else(|| vec![BranchInfo::new("main", BranchKind::Local, true)]))
    }

    async fn diff(
        &self,
        _repository: &ResolvedRepository,
        _options: &DiffOptions,
    ) -> Result<RepoDiff, GitError> {
        Err(GitError::internal(
            "the desktop Git screen never requests a diff",
        ))
    }

    async fn fetch(
        &self,
        repository: &ResolvedRepository,
        _options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        Ok(GitOperationResult::succeeded(
            repository.name.clone(),
            GitOperation::Fetch,
        ))
    }

    async fn pull(
        &self,
        repository: &ResolvedRepository,
        _options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        Ok(GitOperationResult::succeeded(
            repository.name.clone(),
            GitOperation::Pull,
        ))
    }

    async fn checkout(
        &self,
        repository: &ResolvedRepository,
        _branch: &Branch,
        _options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        Ok(GitOperationResult::succeeded(
            repository.name.clone(),
            GitOperation::Checkout,
        ))
    }

    async fn clone(
        &self,
        repository: &ResolvedRepository,
        _options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        Ok(GitOperationResult::succeeded(
            repository.name.clone(),
            GitOperation::Clone,
        ))
    }

    async fn sync(
        &self,
        repository: &ResolvedRepository,
        _options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        Ok(GitOperationResult::succeeded(
            repository.name.clone(),
            GitOperation::Sync,
        ))
    }

    async fn reset_hard(
        &self,
        repository: &ResolvedRepository,
        _options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        Ok(GitOperationResult::succeeded(
            repository.name.clone(),
            GitOperation::ResetHard,
        ))
    }

    async fn clean(
        &self,
        repository: &ResolvedRepository,
        _options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        Ok(GitOperationResult::succeeded(
            repository.name.clone(),
            GitOperation::Clean,
        ))
    }

    async fn discard_local_changes(
        &self,
        repository: &ResolvedRepository,
        _options: &GitOptions,
    ) -> Result<GitOperationResult, GitError> {
        Ok(GitOperationResult::succeeded(
            repository.name.clone(),
            GitOperation::DiscardChanges,
        ))
    }
}

/// An [`ArtifactInspector`] over the shared [`FakeState`].
struct FakeArtifactInspector {
    /// The shared fake state.
    state: Arc<Mutex<FakeState>>,
}

#[async_trait]
impl ArtifactInspector for FakeArtifactInspector {
    async fn locate(
        &self,
        artifact: &ResolvedArtifact,
    ) -> Result<ArtifactPresence, ArtifactInspectionError> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.artifact_present {
            let path = AbsolutePath::new(format!(
                "/tmp/fake-m2/repository/{}",
                artifact.coordinate.repository_path()
            ))
            .expect("a valid fake path");
            Ok(ArtifactPresence::Found {
                path,
                size_bytes: Some(4096),
            })
        } else {
            Ok(ArtifactPresence::Missing {
                expected_path: None,
            })
        }
    }

    async fn dependency_version(
        &self,
        _dependency: &ResolvedDependency,
    ) -> Result<DependencyVersionInspection, ArtifactInspectionError> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        Ok(match &state.declared_version {
            Some(version) => DependencyVersionInspection::Declared {
                version: version.clone(),
            },
            None => DependencyVersionInspection::NotDeclared,
        })
    }
}

/// A [`BuildCapabilityIndex`] over the real Maven/npm capability shape.
///
/// It reports the same operations the real build providers declare, so the
/// desktop adapter's capability projection can be exercised without pulling the
/// build crate into this test binary (`tasks/020.md` §13, §40).
struct FakeBuildCapabilityIndex;

impl BuildCapabilityIndex for FakeBuildCapabilityIndex {
    fn capabilities(&self, system: BuildSystem) -> Option<BuildCapabilities> {
        let operations = match system {
            BuildSystem::Maven => vec![
                BuildOperation::Clean,
                BuildOperation::Build,
                BuildOperation::Test,
                BuildOperation::InstallDependencies,
                BuildOperation::PublishLocalArtifact,
            ],
            BuildSystem::Npm => vec![BuildOperation::Build, BuildOperation::InstallDependencies],
        };
        Some(BuildCapabilities {
            build_system: system,
            operations,
        })
    }
}

/// A test-only Git executor.
///
/// It decodes the workspace-aware [`ResolvedRepository`] from the task's
/// parameters (exactly as the real Git executor does) and reflects the
/// operation into the shared [`FakeState`], so a subsequent `get_git_status`
/// observes the same working copy the sync/checkout produced. An entity named
/// in `blocking` waits for cancellation instead of completing, so cancellation
/// can be exercised without a long-running process.
struct FakeGitExecutor {
    /// The shared fake state.
    state: Arc<Mutex<FakeState>>,
    /// Repository ids whose Git task must block until cancelled.
    blocking: BTreeSet<String>,
}

#[async_trait]
impl TaskExecutor for FakeGitExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        let repository = decode_repository(&context)?;
        if self.blocking.contains(&repository.name) {
            context.cancelled().await;
            return Err(TaskExecutorError::Cancelled);
        }

        let kind = context.kind().clone();
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let default_branch = repository.default_branch.to_string();
            state
                .git
                .entry(repository.name.clone())
                .or_insert_with(|| available_status(&default_branch));
            {
                let entry = state
                    .git
                    .get_mut(&repository.name)
                    .expect("just inserted or present");
                match kind {
                    TaskKind::GitCheckout => {
                        if let Some(branch) = context.parameter(parameter::BRANCH) {
                            entry.branch = Branch::new(branch).ok();
                        }
                    }
                    TaskKind::GitSync => {
                        entry.branch = Some(repository.default_branch.clone());
                        entry.ahead = Some(0);
                        entry.behind = Some(0);
                    }
                    TaskKind::GitPull => entry.behind = Some(0),
                    TaskKind::GitFetch => {}
                    _ => {}
                }
            }
            state
                .recorded
                .push(format!("{} {}", kind.label(), repository.name));
        }

        let _ = context
            .progress()
            .message(format!("{} {}", kind.label(), repository.name));
        Ok(TaskOutcome::succeeded(Some(format!(
            "{} completed for '{}'",
            kind.label(),
            repository.name
        ))))
    }
}

/// A test-only build/library executor.
///
/// It decodes the build target's name from the task parameters and reports a
/// structured failure for an entity named in `failing`, so bulk build
/// partial-failure handling can be asserted. An entity named in `blocking`
/// waits for cancellation.
struct FakeBuildExecutor {
    /// The shared fake state.
    state: Arc<Mutex<FakeState>>,
    /// Entity ids whose build task must fail.
    failing: BTreeSet<String>,
    /// Entity ids whose build task must block until cancelled.
    blocking: BTreeSet<String>,
}

#[async_trait]
impl TaskExecutor for FakeBuildExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        let name = context
            .parameter(build_parameter::NAME)
            .unwrap_or_default()
            .to_owned();
        let operation = context
            .parameter(build_parameter::OPERATION)
            .unwrap_or("build")
            .to_owned();
        if self.blocking.contains(&name) {
            context.cancelled().await;
            return Err(TaskExecutorError::Cancelled);
        }
        if self.failing.contains(&name) {
            return Err(TaskExecutorError::failed(
                "BUILD_COMMAND_FAILED",
                format!("the {operation} command failed for '{name}'"),
            ));
        }
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .recorded
            .push(format!("{operation} {name}"));
        let _ = context
            .progress()
            .message(format!("running {operation} for '{name}'"));
        Ok(TaskOutcome::succeeded(Some(format!(
            "{operation} completed for '{name}'"
        ))))
    }
}

/// A test-only artifact-verification executor.
///
/// It decodes the [`ResolvedArtifact`] from the task parameters and mirrors the
/// real executor: a present artifact succeeds, a missing one is a structured
/// `ARTIFACT_NOT_FOUND` failure.
struct FakeArtifactExecutor {
    /// The shared fake state.
    state: Arc<Mutex<FakeState>>,
}

#[async_trait]
impl TaskExecutor for FakeArtifactExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        let artifact = ResolvedArtifact::from_parameters(context.parameters())
            .map_err(|error| TaskExecutorError::internal(error.to_string()))?;
        let present = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .artifact_present;
        if present {
            let _ = context
                .progress()
                .message(format!("verified '{}'", artifact.coordinate));
            Ok(TaskOutcome::succeeded(Some(format!(
                "artifact '{}' is present",
                artifact.coordinate
            ))))
        } else {
            Err(TaskExecutorError::failed(
                "ARTIFACT_NOT_FOUND",
                format!("artifact '{}' was not found", artifact.coordinate),
            ))
        }
    }
}

/// Decodes a repository from a task's parameters, as the real executor does.
fn decode_repository(context: &TaskContext) -> Result<ResolvedRepository, TaskExecutorError> {
    ResolvedRepository::from_parameters(context.parameters())
        .map_err(|error| TaskExecutorError::failed("INVALID_TARGET", error.to_string()))
}

/// A test-only lifecycle and health executor.
///
/// It decodes the workspace-aware [`ResolvedServiceTarget`] from the task's
/// parameters (exactly as the real runtime executors do) and records the
/// resulting process/health observation into the shared in-memory store. When
/// it `observes`, it also publishes the health observation — with the
/// service's configured checks — into the real [`HealthStore`] the Health
/// screen reads, exactly as the health adapter publishes a completed probe. A
/// service named in `failing` fails the task with a structured runtime code, so
/// bulk partial-failure handling can be asserted.
struct FakeLifecycleExecutor {
    /// The store the application reads observed state from.
    store: Arc<InMemoryRuntimeStateStore>,
    /// The process state to record, or `None` to leave it unchanged.
    process: Option<ProcessState>,
    /// The health state to record.
    health: HealthState,
    /// Service ids whose lifecycle operation must fail.
    failing: BTreeSet<String>,
    /// The health index the observation is published to.
    health_index: Arc<HealthStore>,
    /// The configured checks of each service, in probe-report shape.
    checks: Arc<BTreeMap<String, Vec<CheckDescriptor>>>,
    /// Whether this executor publishes a health observation.
    ///
    /// Stopping a service does not: the index keeps the last observation it
    /// actually made (`tasks/021.md` §18, §22).
    observes: bool,
}

impl FakeLifecycleExecutor {
    /// Publishes one health observation into the real health index.
    fn record_observation(&self, target: &ResolvedServiceTarget) {
        let checked_at = Timestamp::now();
        let outcomes: Vec<HealthCheckOutcome> = self
            .checks
            .get(target.service_id.as_str())
            .map(|descriptors| {
                descriptors
                    .iter()
                    .map(|descriptor| HealthCheckOutcome {
                        check_type: descriptor.check_type.clone(),
                        target: descriptor.target.clone(),
                        required: descriptor.required,
                        status: self.health,
                        duration_ms: 1,
                        message: None,
                        checked_at,
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.health_index.record(
            target.workspace_id.clone(),
            target.service_id.clone(),
            self.health,
            outcomes,
            checked_at,
        );
    }
}

#[async_trait]
impl TaskExecutor for FakeLifecycleExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        let target = decode_target(&context)?;
        if self.failing.contains(target.service_id.as_str()) {
            // Mirror a real runtime executor: a failed start leaves the service
            // in a terminal `failed` process state rather than `starting`.
            let state = self
                .store
                .service_state(&target.service_id)
                .unwrap_or_default()
                .with_process(ProcessState::Failed);
            self.store.set_service_state(&target.service_id, state);
            return Err(TaskExecutorError::failed(
                "RUNTIME_PORT_ALREADY_IN_USE",
                format!("port for '{}' is already in use", target.service_id),
            ));
        }
        let mut state = self
            .store
            .service_state(&target.service_id)
            .unwrap_or_default();
        if let Some(process) = self.process {
            state = state.with_process(process);
        }
        state = state.with_health(self.health);
        self.store.set_service_state(&target.service_id, state);
        if self.observes {
            self.record_observation(&target);
        }
        let _ = context.progress().message(format!(
            "applied lifecycle state to '{}'",
            target.service_id
        ));
        Ok(TaskOutcome::succeeded(Some(format!(
            "service '{}' updated",
            target.service_id
        ))))
    }
}

/// Decodes a lifecycle target from a task's parameters, as the real executors do.
fn decode_target(context: &TaskContext) -> Result<ResolvedServiceTarget, TaskExecutorError> {
    ResolvedServiceTarget::from_parameters(context.parameters())
        .map_err(|error| TaskExecutorError::failed("INVALID_TARGET", error.to_string()))
}

/// A test-only migration executor.
///
/// It decodes the [`ResolvedMigrationTarget`] from the task parameters exactly
/// as the real migration manager does, and then:
///
/// * re-checks the declared environment, so a hand-crafted mutating task that
///   reached the executor without the required authorization still fails;
/// * reflects the operation into the shared [`FakeState`], so a later
///   `list_migrations` sees the task it ran;
/// * writes the SQL a provider would print for a preview into the log hub, so
///   SQL preview and log inspection can be observed over IPC.
///
/// An entity named in `blocking` waits for cancellation, so a long-running
/// migration can be cancelled. No database, migration provider, or network is
/// ever touched (`tasks/021.md` §42 items 4–7, §5 of the harness contract).
struct FakeMigrationExecutor {
    /// The shared fake state.
    state: Arc<Mutex<FakeState>>,
    /// The log hub the preview SQL is recorded into.
    hub: Arc<LogHub>,
    /// Service ids whose migration task must block until cancelled.
    blocking: BTreeSet<String>,
    /// Services whose declared database environment requires authorization.
    requires_authorization: BTreeSet<String>,
}

#[async_trait]
impl TaskExecutor for FakeMigrationExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        let target = ResolvedMigrationTarget::from_parameters(context.parameters())
            .map_err(|error| TaskExecutorError::failed("INVALID_TARGET", error.to_string()))?;
        let service = target.service_id.to_string();
        let operation = target.operation;

        // The manager re-checks authorization at execution time; a task that
        // arrives without it never reaches the database.
        if operation.is_mutating()
            && self.requires_authorization.contains(service.as_str())
            && !target.non_local_authorized
        {
            return Err(TaskExecutorError::failed(
                "MIGRATION_AUTHORIZATION_REQUIRED",
                format!("applying migrations for '{service}' requires explicit confirmation"),
            ));
        }

        if self.blocking.contains(service.as_str()) {
            let _ = context.progress().message(format!(
                "{} for '{}' is running",
                operation.label(),
                service
            ));
            context.cancelled().await;
            return Err(TaskExecutorError::Cancelled);
        }

        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .recorded
            .push(format!("{} {service}", operation.label()));
        let _ = context
            .progress()
            .message(format!("{} for '{service}'", operation.label()));

        if matches!(operation, MigrationOperation::Preview) {
            self.hub.record(
                LogEntry::new(LogLevel::Info, preview_sql(&service))
                    .with_task(context.task_id().clone())
                    .with_service(target.service_id.clone())
                    .with_source(LogSource::Migration),
            );
        }

        Ok(TaskOutcome::succeeded(Some(format!(
            "{} completed for '{service}'",
            operation.label()
        ))))
    }
}

/// The SQL a provider would print for a read-only preview of `service`.
fn preview_sql(service: &str) -> String {
    format!(
        "-- service-orchestrator preview for {service}\n\
         CREATE TABLE {service}_audit (\n    id BIGSERIAL PRIMARY KEY,\n    recorded_at TIMESTAMP NOT NULL\n);\n"
    )
}

/// Waits until a service reports `expected` as its process state.
fn wait_for_process_state(client: &RemoteClient, service: &str, expected: ProcessState) {
    let id = ServiceId::new(service).expect("a valid service id");
    let deadline = Instant::now() + DEADLINE;
    loop {
        let status = client.get_service_status(&id).expect("status answers");
        if status.process_state == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "service '{service}' never reached {expected:?} (last: {:?})",
            status.process_state
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Waits until a task reaches a terminal state and returns it.
fn wait_for_task(client: &RemoteClient, task_id: &str) -> Task {
    let id = service_orchestrator_domain::TaskId::new(task_id).expect("a valid task id");
    let deadline = Instant::now() + DEADLINE;
    loop {
        let task = client.get_task(&id).expect("task answers");
        if task.status.is_terminal() {
            return task;
        }
        assert!(
            Instant::now() < deadline,
            "task '{task_id}' never settled (last: {:?})",
            task.status
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Waits until every step of a workflow settles, then returns its summary.
fn wait_for_workflow(client: &RemoteClient, workflow_id: &WorkflowId) -> BulkOperationSummary {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let summary = client
            .summarize_workflow(workflow_id)
            .expect("workflow answers");
        if summary.is_complete() {
            return summary;
        }
        assert!(
            Instant::now() < deadline,
            "workflow '{workflow_id}' never settled (pending: {})",
            summary.pending
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Waits until a single-task outcome's task settles, and returns it.
fn settle(client: &RemoteClient, outcome: &OperationOutcome) -> Task {
    let task_id = outcome.task_id.as_deref().expect("a task was scheduled");
    wait_for_task(client, task_id)
}

/// Waits until a task's log page matches `expect`, then returns the entries.
///
/// The task ingestor runs concurrently with task settlement, so a test that
/// asserts on task logs polls instead of assuming the entry is already there.
fn wait_for_task_logs(
    client: &RemoteClient,
    task_id: &str,
    expect: impl Fn(&[LogEntryView]) -> bool,
) -> Vec<LogEntryView> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let page = tasks::get_task_logs(client, task_id, &tasks::TaskLogsQuery::default())
            .expect("task logs answer");
        if expect(&page.entries) {
            return page.entries;
        }
        assert!(
            Instant::now() < deadline,
            "the logs of task '{task_id}' never matched"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Extracts the workflow id from a workflow-shaped outcome.
fn workflow_of(outcome: &OperationOutcome) -> WorkflowId {
    let id = outcome
        .workflow_id
        .as_deref()
        .expect("a workflow was scheduled");
    WorkflowId::new(id).expect("a valid workflow id")
}

/// Returns a unique temporary directory for one harness.
fn unique_root() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("so-desktop-{}-{nanos}", std::process::id()))
}

// -- desktop workflow management (`tasks/022.md` §42) ------------------------

/// A prepare-environment input scoped to the fixture's `demo` workspace.
fn prepare_input() -> workflow::PrepareEnvironmentInput {
    workflow::PrepareEnvironmentInput {
        workspace_id: "demo".to_owned(),
        ..workflow::PrepareEnvironmentInput::default()
    }
}

/// Polls a workflow's aggregated status until `expect` holds.
fn wait_for_status(
    client: &RemoteClient,
    workflow_id: &str,
    expect: impl Fn(&WorkflowStatusView) -> bool,
) -> WorkflowStatusView {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let status = workflow::workflow_status(client, workflow_id).expect("status loads");
        if expect(&status) {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "workflow '{workflow_id}' never reached the expected state (state: {})",
            status.state
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The GUI reconnects to a running daemon and loads the configured profiles and
/// groups, with membership scoped to its owning workspace (`tasks/022.md` §42
/// items 1, 2, 3).
#[test]
fn profiles_and_groups_load_and_are_workspace_scoped() {
    let harness = Harness::start_with_spec(CoreSpec::with_second_workspace());
    let client = harness.client();

    let profiles = workflow::list_profiles(&client, "demo").expect("profiles load");
    assert_eq!(profiles.len(), 1);
    let full_stack = &profiles[0];
    assert_eq!(full_stack.id, "full-stack");
    assert_eq!(full_stack.display_name, "Full Stack");
    assert_eq!(full_stack.workspace_id, "demo");
    assert_eq!(full_stack.services, vec!["orders-api", "admin-ui"]);
    assert_eq!(full_stack.resolved_services, vec!["orders-api", "admin-ui"]);
    assert_eq!(
        workflow::get_profile(&client, "demo", "full-stack").expect("profile loads"),
        *full_stack
    );

    let groups = workflow::list_groups(&client, "demo").expect("groups load");
    assert_eq!(groups.len(), 1);
    let core = &groups[0];
    assert_eq!(core.id, "core");
    assert_eq!(core.display_name, "Core");
    assert_eq!(core.workspace_id, "demo");
    assert_eq!(core.services, vec!["orders-api", "admin-ui"]);
    assert_eq!(
        workflow::get_group(&client, "demo", "core").expect("group loads"),
        *core
    );

    // The second workspace owns neither the fixture profile nor the fixture
    // group: scoping comes from configuration, never from the GUI.
    assert!(workflow::list_profiles(&client, "other")
        .expect("profiles load")
        .is_empty());
    assert!(workflow::list_groups(&client, "other")
        .expect("groups load")
        .is_empty());
    assert_eq!(
        workflow::get_profile(&client, "other", "full-stack")
            .expect_err("the profile belongs to demo")
            .code,
        "NOT_FOUND"
    );
    assert_eq!(
        workflow::get_group(&client, "other", "core")
            .expect_err("the group belongs to demo")
            .code,
        "NOT_FOUND"
    );

    // Identifiers are validated before any lookup happens.
    assert_eq!(
        workflow::list_profiles(&client, "")
            .expect_err("an empty workspace is invalid")
            .code,
        "INVALID_INPUT"
    );
    assert_eq!(
        workflow::get_profile(&client, "demo", "bad id")
            .expect_err("spaces are invalid")
            .code,
        "INVALID_INPUT"
    );
    assert_eq!(
        workflow::get_group(&client, "demo", "bad id")
            .expect_err("spaces are invalid")
            .code,
        "INVALID_INPUT"
    );
    assert_eq!(
        workflow::get_profile(&client, "demo", "missing")
            .expect_err("an unknown profile is a structured error")
            .code,
        "ProfileNotFound"
    );
    assert_eq!(
        workflow::get_group(&client, "demo", "missing")
            .expect_err("an unknown group is a structured error")
            .code,
        "GroupNotFound"
    );
}

/// The read-only plan resolves dependencies through the core's planner and
/// schedules nothing (`tasks/022.md` §42 items 4, 5, 6).
#[test]
fn a_plan_resolves_dependencies_without_scheduling_anything() {
    let harness = Harness::start();
    let client = harness.client();

    // Selecting the dependent pulls in the required dependency and its library.
    let mut input = prepare_input();
    input.services = vec!["admin-ui".to_owned()];
    let plan = workflow::plan(&client, &input).expect("a plan is generated");
    assert_eq!(plan.workspace_id, "demo");
    assert_eq!(plan.selected_services, vec!["admin-ui"]);
    assert_eq!(plan.resolved_services, vec!["orders-api", "admin-ui"]);
    assert_eq!(plan.included_dependencies, vec!["orders-api"]);
    assert_eq!(plan.required_libraries, vec!["shared-kit"]);
    assert!(plan.step_count > 0);
    assert!(plan.phases.iter().any(|phase| phase.kind == "git-sync"));
    assert_eq!(plan.target_branch, None, "no branch is forced");
    assert!(!plan.requires_destructive_authorization);

    // Planning is read-only: no task runs, and no workflow enters history.
    assert!(client.list_running_tasks().is_empty());
    assert!(workflow::list_workflows(&client, "demo", None)
        .expect("history loads")
        .is_empty());

    // A group selects its members through the same planner.
    let mut grouped = prepare_input();
    grouped.group = Some("core".to_owned());
    let group_plan = workflow::plan(&client, &grouped).expect("a plan is generated");
    assert_eq!(group_plan.resolved_services, vec!["orders-api", "admin-ui"]);
    assert_eq!(group_plan.target_branch, None);

    // A profile plus an explicit branch is previewed without any checkout.
    let mut branched = prepare_input();
    branched.profile = Some("full-stack".to_owned());
    branched.branch = Some("main".to_owned());
    let branch_plan = workflow::plan(&client, &branched).expect("a plan is generated");
    assert_eq!(branch_plan.target_branch.as_deref(), Some("main"));
    assert!(client.list_running_tasks().is_empty());

    // A plan is never authorization: invalid input is still rejected, and more
    // than one selection source is refused rather than guessed.
    let mut invalid = prepare_input();
    invalid.services = vec!["orders-api".to_owned()];
    invalid.profile = Some("full-stack".to_owned());
    assert_eq!(
        workflow::plan(&client, &invalid)
            .expect_err("two selection sources are rejected")
            .code,
        "INVALID_INPUT"
    );
}

/// A workflow executes through the shared core, its aggregated results are
/// inspectable, and it lands in bounded, workspace-scoped history
/// (`tasks/022.md` §42 items 7, 11).
#[test]
fn a_workflow_executes_and_reports_its_results() {
    let harness = Harness::start();
    let client = harness.client();

    let mut input = prepare_input();
    input.services = vec!["orders-api".to_owned()];
    input.build = true;
    input.start = true;

    let outcome = workflow::execute(&client, &input).expect("the workflow is scheduled");
    assert_eq!(outcome.status, OperationStatus::Workflow);
    assert_eq!(outcome.kind, "prepare_environment");
    let workflow_id = workflow_of(&outcome);

    let summary = wait_for_workflow(&client, &workflow_id);
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.pending, 0);
    assert!(summary.succeeded > 0);

    let status = workflow::workflow_status(&client, workflow_id.as_str()).expect("status loads");
    assert_eq!(status.workflow_id, workflow_id.as_str());
    assert_eq!(status.kind, "prepare_environment");
    assert_eq!(status.state, "succeeded");
    assert_eq!(status.total, summary.total);
    assert_eq!(status.failed, 0);
    assert_eq!(status.cancelled, 0);
    assert_eq!(status.pending, 0);
    assert!(status.started_at.is_some());
    assert!(status.finished_at.is_some());
    assert!(status.failures.is_empty());

    // Progress is reported per phase, in the core's canonical order, and every
    // step of a settled workflow has left the pending count.
    let phases: Vec<&str> = status
        .phases
        .iter()
        .map(|phase| phase.phase.as_str())
        .collect();
    assert!(phases.contains(&"git-sync"));
    assert!(phases.contains(&"service-build"));
    assert!(phases.contains(&"service-start"));
    assert!(phases.contains(&"health-check"));
    assert!(status.phases.iter().all(|phase| phase.pending == 0));

    // The workflow is on the workspace-scoped history list.
    let history = workflow::list_workflows(&client, "demo", None).expect("history loads");
    assert_eq!(history.len(), 1);
    let row = &history[0];
    assert_eq!(row.workflow_id, workflow_id.as_str());
    assert_eq!(row.workspace_id, "demo");
    assert_eq!(row.state, "succeeded");
    assert_eq!(row.total, status.total);
    assert_eq!(row.finished_at, status.finished_at);

    // Another workspace has no rows of its own: history is workspace-scoped.
    assert!(workflow::list_workflows(&client, "other", None)
        .expect("history loads")
        .is_empty());
}

/// A workflow where one step fails reports partial success with the structured
/// failure, never a false "succeeded" (`tasks/022.md` §42 item 9).
#[test]
fn a_workflow_reports_partial_failure_per_step() {
    let harness = Harness::start_with(&[], &["admin-ui"], &[]);
    let client = harness.client();

    let mut input = prepare_input();
    input.services = vec!["admin-ui".to_owned()];
    input.build = true;

    let outcome = workflow::execute(&client, &input).expect("the workflow is scheduled");
    let workflow_id = workflow_of(&outcome);
    let summary = wait_for_workflow(&client, &workflow_id);
    assert_eq!(summary.failed, 1);
    assert!(summary.succeeded > 0, "the dependency build still succeeds");

    let status = workflow::workflow_status(&client, workflow_id.as_str()).expect("status loads");
    assert_eq!(status.state, "partially_succeeded");
    assert_eq!(status.failed, 1);
    assert_eq!(status.failures.len(), 1);
    let failure = &status.failures[0];
    assert_eq!(failure.kind, "build_service");
    assert_eq!(failure.code, "BUILD_COMMAND_FAILED");
    // A workflow step is scheduled with a workflow target, so the failed task
    // carries no service binding of its own; the executor's message names it.
    assert!(
        failure.message.contains("admin-ui"),
        "got: {}",
        failure.message
    );
}

/// A workflow in flight is observable while it runs and can be cancelled, with
/// completed work preserved and active work stopped (`tasks/022.md` §42 items 8,
/// 10).
#[test]
fn a_running_workflow_is_observable_and_cancellable() {
    let harness = Harness::start_with(&[], &[], &["orders-api"]);
    let client = harness.client();

    let mut input = prepare_input();
    input.services = vec!["orders-api".to_owned(), "admin-ui".to_owned()];
    input.build = true;
    let outcome = workflow::execute(&client, &input).expect("the workflow is scheduled");
    let workflow_id = workflow_of(&outcome);

    // While the blocked step waits, the workflow is running and the steps that
    // already finished are reflected in the aggregate.
    let running = wait_for_status(&client, workflow_id.as_str(), |status| {
        status.pending > 0 && status.succeeded > 0
    });
    assert_eq!(running.state, "running");

    // The GUI's cancellation goes through the same engine operation the TUI
    // uses; it is not a frontend-only stop.
    tasks::cancel_workflow(&client, workflow_id.as_str()).expect("cancel is accepted");
    let summary = wait_for_workflow(&client, &workflow_id);
    assert_eq!(summary.pending, 0);
    assert!(summary.cancelled > 0, "the blocked step is cancelled");

    let status = workflow::workflow_status(&client, workflow_id.as_str()).expect("status loads");
    assert_eq!(status.pending, 0);
    // Completed work is kept, so a partly-cancelled workflow is reported as
    // partially succeeded rather than as a total cancellation.
    assert_eq!(status.state, "partially_succeeded");
    assert!(status.failures.is_empty(), "cancellation is not a failure");
}

/// The same workflow is observed identically through the GUI, the TUI, and MCP:
/// one daemon, one task engine, one set of aggregates (`tasks/022.md` §42 items
/// 12, 13).
#[test]
fn the_tui_and_mcp_observe_the_same_workflow() {
    let harness = Harness::start();
    let gui = harness.client();
    let tui = harness.tui_client();
    let mcp = harness.mcp_client();

    let mut input = prepare_input();
    input.services = vec!["orders-api".to_owned()];
    input.build = true;
    let outcome = workflow::execute(&gui, &input).expect("the workflow is scheduled");
    let workflow_id = workflow_of(&outcome);
    let _ = wait_for_workflow(&gui, &workflow_id);

    let from_gui = workflow::workflow_status(&gui, workflow_id.as_str()).expect("gui status");
    let from_tui = workflow::workflow_status(&tui, workflow_id.as_str()).expect("tui status");
    let from_mcp = workflow::workflow_status(&mcp, workflow_id.as_str()).expect("mcp status");
    assert_eq!(from_tui, from_gui);
    assert_eq!(from_mcp, from_gui);
    assert_eq!(from_gui.state, "succeeded");

    // Every interface sees the same bounded history window too.
    let history = workflow::list_workflows(&mcp, "demo", None).expect("history loads");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].workflow_id, workflow_id.as_str());
}

/// A reconnecting GUI reloads configuration-backed state and reports workflow
/// state honestly: the engine's in-memory history is not fabricated after a
/// restart, and planning still works (`tasks/022.md` §42 item 14).
#[test]
fn a_reconnecting_gui_reloads_state_after_a_daemon_restart() {
    let mut harness = Harness::start();
    let client = harness.client();

    let mut input = prepare_input();
    input.services = vec!["orders-api".to_owned()];
    let outcome = workflow::execute(&client, &input).expect("the workflow is scheduled");
    let workflow_id = workflow_of(&outcome);
    let _ = wait_for_workflow(&client, &workflow_id);
    assert_eq!(
        workflow::list_workflows(&client, "demo", None)
            .expect("history loads")
            .len(),
        1
    );

    harness.restart();
    let reconnected = harness.client();

    // Configuration-backed state returns with the new daemon.
    assert_eq!(
        workflow::list_profiles(&reconnected, "demo")
            .expect("profiles load")
            .len(),
        1
    );
    assert_eq!(
        workflow::list_groups(&reconnected, "demo")
            .expect("groups load")
            .len(),
        1
    );

    // The task engine's in-memory history does not survive, and the adapter
    // fabricates nothing to hide that: the list is empty and the old workflow
    // is a structured not-found rather than a phantom success.
    assert!(workflow::list_workflows(&reconnected, "demo", None)
        .expect("history loads")
        .is_empty());
    assert_eq!(
        workflow::workflow_status(&reconnected, workflow_id.as_str())
            .expect_err("the workflow is no longer observable")
            .code,
        "WorkflowNotFound"
    );

    // The reconnected GUI can plan again through the same daemon.
    let plan = workflow::plan(&reconnected, &input).expect("a plan is generated");
    assert!(plan.step_count > 0);
}

// -- configuration management -------------------------------------------------

/// A repository inspector that reports a fixed, structured detection for any
/// path, without touching the filesystem.
///
/// It exercises the adapter's inspection/detection mapping over IPC while
/// keeping the test hermetic: no real directory, no build, no script
/// (`tasks/023.md` §42).
struct FakeRepositoryInspector;

impl RepositoryInspector for FakeRepositoryInspector {
    fn inspect(&self, path: &str) -> Result<RepositoryInspection, RepositoryInspectionError> {
        let mut inspection = RepositoryInspection::missing(path);
        inspection.exists = true;
        inspection.is_directory = true;
        inspection.is_git_repository = true;
        inspection.branch = Some("main".to_owned());
        inspection.remote = Some("git@example.com:demo/orders-api.git".to_owned());
        inspection.project_files = vec![
            "pom.xml".to_owned(),
            "mvnw".to_owned(),
            "package.json".to_owned(),
            "build.gradle.kts".to_owned(),
        ];
        inspection.build_systems = vec![
            DetectedBuildSystem {
                system: "maven".to_owned(),
                label: "Maven".to_owned(),
                availability: ProviderAvailability::Available,
                evidence: vec!["pom.xml".to_owned()],
                note: None,
            },
            DetectedBuildSystem {
                system: "npm".to_owned(),
                label: "npm".to_owned(),
                availability: ProviderAvailability::Available,
                evidence: vec!["package.json".to_owned()],
                note: None,
            },
            DetectedBuildSystem {
                system: "gradle".to_owned(),
                label: "Gradle".to_owned(),
                availability: ProviderAvailability::ProviderNotInstalled,
                evidence: vec!["build.gradle.kts".to_owned()],
                note: Some("Detected: Gradle — execution provider not installed.".to_owned()),
            },
        ];
        inspection.runtimes = vec![
            DetectedRuntime {
                kind: "java".to_owned(),
                label: "Java".to_owned(),
                required_version: Some("21".to_owned()),
                evidence: vec!["pom.xml".to_owned()],
            },
            DetectedRuntime {
                kind: "node".to_owned(),
                label: "Node.js".to_owned(),
                required_version: Some("20.19.4".to_owned()),
                evidence: vec!["package.json".to_owned()],
            },
        ];
        inspection.integrations = vec![DetectedIntegration {
            kind: "spring_boot".to_owned(),
            label: "Spring Boot".to_owned(),
            evidence: vec!["pom.xml".to_owned()],
        }];
        inspection.maven = Some(MavenMetadata {
            group_id: Some("com.demo".to_owned()),
            artifact_id: Some("orders-api".to_owned()),
            version: Some("1.0.0".to_owned()),
            packaging: Some("jar".to_owned()),
            java_version: Some("21".to_owned()),
            modules: Vec::new(),
            wrapper_available: true,
            spring_boot: true,
        });
        inspection.node = Some(NodeMetadata {
            name: Some("orders-api-ui".to_owned()),
            version: Some("1.0.0".to_owned()),
            required_node_version: Some("20.19.4".to_owned()),
            package_manager: Some("npm".to_owned()),
            scripts: vec!["build".to_owned(), "start".to_owned()],
            react: false,
        });
        Ok(inspection)
    }
}

/// Builds a new, minimal, valid workspace entity.
fn new_workspace(id: &str) -> service_orchestrator_config::raw::RawWorkspace {
    service_orchestrator_config::raw::RawWorkspace {
        id: id.to_owned(),
        name: Some(id.to_owned()),
        root: format!("~/Projects/{id}"),
        environment: BTreeMap::new(),
    }
}

#[test]
fn configuration_snapshot_validation_and_preview_run_over_ipc() {
    let harness = Harness::start();
    let client = harness.client();

    let snapshot = configuration::snapshot(&client).expect("a snapshot is returned");
    assert!(!snapshot.revision.is_empty());
    assert_eq!(snapshot.workspaces.len(), 1);
    assert_eq!(snapshot.services.len(), 2);

    let validation = configuration::validate(&client).expect("validation runs");
    assert!(validation.valid);
    assert!(validation.issues.is_empty());

    // Previewing a create reports a diff and a proposed revision, and writes
    // nothing: the daemon is the only writer and it has not committed yet.
    let change = ConfigChange::workspace(EntityChange::Create {
        entity: new_workspace("payments"),
    });
    let preview = configuration::preview(&client, &change, &snapshot.revision)
        .expect("a preview is returned");
    assert!(preview.valid);
    assert_ne!(preview.proposed_revision, snapshot.revision);
    assert!(!preview.diff.is_empty());

    let after = configuration::snapshot(&client).expect("a snapshot is returned");
    assert_eq!(after.revision, snapshot.revision, "preview wrote nothing");
}

#[test]
fn a_committed_change_persists_and_is_observable_after_reload() {
    let harness = Harness::start();
    let client = harness.client();
    let revision = configuration::snapshot(&client)
        .expect("a snapshot")
        .revision;

    let change = ConfigChange::workspace(EntityChange::Create {
        entity: new_workspace("payments"),
    });
    let commit =
        configuration::commit(&client, &change, &revision, false).expect("the change is committed");
    assert!(commit.description.contains("payments"));

    let reload = configuration::reload(&client).expect("the file reloads");
    assert_eq!(reload.revision, commit.revision);
    assert_eq!(reload.counts.workspaces, 2);

    let snapshot = configuration::snapshot(&client).expect("a snapshot is returned");
    assert!(snapshot.workspaces.iter().any(|w| w.id == "payments"));
}

#[test]
fn a_stale_revision_is_reported_as_a_conflict() {
    let harness = Harness::start();
    let client = harness.client();
    let change = ConfigChange::workspace(EntityChange::Create {
        entity: new_workspace("payments"),
    });

    let error = configuration::commit(&client, &change, "not-the-current-revision", false)
        .expect_err("a stale revision is refused");
    assert_eq!(error.code, "CONFIG_CONFLICT");
}

#[test]
fn an_unconfirmed_removal_is_denied_but_a_confirmed_one_persists() {
    let harness = Harness::start();
    let client = harness.client();

    // Without confirmation the removal is refused by the daemon's policy.
    let revision = configuration::snapshot(&client)
        .expect("a snapshot")
        .revision;
    let change = ConfigChange::group(EntityChange::Remove {
        id: "core".to_owned(),
    });
    let denied = configuration::commit(&client, &change, &revision, false)
        .expect_err("an unconfirmed removal is refused");
    assert_eq!(denied.code, "PermissionDenied");

    // The file is untouched: the group survives the denial.
    let snapshot = configuration::snapshot(&client).expect("a snapshot is returned");
    assert!(snapshot.groups.iter().any(|g| g.id == "core"));

    // With confirmation the interactive desktop client is allowed to remove it.
    let confirmed = configuration::commit(&client, &change, &snapshot.revision, true)
        .expect("a confirmed removal commits");
    let after = configuration::snapshot(&client).expect("a snapshot is returned");
    assert_eq!(after.revision, confirmed.revision);
    assert!(!after.groups.iter().any(|g| g.id == "core"));
}

#[test]
fn an_invalid_change_is_reported_as_a_validation_failure() {
    let harness = Harness::start();
    let client = harness.client();
    let revision = configuration::snapshot(&client)
        .expect("a snapshot")
        .revision;
    // A service that names a workspace that does not exist fails validation.
    let change = ConfigChange::service(EntityChange::Create {
        entity: service_orchestrator_config::raw::RawService {
            id: "orphan".to_owned(),
            name: None,
            service_type: "backend".to_owned(),
            workspace: "ghost".to_owned(),
            path: Some("backend/orphan".to_owned()),
            default_branch: Some("main".to_owned()),
            repository: None,
            runtime: None,
            runtimes: Vec::new(),
            commands: Default::default(),
            build_system: None,
            liquibase: None,
            health_checks: Vec::new(),
            ports: Vec::new(),
            dependencies: Default::default(),
            environment: BTreeMap::new(),
            capabilities: Vec::new(),
        },
    });
    let error = configuration::commit(&client, &change, &revision, false)
        .expect_err("an invalid change is refused");
    assert_eq!(error.code, "ConfigurationInvalid");
}

#[test]
fn repository_inspection_and_detection_are_structured_and_read_only() {
    let harness = Harness::start();
    let client = harness.client();

    let inspection =
        configuration::inspect(&client, "/tmp/demo/orders-api").expect("inspection succeeds");
    assert!(inspection.exists && inspection.is_directory && inspection.is_git_repository);
    assert_eq!(inspection.branch.as_deref(), Some("main"));
    assert_eq!(
        inspection
            .operational_build_system()
            .map(|s| s.system.as_str()),
        Some("maven")
    );
    let gradle = inspection
        .build_systems
        .iter()
        .find(|system| system.system == "gradle")
        .expect("Gradle is detected but not operational");
    assert_eq!(
        gradle.availability,
        ProviderAvailability::ProviderNotInstalled
    );

    let suggestion =
        configuration::detect(&client, "/tmp/demo/orders-api").expect("detection succeeds");
    assert_eq!(suggestion.service_type.as_deref(), Some("backend"));
    assert_eq!(suggestion.build_system.as_deref(), Some("maven"));
    assert!(
        !suggestion.warnings.is_empty(),
        "the Gradle caveat is surfaced"
    );
    assert!(suggestion
        .warnings
        .iter()
        .any(|warning| warning.contains("Gradle")));
}

#[test]
fn a_relative_inspection_path_is_rejected_before_any_inspection() {
    let harness = Harness::start();
    let client = harness.client();
    let error =
        configuration::inspect(&client, "../escape").expect_err("a traversal path is refused");
    assert_eq!(error.code, "InvalidPath");
}
