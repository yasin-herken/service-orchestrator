//! The process manager.
//!
//! [`ProcessManager`] is the only component in Service Orchestrator that talks
//! to the operating system's process API. It spawns, monitors, streams, stops,
//! and kills local processes, and it is the sole owner of process pids, child
//! processes, exit status, and output capture. Every other subsystem — Git,
//! build, Liquibase, health, and the service executors — must ask the manager to
//! run a process rather than constructing a [`std::process::Command`] itself.
//!
//! # Ownership and runtime
//!
//! The manager does not own a Tokio runtime. It is built inside a runtime
//! context and captures a [`Handle`], so the composition root controls runtime
//! creation and shutdown. [`ProcessManager::spawn`] is synchronous (Tokio's
//! spawn is non-blocking); the supervisor that reaps the child and streams its
//! output runs on the captured runtime.
//!
//! # Supervision
//!
//! Each spawned process is driven by one supervisor task. It captures `stdout`
//! and `stderr` line-by-line, waits for the process to exit, honours a requested
//! stop (graceful signal, grace period, then `SIGKILL`), enforces the configured
//! timeout, and applies the restart policy. The supervisor is the only owner of
//! the Tokio [`Child`]; the rest of the system interacts through
//! [`ProcessHandle`].

use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use service_orchestrator_domain::{ServiceId, Timestamp};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::runtime::Handle;

use crate::entry::{ProcessEntry, ShutdownRequest};
use crate::error::ProcessError;
use crate::event::{EventBus, ProcessEvent, ProcessEventStream};
use crate::handle::ProcessHandle;
use crate::id::ProcessId;
use crate::log::{LogStream, LogStreamKind};
use crate::registry::ProcessRegistry;
use crate::snapshot::ProcessSnapshot;
use crate::spec::{ProcessSpec, TerminationSignal};
use crate::state::{ExitInfo, ExitReason, ProcessStatus};

/// Runtime configuration for the process manager.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessManagerConfig {
    /// The event broadcast channel capacity.
    pub event_capacity: usize,
    /// A default maximum runtime applied to processes that do not set one.
    pub default_timeout: Option<Duration>,
    /// How long [`ProcessManager::shutdown`] waits for graceful stops.
    pub shutdown_grace: Duration,
}

impl Default for ProcessManagerConfig {
    fn default() -> Self {
        Self {
            event_capacity: 4096,
            default_timeout: None,
            shutdown_grace: Duration::from_secs(10),
        }
    }
}

/// The exit parts extracted from a wait: reason, exit code, and signal.
type ExitParts = (ExitReason, Option<i32>, Option<i32>);

/// Shared manager state, behind a single [`Arc`].
struct ManagerInner {
    registry: ProcessRegistry,
    events: EventBus,
    config: ProcessManagerConfig,
    counter: AtomicU64,
    handle: Handle,
    shutdown: AtomicBool,
}

impl ManagerInner {
    fn next_id(&self) -> Result<ProcessId, ProcessError> {
        let value = self.counter.fetch_add(1, Ordering::SeqCst);
        ProcessId::new(format!("proc-{value}"))
    }

    fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }
}

/// The local process manager.
///
/// The manager is cheaply cloneable so the TUI, the MCP server, and the service
/// executors can each hold one observing the same registry and event stream.
#[derive(Clone)]
pub struct ProcessManager {
    inner: Arc<ManagerInner>,
}

impl ProcessManager {
    /// Creates a manager with default configuration inside the current runtime.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::NoRuntime`] when called outside a Tokio runtime.
    pub fn new() -> Result<Self, ProcessError> {
        Self::with_config(ProcessManagerConfig::default())
    }

    /// Creates a manager with explicit configuration inside the current runtime.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::NoRuntime`] when called outside a Tokio runtime.
    pub fn with_config(config: ProcessManagerConfig) -> Result<Self, ProcessError> {
        let handle = Handle::try_current().map_err(|_| ProcessError::NoRuntime)?;
        Ok(Self {
            inner: Arc::new(ManagerInner {
                registry: ProcessRegistry::new(),
                events: EventBus::new(config.event_capacity),
                config,
                counter: AtomicU64::new(0),
                handle,
                shutdown: AtomicBool::new(false),
            }),
        })
    }

