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
    Application, ApplicationError, LogStreamError, LogSubscription,
};
use service_orchestrator_client::{
    codes, unavailable, DaemonStatus, RequestEnvelope, ResponseEnvelope, ServerEvent,
    ServerMessage, MAX_FRAME_BYTES, PROTOCOL_VERSION,
};
use service_orchestrator_domain::ServiceId;
use service_orchestrator_execution::TaskEngine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;

use crate::dispatch;
use crate::error::DaemonError;
use crate::lock::{write_pid_file, InstanceLock, SocketGuard};

/// How long the daemon waits for connections to flush a shutdown notice.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_millis(150);

/// How long the accept loop pauses after a transient accept error.
const ACCEPT_BACKOFF: std::time::Duration = std::time::Duration::from_millis(50);

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
        let shared = Arc::new(Shared {
            application: self.application,
            engine: self.engine,
            socket_path: socket.clone(),
            started_at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs())
                .unwrap_or(0),
            clients: AtomicUsize::new(0),
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
    application: Application,
    engine: Arc<TaskEngine>,
    socket_path: PathBuf,
    started_at_unix: u64,
    clients: AtomicUsize,
    shutdown_tx: watch::Sender<bool>,
}

impl Shared {
    /// Requests a graceful shutdown.
    fn request_shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Builds the daemon's diagnostic snapshot.
    fn status(&self) -> DaemonStatus {
        let mut managed_services = 0usize;
        for workspace in self.application.list_workspaces() {
            if let Ok(running) = self.application.list_running_services(&workspace.id) {
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
                        shared.clients.fetch_add(1, Ordering::SeqCst);
                        let shared = Arc::clone(&shared);
                        let shutdown = shutdown.clone();
                        tokio::spawn(async move {
                            handle_connection(stream, shared, shutdown).await;
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
) {
    let (mut reader, mut writer) = stream.into_split();
    let (out, mut out_rx) = mpsc::unbounded_channel::<ServerMessage>();

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
                }));
                break;
            }
            frame = read_frame_async::<_, RequestEnvelope>(&mut reader) => {
                match frame {
                    Ok(Some(request)) => {
                        handle_request(&shared, &out, request, &mut subscriptions).await;
                    }
                    Ok(None) | Err(_) => break,
                }
            }
        }
    }

    for (_, handle) in subscriptions.drain() {
        handle.abort();
    }
    event_pump.abort();
    drop(out);
    let _ = writer_task.await;
    shared.clients.fetch_sub(1, Ordering::SeqCst);
}

/// Handles a single decoded request.
async fn handle_request(
    shared: &Arc<Shared>,
    out: &mpsc::UnboundedSender<ServerMessage>,
    request: RequestEnvelope,
    subscriptions: &mut HashMap<ServiceId, JoinHandle<()>>,
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
        );
        return;
    }

    match request.method.as_str() {
        service_orchestrator_client::ops::DAEMON_STATUS => {
            respond(out, &request, encode(shared.status()));
        }
        service_orchestrator_client::ops::DAEMON_SHUTDOWN => {
            shared.request_shutdown();
            respond(out, &request, encode(()));
        }
        service_orchestrator_client::ops::SUBSCRIBE_SERVICE_LOGS => {
            let decoded: Result<(ServiceId,), _> = serde_json::from_value(request.params.clone());
            match decoded {
                Ok((service_id,)) => match subscriptions.entry(service_id.clone()) {
                    std::collections::hash_map::Entry::Occupied(_) => {
                        respond(out, &request, encode(()));
                    }
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        match shared.application.subscribe_service_logs(&service_id) {
                            Ok(subscription) => {
                                let handle =
                                    spawn_log_pump(service_id.clone(), subscription, out.clone());
                                entry.insert(handle);
                                respond(out, &request, encode(()));
                            }
                            Err(error) => respond(out, &request, Err(error)),
                        }
                    }
                },
                Err(error) => respond(
                    out,
                    &request,
                    Err(unavailable(
                        codes::INVALID_REQUEST,
                        format!("invalid subscribe parameters: {error}"),
                    )),
                ),
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
            respond(out, &request, encode(()));
        }
        _ => {
            let result = dispatch::route(&shared.application, &shared.engine, &request).await;
            respond(out, &request, result);
        }
    }
}

/// Pumps a service's live log entries to a connection.
fn spawn_log_pump(
    service_id: ServiceId,
    mut subscription: Box<dyn LogSubscription>,
    out: mpsc::UnboundedSender<ServerMessage>,
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
fn respond(
    out: &mpsc::UnboundedSender<ServerMessage>,
    request: &RequestEnvelope,
    result: Result<Value, ApplicationError>,
) {
    let _ = out.send(ServerMessage::Response(ResponseEnvelope {
        request_id: request.request_id.clone(),
        result,
    }));
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
