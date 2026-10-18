//! End-to-end integration tests for the shared local runtime (`tasks/015.md`
//! §31–§34).
//!
//! Each test owns a real [`Daemon`] running on a private Unix domain socket in a
//! throwaway temporary directory, and talks to it through the real
//! [`RemoteClient`] over the versioned IPC protocol. That is the whole point:
//! the tests exercise the daemon, the transport, the request router, the shared
//! task engine, and the policy engine together, not a mock of any of them.
//!
//! The daemon is wired with **no executors** and only an in-memory runtime-state
//! store, so a scheduled task is created and observable but never runs: no Git
//! command, Maven build, Liquibase migration, or operating-system process is
//! ever started. No company repository, database, credential, or host is
//! touched.
//!
//! # Why the tests are serialized
//!
//! The socket path is resolved from the `SERVICE_ORCHESTRATOR_SOCKET`
//! environment variable so a client and its daemon always agree on the endpoint
//! (`tasks/015.md` §7). The tests override it to keep every daemon isolated in
//! its own temporary directory. The environment is process-global, so the
//! harness holds a process-wide lock for its whole lifetime and restores the
//! previous value on drop. Tests in this binary therefore run one at a time;
//! tests in other binaries run in separate processes and are unaffected.

use std::ffi::OsString;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use service_orchestrator_application::{
    Application, ApplicationError, Authorization, InMemoryRuntimeStateStore, NullLogService,
    OperationResult,
};
use service_orchestrator_client::{
    codes, ops, read_frame, socket_path, write_frame, ClientKind, OrchestratorClient, RemoteClient,
    RequestEnvelope, ResponseEnvelope, ServerMessage, MAX_FRAME_BYTES, PROTOCOL_VERSION,
    SOCKET_ENV,
};
use service_orchestrator_config::{
    load_from_str, ConfigChange, ConfigManager, EntityChange, FileConfigManager,
};
use service_orchestrator_daemon::{Daemon, DaemonError, DaemonOptions};
use service_orchestrator_domain::{Permission, ProcessState, ServiceId, TaskId};
use service_orchestrator_execution::{TaskEngine, TaskEvent};

/// The fixture configuration, shared with the MCP adapter tests. It is generic:
/// no real repository, credential, or host.
const FIXTURE: &str = include_str!("../../mcp/tests/fixtures/config.toml");

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
    /// The daemon's own task, until it is shut down explicitly.
    daemon: Option<tokio::task::JoinHandle<Result<(), DaemonError>>>,
    /// The configuration file the socket path is derived from.
    config_path: PathBuf,
    /// The daemon's IPC endpoint.
    socket: PathBuf,
    /// The throwaway directory that holds the configuration, socket, and lock.
    root: PathBuf,
}

impl Harness {
    /// Starts an isolated daemon over a fresh temporary directory.
    fn start() -> Self {
        Self::launch(false)
    }

    /// Starts an isolated daemon over a directory that already holds a stale
    /// socket file and a stale lock file, to prove both are recovered safely
    /// (`tasks/015.md` §15).
    fn start_over_stale_runtime() -> Self {
        Self::launch(true)
    }

