//! The local IPC server: accept loop, per-connection handling, and event fanout.
//!
//! The server owns no business logic. It validates the protocol version,
//! dispatches each request to the shared [`Application`](service_orchestrator_application::Application)
//! through [`crate::dispatch`], and fans task and log events out to connected
//! clients over bounded channels so a slow client can never block the daemon
//! (`tasks/015.md` §9, §20, §35).

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener as StdUnixListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use service_orchestrator_application::{
    Application, ApplicationError, Authorization, LogStreamError, LogSubscription,
};
use service_orchestrator_client::{
    codes, unavailable, ConfigurationChanged, DaemonStatus, RequestEnvelope, ResponseEnvelope,
    ServerEvent, ServerMessage, MAX_FRAME_BYTES, PROTOCOL_VERSION,
};
use service_orchestrator_config::ConfigChange;
use service_orchestrator_domain::ServiceId;
use service_orchestrator_execution::TaskEngine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::{broadcast, mpsc, watch, RwLock};
use tokio::task::JoinHandle;

use crate::dispatch;
use crate::error::DaemonError;
use crate::lock::{write_pid_file, InstanceLock, SocketGuard};

/// How long the daemon waits for connections to flush a shutdown notice.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_millis(150);

/// How long the accept loop pauses after a transient accept error.
const ACCEPT_BACKOFF: std::time::Duration = std::time::Duration::from_millis(50);

/// The per-connection outbound queue capacity.
///
/// Responses and pushed events share one bounded queue so a client that stops
/// reading can never grow the daemon's memory without limit
/// (`tasks/017.md` §18, defect D7).
const OUTBOUND_CAPACITY: usize = 256;

/// How many configuration-change events a slow client may fall behind before it
/// is told it lagged. Configuration changes are rare, so this only needs to
/// cover a burst of edits.
const CONFIG_EVENT_CAPACITY: usize = 64;

/// Configuration for a running daemon.
#[derive(Clone, Debug)]
pub struct DaemonOptions {
    /// The IPC endpoint to bind.
    pub socket_path: PathBuf,
}

/// The local shared runtime daemon.
pub struct Daemon {
    application: Application,
    engine: Arc<TaskEngine>,
    options: DaemonOptions,
}

impl Daemon {
    /// Creates a daemon over an application core and its task engine.
    #[must_use]
    pub fn new(application: Application, engine: Arc<TaskEngine>, options: DaemonOptions) -> Self {
        Self {
            application,
            engine,
            options,
        }
    }

    /// Binds the IPC endpoint and serves until a shutdown is requested.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError`] when the runtime directory cannot be prepared,
    /// another daemon already owns it, or the socket cannot be bound.
    pub async fn run(self) -> Result<(), DaemonError> {
        let socket = self.options.socket_path.clone();
        service_orchestrator_client::ensure_run_dir(&socket)
            .map_err(|error| DaemonError::io("prepare the runtime directory", &socket, error))?;

        // Acquire the single-instance lock before touching the socket: the lock
        // is released by the kernel on process death, so a stale socket file can
        // be removed safely once the lock is held (`tasks/015.md` §15).
        let lock = InstanceLock::acquire(&socket)?;
        if socket.exists() {
            std::fs::remove_file(&socket)
                .map_err(|error| DaemonError::io("remove the stale socket", &socket, error))?;
        }

        let std_listener = StdUnixListener::bind(&socket)
            .map_err(|error| DaemonError::io("bind the socket", &socket, error))?;
        std_listener
            .set_nonblocking(true)
            .map_err(|error| DaemonError::io("configure the socket", &socket, error))?;
        // The socket is owner-only: another local user cannot connect
        // (`tasks/015.md` §24).
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| DaemonError::io("restrict the socket", &socket, error))?;
        let listener = UnixListener::from_std(std_listener)
            .map_err(|error| DaemonError::io("adopt the socket", &socket, error))?;

        let pid = std::process::id();
        write_pid_file(&socket, pid)?;
        let guard = SocketGuard::new(&socket);

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let (config_events, _config_receiver) = broadcast::channel(CONFIG_EVENT_CAPACITY);
        let shared = Arc::new(Shared {
            application: RwLock::new(self.application),
            engine: self.engine,
            socket_path: socket.clone(),
            started_at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs())
                .unwrap_or(0),
            clients: AtomicUsize::new(0),
            config_events,
            shutdown_tx: shutdown_tx.clone(),
        });

        let accept = tokio::spawn(accept_loop(
            listener,
            Arc::clone(&shared),
            shutdown_rx.clone(),
        ));

        wait_for_shutdown(&shared, shutdown_rx).await;
        let _ = shutdown_tx.send(true);
        tokio::time::sleep(SHUTDOWN_GRACE).await;
        accept.abort();

        guard.remove_pid();
        drop(lock);
        let _ = guard;
        Ok(())
    }
}

