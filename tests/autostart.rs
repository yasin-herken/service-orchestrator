//! Integration tests for daemon auto-start (`tasks/031.md` §14, §15, §40, §45).
//!
//! These drive the **real `service-orchestrator` binary** through the shared
//! client's auto-start path — `RemoteClient::connect` / `connect_or_start` —
//! which is exactly the mechanism the desktop GUI uses when it launches without
//! a daemon already running. Unlike the daemon adapter tests (which run a daemon
//! in-process), every scenario here either lets the client spawn a detached
//! daemon or starts one as a child, so the whole path is covered: executable
//! resolution, spawn, single-instance coordination, readiness polling, stale
//! runtime files, failure diagnostics, and concurrent start attempts.
//!
//! A generated, throwaway configuration points at a temporary workspace, so no
//! real repository, credential, service, or daemon is ever touched, and the
//! socket is pinned so the tests cannot collide with a developer's daemon.

use std::ffi::{OsStr, OsString};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Barrier, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use service_orchestrator_application::ApplicationError;
use service_orchestrator_client::{
    codes, connect_or_start, is_reachable, read_frame, write_frame, ClientKind, OrchestratorClient,
    RemoteClient, RequestEnvelope, ServerMessage, DAEMON_BIN_ENV, PROTOCOL_VERSION, SOCKET_ENV,
};

/// The fixture configuration, shared with the daemon and MCP adapter tests.
const FIXTURE: &str = include_str!("../crates/mcp/tests/fixtures/config.toml");

/// Serializes the tests: `SERVICE_ORCHESTRATOR_SOCKET` and
/// `SERVICE_ORCHESTRATOR_DAEMON_BIN` are process-global, so only one test may
/// mutate them at a time.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// How long any single wait may take before the test fails.
const DEADLINE: Duration = Duration::from_secs(20);

/// The real orchestration binary under test.
fn daemon_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_service-orchestrator"))
}

/// A unique temporary directory removed when the scratch is dropped.
struct Scratch {
    /// The throwaway directory holding the configuration and runtime files.
    root: PathBuf,
    /// The configuration file the client and daemon agree on.
    config: PathBuf,
    /// The IPC endpoint the test pins the client to.
    socket: PathBuf,
    /// The previous socket override, restored on drop.
    previous_socket: Option<OsString>,
    /// The previous daemon-binary override, restored on drop.
    previous_binary: Option<OsString>,
    /// Held for the whole test so the environment overrides cannot race.
    _lock: MutexGuard<'static, ()>,
}

