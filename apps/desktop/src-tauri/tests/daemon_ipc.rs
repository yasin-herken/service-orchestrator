//! Integration tests for the desktop adapter's read path
//! (`tasks/018.md` §32).
//!
//! Each test owns a real [`Daemon`] running on a private Unix domain socket in
//! a throwaway temporary directory, and drives the adapter's own
//! [`queries`] — the exact functions behind the Tauri commands — through a
//! real [`RemoteClient`] over the versioned IPC protocol. Nothing in the
//! adapter is mocked here: command mapping, input validation, error shaping,
//! and the view models are exercised against the real shared core.
//!
//! The daemon is wired with **no executors** and only an in-memory
//! runtime-state store, so no Git command, Maven build, Liquibase migration,
//! or operating-system process is ever started, and no company repository,
//! database, credential, or host is touched.
//!
//! # Why the tests are serialized
//!
//! The socket path is resolved from the `SERVICE_ORCHESTRATOR_SOCKET`
//! environment variable, which is process-global. The harness holds a
//! process-wide lock for its whole lifetime and restores the previous value on
//! drop, so tests in this binary run one at a time and stay isolated from the
//! daemon and MCP test binaries (separate processes).

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use service_orchestrator_application::{Application, InMemoryRuntimeStateStore, NullLogService};
use service_orchestrator_client::{
    socket_path, ClientKind, RemoteClient, PROTOCOL_VERSION, SOCKET_ENV,
};
use service_orchestrator_config::load_from_str;
use service_orchestrator_daemon::{Daemon, DaemonError, DaemonOptions};
use service_orchestrator_desktop_lib::queries;
use service_orchestrator_execution::TaskEngine;

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
        let (application, engine) = runtime.block_on(build_core());
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

/// Builds a real application core with no executors and in-memory state.
async fn build_core() -> (Application, Arc<TaskEngine>) {
    let config = load_from_str(FIXTURE, "<desktop-test>").expect("the fixture is valid");
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
    );
    (application, engine)
}

/// Returns a unique temporary directory for one harness.
fn unique_root() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("so-desktop-{}-{nanos}", std::process::id()))
}