    /// Shared launcher.
    fn launch(stale: bool) -> Self {
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

        if stale {
            std::fs::create_dir_all(socket.parent().expect("the socket has a parent"))
                .expect("the run directory is created");
            // A leftover socket file from a crashed daemon, plus a leftover lock
            // file whose advisory lock the kernel already released.
            std::fs::write(&socket, b"stale").expect("a stale socket file is written");
            std::fs::write(socket.with_extension("lock"), b"stale")
                .expect("a stale lock file is written");
        }

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a Tokio runtime is built");
        let (application, engine) = runtime.block_on(build_core(&config_path));
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
            socket,
            root,
        };
        harness.wait_ready();
        harness
    }

    /// Connects a fresh client, never starting a daemon of its own.
    fn client(&self, kind: ClientKind) -> RemoteClient {
        RemoteClient::connect_existing(&self.config_path, kind).expect("the daemon is reachable")
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

    /// Requests a graceful shutdown and waits for the daemon task to finish.
    fn shutdown(&mut self) -> Result<(), DaemonError> {
        let client = self.client(ClientKind::Cli);
        client
            .shutdown_daemon()
            .expect("the shutdown request is accepted");
        let handle = self.daemon.take().expect("the daemon has not stopped yet");
        self.runtime
            .block_on(async { tokio::time::timeout(DEADLINE, handle).await })
            .expect("the daemon stops before the deadline")
            .expect("the daemon task is not cancelled")
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

/// Builds a real application core with no executors and in-memory state, backed
/// by the real file configuration manager over the harness's config file.
///
/// Wiring the file manager is what makes the daemon the single configuration
/// writer end to end: a commit through IPC rewrites the file and refreshes this
/// core (`tasks/023.md` §38, §40, §49).
async fn build_core(config_path: &Path) -> (Application, Arc<TaskEngine>) {
    let config = load_from_str(FIXTURE, "<daemon-test>").expect("the fixture is valid");
    let manager: Arc<dyn ConfigManager> = Arc::new(FileConfigManager::new(
        config_path.to_path_buf(),
        config.clone(),
    ));
    let engine = Arc::new(
        TaskEngine::builder()
            .max_parallel_tasks(4)
            .build()
            .expect("a Tokio runtime is active"),
    );
    let application = Application::new(
        config,
        engine.clone(),
        Arc::new(InMemoryRuntimeStateStore::new()),
        Arc::new(NullLogService),
    )
    .with_configuration(manager);
    (application, engine)
}

/// Returns a unique temporary directory for one harness.
fn unique_root() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("so-daemon-{}-{nanos}", std::process::id()))
}

/// Returns the `orders-api` service id from the fixture.
fn orders_api() -> ServiceId {
    ServiceId::new("orders-api").expect("a valid service id")
}

/// Returns the `demo` workspace id from the fixture.
fn demo_workspace() -> service_orchestrator_domain::WorkspaceId {
    service_orchestrator_domain::WorkspaceId::new("demo").expect("a valid workspace id")
}

/// Sends one raw request frame and returns the correlated response result.
fn raw_request(
    socket: &Path,
    request: &RequestEnvelope,
) -> Result<serde_json::Value, ApplicationError> {
    let mut stream = UnixStream::connect(socket).expect("the daemon accepts the connection");
    write_frame(&mut stream, request).expect("the request frame is written");
    loop {
        let message: ServerMessage = read_frame(&mut stream)
            .expect("the response frame reads")
            .expect("the daemon answers with a frame");
        match message {
            ServerMessage::Response(ResponseEnvelope { request_id, result })
                if request_id == request.request_id =>
            {
                return result
            }
            // Events or responses to other requests are ignored.
            _ => continue,
        }
    }
}

/// Builds a well-formed request envelope.
fn request(method: &str, params: serde_json::Value) -> RequestEnvelope {
    RequestEnvelope {
        protocol: PROTOCOL_VERSION,
        request_id: format!("raw-{method}"),
        client: ClientKind::Cli,
        allow_destructive: false,
        method: method.to_owned(),
        params,
    }
}

/// Asserts an error is `RuntimeUnavailable` carrying `expected_code`.
fn assert_runtime_code(error: &ApplicationError, expected_code: &str) {
    match error {
        ApplicationError::RuntimeUnavailable { code, .. } => assert_eq!(code, expected_code),
        other => panic!("expected runtime-unavailable '{expected_code}', got {other:?}"),
    }
}

// -- shared state across clients ---------------------------------------------

#[test]
fn two_clients_observe_the_same_workspace_and_service_state() {
    let harness = Harness::start();
    let tui = harness.client(ClientKind::Tui);
    let mcp = harness.client(ClientKind::Mcp);

    let workspaces = tui.list_workspaces().expect("workspaces are listed");
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0].id.as_str(), "demo");

    let services = mcp
        .list_services(&demo_workspace())
        .expect("services are listed");
    let ids: Vec<&str> = services.iter().map(|service| service.id.as_str()).collect();
    assert_eq!(ids, vec!["orders-api", "admin-ui"]);

    let status = mcp.daemon_status().expect("the daemon reports its status");
    assert_eq!(status.protocol_version, PROTOCOL_VERSION);
    assert_eq!(status.socket_path, harness.socket.display().to_string());
    assert!(
        status.clients >= 2,
        "both connected clients are counted: {}",
        status.clients
    );
}

