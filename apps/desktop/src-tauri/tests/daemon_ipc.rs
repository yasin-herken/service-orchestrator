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

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use service_orchestrator_application::{
    build_parameter, parameter, Application, ArtifactInspectionError, ArtifactInspector,
    ArtifactPresence, BuildCapabilities, BuildCapabilityIndex, BulkOperationSummary,
    DependencyVersionInspection, DiffOptions, GitError, GitManager, GitOptions, HealthState,
    InMemoryRuntimeStateStore, NullLogService, ResolvedArtifact, ResolvedDependency,
    ResolvedRepository, ResolvedServiceTarget, RuntimeStateStore,
};
use service_orchestrator_client::{
    socket_path, ClientKind, OrchestratorClient, RemoteClient, PROTOCOL_VERSION, SOCKET_ENV,
};
use service_orchestrator_config::load_from_str;
use service_orchestrator_daemon::{Daemon, DaemonError, DaemonOptions};
use service_orchestrator_desktop_lib::{
    build,
    dto::{OperationOutcome, OperationStatus},
    git, library, mutations, queries, tasks,
};
use service_orchestrator_domain::{
    AbsolutePath, Branch, BranchInfo, BranchKind, BuildOperation, BuildSystem, GitOperation,
    GitOperationResult, ProcessState, RepoDiff, RepositoryLifecycle, RepositoryStatus, ServiceId,
    Task, TaskKind, TaskStatus, Version, WorkflowId,
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
    /// Test-only port state shared with the fake Git manager, artifact
    /// inspector, and task executors, so a test can observe and steer what the
    /// "infrastructure" reports without any real Git, Maven, or npm execution.
    fakes: Arc<Mutex<FakeState>>,
}

impl Harness {
    /// Starts an isolated daemon over a fresh temporary directory.
    fn start() -> Self {
        Self::start_with(&[], &[], &[])
    }

    /// Starts an isolated daemon whose start executor fails for the named
    /// services, so bulk partial-failure handling can be exercised.
    fn start_with_failures(failing_starts: &[&str]) -> Self {
        Self::start_with(failing_starts, &[], &[])
    }

    /// Starts an isolated daemon with configurable fake-port behaviour.
    ///
    /// `failing_starts` fails service lifecycle operations, `failing_builds`
    /// fails service/library build tasks, and `blocking` makes a Git or build
    /// task on that entity wait until it is cancelled, so cancellation can be
    /// observed without any real long-running process.
    fn start_with(failing_starts: &[&str], failing_builds: &[&str], blocking: &[&str]) -> Self {
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
        let failing_builds: BTreeSet<String> =
            failing_builds.iter().map(|id| (*id).to_owned()).collect();
        let blocking: BTreeSet<String> = blocking.iter().map(|id| (*id).to_owned()).collect();
        let (application, engine, fakes) =
            runtime.block_on(build_core(failing, failing_builds, blocking));
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
            fakes,
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
    assert!(logs.entries.is_empty(), "the harness wires no log service");

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

/// Builds a real application core with fake Git, artifact, build-capability,
/// and task executors and in-memory state, so Git, build, and library tasks
/// settle without ever touching an operating-system process, a real Git
/// repository, a Maven/npm project, a company database, or the network
/// (`tasks/020.md` §40).
///
/// Returns the shared [`FakeState`] so a test can observe and steer what the
/// fake "infrastructure" reports.
async fn build_core(
    failing_starts: BTreeSet<String>,
    failing_builds: BTreeSet<String>,
    blocking: BTreeSet<String>,
) -> (Application, Arc<TaskEngine>, Arc<Mutex<FakeState>>) {
    let config = load_from_str(FIXTURE, "<desktop-test>").expect("the fixture is valid");
    let store = Arc::new(InMemoryRuntimeStateStore::new());
    let fakes = Arc::new(Mutex::new(FakeState::seeded()));

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

    let git_executor = Arc::new(FakeGitExecutor {
        state: Arc::clone(&fakes),
        blocking: blocking.clone(),
    });
    let build_executor = Arc::new(FakeBuildExecutor {
        state: Arc::clone(&fakes),
        failing: failing_builds,
        blocking,
    });
    let verify_executor = Arc::new(FakeArtifactExecutor {
        state: Arc::clone(&fakes),
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
            .executor(TaskKind::GitSync, git_executor.clone())
            .executor(TaskKind::GitFetch, git_executor.clone())
            .executor(TaskKind::GitPull, git_executor.clone())
            .executor(TaskKind::GitCheckout, git_executor)
            .executor(TaskKind::BuildService, build_executor.clone())
            .executor(TaskKind::BuildLibrary, build_executor.clone())
            .executor(TaskKind::LibraryInstall, build_executor)
            .executor(TaskKind::ArtifactVerification, verify_executor)
            .build()
            .expect("a Tokio runtime is active"),
    );
    // The read-only Git/build/library queries reach the fake ports directly,
    // exactly as they reach the real adapters in production; the mutating
    // operations go through the fake task executors above.
    let application = Application::new(config, engine.clone(), store, Arc::new(NullLogService))
        .with_git_manager(Arc::new(FakeGitManager {
            state: Arc::clone(&fakes),
        }))
        .with_artifact_inspector(Arc::new(FakeArtifactInspector {
            state: Arc::clone(&fakes),
        }))
        .with_build_capabilities(Arc::new(FakeBuildCapabilityIndex));
    (application, engine, fakes)
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
