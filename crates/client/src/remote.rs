//! The daemon-backed [`OrchestratorClient`](crate::OrchestratorClient).
//!
//! [`RemoteClient`] is a thin proxy: each trait method serializes its arguments,
//! sends one request over the Unix domain socket, and returns the structured
//! result. It contains no business logic and never executes infrastructure
//! (`tasks/015.md` §9, §11, §13).
//!
//! # Transport
//!
//! A dedicated reader thread owns the read half of the socket and demultiplexes
//! server messages: responses are matched to the waiting caller by `request_id`,
//! task events are republished on a bounded broadcast channel, and log events
//! are routed to the subscription that asked for them. The write half is
//! serialized behind a mutex. Calls are therefore blocking, which matches the
//! synchronous interface state machine; the local round trip is sub-millisecond.
//!
//! # Reconnection
//!
//! If the connection is already gone when a request is about to be written, the
//! client transparently reconnects (starting the daemon if needed) and retries
//! **once**. A request that was written but produced no response is *never*
//! retried, because the mutation may have been applied — the caller receives
//! [`ApplicationError::RuntimeUnavailable`] instead (`tasks/015.md` §21, §22).

use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::Duration;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use service_orchestrator_application::{
    ApplicationError, Authorization, BulkOperationSummary, DiffOptions, EnvironmentPlan,
    HealthCheckScope, LibraryArtifactVerification, LibraryDependent, LibraryPrepareOptions,
    LibraryStatus, LogEntry, LogPage, LogRequest, LogStreamError, LogSubscription, MigrationStatus,
    OperationResult, PrepareEnvironmentRequest, PrepareSelectedLibrariesRequest,
    ServiceDiagnostics, ServiceHealth, ServiceRuntimeStatus, ServiceStatus, WorkflowCancellation,
    WorkflowStatus, WorkspaceStatus, WorkspaceValidationReport,
};
use service_orchestrator_domain::{
    Branch, BranchInfo, GroupId, Library, LibraryId, Profile, ProfileId, RepoDiff,
    RepositoryStatus, Service, ServiceGroup, ServiceId, Task, TaskId, Workflow, WorkflowId,
    Workspace, WorkspaceId,
};
use service_orchestrator_execution::TaskEvent;
use tokio::sync::broadcast;
use tokio::sync::mpsc as tokio_mpsc;

use crate::client::{ConfigSummary, OrchestratorClient};
use crate::ops;
use crate::protocol::{
    codes, read_frame, unavailable, write_frame, ClientKind, DiffParams, RequestEnvelope,
    ResponseEnvelope, ServerEvent, ServerMessage, PROTOCOL_VERSION,
};
use crate::socket::{connect_existing, connect_or_start, socket_path};

/// How long the client waits for a request's response.
///
/// A local round trip is sub-millisecond; the generous bound exists only so a
/// hung daemon cannot block an interface forever.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// A diagnostic snapshot of the daemon (`tasks/015.md` §30).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonStatus {
    /// The daemon's operating-system process id.
    pub pid: u32,
    /// The IPC endpoint the daemon is listening on.
    pub socket_path: String,
    /// The protocol version the daemon speaks.
    pub protocol_version: u32,
    /// How many clients are currently connected.
    pub clients: usize,
    /// How many tasks are active.
    pub active_tasks: usize,
    /// How many services are currently managed as running processes.
    pub managed_services: usize,
    /// When the daemon started, in seconds since the Unix epoch.
    pub started_at_unix: u64,
}

/// The outcome of one send attempt.
enum Attempt {
    /// A response was received.
    Done(serde_json::Value),
    /// The daemon answered with a structured application error.
    App(ApplicationError),
    /// The request was never written; retrying is safe.
    NotSent,
    /// The request was written but no response arrived; retrying is unsafe.
    Lost,
}