/// Validation over IPC is read-only: it submits no task and reaches no
/// infrastructure, because the daemon's probe is the permissive default in
/// these tests (`tasks/017.md` §7).
#[test]
fn validate_workspace_is_available_over_ipc_and_submits_nothing() {
    let harness = Harness::start();
    let tui = harness.client(ClientKind::Tui);
    let mcp = harness.client(ClientKind::Mcp);

    let report = tui
        .validate_workspace(&demo_workspace())
        .expect("the report is produced");
    assert_eq!(report.workspace_id, demo_workspace());
    assert_eq!(report.totals.services, 2);
    assert_eq!(report.totals.libraries, 1);
    assert!(
        report.totals.repositories_valid >= 3,
        "every configured repository is checked: {:?}",
        report.totals
    );
    assert_eq!(
        report.totals.errors, 0,
        "the permissive in-test probe reports no environment failures: {:#?}",
        report.diagnostics
    );

    // The same report is visible from the MCP client, and neither client
    // scheduled anything.
    let from_mcp = mcp
        .validate_workspace(&demo_workspace())
        .expect("the MCP client gets the same report");
    assert_eq!(from_mcp.totals, report.totals);
    assert!(tui.list_tasks().is_empty(), "validation scheduled no task");
}

#[test]
fn validate_workspace_of_an_unknown_workspace_is_reported_over_ipc() {
    let harness = Harness::start();
    let client = harness.client(ClientKind::Tui);
    let missing = service_orchestrator_domain::WorkspaceId::new("missing").unwrap();
    let error = client.validate_workspace(&missing).unwrap_err();
    assert!(
        matches!(error, ApplicationError::WorkspaceNotFound(_)),
        "expected a workspace-not-found error, got {error:?}"
    );
}

#[test]
fn a_task_submitted_by_one_client_is_visible_to_another() {
    let harness = Harness::start();
    let submitter = harness.client(ClientKind::Mcp);
    let observer = harness.client(ClientKind::Tui);

    let result = submitter
        .build_service(&orders_api())
        .expect("the build is scheduled");
    let task_id = result.task_id().expect("a task id is returned").clone();

    // The TUI client sees the same task in the daemon's single task engine.
    let task = observer.get_task(&task_id).expect("the task is found");
    assert_eq!(task.id, task_id);

    let listed = observer.list_tasks();
    assert!(
        listed.iter().any(|task| task.id == task_id),
        "the task submitted over one connection is listed over another"
    );
}

#[test]
fn service_start_and_stop_state_is_shared_and_owned_by_the_daemon() {
    let harness = Harness::start();
    let tui = harness.client(ClientKind::Tui);
    let mcp = harness.client(ClientKind::Mcp);
    let service = orders_api();

    // The TUI starts the service; the daemon records the transition in its one
    // shared runtime-state store.
    let started = tui.start_service(&service).expect("the start is scheduled");
    assert!(matches!(started, OperationResult::TaskScheduled { .. }));

    // MCP observes the exact same state and can act on it.
    let status = mcp
        .get_service_runtime_status(&service)
        .expect("the runtime status is read");
    assert_eq!(status.process_state, ProcessState::Starting);

    let running = mcp
        .list_running_services(&demo_workspace())
        .expect("running services are listed");
    assert!(
        running.iter().any(|entry| entry.service_id == service),
        "a service started from the TUI is visible to MCP"
    );

    // MCP stops the service the TUI started.
    let stopped = mcp.stop_service(&service).expect("the stop is scheduled");
    assert!(matches!(stopped, OperationResult::TaskScheduled { .. }));

    let after = tui
        .get_service_runtime_status(&service)
        .expect("the runtime status is read");
    assert_eq!(after.process_state, ProcessState::Stopping);
}