    /// Spawns a process and returns a handle to it.
    ///
    /// The call validates the specification, starts the child, and returns as
    /// soon as the process is running. Monitoring (exit detection, output
    /// capture, stop handling, timeout, restart) continues on the runtime.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::ShuttingDown`] when the manager is shutting down,
    /// [`ProcessError::InvalidSpec`] for an invalid specification, and
    /// [`ProcessError::Spawn`] when the operating system refuses to start the
    /// process. A failed spawn still records a `Failed` process in the registry.
    pub fn spawn(&self, spec: ProcessSpec) -> Result<ProcessHandle, ProcessError> {
        spec.validate()?;
        if self.inner.is_shutdown() {
            return Err(ProcessError::ShuttingDown);
        }
        let id = self.inner.next_id()?;
        let program = spec.program.clone();
        let entry = Arc::new(ProcessEntry::new(
            id.clone(),
            spec,
            self.inner.events.clone(),
        ));
        entry.publish(ProcessEvent::Created {
            id: id.clone(),
            program: program.clone(),
        });
        self.inner.registry.insert(entry.clone());

        entry.publish(ProcessEvent::Starting { id: id.clone() });
        entry.transition(ProcessStatus::Starting)?;

        let child = match spawn_child(entry.spec()) {
            Ok(child) => child,
            Err(error) => {
                entry.fail_spawn(error.to_string());
                return Err(ProcessError::Spawn {
                    id,
                    program,
                    message: error.to_string(),
                });
            }
        };

        let pid = child
            .id()
            .ok_or_else(|| ProcessError::internal("spawned process has no pid"))?;
        entry.mark_running(pid)?;

        let config = self.inner.config.clone();
        let supervisor_entry = entry.clone();
        self.inner.handle.spawn(async move {
            supervise(supervisor_entry, child, config).await;
        });

        Ok(ProcessHandle::new(entry))
    }

    /// Spawns a process and stops it when `cancellation` resolves.
    ///
    /// This is how the process manager integrates with the task engine's
    /// cooperative cancellation model without depending on it: the caller
    /// passes a future that resolves when its task is cancelled (for example
    /// `async move { context.cancellation().clone().cancelled().await }`), and
    /// the manager performs a graceful stop with
    /// [`ExitReason::Cancelled`].
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`spawn`](Self::spawn).
    pub fn spawn_with_cancellation<F>(
        &self,
        spec: ProcessSpec,
        cancellation: F,
    ) -> Result<ProcessHandle, ProcessError>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let handle = self.spawn(spec)?;
        let watcher = handle.clone();
        self.inner.handle.spawn(async move {
            cancellation.await;
            let _ = watcher.cancel();
        });
        Ok(handle)
    }

    /// Restarts a process, preserving its specification.
    ///
    /// The existing process is stopped gracefully (and awaited), then a fresh
    /// execution instance is spawned with a new [`ProcessId`].
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::NotFound`] if the process is unknown, or the same
    /// errors as [`spawn`](Self::spawn).
    pub async fn restart(&self, id: &ProcessId) -> Result<ProcessHandle, ProcessError> {
        let entry = self
            .inner
            .registry
            .entry(id)
            .ok_or_else(|| ProcessError::NotFound(id.clone()))?;
        let spec = entry.spec().clone();
        if !entry.status().is_terminal() {
            let _ = entry.request_shutdown(ShutdownRequest::Graceful(ExitReason::Terminated));
            entry.wait_terminal().await;
        }
        self.spawn(spec)
    }

    /// Requests a graceful stop of a process.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::NotFound`] if the process is unknown.
    pub fn stop(&self, id: &ProcessId) -> Result<(), ProcessError> {
        self.handle(id)
            .ok_or_else(|| ProcessError::NotFound(id.clone()))?
            .stop()
    }

    /// Requests a stop of a process because its task was cancelled.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::NotFound`] if the process is unknown.
    pub fn cancel(&self, id: &ProcessId) -> Result<(), ProcessError> {
        self.handle(id)
            .ok_or_else(|| ProcessError::NotFound(id.clone()))?
            .cancel()
    }

    /// Force-kills a process.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::NotFound`] if the process is unknown.
    pub fn kill(&self, id: &ProcessId) -> Result<(), ProcessError> {
        self.handle(id)
            .ok_or_else(|| ProcessError::NotFound(id.clone()))?
            .kill()
    }

    /// Returns a handle for a process, if it exists.
    #[must_use]
    pub fn handle(&self, id: &ProcessId) -> Option<ProcessHandle> {
        self.inner.registry.get(id)
    }

    /// Returns a snapshot of a process, if it exists.
    #[must_use]
    pub fn snapshot(&self, id: &ProcessId) -> Option<ProcessSnapshot> {
        self.inner.registry.snapshot(id)
    }

    /// Lists snapshots of every registered process.
    #[must_use]
    pub fn list(&self) -> Vec<ProcessSnapshot> {
        self.inner.registry.list()
    }

    /// Lists snapshots of every active (non-terminal) process.
    #[must_use]
    pub fn list_active(&self) -> Vec<ProcessSnapshot> {
        self.inner.registry.list_active()
    }

    /// Returns handles for every process associated with `service`.
    #[must_use]
    pub fn find_by_service(&self, service: &ServiceId) -> Vec<ProcessHandle> {
        self.inner.registry.find_by_service(service)
    }

    /// Removes a finished process from the registry.
    ///
    /// Removing an active process is refused; stop or kill it first.
    pub fn forget(&self, id: &ProcessId) -> bool {
        match self.inner.registry.entry(id) {
            Some(entry) if entry.status().is_terminal() => self.inner.registry.remove(id),
            _ => false,
        }
    }

    /// Returns the number of registered processes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.registry.len()
    }

    /// Returns `true` if no processes are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.registry.is_empty()
    }

    /// Subscribes to every process event.
    #[must_use]
    pub fn subscribe_events(&self) -> ProcessEventStream {
        ProcessEventStream::new(None, self.inner.events.subscribe())
    }

    /// Subscribes to every process's log lines.
    #[must_use]
    pub fn subscribe_logs(&self) -> LogStream {
        LogStream::new(None, self.inner.events.subscribe())
    }

    /// Returns `true` once shutdown has begun.
    #[must_use]
    pub fn is_shutdown(&self) -> bool {
        self.inner.is_shutdown()
    }

    /// Gracefully shuts every process down.
    ///
    /// New spawns are refused, every active process is asked to stop
    /// gracefully, and this waits up to the configured grace period before
    /// force-killing anything that remains.
    pub async fn shutdown(&self) {
        self.inner.shutdown.store(true, Ordering::SeqCst);
        let handles = self.inner.registry.active_handles();
        for handle in &handles {
            let _ = handle.stop();
        }
        let wait = async {
            for handle in &handles {
                let _ = handle.wait().await;
            }
        };
        let _ = tokio::time::timeout(self.inner.config.shutdown_grace, wait).await;
        for handle in &handles {
            if !handle.is_terminal() {
                let _ = handle.kill();
            }
        }
    }
}