/// State shared by every connection.
struct Shared {
    /// The authoritative application core.
    ///
    /// Behind a lock so a configuration commit can refresh the core's cached
    /// snapshot while a reader request holds a cheap clone
    /// (`tasks/023.md` §38, §40).
    application: RwLock<Application>,
    engine: Arc<TaskEngine>,
    socket_path: PathBuf,
    started_at_unix: u64,
    clients: AtomicUsize,
    /// Configuration-change events fanned out to every client.
    config_events: broadcast::Sender<ConfigurationChanged>,
    shutdown_tx: watch::Sender<bool>,
}

impl Shared {
    /// Requests a graceful shutdown.
    fn request_shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Builds the daemon's diagnostic snapshot.
    fn status(&self, application: &Application) -> DaemonStatus {
        let mut managed_services = 0usize;
        for workspace in application.list_workspaces() {
            if let Ok(running) = application.list_running_services(&workspace.id) {
                managed_services += running.len();
            }
        }
        DaemonStatus {
            pid: std::process::id(),
            socket_path: self.socket_path.display().to_string(),
            protocol_version: PROTOCOL_VERSION,
            clients: self.clients.load(Ordering::SeqCst),
            active_tasks: self.engine.list_running_tasks().len(),
            managed_services,
            started_at_unix: self.started_at_unix,
        }
    }
}

/// Accepts connections until shutdown.
async fn accept_loop(
    listener: UnixListener,
    shared: Arc<Shared>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        // The connecting peer's identity is taken from the
                        // kernel, never from the request envelope: a connection
                        // whose UID does not match the daemon's is never treated
                        // as an interactive (TUI) peer, so it can never obtain
                        // destructive authorization (`tasks/017.md` §27,
                        // finding H-IPC-1).
                        let same_user = stream
                            .peer_cred()
                            .is_ok_and(|cred| cred.uid() == nix::unistd::geteuid().as_raw());
                        shared.clients.fetch_add(1, Ordering::SeqCst);
                        let shared = Arc::clone(&shared);
                        let shutdown = shutdown.clone();
                        tokio::spawn(async move {
                            handle_connection(stream, shared, shutdown, same_user).await;
                        });
                    }
                    Err(_) => tokio::time::sleep(ACCEPT_BACKOFF).await,
                }
            }
        }
    }
}

/// Serves one client connection.
async fn handle_connection(
    stream: tokio::net::UnixStream,
    shared: Arc<Shared>,
    mut shutdown: watch::Receiver<bool>,
    same_user: bool,
) {
    let (mut reader, mut writer) = stream.into_split();
    let (out, mut out_rx) = mpsc::channel::<ServerMessage>(OUTBOUND_CAPACITY);

    // The writer task serializes every outgoing message so a response and a
    // pushed event can never interleave mid-frame.
    let writer_task = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            if write_frame_async(&mut writer, &message).await.is_err() {
                break;
            }
        }
    });

    // Forward the engine's task events to this client.
    let mut events = shared.engine.subscribe();
    let event_out = out.clone();
    let event_pump = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if event_out
                        .send(ServerMessage::Event(ServerEvent::Task { event }))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    });

    // Forward configuration-change events so every client refreshes its view of
    // the active configuration (`tasks/023.md` §39, §40).
    let mut config_events = shared.config_events.subscribe();
    let config_out = out.clone();
    let config_pump = tokio::spawn(async move {
        loop {
            match config_events.recv().await {
                Ok(change) => {
                    if config_out
                        .send(ServerMessage::Event(ServerEvent::ConfigurationChanged(
                            change,
                        )))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    });

    let mut subscriptions: HashMap<ServiceId, JoinHandle<()>> = HashMap::new();

    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                let _ = out.send(ServerMessage::Event(ServerEvent::Shutdown {
                    reason: "the daemon is shutting down".to_owned(),
                })).await;
                break;
            }
            frame = read_frame_async::<_, RequestEnvelope>(&mut reader) => {
                match frame {
                    Ok(Some(request)) => {
                        handle_request(&shared, &out, request, &mut subscriptions, same_user).await;
                    }
                    Ok(None) => break,
                    Err(error) => {
                        // Malformed input never crashes the daemon, but the
                        // peer now receives a structured error instead of a
                        // bare disconnect (`tasks/017.md` §18, finding G3).
                        let _ = out.send(ServerMessage::Response(ResponseEnvelope {
                            request_id: "unknown".to_owned(),
                            result: Err(unavailable(
                                codes::INVALID_REQUEST,
                                format!("the request frame could not be read: {error}"),
                            )),
                        })).await;
                        break;
                    }
                }
            }
        }
    }

    for (_, handle) in subscriptions.drain() {
        handle.abort();
    }
    event_pump.abort();
    config_pump.abort();
    drop(out);
    let _ = writer_task.await;
    shared.clients.fetch_sub(1, Ordering::SeqCst);
}