// -- protocol handling -------------------------------------------------------

#[test]
fn an_incompatible_protocol_version_is_rejected_with_an_actionable_code() {
    let harness = Harness::start();
    let mut incompatible = request("list_workspaces", serde_json::json!([]));
    incompatible.protocol = PROTOCOL_VERSION + 1;

    let error = raw_request(&harness.socket, &incompatible)
        .expect_err("an incompatible version is refused");
    assert_runtime_code(&error, codes::PROTOCOL_INCOMPATIBLE);
    assert!(
        error.to_string().contains("protocol version"),
        "the error names the problem: {error}"
    );
}

#[test]
fn invalid_parameters_are_reported_as_an_invalid_request() {
    let harness = Harness::start();
    // `list_services` expects a single workspace id, not a number.
    let error = raw_request(
        &harness.socket,
        &request("list_services", serde_json::json!([123])),
    )
    .expect_err("malformed parameters are refused");
    assert_runtime_code(&error, codes::INVALID_REQUEST);
}

#[test]
fn an_unknown_operation_is_reported_without_executing_anything() {
    let harness = Harness::start();
    let error = raw_request(
        &harness.socket,
        &request("frobnicate", serde_json::json!([])),
    )
    .expect_err("an unknown operation is refused");
    assert_runtime_code(&error, codes::UNKNOWN_OPERATION);
}

// -- authorization -----------------------------------------------------------

#[test]
fn mcp_cannot_self_confirm_a_destructive_operation() {
    let harness = Harness::start();
    let mcp = harness.client(ClientKind::Mcp);
    // Even presenting an `allow_destructive` flag, a non-interactive peer is
    // refused: the daemon runs the shared policy engine and ignores the flag
    // (`tasks/015.md` §23).
    let client = mcp.with_authorization(Authorization::allow_destructive());
    let error = client
        .reset_repository_hard(&orders_api())
        .expect_err("MCP cannot self-confirm");
    assert!(matches!(error, ApplicationError::PermissionDenied { .. }));
}

#[test]
fn an_interactive_client_can_request_a_destructive_operation() {
    let harness = Harness::start();
    let tui = harness.client(ClientKind::Tui);

    // Without confirmation the interactive client is also refused.
    let error = tui
        .reset_repository_hard(&orders_api())
        .expect_err("confirmation is still required");
    assert!(matches!(error, ApplicationError::PermissionDenied { .. }));

    // With confirmation the daemon honors it for an interactive peer and the
    // destructive work is scheduled through the shared task engine.
    let confirmed = tui.with_authorization(Authorization::allow_destructive());
    let result = confirmed
        .reset_repository_hard(&orders_api())
        .expect("the confirmed reset is scheduled");
    assert!(matches!(result, OperationResult::TaskScheduled { .. }));
}

// -- event streaming ---------------------------------------------------------

#[test]
fn task_events_are_streamed_to_subscribing_clients() {
    let harness = Harness::start();
    let observer = harness.client(ClientKind::Tui);
    let submitter = harness.client(ClientKind::Mcp);

    let mut events = observer.subscribe_task_events();
    // A round trip guarantees the server-side event pump for this connection is
    // registered before the triggering task is submitted.
    observer.list_workspaces().expect("the daemon answers");

    let task_id = submitter
        .build_service(&orders_api())
        .expect("the build is scheduled")
        .task_id()
        .expect("a task id is returned")
        .clone();

    let created = wait_for_created(&harness, &mut events, &task_id);
    assert_eq!(created, task_id);
}

/// Waits until a `TaskCreated` event for `task_id` arrives.
fn wait_for_created(
    harness: &Harness,
    events: &mut tokio::sync::broadcast::Receiver<TaskEvent>,
    task_id: &TaskId,
) -> TaskId {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .expect("the task event arrived before the deadline");
        let received = harness
            .runtime
            .block_on(async { tokio::time::timeout(remaining, events.recv()).await });
        match received {
            Ok(Ok(TaskEvent::TaskCreated { task })) if task.id == *task_id => {
                return task_id.clone()
            }
            Ok(Ok(_)) => continue,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                panic!("the event stream closed before the task event arrived")
            }
            Err(_) => panic!("no task event arrived before the deadline"),
        }
    }
}