impl Scratch {
    /// Creates an isolated configuration, socket, and daemon-binary override.
    fn new(name: &str) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        // A Unix domain socket path must fit in `SUN_LEN` (104 bytes on macOS),
        // so the scratch root must stay short: `std::env::temp_dir()` under
        // macOS is far too long once the per-test name is appended.
        let root = PathBuf::from("/tmp").join(format!(
            "so-auto-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("the clock is after the epoch")
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the scratch root is created");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).expect("the workspace directory is created");

        let config = root.join("config.toml");
        let text = FIXTURE.replace(
            "~/Projects/demo",
            workspace
                .to_str()
                .expect("the temporary path is valid UTF-8"),
        );
        std::fs::write(&config, text).expect("the configuration is written");

        let socket = root.join("run/daemon.sock");
        let previous_socket = std::env::var_os(SOCKET_ENV);
        let previous_binary = std::env::var_os(DAEMON_BIN_ENV);
        std::env::set_var(SOCKET_ENV, &socket);
        std::env::set_var(DAEMON_BIN_ENV, daemon_binary());

        Self {
            root,
            config,
            socket,
            previous_socket,
            previous_binary,
            _lock: lock,
        }
    }

    /// The configuration file path.
    fn config(&self) -> &Path {
        &self.config
    }

    /// The IPC endpoint under test.
    fn socket(&self) -> &Path {
        &self.socket
    }

    /// Overrides the daemon binary the client will spawn.
    fn set_daemon_binary(&self, path: impl AsRef<OsStr>) {
        std::env::set_var(DAEMON_BIN_ENV, path);
    }

    /// Stops any daemon on this scratch's socket, ignoring a missing one.
    fn stop_daemon(&self) {
        if let Ok(client) = RemoteClient::connect_existing(&self.config, ClientKind::Cli) {
            let _ = client.shutdown_daemon();
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        self.stop_daemon();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && is_reachable(&self.socket) {
            std::thread::sleep(Duration::from_millis(20));
        }
        match &self.previous_socket {
            Some(value) => std::env::set_var(SOCKET_ENV, value),
            None => std::env::remove_var(SOCKET_ENV),
        }
        match &self.previous_binary {
            Some(value) => std::env::set_var(DAEMON_BIN_ENV, value),
            None => std::env::remove_var(DAEMON_BIN_ENV),
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Spawns a real daemon child against the scratch configuration.
fn spawn_daemon(scratch: &Scratch) -> Child {
    Command::new(daemon_binary())
        .arg("daemon")
        .arg("--config")
        .arg(scratch.config())
        .env(SOCKET_ENV, scratch.socket())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the daemon launches")
}

/// Polls until the daemon at `socket` is reachable, failing on child exit.
fn wait_for_socket(socket: &Path, child: &mut Child) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if UnixStream::connect(socket).is_ok() {
            return;
        }
        if let Some(status) = child.try_wait().expect("the daemon is waitable") {
            panic!("the daemon exited during startup with {status}");
        }
        assert!(
            Instant::now() < deadline,
            "the daemon did not become reachable in time"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Writes an executable stand-in daemon script and returns its path.
fn write_script(root: &Path, name: &str, body: &str) -> PathBuf {
    let path = root.join(name);
    std::fs::write(&path, body).expect("the stand-in daemon is written");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("the stand-in daemon is executable");
    path
}

/// The pid of the daemon currently serving this scratch's socket.
fn daemon_pid(scratch: &Scratch) -> u32 {
    RemoteClient::connect_existing(scratch.config(), ClientKind::Cli)
        .expect("the daemon is reachable")
        .daemon_status()
        .expect("status answers")
        .pid
}

/// A missing daemon is started automatically and immediately serves requests.
#[test]
fn a_missing_daemon_is_auto_started_and_serves() {
    let scratch = Scratch::new("cold-start");
    assert!(
        !is_reachable(scratch.socket()),
        "no daemon is listening yet"
    );

    let client = RemoteClient::connect(scratch.config(), ClientKind::Desktop)
        .expect("the daemon auto-starts");

    let workspaces = client.list_workspaces().expect("the started daemon serves");
    assert_eq!(workspaces.len(), 1);
    let status = client.daemon_status().expect("status answers");
    assert_ne!(
        status.pid,
        std::process::id(),
        "a separate daemon process serves the request"
    );
    assert!(is_reachable(scratch.socket()), "the socket now exists");
}

/// An already-running daemon is reused; no second daemon is started.
#[test]
fn an_existing_daemon_is_reused_without_a_second_start() {
    let scratch = Scratch::new("reuse");
    let mut daemon = spawn_daemon(&scratch);
    wait_for_socket(scratch.socket(), &mut daemon);
    let existing = daemon_pid(&scratch);

    let client = RemoteClient::connect(scratch.config(), ClientKind::Desktop)
        .expect("the existing daemon is reached");
    assert_eq!(
        client.daemon_status().expect("status answers").pid,
        existing,
        "the same daemon process is reused"
    );

    // A second autostart against the same socket also attaches to it.
    let second = RemoteClient::connect(scratch.config(), ClientKind::Desktop)
        .expect("the existing daemon is reached again");
    assert_eq!(
        second.daemon_status().expect("status answers").pid,
        existing
    );

    let _ = daemon.kill();
    let _ = daemon.wait();
}

/// A stale socket file left by a dead daemon is replaced safely on start.
#[test]
fn a_stale_socket_file_is_replaced_on_start() {
    let scratch = Scratch::new("stale-socket");
    std::fs::create_dir_all(scratch.socket().parent().expect("the run dir has a parent"))
        .expect("the run directory is created");
    std::fs::write(scratch.socket(), b"stale socket file").expect("the stale file is written");
    assert!(
        !is_reachable(scratch.socket()),
        "a plain stale file is not a live listener"
    );

    let client = RemoteClient::connect(scratch.config(), ClientKind::Desktop)
        .expect("the stale socket is replaced and the daemon starts");
    assert_eq!(
        client.list_workspaces().expect("the daemon serves").len(),
        1
    );
}

/// A missing daemon binary is reported with the override named, never spawned.
#[test]
fn a_missing_daemon_binary_is_reported_actionably() {
    let scratch = Scratch::new("missing-binary");
    let missing = scratch.root.join("no-such-service-orchestrator");
    scratch.set_daemon_binary(&missing);

    let error = connect_or_start(scratch.config(), scratch.socket())
        .expect_err("there is no binary to run");
    assert!(
        error.to_string().contains(DAEMON_BIN_ENV),
        "the error names the override: {error}"
    );
}

/// A daemon that exits during startup is reported with actionable diagnostics.
#[test]
fn a_daemon_that_exits_is_reported_with_diagnostics() {
    let scratch = Scratch::new("failing-daemon");
    let script = write_script(&scratch.root, "failing-daemon", "#!/bin/sh\nexit 7\n");
    scratch.set_daemon_binary(&script);

    let error = connect_or_start(scratch.config(), scratch.socket())
        .expect_err("the daemon exits before binding");
    let message = error.to_string();
    assert!(
        message.contains(&script.display().to_string()),
        "names the executable: {message}"
    );
    assert!(
        message.contains(scratch.socket().to_str().expect("utf-8 path")),
        "names the endpoint: {message}"
    );
    assert!(message.contains('7'), "reports the exit status: {message}");
}

/// Concurrent start attempts converge on a single daemon process.
#[test]
fn concurrent_start_attempts_converge_on_one_daemon() {
    let scratch = Scratch::new("concurrent");
    let barrier = Arc::new(Barrier::new(4));
    let config = scratch.config().to_path_buf();

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let config = config.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                RemoteClient::connect(config, ClientKind::Desktop)
            })
        })
        .collect();

    let clients: Vec<RemoteClient> = handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .expect("the start thread finishes")
                .expect("every concurrent attempt connects")
        })
        .collect();