/// Handles a single decoded request.
async fn handle_request(
    shared: &Arc<Shared>,
    out: &mpsc::Sender<ServerMessage>,
    request: RequestEnvelope,
    subscriptions: &mut HashMap<ServiceId, JoinHandle<()>>,
    same_user: bool,
) {
    // Enforce protocol compatibility before interpreting anything
    // (`tasks/015.md` §25).
    if request.protocol != PROTOCOL_VERSION {
        respond(
            out,
            &request,
            Err(unavailable(
                codes::PROTOCOL_INCOMPATIBLE,
                format!(
                    "the client speaks protocol version {} but this daemon speaks version {}; \
                     upgrade or restart the client",
                    request.protocol, PROTOCOL_VERSION
                ),
            )),
        )
        .await;
        return;
    }

    match request.method.as_str() {
        service_orchestrator_client::ops::DAEMON_STATUS => {
            let application = shared.application.read().await;
            respond(out, &request, encode(shared.status(&application))).await;
        }
        service_orchestrator_client::ops::DAEMON_SHUTDOWN => {
            shared.request_shutdown();
            respond(out, &request, encode(())).await;
        }
        // A configuration commit takes the write lock so it can validate and
        // persist the change and then refresh the core's cached snapshot before
        // any other request observes it (`tasks/023.md` §38, §40).
        service_orchestrator_client::ops::COMMIT_CONFIGURATION => {
            handle_commit_configuration(shared, out, request, same_user).await;
        }
        // A reload re-reads the file and refreshes the cached snapshot.
        service_orchestrator_client::ops::RELOAD_CONFIGURATION => {
            handle_reload_configuration(shared, out, request).await;
        }
        service_orchestrator_client::ops::SUBSCRIBE_SERVICE_LOGS => {
            let decoded: Result<(ServiceId,), _> = serde_json::from_value(request.params.clone());
            match decoded {
                Ok((service_id,)) => match subscriptions.entry(service_id.clone()) {
                    std::collections::hash_map::Entry::Occupied(_) => {
                        respond(out, &request, encode(())).await;
                    }
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        let subscription = shared
                            .application
                            .read()
                            .await
                            .subscribe_service_logs(&service_id);
                        match subscription {
                            Ok(subscription) => {
                                let handle =
                                    spawn_log_pump(service_id.clone(), subscription, out.clone());
                                entry.insert(handle);
                                respond(out, &request, encode(())).await;
                            }
                            Err(error) => respond(out, &request, Err(error)).await,
                        }
                    }
                },
                Err(error) => {
                    respond(
                        out,
                        &request,
                        Err(unavailable(
                            codes::INVALID_REQUEST,
                            format!("invalid subscribe parameters: {error}"),
                        )),
                    )
                    .await
                }
            }
        }
        service_orchestrator_client::ops::UNSUBSCRIBE_SERVICE_LOGS => {
            if let Ok((service_id,)) =
                serde_json::from_value::<(ServiceId,)>(request.params.clone())
            {
                if let Some(handle) = subscriptions.remove(&service_id) {
                    handle.abort();
                }
            }
            respond(out, &request, encode(())).await;
        }
        _ => {
            // A read-only request works on a cheap clone taken under a read
            // guard, so it never blocks a concurrent configuration commit for
            // the duration of the operation and a commit is always applied to a
            // fresh snapshot (`tasks/023.md` §38, §40).
            let application = shared.application.read().await.clone();
            let result = dispatch::route(&application, &shared.engine, &request, same_user).await;
            respond(out, &request, result).await;
        }
    }
}