// -- IPC robustness ----------------------------------------------------------

#[test]
fn an_oversized_request_frame_is_rejected_without_crashing_the_daemon() {
    use std::io::Write as _;

    let harness = Harness::start();
    let mut stream =
        UnixStream::connect(&harness.socket).expect("the daemon accepts the connection");

    // A length prefix one byte past the hard maximum: the daemon must reject it
    // before allocating the body, answer with a structured error, and stay up.
    let oversized = u32::try_from(MAX_FRAME_BYTES + 1).expect("fits in a u32");
    stream
        .write_all(&oversized.to_be_bytes())
        .expect("the length prefix is written");
    stream.flush().expect("the prefix reaches the daemon");

    let response = loop {
        match read_frame::<_, ServerMessage>(&mut stream).expect("the response frame reads") {
            Some(ServerMessage::Response(response)) => break response,
            Some(_) => continue,
            None => panic!("the daemon closed without a structured error"),
        }
    };
    let error = response
        .result
        .expect_err("an oversized frame is rejected, not executed");
    assert_runtime_code(&error, codes::INVALID_REQUEST);

    // The daemon is still serving other clients after the hostile frame.
    let client = harness.client(ClientKind::Cli);
    assert!(client.list_workspaces().is_ok());
}

#[test]
fn concurrent_clients_receive_their_own_correlated_responses() {
    let harness = Harness::start();

    // Send a distinct request on several connections before reading any
    // response, so the daemon must interleave them and correlate each reply to
    // the request that produced it (`tasks/027.md` §26).
    let mut pending = Vec::new();
    for index in 0..8 {
        let mut stream =
            UnixStream::connect(&harness.socket).expect("the daemon accepts the connection");
        let mut envelope = request(ops::LIST_WORKSPACES, serde_json::json!([]));
        envelope.request_id = format!("concurrent-{index}");
        write_frame(&mut stream, &envelope).expect("the request frame is written");
        pending.push((envelope.request_id, stream));
    }

    for (request_id, mut stream) in pending {
        loop {
            match read_frame::<_, ServerMessage>(&mut stream).expect("the response frame reads") {
                Some(ServerMessage::Response(response)) => {
                    assert_eq!(
                        response.request_id, request_id,
                        "each response is correlated to its own request id"
                    );
                    let value = response.result.expect("the request succeeds");
                    assert!(
                        value.as_array().is_some(),
                        "the fixture workspace list is returned"
                    );
                    break;
                }
                Some(_) => continue,
                None => panic!("the daemon closed before answering {request_id}"),
            }
        }
    }
}

// -- lifecycle ---------------------------------------------------------------

#[test]
fn the_daemon_survives_a_client_disconnect() {
    let harness = Harness::start();

    {
        let client = harness.client(ClientKind::Tui);
        assert!(client.list_workspaces().is_ok());
    } // the client is dropped here; the daemon must keep running

    let client = harness.client(ClientKind::Mcp);
    assert!(
        client.list_workspaces().is_ok(),
        "the daemon is still serving after a client disconnects"
    );
    assert_eq!(
        client
            .daemon_status()
            .expect("the daemon still reports its status")
            .protocol_version,
        PROTOCOL_VERSION
    );
}

#[test]
fn an_explicit_shutdown_stops_the_daemon_and_releases_the_socket() {
    let mut harness = Harness::start();
    let client = harness.client(ClientKind::Cli);
    assert!(client.daemon_status().is_ok());

    harness.shutdown().expect("the daemon shuts down cleanly");

    assert!(
        !harness.socket.exists(),
        "the socket file is removed on shutdown"
    );
    assert!(
        RemoteClient::connect_existing(&harness.config_path, ClientKind::Cli).is_err(),
        "no daemon is reachable after shutdown"
    );
}

