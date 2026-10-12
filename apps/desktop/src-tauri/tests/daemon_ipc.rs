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
//! The daemon is wired with **fake lifecycle executors** and only an in-memory
//! runtime-state store. The fake executors update that store exactly as a real
//! runtime/health adapter would (process → running/stopped, health → healthy),
//! so the tests can observe `running`/`stopped` state end-to-end without ever
//! starting an operating-system process. No Git command, Maven build,
//! Liquibase migration, or company repository, database, credential, or host is
//! touched.
//!
//! # Why the tests are serialized
//!
//! The socket path is resolved from the `SERVICE_ORCHESTRATOR_SOCKET`
//! environment variable, which is process-global. The harness holds a
//! process-wide lock for its whole lifetime and restores the previous value on
//! drop, so tests in this binary run one at a time and stay isolated from the
//! daemon and MCP test binaries (separate processes).

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use service_orchestrator_application::{
    Application, BulkOperationSummary, HealthState, InMemoryRuntimeStateStore, NullLogService,
    ResolvedServiceTarget, RuntimeStateStore,
};
use service_orchestrator_client::{
    socket_path, ClientKind, OrchestratorClient, RemoteClient, PROTOCOL_VERSION, SOCKET_ENV,
};
use service_orchestrator_config::load_from_str;
use service_orchestrator_daemon::{Daemon, DaemonError, DaemonOptions};
use service_orchestrator_desktop_lib::{
    dto::{OperationOutcome, OperationStatus},
    mutations, queries,
};
use service_orchestrator_domain::{
    ProcessState, ServiceId, Task, TaskKind, TaskStatus, WorkflowId,
};
use service_orchestrator_execution::{
    TaskContext, TaskEngine, TaskExecutor, TaskExecutorError, TaskOutcome,
};

/// The fixture configuration, shared with the daemon and MCP adapter tests.
const FIXTURE: &str = include_str!("../../../../crates/mcp/tests/fixtures/config.toml");

/// Serializes the tests so the process-global socket override cannot race.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// How long a test waits for the daemon to become reachable or to stop.
const DEADLINE: Duration = Duration::from_secs(15);

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
    /// The throwaway directory that holds the configuration, socket, and lock.
    root: PathBuf,
}

impl Harness {
    /// Starts an isolated daemon over a fresh temporary directory.
    fn start() -> Self {
        Self::start_with_failures(&[])
    }

    /// Starts an isolated daemon whose start executor fails for the named
    /// services, so bulk partial-failure handling can be exercised.
    fn start_with_failures(failing_starts: &[&str]) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());

        let root = unique_root();
        std::fs::create_dir_all(&root).expect("the temporary directory is created");
        let config_path = root.join("config.toml");
        std::fs::write(&config_path, FIXTURE).expect("the fixture configuration is written");

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
        let failing: BTreeSet<String> = failing_starts.iter().map(|id| (*id).to_owned()).collect();
        let (application, engine) = runtime.block_on(build_core(failing));
        let daemon = Daemon::new(
            application,
            engine,
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
            root,
        };
        harness.wait_ready();
        harness
    }

    /// Connects a fresh client, never starting a daemon of its own.
    ///
    /// This is also the transport the GUI adapter uses in production when no
    /// daemon binary could be resolved: attach to what is already running.
    fn client(&self) -> RemoteClient {
        RemoteClient::connect_existing(&self.config_path, ClientKind::Cli)
            .expect("the daemon is reachable")
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

/// Builds a real application core with fake lifecycle executors and in-memory
/// state, so service start/stop/restart/health tasks settle without ever
/// touching an operating-system process.
async fn build_core(failing_starts: BTreeSet<String>) -> (Application, Arc<TaskEngine>) {
    let config = load_from_str(FIXTURE, "<desktop-test>").expect("the fixture is valid");
    let store = Arc::new(InMemoryRuntimeStateStore::new());

    // Start/restart observe a running, healthy process; stop observes a stopped
    // process with unknown health. Health checks refresh health only.
    let starting = Arc::new(FakeLifecycleExecutor {
        store: Arc::clone(&store),
        process: Some(ProcessState::Running),
        health: HealthState::Healthy,
        failing: failing_starts,
    });
    let stopping = Arc::new(FakeLifecycleExecutor {
        store: Arc::clone(&store),
        process: Some(ProcessState::Stopped),
        health: HealthState::Unknown,
        failing: BTreeSet::new(),
    });
    let health = Arc::new(FakeLifecycleExecutor {
        store: Arc::clone(&store),
        process: None,
        health: HealthState::Healthy,
        failing: BTreeSet::new(),
    });

    let engine = Arc::new(
        TaskEngine::builder()
            .max_parallel_tasks(4)
            .executor(TaskKind::StartService, starting.clone())
            .executor(TaskKind::RestartService, starting)
            .executor(TaskKind::StopService, stopping)
            .executor(TaskKind::HealthCheck, health.clone())
            .executor(TaskKind::ProcessHealthCheck, health.clone())
            .executor(TaskKind::TcpHealthCheck, health.clone())
            .executor(TaskKind::HttpHealthCheck, health)
            .build()
            .expect("a Tokio runtime is active"),
    );
    let application = Application::new(config, engine.clone(), store, Arc::new(NullLogService));
    (application, engine)
}

/// A test-only lifecycle executor.
///
/// It decodes the workspace-aware [`ResolvedServiceTarget`] from the task's
/// parameters (exactly as the real runtime executors do) and records the
/// resulting process/health observation into the shared in-memory store. A
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

/// Returns a unique temporary directory for one harness.
fn unique_root() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("so-desktop-{}-{nanos}", std::process::id()))
}