impl std::fmt::Debug for ProcessManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessManager")
            .field("processes", &self.inner.registry.len())
            .field("shutdown", &self.inner.is_shutdown())
            .finish()
    }
}

/// Builds a Tokio command from a structured specification.
fn build_command(spec: &ProcessSpec) -> Command {
    let mut command = Command::new(&spec.program);
    command.args(&spec.args);
    if let Some(dir) = &spec.working_dir {
        command.current_dir(dir);
    }
    for variable in spec.environment.iter() {
        command.env(&variable.key, &variable.value);
    }
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    // Put the process in its own group so a stop can signal the whole tree and
    // never orphan a child process (`AGENTS.md` §15).
    #[cfg(unix)]
    command.process_group(0);
    command
}

/// Spawns a child process from a specification.
fn spawn_child(spec: &ProcessSpec) -> std::io::Result<Child> {
    build_command(spec).spawn()
}

/// Supervises one process across its execution and any automatic restarts.
async fn supervise(entry: Arc<ProcessEntry>, child: Child, config: ProcessManagerConfig) {
    let mut restarts: u32 = 0;
    let mut current = child;
    loop {
        let exit = run_execution(&entry, current, &config).await;
        if entry.spec().restart.should_restart(exit.reason, restarts) {
            restarts += 1;
            entry.publish(ProcessEvent::Restarting {
                id: entry.id().clone(),
                attempt: restarts,
            });
            if entry.begin_restart().is_err() {
                return;
            }
            let backoff = entry.spec().restart.backoff();
            if !backoff.is_zero() {
                tokio::time::sleep(backoff).await;
            }
            match spawn_child(entry.spec()) {
                Ok(next) => match next.id() {
                    Some(pid) if entry.mark_running(pid).is_ok() => {
                        entry.publish(ProcessEvent::Starting {
                            id: entry.id().clone(),
                        });
                        current = next;
                        continue;
                    }
                    _ => {
                        entry.fail_spawn("restarted process could not be started".to_owned());
                        return;
                    }
                },
                Err(error) => {
                    entry.fail_spawn(error.to_string());
                    return;
                }
            }
        }
        entry.complete(exit);
        return;
    }
}