#[test]
fn a_client_reports_the_daemon_as_unavailable_after_shutdown() {
    let mut harness = Harness::start();
    let client = harness.client(ClientKind::Tui);
    assert!(client.list_workspaces().is_ok());

    harness.shutdown().expect("the daemon shuts down cleanly");

    // A `connect_existing` client never starts a daemon, so the next call
    // reports a structured `DAEMON_UNAVAILABLE` rather than silently reviving
    // one (`tasks/015.md` §21, §30).
    let error = client.list_workspaces().expect_err("the daemon is gone");
    assert_runtime_code(&error, codes::DAEMON_UNAVAILABLE);
}

// -- single-instance coordination and stale files ----------------------------

#[test]
fn a_second_daemon_on_the_same_socket_is_refused() {
    let harness = Harness::start();

    let (application, engine) = harness.runtime.block_on(build_core(&harness.config_path));
    let second = Daemon::new(
        application,
        engine,
        DaemonOptions {
            socket_path: harness.socket.clone(),
        },
    );
    let outcome = harness.runtime.block_on(second.run());
    assert!(
        matches!(outcome, Err(DaemonError::AlreadyRunning(_))),
        "the second daemon must not become authoritative: {outcome:?}"
    );

    // The first daemon is unaffected.
    let client = harness.client(ClientKind::Cli);
    assert!(client.daemon_status().is_ok());
}

#[test]
fn a_stale_socket_and_lock_file_are_replaced_safely() {
    let harness = Harness::start_over_stale_runtime();
    // Reaching here means the daemon acquired the lock and replaced the stale
    // socket; it must be serving normally.
    let client = harness.client(ClientKind::Tui);
    assert_eq!(
        client
            .list_workspaces()
            .expect("workspaces are listed")
            .len(),
        1
    );
}

// -- process recovery --------------------------------------------------------

#[test]
fn a_fresh_daemon_adopts_nothing_and_never_kills_unrelated_processes() {
    // A process the daemon did not start. It must survive untouched.
    let mut decoy = std::process::Command::new("sleep")
        .arg("300")
        .spawn()
        .expect("the decoy process starts");

    let harness = Harness::start();
    let client = harness.client(ClientKind::Tui);

    // A new daemon owns no managed processes and reports no active tasks.
    let status = client
        .daemon_status()
        .expect("the daemon reports its status");
    assert_eq!(status.managed_services, 0);
    assert_eq!(status.active_tasks, 0);
    assert!(
        client
            .list_running_services(&demo_workspace())
            .expect("running services are listed")
            .is_empty(),
        "a fresh daemon adopts no service processes"
    );

    assert!(
        decoy.try_wait().expect("the decoy is waitable").is_none(),
        "the daemon must never signal an unrelated process"
    );

    let _ = decoy.kill();
    let _ = decoy.wait();
}

// -- configuration management ------------------------------------------------

/// Builds a new minimal, valid backend service entity.
fn new_service(id: &str) -> service_orchestrator_config::raw::RawService {
    use service_orchestrator_config::raw::{RawCommand, RawCommands, RawDependencies, RawService};
    RawService {
        id: id.to_owned(),
        name: Some(id.to_owned()),
        service_type: "backend".to_owned(),
        workspace: "demo".to_owned(),
        path: Some(format!("backend/{id}")),
        default_branch: Some("main".to_owned()),
        repository: None,
        runtime: None,
        runtimes: Vec::new(),
        commands: RawCommands {
            build: Some(RawCommand::Line("./mvnw clean install".to_owned())),
            start: Some(RawCommand::Line("./mvnw spring-boot:run".to_owned())),
            ..RawCommands::default()
        },
        build_system: Some("maven".to_owned()),
        build_operation: None,
        liquibase: None,
        health_checks: Vec::new(),
        ports: Vec::new(),
        dependencies: RawDependencies::default(),
        environment: std::collections::BTreeMap::new(),
        capabilities: Vec::new(),
    }
}