/// The connection state shared by every clone of a [`RemoteClient`].
struct Inner {
    config_path: PathBuf,
    socket_path: PathBuf,
    kind: ClientKind,
    /// Whether a missing daemon may be started on demand.
    autostart: bool,
    /// The live connection, or `None` while disconnected.
    connection: Mutex<Option<Arc<Connection>>>,
    /// Task events republished from the server to local subscribers.
    events: broadcast::Sender<TaskEvent>,
    /// Realtime log subscriptions, by service. Kept here (not on a connection)
    /// so they survive a reconnect.
    logs: Mutex<HashMap<ServiceId, tokio_mpsc::UnboundedSender<LogEntry>>>,
}

/// One socket connection and its in-flight requests.
struct Connection {
    /// The write half, serialized behind a mutex.
    writer: Mutex<UnixStream>,
    /// Requests awaiting a response, keyed by request id.
    pending: Mutex<HashMap<String, Sender<Result<serde_json::Value, ApplicationError>>>>,
    /// The next request id.
    next_id: AtomicU64,
    /// Whether the reader thread has observed the connection close.
    alive: AtomicBool,
}

impl Connection {
    /// Generates a unique request identifier.
    fn next_request_id(&self) -> String {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        format!("req-{id}")
    }

    /// Registers a pending request, returning the id and its response channel.
    fn register(
        &self,
    ) -> (
        String,
        Receiver<Result<serde_json::Value, ApplicationError>>,
    ) {
        let (sender, receiver) = mpsc::channel();
        let id = self.next_request_id();
        self.pending
            .lock()
            .expect("pending map is not poisoned")
            .insert(id.clone(), sender);
        (id, receiver)
    }

    /// Delivers a response to its waiting caller, if any.
    fn deliver(&self, response: ResponseEnvelope) {
        let sender = self
            .pending
            .lock()
            .expect("pending map is not poisoned")
            .remove(&response.request_id);
        if let Some(sender) = sender {
            let _ = sender.send(response.result);
        }
    }

    /// Marks the connection dead and fails every in-flight request.
    fn mark_dead(&self) {
        self.alive.store(false, Ordering::SeqCst);
        let mut pending = self.pending.lock().expect("pending map is not poisoned");
        for (_, sender) in pending.drain() {
            let _ = sender.send(Err(unavailable(
                codes::DAEMON_UNAVAILABLE,
                "the daemon connection was closed",
            )));
        }
    }
}

/// A client that proxies the application core to the local daemon.
pub struct RemoteClient {
    inner: Arc<Inner>,
    allow_destructive: bool,
}

impl RemoteClient {
    /// Connects to the daemon for `config_path`, starting it if necessary.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::RuntimeUnavailable`] when no daemon can be
    /// reached or started.
    pub fn connect(
        config_path: impl Into<PathBuf>,
        kind: ClientKind,
    ) -> Result<Self, ApplicationError> {
        Self::connect_with(config_path.into(), kind, true)
    }

