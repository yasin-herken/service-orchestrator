//! Shared application state behind the Tauri commands.
//!
//! One [`AppState`] instance is managed by Tauri and holds the single
//! authoritative daemon connection (`tasks/018.md` §10): the [`RemoteClient`]
//! every command reuses, the connection tracker that is the source of truth
//! for connection events, and the task-event forwarder. The adapter never
//! re-implements configuration parsing, the task engine, or process
//! management — those stay in the shared crates.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use service_orchestrator_client::{
    codes, is_reachable, log_path, socket_path, ClientKind, OrchestratorClient, RemoteClient,
    DAEMON_BIN_ENV,
};
use service_orchestrator_config::default_config_file;
use tauri::{AppHandle, Emitter, Manager};

use crate::connection::{classify_probe, ConnectionSnapshot, ConnectionTracker, ProbeOutcome};
use crate::daemon_bin::{self, DAEMON_BINARY};
use crate::error::CommandError;
use crate::events::{
    lagged_payload, task_event_payload, CONFIGURATION_EVENT, CONNECTION_EVENT, TASK_EVENT,
};

/// How often the monitor probes a connected daemon, in seconds.
const PROBE_INTERVAL: Duration = Duration::from_secs(5);

/// The state shared by every Tauri command and background task.
pub struct AppState {
    /// The configuration file whose daemon this GUI talks to.
    config_path: PathBuf,
    /// The IPC socket endpoint for that configuration.
    socket: PathBuf,
    /// The resolved daemon binary, exported through [`DAEMON_BIN_ENV`].
    daemon_binary: Option<PathBuf>,
    /// The application handle, used to emit connection and task events.
    app: AppHandle,
    /// The single source of truth for connection state.
    tracker: ConnectionTracker,
    /// Serializes concurrent first-time connections so a burst of commands
    /// spawns at most one daemon.
    connect_lock: tokio::sync::Mutex<()>,
    /// The shared client, created on first use and reused afterwards.
    client: Mutex<Option<Arc<RemoteClient>>>,
    /// Ensures the task-event forwarder task starts exactly once.
    forwarder_started: AtomicBool,
    /// The per-service realtime log multiplexer used by the Logs screen.
    log_fanout: Arc<crate::log_fanout::LogFanout>,
}