/// Waits for the next configuration-change event, or fails at the deadline.
fn wait_for_configuration_change(
    events: &mut tokio::sync::broadcast::Receiver<
        service_orchestrator_client::ConfigurationChanged,
    >,
) -> service_orchestrator_client::ConfigurationChanged {
    let deadline = Instant::now() + DEADLINE;
    loop {
        match events.try_recv() {
            Ok(change) => return change,
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                assert!(
                    Instant::now() < deadline,
                    "the configuration-change event did not arrive in time"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("the configuration-change stream failed: {error}"),
        }
    }
}

#[test]
fn configuration_snapshot_and_validation_are_available_over_ipc() {
    let mut harness = Harness::start();
    let client = harness.client(ClientKind::Desktop);

    let snapshot = client
        .configuration_snapshot()
        .expect("the daemon returns a snapshot");
    assert!(!snapshot.revision.is_empty());
    assert_eq!(snapshot.workspaces.len(), 1);
    assert_eq!(snapshot.services.len(), 2);

    let validation = client
        .validate_configuration()
        .expect("the daemon validates the active file");
    assert!(validation.valid);

    harness.shutdown().expect("the daemon stops");
}

#[test]
fn committing_a_service_is_broadcast_to_every_connected_client() {
    let mut harness = Harness::start();
    let writer = harness.client(ClientKind::Desktop);
    let observer = harness.client(ClientKind::Tui);

    let mut events = observer.subscribe_configuration_events();
    // A round trip guarantees the observer's server-side event pump is
    // registered before the commit fires.
    observer
        .configuration_snapshot()
        .expect("the daemon answers");

    let revision = writer
        .configuration_snapshot()
        .expect("a snapshot")
        .revision;
    let change = ConfigChange::service(EntityChange::Create {
        entity: new_service("notify-api"),
    });
    let commit = writer
        .commit_configuration(&change, &revision)
        .expect("the change is committed");
    assert!(commit.description.contains("notify-api"));

    let event = wait_for_configuration_change(&mut events);
    assert_eq!(event.revision, commit.revision);
    assert!(event.description.contains("notify-api"));

    // Every client sees the same new revision from the one source of truth.
    let snapshot = observer
        .configuration_snapshot()
        .expect("the observer re-reads the snapshot");
    assert_eq!(snapshot.revision, commit.revision);
    assert!(snapshot.services.iter().any(|s| s.id == "notify-api"));

    harness.shutdown().expect("the daemon stops");
}

#[test]
fn a_stale_configuration_revision_is_rejected_over_ipc() {
    let mut harness = Harness::start();
    let client = harness.client(ClientKind::Desktop);
    let change = ConfigChange::service(EntityChange::Create {
        entity: new_service("notify-api"),
    });

    let error = client
        .commit_configuration(&change, "a-revision-from-the-past")
        .expect_err("a stale edit is refused");
    assert!(matches!(
        error,
        ApplicationError::Configuration { ref code, .. } if code == "CONFIG_CONFLICT"
    ));

    harness.shutdown().expect("the daemon stops");
}

#[test]
fn a_removal_cannot_be_self_confirmed_over_ipc() {
    let mut harness = Harness::start();
    let client = harness.client(ClientKind::Desktop);
    let revision = client
        .configuration_snapshot()
        .expect("a snapshot")
        .revision;
    let change = ConfigChange::group(EntityChange::Remove {
        id: "core".to_owned(),
    });

    let error = client
        .commit_configuration(&change, &revision)
        .expect_err("a removal needs confirmation");
    assert!(matches!(
        error,
        ApplicationError::PermissionDenied {
            permission: Permission::Destructive,
            ..
        }
    ));

    harness.shutdown().expect("the daemon stops");
}

#[test]
fn a_confirmed_removal_persists_and_preserves_unrelated_secrets() {
    let mut harness = Harness::start();
    let client = harness
        .client(ClientKind::Desktop)
        .with_authorization(Authorization::allow_destructive());
    let revision = client
        .configuration_snapshot()
        .expect("a snapshot")
        .revision;
    // `full-stack` is a leaf entity no other declaration references, so its
    // removal is a clean drop. `core` is not used here because the saved
    // operation preset references the group, and that is a validation failure
    // rather than a persistence test.
    let change = ConfigChange::profile(EntityChange::Remove {
        id: "full-stack".to_owned(),
    });
    client
        .commit_configuration(&change, &revision)
        .expect("a confirmed removal commits");

    let snapshot = client
        .configuration_snapshot()
        .expect("the snapshot is re-read");
    assert!(!snapshot.profiles.iter().any(|p| p.id == "full-stack"));

    // A secret elsewhere in the file survives the rewrite verbatim.
    let text = std::fs::read_to_string(&harness.config_path).expect("the file is readable");
    assert!(text.contains("super-secret-value"));

    harness.shutdown().expect("the daemon stops");
}

#[test]
fn reload_validation_reports_the_active_revision() {
    let mut harness = Harness::start();
    let client = harness.client(ClientKind::Desktop);
    let before = client
        .configuration_snapshot()
        .expect("a snapshot")
        .revision;
    let reload = client.reload_configuration().expect("the file reloads");
    assert_eq!(reload.revision, before);
    harness.shutdown().expect("the daemon stops");
}

// -- performance measurement -------------------------------------------------

/// Measures daemon startup and steady-state IPC round-trip latency over the
/// real Unix-socket transport (`tasks/025.md` §7, §16, §18, §28).
///
/// `#[ignore]`d so the normal suite stays fast; run with
/// `--ignored --nocapture --test-threads=1`. The measurement launches its own
/// daemons with a 1 ms readiness poll so startup is not quantised to the shared
/// harness's slower loop.
#[test]
#[ignore = "measurement; run with --ignored --nocapture --test-threads=1"]
fn measure_daemon_startup_and_ipc_latency() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    const LAUNCHES: usize = 5;
    const CALLS: usize = 500;

    let mut startups = Vec::with_capacity(LAUNCHES);
    for sample in 0..LAUNCHES {
        let root = unique_root();
        std::fs::create_dir_all(&root).expect("the temporary directory is created");
        let config_path = root.join("config.toml");
        std::fs::write(&config_path, FIXTURE).expect("the fixture configuration is written");
        let socket = root.join("run/daemon.sock");
        let previous_socket = std::env::var_os(SOCKET_ENV);
        std::env::set_var(SOCKET_ENV, &socket);

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a Tokio runtime is built");
        let (application, engine) = runtime.block_on(build_core(&config_path));
        let daemon = Daemon::new(
            application,
            engine,
            DaemonOptions {
                socket_path: socket.clone(),
            },
        );

        let started = Instant::now();
        let handle = runtime.spawn(daemon.run());
        let deadline = Instant::now() + DEADLINE;
        loop {
            if RemoteClient::connect_existing(&config_path, ClientKind::Cli).is_ok() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the daemon never became reachable"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let startup = started.elapsed();
        startups.push(startup);

        // Steady-state round-trip latency over one persistent client.
        let client = RemoteClient::connect_existing(&config_path, ClientKind::Cli)
            .expect("the daemon is reachable");
        let ws = demo_workspace();
        let _ = client.list_services(&ws);
        let mut latencies = Vec::with_capacity(CALLS);
        for _ in 0..CALLS {
            let called = Instant::now();
            let listed = client.list_services(&ws).expect("list_services succeeds");
            assert!(!listed.is_empty(), "the fixture declares services");
            latencies.push(called.elapsed());
        }
        latencies.sort();
        let at = |values: &[Duration], q: f64| -> Duration {
            values[((values.len() as f64 - 1.0) * q).round() as usize]
        };
        eprintln!(
            "launch {sample}: startup={startup:?}  list_services median={:?} p95={:?} max={:?}",
            at(&latencies, 0.5),
            at(&latencies, 0.95),
            at(&latencies, 1.0),
        );

        let _ = client.shutdown_daemon();
        let _ = runtime.block_on(async { tokio::time::timeout(DEADLINE, handle).await });
        match previous_socket {
            Some(value) => std::env::set_var(SOCKET_ENV, value),
            None => std::env::remove_var(SOCKET_ENV),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    startups.sort();
    eprintln!(
        "daemon startup over {LAUNCHES} launches: min={:?} median={:?} max={:?}",
        startups[0],
        startups[LAUNCHES / 2],
        startups[LAUNCHES - 1],
    );
}