/// Runs one execution of a process to completion and returns its exit info.
async fn run_execution(
    entry: &Arc<ProcessEntry>,
    mut child: Child,
    config: &ProcessManagerConfig,
) -> ExitInfo {
    let started = entry.started_at().unwrap_or_else(Timestamp::now);

    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        readers.push(tokio::spawn(read_lines(
            entry.clone(),
            stdout,
            LogStreamKind::Stdout,
        )));
    }
    if let Some(stderr) = child.stderr.take() {
        readers.push(tokio::spawn(read_lines(
            entry.clone(),
            stderr,
            LogStreamKind::Stderr,
        )));
    }

    let grace = entry.spec().shutdown.grace_period;
    let signal = entry.spec().shutdown.signal;
    let timeout = entry.timeout().or(config.default_timeout);
    let mut shutdown_rx = entry.shutdown_receiver();

    let parts = if let Some(request) = entry.pending_shutdown() {
        handle_shutdown(entry, &mut child, request, grace, signal, &mut shutdown_rx).await
    } else {
        tokio::select! {
            result = child.wait() => classify_wait(result),
            _ = shutdown_rx.changed() => {
                let request = *shutdown_rx.borrow();
                handle_shutdown(entry, &mut child, request, grace, signal, &mut shutdown_rx).await
            }
            _ = sleep_optional(timeout) => {
                handle_shutdown(
                    entry,
                    &mut child,
                    ShutdownRequest::Graceful(ExitReason::TimedOut),
                    grace,
                    signal,
                    &mut shutdown_rx,
                )
                .await
            }
        }
    };

    // Wait for the pipe readers so no trailing output line is lost.
    for reader in readers {
        let _ = reader.await;
    }

    let finished = Timestamp::now();
    ExitInfo::new(parts.1, parts.2, parts.0, started, finished)
}

/// Performs a requested shutdown and returns the exit parts.
async fn handle_shutdown(
    entry: &Arc<ProcessEntry>,
    child: &mut Child,
    request: ShutdownRequest,
    grace: Duration,
    signal: TerminationSignal,
    shutdown_rx: &mut tokio::sync::watch::Receiver<ShutdownRequest>,
) -> ExitParts {
    match request {
        ShutdownRequest::None => classify_wait(child.wait().await),
        ShutdownRequest::Force(reason) => {
            let _ = entry.signal(TerminationSignal::Kill);
            let parts = classify_wait(child.wait().await);
            (forced_reason(reason), parts.1, parts.2)
        }
        ShutdownRequest::Graceful(reason) => {
            entry.mark_stopping();
            let _ = entry.signal(signal);
            let deadline = tokio::time::Instant::now() + grace;
            loop {
                tokio::select! {
                    result = child.wait() => {
                        let parts = classify_wait(result);
                        return (reason, parts.1, parts.2);
                    }
                    _ = tokio::time::sleep_until(deadline) => {
                        let _ = entry.signal(TerminationSignal::Kill);
                        let parts = classify_wait(child.wait().await);
                        return (forced_reason(reason), parts.1, parts.2);
                    }
                    changed = shutdown_rx.changed() => {
                        if changed.is_err()
                            || matches!(*shutdown_rx.borrow(), ShutdownRequest::Force(_))
                        {
                            let _ = entry.signal(TerminationSignal::Kill);
                            let parts = classify_wait(child.wait().await);
                            return (forced_reason(reason), parts.1, parts.2);
                        }
                    }
                }
            }
        }
    }
}

/// Returns the exit reason to record after an escalation to `SIGKILL`.
fn forced_reason(reason: ExitReason) -> ExitReason {
    if reason == ExitReason::TimedOut {
        ExitReason::TimedOut
    } else {
        ExitReason::Killed
    }
}

/// Sleeps for `timeout`, or forever when there is no timeout.
async fn sleep_optional(timeout: Option<Duration>) {
    match timeout {
        Some(duration) => tokio::time::sleep(duration).await,
        None => std::future::pending::<()>().await,
    }
}

/// Classifies a wait result into exit parts.
fn classify_wait(result: std::io::Result<ExitStatus>) -> ExitParts {
    match result {
        Ok(status) => classify_exit_status(status),
        Err(_) => (ExitReason::Failed, None, None),
    }
}

/// Classifies an exit status into exit parts.
fn classify_exit_status(status: ExitStatus) -> ExitParts {
    let code = status.code();
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal: Option<i32> = None;

    let reason = match (code, signal) {
        (Some(0), _) => ExitReason::Exited,
        (Some(_), _) => ExitReason::Failed,
        (None, Some(_)) => ExitReason::Signalled,
        (None, None) => ExitReason::Failed,
    };
    (reason, code, signal)
}

/// Reads a child stream line by line and publishes each line as a log event.
async fn read_lines<R>(entry: Arc<ProcessEntry>, reader: R, kind: LogStreamKind)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        entry.publish_log(kind, line);
    }
}