    /// Connects to an already-running daemon for `config_path`, never starting
    /// one.
    ///
    /// Used by the `daemon status`/`daemon stop` diagnostics, which must report
    /// a stopped daemon rather than bring one up (`tasks/015.md` §30).
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::RuntimeUnavailable`] when no daemon is
    /// listening.
    pub fn connect_existing(
        config_path: impl Into<PathBuf>,
        kind: ClientKind,
    ) -> Result<Self, ApplicationError> {
        Self::connect_with(config_path.into(), kind, false)
    }

    /// Shared constructor for both connection policies.
    fn connect_with(
        config_path: PathBuf,
        kind: ClientKind,
        autostart: bool,
    ) -> Result<Self, ApplicationError> {
        let socket = socket_path(&config_path);
        let (events, _receiver) = broadcast::channel(1024);
        let inner = Arc::new(Inner {
            config_path,
            socket_path: socket,
            kind,
            autostart,
            connection: Mutex::new(None),
            events,
            logs: Mutex::new(HashMap::new()),
        });
        {
            let connection = inner.open()?;
            *inner
                .connection
                .lock()
                .expect("connection lock is not poisoned") = Some(connection);
        }
        Ok(Self {
            inner,
            allow_destructive: false,
        })
    }

    /// Returns the socket path the client is connected to.
    #[must_use]
    pub fn endpoint(&self) -> &Path {
        &self.inner.socket_path
    }

    /// Returns the daemon's diagnostic status.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::RuntimeUnavailable`] when the daemon cannot
    /// be reached.
    pub fn daemon_status(&self) -> Result<DaemonStatus, ApplicationError> {
        self.call(ops::DAEMON_STATUS, ())
    }

    /// Requests a graceful daemon shutdown.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::RuntimeUnavailable`] when the daemon cannot
    /// be reached.
    pub fn shutdown_daemon(&self) -> Result<(), ApplicationError> {
        self.call::<()>(ops::DAEMON_SHUTDOWN, ())
    }

    /// Sends one request and deserializes the response.
    fn call<T: DeserializeOwned>(
        &self,
        method: &str,
        params: impl Serialize,
    ) -> Result<T, ApplicationError> {
        let params = serde_json::to_value(params).map_err(|error| {
            unavailable(
                codes::INVALID_REQUEST,
                format!("could not serialize the request parameters: {error}"),
            )
        })?;

        let mut retried = false;
        loop {
            let connection = match self.inner.current() {
                Ok(connection) => connection,
                Err(error) => {
                    if retried {
                        return Err(error);
                    }
                    retried = true;
                    self.inner.reconnect()?;
                    continue;
                }
            };

            match send_once(
                &connection,
                method,
                params.clone(),
                self.allow_destructive,
                self.inner.kind,
            ) {
                Attempt::Done(value) => {
                    return serde_json::from_value(value).map_err(|error| {
                        unavailable(
                            codes::INVALID_REQUEST,
                            format!("the daemon returned an unexpected response: {error}"),
                        )
                    });
                }
                Attempt::App(error) => return Err(error),
                Attempt::Lost => {
                    connection.mark_dead();
                    return Err(unavailable(
                        codes::DAEMON_UNAVAILABLE,
                        "the daemon did not answer; the request was not retried because it may have \
                         been applied",
                    ));
                }
                Attempt::NotSent => {
                    connection.mark_dead();
                    if retried {
                        return Err(unavailable(
                            codes::DAEMON_UNAVAILABLE,
                            "the daemon connection is not available",
                        ));
                    }
                    retried = true;
                    self.inner.reconnect()?;
                }
            }
        }
    }
}

impl Inner {
    /// Returns the live connection, reconnecting once if it is missing or dead.
    fn current(self: &Arc<Self>) -> Result<Arc<Connection>, ApplicationError> {
        if let Some(connection) = self
            .connection
            .lock()
            .expect("connection lock is not poisoned")
            .clone()
        {
            if connection.alive.load(Ordering::SeqCst) {
                return Ok(connection);
            }
        }
        self.reconnect()
    }

    /// Opens a new connection, replacing any dead one.
    fn reconnect(self: &Arc<Self>) -> Result<Arc<Connection>, ApplicationError> {
        let connection = self.open()?;
        *self
            .connection
            .lock()
            .expect("connection lock is not poisoned") = Some(connection.clone());
        Ok(connection)
    }