/// Validates and persists a configuration change, then refreshes the daemon and
/// broadcasts a change event.
async fn handle_commit_configuration(
    shared: &Arc<Shared>,
    out: &mpsc::Sender<ServerMessage>,
    request: RequestEnvelope,
    same_user: bool,
) {
    let allow_destructive = dispatch::destructive_allowed(&request, same_user);
    let authorization = if allow_destructive {
        Authorization::allow_destructive()
    } else {
        Authorization::standard()
    };
    let decoded: Result<(ConfigChange, String), _> = serde_json::from_value(request.params.clone());
    let (change, revision) = match decoded {
        Ok(decoded) => decoded,
        Err(error) => {
            respond(
                out,
                &request,
                Err(unavailable(
                    codes::INVALID_REQUEST,
                    format!("invalid configuration change parameters: {error}"),
                )),
            )
            .await;
            return;
        }
    };

    let mut application = shared.application.write().await;
    // The commit runs on an authorized clone; the clone shares the same
    // configuration manager, so the shared core's snapshot is refreshed from the
    // manager below (`tasks/015.md` §23).
    let authorized = application.clone().with_authorization(authorization);
    match authorized.commit_configuration(&change, &revision) {
        Ok(commit) => {
            application.refresh_configuration();
            drop(application);
            let _ = shared.config_events.send(ConfigurationChanged {
                revision: commit.revision.clone(),
                description: commit.description.clone(),
            });
            respond(out, &request, encode(commit)).await;
        }
        Err(error) => respond(out, &request, Err(error)).await,
    }
}

/// Re-reads and validates the configuration file and refreshes the daemon.
async fn handle_reload_configuration(
    shared: &Arc<Shared>,
    out: &mpsc::Sender<ServerMessage>,
    request: RequestEnvelope,
) {
    let mut application = shared.application.write().await;
    match application.reload_configuration() {
        Ok(reload) => {
            application.refresh_configuration();
            drop(application);
            let _ = shared.config_events.send(ConfigurationChanged {
                revision: reload.revision.clone(),
                description: "reloaded configuration from disk".to_owned(),
            });
            respond(out, &request, encode(reload)).await;
        }
        Err(error) => respond(out, &request, Err(error)).await,
    }
}

/// Pumps a service's live log entries to a connection.
fn spawn_log_pump(
    service_id: ServiceId,
    mut subscription: Box<dyn LogSubscription>,
    out: mpsc::Sender<ServerMessage>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match subscription.recv().await {
                Ok(entry) => {
                    if out
                        .send(ServerMessage::Event(ServerEvent::Log {
                            service_id: service_id.clone(),
                            entry,
                        }))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Err(LogStreamError::Lagged { .. }) => continue,
                Err(LogStreamError::Closed) => return,
            }
        }
    })
}

/// Sends a response envelope.
async fn respond(
    out: &mpsc::Sender<ServerMessage>,
    request: &RequestEnvelope,
    result: Result<Value, ApplicationError>,
) {
    let _ = out
        .send(ServerMessage::Response(ResponseEnvelope {
            request_id: request.request_id.clone(),
            result,
        }))
        .await;
}

/// Serializes an operation result into a JSON value.
fn encode<T: Serialize>(value: T) -> Result<Value, ApplicationError> {
    serde_json::to_value(value).map_err(|error| {
        unavailable(
            codes::INVALID_REQUEST,
            format!("the operation's result could not be serialized: {error}"),
        )
    })
}

/// Waits for an explicit shutdown request or a termination signal.
async fn wait_for_shutdown(shared: &Arc<Shared>, mut shutdown: watch::Receiver<bool>) {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("a SIGTERM handler can be installed");

    tokio::select! {
        _ = shutdown.changed() => {}
        _ = tokio::signal::ctrl_c() => shared.request_shutdown(),
        _ = sigterm.recv() => shared.request_shutdown(),
    }
}

// -- asynchronous framing ----------------------------------------------------

/// Writes one length-prefixed JSON frame.
async fn write_frame_async<W, T>(writer: &mut W, value: &T) -> std::io::Result<()>
where
    W: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let bytes = serde_json::to_vec(value)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "frame exceeds the maximum size",
        ));
    }
    let length = u32::try_from(bytes.len())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "frame is too large"))?;
    writer.write_all(&length.to_be_bytes()).await?;
    writer.write_all(&bytes).await?;
    writer.flush().await
}

/// Reads one length-prefixed JSON frame.
async fn read_frame_async<R, T>(reader: &mut R) -> std::io::Result<Option<T>>
where
    R: AsyncReadExt + Unpin,
    T: serde::de::DeserializeOwned,
{
    let mut length_bytes = [0u8; 4];
    match reader.read_exact(&mut length_bytes).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = u32::from_be_bytes(length_bytes) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame exceeds the maximum size",
        ));
    }
    let mut buffer = vec![0u8; length];
    reader.read_exact(&mut buffer).await?;
    let value = serde_json::from_slice(&buffer)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    Ok(Some(value))
}