impl AppState {
    /// Creates the state for `app`.
    ///
    /// Resolves the configuration file, socket, and daemon binary eagerly on
    /// the main thread, before background tasks start reading the environment.
    #[must_use]
    pub fn new(app: AppHandle) -> Self {
        let config_path = std::env::var_os("SERVICE_ORCHESTRATOR_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(default_config_file);
        let socket = socket_path(&config_path);
        let daemon_binary = daemon_bin::resolve_from_environment();
        if let Some(binary) = &daemon_binary {
            // Export for the shared client's autostart so every later
            // reconnect (including a manual Retry) spawns this binary.
            std::env::set_var(DAEMON_BIN_ENV, binary);
        }
        Self {
            config_path,
            socket,
            daemon_binary,
            app,
            tracker: ConnectionTracker::new(),
            connect_lock: tokio::sync::Mutex::new(()),
            client: Mutex::new(None),
            forwarder_started: AtomicBool::new(false),
            log_fanout: Arc::new(crate::log_fanout::LogFanout::default()),
        }
    }

    /// Returns the realtime log multiplexer shared by the log commands.
    #[must_use]
    pub fn log_fanout(&self) -> &Arc<crate::log_fanout::LogFanout> {
        &self.log_fanout
    }

    /// Returns the IPC socket endpoint this state probes.
    #[must_use]
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Returns the current connection snapshot.
    #[must_use]
    pub fn connection_snapshot(&self) -> ConnectionSnapshot {
        self.tracker.snapshot()
    }

    /// Returns the client when one is already connected.
    #[must_use]
    pub fn client_if_present(&self) -> Option<Arc<RemoteClient>> {
        self.client.lock().expect("client lock poisoned").clone()
    }

    /// Returns a connected client, connecting (autostarting the daemon when
    /// needed) on first use.
    ///
    /// # Errors
    ///
    /// Returns a structured error — and pushes the matching connection
    /// transition — when the daemon cannot be reached or the connection
    /// fails for any other reason.
    pub async fn ensure_client(&self) -> Result<Arc<RemoteClient>, CommandError> {
        if let Some(client) = self.client_if_present() {
            return Ok(client);
        }
        let _guard = self.connect_lock.lock().await;
        if let Some(client) = self.client_if_present() {
            return Ok(client);
        }
        let config_path = self.config_path.clone();
        let socket = self.socket.clone();
        let daemon_binary = self.daemon_binary.clone();
        let outcome = tauri::async_runtime::spawn_blocking(move || {
            connect_once(&config_path, &socket, daemon_binary.as_deref())
        })
        .await
        .map_err(|error| CommandError::internal(format!("connection task failed: {error}")))?;
        match outcome {
            Ok(client) => {
                let client = Arc::new(client);
                self.client
                    .lock()
                    .expect("client lock poisoned")
                    .get_or_insert(Arc::clone(&client));
                self.start_forwarder(Arc::clone(&client));
                self.push(&ProbeOutcome::Connected);
                Ok(client)
            }
            Err(error) => {
                self.observe(&error);
                Err(error)
            }
        }
    }

    /// Records a command-side failure, updating the connection state when the
    /// failure is connection-shaped.
    ///
    /// Every command funnels failures through here so a daemon that dies
    /// mid-session is reflected in the pushed connection events, not only in
    /// the failed command's own error (`tasks/018.md` §20).
    pub fn observe(&self, error: &CommandError) {
        match error.code.as_str() {
            code if code == codes::DAEMON_UNAVAILABLE => {
                self.push(&ProbeOutcome::Unreachable(error.message.clone()));
            }
            code if code == codes::PROTOCOL_INCOMPATIBLE => {
                self.push(&ProbeOutcome::Incompatible(error.message.clone()));
            }
            _ => {}
        }
    }

    /// Records an outcome and emits a connection event when it is a transition.
    pub fn push(&self, outcome: &ProbeOutcome) {
        if self.tracker.record(outcome) {
            let snapshot = self.tracker.snapshot();
            let _ = self.app.emit(CONNECTION_EVENT, &snapshot);
        }
    }

    /// Starts forwarding task-engine events to the webview, exactly once.
    fn start_forwarder(&self, client: Arc<RemoteClient>) {
        if self.forwarder_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut receiver = client.subscribe_task_events();
        let mut configuration = client.subscribe_configuration_events();
        let app = self.app.clone();
        let configuration_app = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        if let Some(payload) = task_event_payload(&event) {
                            let _ = app.emit(TASK_EVENT, payload);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(dropped)) => {
                        let _ = app.emit(TASK_EVENT, lagged_payload(dropped));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        // Configuration changes are broadcast to every connected client; the
        // Configuration screen refreshes its snapshot when one arrives
        // (`tasks/023.md` §39, §40).
        tauri::async_runtime::spawn(async move {
            loop {
                match configuration.recv().await {
                    Ok(change) => {
                        let _ = configuration_app.emit(CONFIGURATION_EVENT, change);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }
}

/// Performs the blocking connect attempt.
///
/// With a resolved daemon binary the shared client's autostart is enabled, so
/// an explicit user action (Retry, a real command) can (re)start the daemon —
/// always the resolved `service-orchestrator` binary, never the GUI's own
/// executable. Without one, the client only ever attaches to an
/// already-running daemon; passive probes are likewise guarded by reachability
/// so background monitoring never respawns anything
/// (`tasks/018.md` §9, §14).
fn connect_once(
    config_path: &Path,
    socket: &Path,
    daemon_binary: Option<&Path>,
) -> Result<RemoteClient, CommandError> {
    let connected = match daemon_binary {
        Some(binary) => {
            // Export before connecting so every later reconnect inside the
            // shared client spawns this same binary.
            std::env::set_var(DAEMON_BIN_ENV, binary);
            RemoteClient::connect(config_path, ClientKind::Desktop)
        }
        None if is_reachable(socket) => {
            // A daemon is already running; never risk spawning the GUI itself.
            RemoteClient::connect_existing(config_path, ClientKind::Desktop)
        }
        None => {
            return Err(CommandError::new(
                codes::DAEMON_UNAVAILABLE,
                format!(
                    "The daemon is not running and `{DAEMON_BINARY}` could not be found. \
                     Build it (`cargo build --bin service-orchestrator`), put it on PATH, \
                     or set {DAEMON_BIN_ENV}."
                ),
            ));
        }
    };
    connected.map_err(|error| {
        CommandError::from(error).with_hint(format!("daemon log: {}", log_path(socket).display()))
    })
}

/// Spawns the initial connection attempt.
///
/// The result (success or failure) is already reflected in the pushed
/// connection events, so the return value is intentionally dropped.
pub fn spawn_initial_connect(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let _ = state.ensure_client().await;
    });
}

/// Spawns the passive connection monitor.
///
/// Probing is guarded by a socket reachability check, so an idle monitor never
/// issues requests against a daemon the user stopped; starting a daemon remains
/// the business of an explicit action (a Retry, or a real command) and of the
/// shared client's single reconnect-retry — the same behaviour the TUI and MCP
/// have while they are attached (`tasks/018.md` §9).
pub fn spawn_monitor(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(PROBE_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            probe_once(&app).await;
        }
    });
}

/// Runs one passive probe against the daemon.
async fn probe_once(app: &AppHandle) {
    let client = app.state::<AppState>().client_if_present();
    let Some(client) = client else {
        // No connection yet; the initial connect attempt owns the state.
        return;
    };
    let socket = app.state::<AppState>().socket().to_path_buf();
    if !is_reachable(&socket) {
        app.state::<AppState>()
            .push(&ProbeOutcome::Unreachable(format!(
                "the daemon is not reachable at {}",
                socket.display()
            )));
        return;
    }
    // The socket answered, so this status call is a passive read. Should the
    // daemon stop between the check and the call, the shared client's single
    // reconnect-retry may start it again — the same behaviour the TUI and MCP
    // have while they are attached.
    let probe = tauri::async_runtime::spawn_blocking(move || client.daemon_status()).await;
    let outcome = match probe {
        Ok(result) => classify_probe(result),
        Err(join_error) => ProbeOutcome::Failed(format!("status probe failed: {join_error}")),
    };
    app.state::<AppState>().push(&outcome);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Extracts the error from a result without requiring `Debug` on the
    /// (non-debuggable) client type.
    fn failure<T>(result: Result<T, CommandError>) -> CommandError {
        match result {
            Ok(_) => panic!("expected a connection failure"),
            Err(error) => error,
        }
    }

    #[test]
    fn connect_once_reports_one_actionable_error_when_nothing_exists() {
        let missing = std::env::temp_dir().join("service-orchestrator-missing-socket.sock");
        let config = std::env::temp_dir().join("service-orchestrator-missing-config.toml");
        let error = failure(connect_once(&config, &missing, None));
        assert_eq!(error.code, codes::DAEMON_UNAVAILABLE);
        assert!(error.message.contains(DAEMON_BINARY), "names the binary");
        assert!(error.message.contains(DAEMON_BIN_ENV), "names the override");
    }

    #[test]
    fn connect_once_reports_the_config_and_log_when_autostart_fails() {
        // A resolved binary that does not exist leads to an autostart attempt
        // whose failure surfaces the daemon log location as a hint.
        let socket = std::env::temp_dir().join("service-orchestrator-broken-socket.sock");
        let config = std::env::temp_dir().join("service-orchestrator-broken-config.toml");
        let bogus = std::env::temp_dir().join("service-orchestrator-bogus-binary");
        let error = failure(connect_once(&config, &socket, Some(&bogus)));
        assert!(!error.message.is_empty());
        // The hint always points at the daemon log for diagnosis.
        assert!(error.message.contains("daemon log"), "hint included");
    }
}