    /// Opens one connection and starts its reader thread.
    fn open(self: &Arc<Self>) -> Result<Arc<Connection>, ApplicationError> {
        let stream = if self.autostart {
            connect_or_start(&self.config_path, &self.socket_path)
        } else {
            connect_existing(&self.socket_path)
        }
        .map_err(|error| {
            unavailable(
                codes::DAEMON_UNAVAILABLE,
                format!(
                    "could not reach the daemon at {}: {error}",
                    self.socket_path.display()
                ),
            )
        })?;
        let reader_half = stream.try_clone().map_err(|error| {
            unavailable(
                codes::DAEMON_UNAVAILABLE,
                format!("could not duplicate the daemon socket: {error}"),
            )
        })?;
        let connection = Arc::new(Connection {
            writer: Mutex::new(stream),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            alive: AtomicBool::new(true),
        });
        spawn_reader(Arc::downgrade(self), Arc::clone(&connection), reader_half);
        // Re-establish any realtime subscriptions the caller had before the
        // reconnect. Failures are ignored: the next `recv` observes the close.
        let services: Vec<ServiceId> = self
            .logs
            .lock()
            .expect("logs lock is not poisoned")
            .keys()
            .cloned()
            .collect();
        for service in services {
            let _ = send_once(
                &connection,
                ops::SUBSCRIBE_SERVICE_LOGS,
                serde_json::json!([service]),
                false,
                self.kind,
            );
        }
        Ok(connection)
    }

    /// Routes a pushed event to its local consumer.
    fn dispatch_event(&self, event: ServerEvent) {
        match event {
            ServerEvent::Task { event } => {
                let _ = self.events.send(event);
            }
            ServerEvent::Log { service_id, entry } => {
                let sender = self
                    .logs
                    .lock()
                    .expect("logs lock is not poisoned")
                    .get(&service_id)
                    .cloned();
                if let Some(sender) = sender {
                    let _ = sender.send(entry);
                }
            }
            ServerEvent::Shutdown { .. } => {
                if let Some(connection) = self
                    .connection
                    .lock()
                    .expect("connection lock is not poisoned")
                    .clone()
                {
                    connection.mark_dead();
                }
            }
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Ok(connection) = self.connection.lock() {
            if let Some(connection) = connection.as_ref() {
                connection.alive.store(false, Ordering::SeqCst);
                if let Ok(writer) = connection.writer.lock() {
                    let _ = writer.shutdown(std::net::Shutdown::Both);
                }
            }
        }
    }
}

/// Sends one request and classifies the outcome.
fn send_once(
    connection: &Arc<Connection>,
    method: &str,
    params: serde_json::Value,
    allow_destructive: bool,
    kind: ClientKind,
) -> Attempt {
    let (request_id, receiver) = connection.register();
    let request = RequestEnvelope {
        protocol: PROTOCOL_VERSION,
        request_id: request_id.clone(),
        client: kind,
        allow_destructive,
        method: method.to_owned(),
        params,
    };
    {
        let mut writer = match connection.writer.lock() {
            Ok(writer) => writer,
            Err(_) => return Attempt::NotSent,
        };
        if write_frame(&mut *writer, &request).is_err() {
            drop(writer);
            remove_pending(connection, &request_id);
            return Attempt::NotSent;
        }
    }

    match receiver.recv_timeout(REQUEST_TIMEOUT) {
        Ok(Ok(value)) => Attempt::Done(value),
        Ok(Err(error)) => Attempt::App(error),
        Err(RecvTimeoutError::Timeout) => attempt_after_timeout(connection, &request_id),
        Err(RecvTimeoutError::Disconnected) => Attempt::Lost,
    }
}

/// Handles a timeout: a daemon that is still alive simply did not answer, which
/// is treated as a lost response (never retried).
fn attempt_after_timeout(connection: &Arc<Connection>, request_id: &str) -> Attempt {
    remove_pending(connection, request_id);
    Attempt::Lost
}

/// Removes a pending request, ignoring a missing entry.
fn remove_pending(connection: &Arc<Connection>, request_id: &str) {
    connection
        .pending
        .lock()
        .expect("pending map is not poisoned")
        .remove(request_id);
}

/// Reads server messages for one connection until it closes.
fn spawn_reader(inner: Weak<Inner>, connection: Arc<Connection>, mut reader: UnixStream) {
    thread::spawn(move || {
        loop {
            match read_frame::<_, ServerMessage>(&mut reader) {
                Ok(Some(ServerMessage::Response(response))) => connection.deliver(response),
                Ok(Some(ServerMessage::Event(event))) => match inner.upgrade() {
                    Some(inner) => inner.dispatch_event(event),
                    None => break,
                },
                Ok(None) | Err(_) => break,
            }
        }
        connection.mark_dead();
    });
}

/// A realtime log subscription delivered over IPC.
pub struct RemoteLogSubscription {
    service_id: ServiceId,
    inner: Arc<Inner>,
    receiver: tokio_mpsc::UnboundedReceiver<LogEntry>,
}

#[async_trait]
impl LogSubscription for RemoteLogSubscription {
    async fn recv(&mut self) -> Result<LogEntry, LogStreamError> {
        self.receiver.recv().await.ok_or(LogStreamError::Closed)
    }
}

impl Drop for RemoteLogSubscription {
    fn drop(&mut self) {
        self.inner
            .logs
            .lock()
            .expect("logs lock is not poisoned")
            .remove(&self.service_id);
        if let Ok(connection) = self.inner.current() {
            let _ = send_once(
                &connection,
                ops::UNSUBSCRIBE_SERVICE_LOGS,
                serde_json::json!([self.service_id]),
                false,
                self.inner.kind,
            );
        }
    }
}

#[async_trait]
impl OrchestratorClient for RemoteClient {
    fn config_summary(&self) -> ConfigSummary {
        self.call(ops::CONFIG_SUMMARY, ()).unwrap_or_default()
    }