    let expected = clients[0].daemon_status().expect("status answers").pid;
    for client in &clients {
        assert_eq!(
            client.daemon_status().expect("status answers").pid,
            expected,
            "all concurrent clients share one daemon"
        );
    }
}

/// An existing daemon that accepts the connection but never answers (the
/// fingerprint of a daemon too old to parse the client's request) is reported
/// with actionable guidance instead of a bare "connection was closed".
#[test]
fn a_daemon_that_closes_without_answering_is_reported_with_guidance() {
    use std::io::Read;

    let scratch = Scratch::new("silent-daemon");
    std::fs::create_dir_all(scratch.socket().parent().expect("the run dir has a parent"))
        .expect("the run directory is created");
    let socket = scratch.socket().to_path_buf();
    let listener = std::os::unix::net::UnixListener::bind(&socket).expect("the endpoint binds");
    let server = std::thread::spawn(move || {
        // Read one request frame, then close without answering — exactly what a
        // daemon whose `ClientKind` predates this client does.
        if let Ok((mut stream, _)) = listener.accept() {
            let mut length = [0u8; 4];
            if stream.read_exact(&mut length).is_ok() {
                let mut body = vec![0u8; u32::from_be_bytes(length) as usize];
                let _ = stream.read_exact(&mut body);
            }
        }
    });

    let client = RemoteClient::connect_existing(scratch.config(), ClientKind::Desktop)
        .expect("the endpoint is reachable");
    let error = client
        .daemon_status()
        .expect_err("a silent daemon never answers");
    let message = error.to_string();
    assert!(
        message.contains("older or incompatible"),
        "names the likely cause: {message}"
    );
    assert!(
        message.contains("restart"),
        "tells the user how to recover: {message}"
    );

    server.join().expect("the fake daemon finishes");
}

/// An incompatible daemon is reported and never replaced or terminated.
#[test]
fn an_incompatible_daemon_is_not_replaced() {
    let scratch = Scratch::new("incompatible");
    let client = RemoteClient::connect(scratch.config(), ClientKind::Desktop)
        .expect("the daemon auto-starts");
    let existing = client.daemon_status().expect("status answers").pid;
    drop(client);

    // Speak an impossible future protocol directly over the socket.
    let mut stream = UnixStream::connect(scratch.socket()).expect("the daemon is reachable");
    let request = RequestEnvelope {
        protocol: PROTOCOL_VERSION + 1,
        request_id: "req-incompatible".to_owned(),
        client: ClientKind::Desktop,
        allow_destructive: false,
        method: "daemon_status".to_owned(),
        params: serde_json::json!([]),
    };
    write_frame(&mut stream, &request).expect("the request is written");
    let message: ServerMessage = read_frame(&mut stream)
        .expect("the response is read")
        .expect("the daemon answers");
    match message {
        ServerMessage::Response(response) => match response.result {
            Err(ApplicationError::RuntimeUnavailable { code, .. }) => {
                assert_eq!(code, codes::PROTOCOL_INCOMPATIBLE);
            }
            other => panic!("expected a protocol error, got {other:?}"),
        },
        other => panic!("expected a response, got {other:?}"),
    }

    // The GUI must not replace the running daemon: the same process still serves.
    let after = RemoteClient::connect(scratch.config(), ClientKind::Desktop)
        .expect("the daemon is still reachable");
    assert_eq!(
        after.daemon_status().expect("status answers").pid,
        existing,
        "the incompatible daemon was neither replaced nor terminated"
    );
}