    fn with_authorization(&self, authorization: Authorization) -> Arc<dyn OrchestratorClient> {
        Arc::new(Self {
            inner: Arc::clone(&self.inner),
            allow_destructive: authorization.permits_destructive(),
        })
    }

    fn list_tasks(&self) -> Vec<Task> {
        self.call(ops::LIST_TASKS, ()).unwrap_or_default()
    }

    fn list_running_tasks(&self) -> Vec<Task> {
        self.call(ops::LIST_RUNNING_TASKS, ()).unwrap_or_default()
    }

    fn list_recent_tasks(&self, limit: usize) -> Vec<Task> {
        self.call(ops::LIST_RECENT_TASKS, (limit,))
            .unwrap_or_default()
    }

    fn subscribe_task_events(&self) -> broadcast::Receiver<TaskEvent> {
        self.inner.events.subscribe()
    }

    fn list_workspaces(&self) -> Result<Vec<Workspace>, ApplicationError> {
        self.call(ops::LIST_WORKSPACES, ())
    }

    fn get_workspace(&self, workspace_id: &WorkspaceId) -> Result<Workspace, ApplicationError> {
        self.call(ops::GET_WORKSPACE, (workspace_id,))
    }

    fn get_workspace_status(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<WorkspaceStatus, ApplicationError> {
        self.call(ops::GET_WORKSPACE_STATUS, (workspace_id,))
    }

    fn validate_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<WorkspaceValidationReport, ApplicationError> {
        self.call(ops::VALIDATE_WORKSPACE, (workspace_id,))
    }

    fn list_groups(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<ServiceGroup>, ApplicationError> {
        self.call(ops::LIST_GROUPS, (workspace_id,))
    }

    fn get_group(&self, group_id: &GroupId) -> Result<ServiceGroup, ApplicationError> {
        self.call(ops::GET_GROUP, (group_id,))
    }

    fn list_profiles(&self, workspace_id: &WorkspaceId) -> Result<Vec<Profile>, ApplicationError> {
        self.call(ops::LIST_PROFILES, (workspace_id,))
    }

    fn get_profile(&self, profile_id: &ProfileId) -> Result<Profile, ApplicationError> {
        self.call(ops::GET_PROFILE, (profile_id,))
    }

    fn list_services(&self, workspace_id: &WorkspaceId) -> Result<Vec<Service>, ApplicationError> {
        self.call(ops::LIST_SERVICES, (workspace_id,))
    }

    fn get_service(&self, service_id: &ServiceId) -> Result<Service, ApplicationError> {
        self.call(ops::GET_SERVICE, (service_id,))
    }

    fn get_service_status(
        &self,
        service_id: &ServiceId,
    ) -> Result<ServiceStatus, ApplicationError> {
        self.call(ops::GET_SERVICE_STATUS, (service_id,))
    }

    fn get_service_runtime_status(
        &self,
        service_id: &ServiceId,
    ) -> Result<ServiceRuntimeStatus, ApplicationError> {
        self.call(ops::GET_SERVICE_RUNTIME_STATUS, (service_id,))
    }

    fn list_running_services(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<ServiceRuntimeStatus>, ApplicationError> {
        self.call(ops::LIST_RUNNING_SERVICES, (workspace_id,))
    }

    fn list_libraries(&self, workspace_id: &WorkspaceId) -> Result<Vec<Library>, ApplicationError> {
        self.call(ops::LIST_LIBRARIES, (workspace_id,))
    }

    fn get_library(&self, library_id: &LibraryId) -> Result<Library, ApplicationError> {
        self.call(ops::GET_LIBRARY, (library_id,))
    }

    async fn get_library_status(
        &self,
        library_id: &LibraryId,
    ) -> Result<LibraryStatus, ApplicationError> {
        self.call(ops::GET_LIBRARY_STATUS, (library_id,))
    }

    fn find_library_dependents(
        &self,
        library_id: &LibraryId,
    ) -> Result<Vec<LibraryDependent>, ApplicationError> {
        self.call(ops::FIND_LIBRARY_DEPENDENTS, (library_id,))
    }

    async fn verify_library_artifact(
        &self,
        library_id: &LibraryId,
    ) -> Result<LibraryArtifactVerification, ApplicationError> {
        self.call(ops::VERIFY_LIBRARY_ARTIFACT, (library_id,))
    }

    async fn inspect_library_version_alignment(
        &self,
        library_id: &LibraryId,
    ) -> Result<service_orchestrator_application::VersionAlignmentPlan, ApplicationError> {
        self.call(ops::INSPECT_LIBRARY_VERSION_ALIGNMENT, (library_id,))
    }

    async fn get_git_status(
        &self,
        service_id: &ServiceId,
    ) -> Result<RepositoryStatus, ApplicationError> {
        self.call(ops::GET_GIT_STATUS, (service_id,))
    }

    async fn list_branches(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<BranchInfo>, ApplicationError> {
        self.call(ops::LIST_BRANCHES, (service_id,))
    }

    async fn get_repository_diff(
        &self,
        service_id: &ServiceId,
        options: &DiffOptions,
    ) -> Result<RepoDiff, ApplicationError> {
        self.call(
            ops::GET_REPOSITORY_DIFF,
            (service_id, DiffParams::from_options(options)),
        )
    }

    fn migration_status(
        &self,
        service_id: &ServiceId,
    ) -> Result<MigrationStatus, ApplicationError> {
        self.call(ops::MIGRATION_STATUS, (service_id,))
    }

    fn get_service_health(
        &self,
        service_id: &ServiceId,
    ) -> Result<ServiceHealth, ApplicationError> {
        self.call(ops::GET_SERVICE_HEALTH, (service_id,))
    }

    fn list_service_health(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<ServiceHealth>, ApplicationError> {
        self.call(ops::LIST_SERVICE_HEALTH, (workspace_id,))
    }

    fn get_service_diagnostics(
        &self,
        service_id: &ServiceId,
    ) -> Result<ServiceDiagnostics, ApplicationError> {
        self.call(ops::GET_SERVICE_DIAGNOSTICS, (service_id,))
    }

    fn get_service_logs(
        &self,
        service_id: &ServiceId,
        request: &LogRequest,
    ) -> Result<LogPage, ApplicationError> {
        self.call(ops::GET_SERVICE_LOGS, (service_id, request))
    }

    fn get_task_logs(
        &self,
        task_id: &TaskId,
        request: &LogRequest,
    ) -> Result<LogPage, ApplicationError> {
        self.call(ops::GET_TASK_LOGS, (task_id, request))
    }

    fn get_logs(&self, request: &LogRequest) -> Result<Vec<LogEntry>, ApplicationError> {
        self.call(ops::GET_LOGS, (request,))
    }

    fn subscribe_service_logs(
        &self,
        service_id: &ServiceId,
    ) -> Result<Box<dyn LogSubscription>, ApplicationError> {
        let (sender, receiver) = tokio_mpsc::unbounded_channel();
        self.inner
            .logs
            .lock()
            .expect("logs lock is not poisoned")
            .insert(service_id.clone(), sender);
        let result: Result<(), ApplicationError> =
            self.call(ops::SUBSCRIBE_SERVICE_LOGS, (service_id,));
        match result {
            Ok(()) => Ok(Box::new(RemoteLogSubscription {
                service_id: service_id.clone(),
                inner: Arc::clone(&self.inner),
                receiver,
            })),
            Err(error) => {
                self.inner
                    .logs
                    .lock()
                    .expect("logs lock is not poisoned")
                    .remove(service_id);
                Err(error)
            }
        }
    }

    fn get_task(&self, task_id: &TaskId) -> Result<Task, ApplicationError> {
        self.call(ops::GET_TASK, (task_id,))
    }

    fn get_workflow(&self, workflow_id: &WorkflowId) -> Result<Workflow, ApplicationError> {
        self.call(ops::GET_WORKFLOW, (workflow_id,))
    }

    fn summarize_workflow(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<BulkOperationSummary, ApplicationError> {
        self.call(ops::SUMMARIZE_WORKFLOW, (workflow_id,))
    }

    fn cancel_task(&self, task_id: &TaskId) -> Result<(), ApplicationError> {
        self.call(ops::CANCEL_TASK, (task_id,))
    }

    fn sync_service(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError> {
        self.call(ops::SYNC_SERVICE, (service_id,))
    }

    fn sync_repository(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError> {
        self.call(ops::SYNC_REPOSITORY, (service_id,))
    }

    fn fetch_repository(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::FETCH_REPOSITORY, (service_id,))
    }

    fn pull_repository(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError> {
        self.call(ops::PULL_REPOSITORY, (service_id,))
    }

    fn checkout_repository(
        &self,
        service_id: &ServiceId,
        branch: &Branch,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::CHECKOUT_REPOSITORY, (service_id, branch))
    }

    fn clone_repository(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::CLONE_REPOSITORY, (service_id,))
    }

    fn reset_repository_hard(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::RESET_REPOSITORY_HARD, (service_id,))
    }

    fn clean_repository(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::CLEAN_REPOSITORY, (service_id,))
    }

    fn discard_local_changes(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::DISCARD_LOCAL_CHANGES, (service_id,))
    }

    fn sync_repositories(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::SYNC_REPOSITORIES, (workspace_id, services))
    }

    fn fetch_repositories(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::FETCH_REPOSITORIES, (workspace_id, services))
    }

    fn checkout_repositories(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
        branch: &Branch,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::CHECKOUT_REPOSITORIES, (workspace_id, services, branch))
    }

    fn build_service(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError> {
        self.call(ops::BUILD_SERVICE, (service_id,))
    }

    fn build_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::BUILD_WORKSPACE, (workspace_id,))
    }

    fn build_library(&self, library_id: &LibraryId) -> Result<OperationResult, ApplicationError> {
        self.call(ops::BUILD_LIBRARY, (library_id,))
    }

    fn install_library(&self, library_id: &LibraryId) -> Result<OperationResult, ApplicationError> {
        self.call(ops::INSTALL_LIBRARY, (library_id,))
    }

    fn sync_library(&self, library_id: &LibraryId) -> Result<OperationResult, ApplicationError> {
        self.call(ops::SYNC_LIBRARY, (library_id,))
    }

    fn prepare_library(
        &self,
        library_id: &LibraryId,
        options: &LibraryPrepareOptions,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::PREPARE_LIBRARY, (library_id, options))
    }

    fn prepare_selected_libraries(
        &self,
        request: PrepareSelectedLibrariesRequest,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::PREPARE_SELECTED_LIBRARIES, (request,))
    }

    fn start_service(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError> {
        self.call(ops::START_SERVICE, (service_id,))
    }

    fn stop_service(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError> {
        self.call(ops::STOP_SERVICE, (service_id,))
    }

    fn restart_service(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError> {
        self.call(ops::RESTART_SERVICE, (service_id,))
    }

    fn start_services(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::START_SERVICES, (workspace_id, services))
    }

    fn stop_services(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::STOP_SERVICES, (workspace_id, services))
    }

    fn restart_services(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::RESTART_SERVICES, (workspace_id, services))
    }

    fn validate_migration(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::VALIDATE_MIGRATION, (service_id,))
    }

    fn get_migration_status(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::GET_MIGRATION_STATUS, (service_id,))
    }

    fn inspect_migration_pending(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::INSPECT_MIGRATION_PENDING, (service_id,))
    }

    fn preview_migration(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::PREVIEW_MIGRATION, (service_id,))
    }

    fn apply_migration(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError> {
        self.call(ops::APPLY_MIGRATION, (service_id,))
    }

    fn apply_selected_migrations(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::APPLY_SELECTED_MIGRATIONS, (workspace_id, services))
    }

    fn run_liquibase(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError> {
        self.call(ops::RUN_LIQUIBASE, (service_id,))
    }

    fn check_service_health(
        &self,
        service_id: &ServiceId,
        scope: HealthCheckScope,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::CHECK_SERVICE_HEALTH, (service_id, scope))
    }

    fn check_service_readiness(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::CHECK_SERVICE_READINESS, (service_id,))
    }

    fn sync_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::SYNC_WORKSPACE, (workspace_id,))
    }

    fn prepare_environment(
        &self,
        request: PrepareEnvironmentRequest,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::PREPARE_ENVIRONMENT, (request,))
    }

    fn plan_prepare_environment(
        &self,
        request: &PrepareEnvironmentRequest,
    ) -> Result<EnvironmentPlan, ApplicationError> {
        self.call(ops::PLAN_PREPARE_ENVIRONMENT, (request,))
    }

    fn get_workflow_status(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<WorkflowStatus, ApplicationError> {
        self.call(ops::GET_WORKFLOW_STATUS, (workflow_id,))
    }

    fn cancel_workflow(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<WorkflowCancellation, ApplicationError> {
        self.call(ops::CANCEL_WORKFLOW, (workflow_id,))
    }

    fn start_group(
        &self,
        workspace_id: &WorkspaceId,
        group_id: &GroupId,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::START_GROUP, (workspace_id, group_id))
    }

    fn stop_group(
        &self,
        workspace_id: &WorkspaceId,
        group_id: &GroupId,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::STOP_GROUP, (workspace_id, group_id))
    }

    fn start_profile(
        &self,
        workspace_id: &WorkspaceId,
        profile_id: &ProfileId,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::START_PROFILE, (workspace_id, profile_id))
    }

    fn stop_profile(
        &self,
        workspace_id: &WorkspaceId,
        profile_id: &ProfileId,
    ) -> Result<OperationResult, ApplicationError> {
        self.call(ops::STOP_PROFILE, (workspace_id, profile_id))
    }
}
