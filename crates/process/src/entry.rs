//! The shared, per-process state cell.
//!
//! Every managed process is represented by one [`ProcessEntry`], held behind an
//! [`Arc`] and shared by the manager, the supervisor task, and every
//! [`ProcessHandle`](crate::ProcessHandle) clone. The entry owns the process's
//! lifecycle status, its current pid, its last exit information, and the
//! channels that carry status changes, shutdown requests, events, and log lines.
//!
//! # Locking
//!
//! A single `std::sync::Mutex` guards the mutable state. Critical sections are
//! short and the lock is never held across an `.await`. Status waiters use a
//! [`tokio::sync::watch`] channel so `wait` observes transitions without
//! polling, and shutdown requests use a second watch channel so the supervisor
//! wakes when a stop or kill is requested.

use std::sync::Mutex;
use std::time::Duration;

use service_orchestrator_domain::{ServiceId, Timestamp};
use tokio::sync::watch;

use crate::error::ProcessError;
use crate::event::{EventBus, ProcessEvent, ProcessEventStream};
use crate::id::ProcessId;
use crate::log::{LogLine, LogStream, LogStreamKind};
use crate::signal::send_signal;
use crate::snapshot::ProcessSnapshot;
use crate::spec::{ProcessSpec, TerminationSignal};
use crate::state::{ExitInfo, ExitReason, ProcessStatus};

/// A request to stop a running process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShutdownRequest {
    /// No shutdown has been requested.
    None,
    /// Stop gracefully, then escalate if necessary.
    Graceful(ExitReason),
    /// Stop immediately with `SIGKILL`.
    Force(ExitReason),
}

/// The mutable portion of a process entry.
struct EntryState {
    status: ProcessStatus,
    pid: Option<u32>,
    exit: Option<ExitInfo>,
    started_at: Option<Timestamp>,
    last_error: Option<String>,
}

/// The shared state of one managed process.
pub(crate) struct ProcessEntry {
    id: ProcessId,
    spec: ProcessSpec,
    service: Option<ServiceId>,
    inner: Mutex<EntryState>,
    status_tx: watch::Sender<ProcessStatus>,
    shutdown_tx: watch::Sender<ShutdownRequest>,
    events: EventBus,
}

impl ProcessEntry {
    /// Creates a new entry in the [`ProcessStatus::Created`] state.
    pub(crate) fn new(id: ProcessId, spec: ProcessSpec, events: EventBus) -> Self {
        let service = spec.service.clone();
        let (status_tx, _receiver) = watch::channel(ProcessStatus::Created);
        let (shutdown_tx, _receiver) = watch::channel(ShutdownRequest::None);
        Self {
            id,
            spec,
            service,
            inner: Mutex::new(EntryState {
                status: ProcessStatus::Created,
                pid: None,
                exit: None,
                started_at: None,
                last_error: None,
            }),
            status_tx,
            shutdown_tx,
            events,
        }
    }

    /// Returns the process identifier.
    pub(crate) fn id(&self) -> &ProcessId {
        &self.id
    }

    /// Returns the process specification.
    pub(crate) fn spec(&self) -> &ProcessSpec {
        &self.spec
    }

    /// Returns the associated service, if any.
    pub(crate) fn service(&self) -> Option<&ServiceId> {
        self.service.as_ref()
    }

    /// Returns the current status.
    pub(crate) fn status(&self) -> ProcessStatus {
        self.lock().status
    }

    /// Returns the current operating-system process id, if alive.
    pub(crate) fn pid(&self) -> Option<u32> {
        self.lock().pid
    }

    /// Returns the start time of the current execution, if any.
    pub(crate) fn started_at(&self) -> Option<Timestamp> {
        self.lock().started_at
    }

    /// Returns a snapshot of the process.
    pub(crate) fn snapshot(&self) -> ProcessSnapshot {
        let state = self.lock();
        ProcessSnapshot {
            id: self.id.clone(),
            service: self.service.clone(),
            status: state.status,
            pid: state.pid,
            program: self.spec.program.clone(),
            args: self.spec.args.clone(),
            working_dir: self.spec.working_dir.clone(),
            environment: self.spec.environment.clone(),
            started_at: state.started_at,
            exit: state.exit.clone(),
            last_error: state.last_error.clone(),
        }
    }

    /// Returns the current status together with a revision observer.
    ///
    /// Reading the status and subscribing happen under one lock, so a waiter
    /// that observes a non-terminal status is guaranteed to be woken by the
    /// next change.
    pub(crate) fn status_and_revision(&self) -> (ProcessStatus, watch::Receiver<ProcessStatus>) {
        let state = self.lock();
        (state.status, self.status_tx.subscribe())
    }

    /// Moves the process to `next`, publishing the corresponding event.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::InvalidTransition`] if the domain-of-record state
    /// machine forbids the move.
    pub(crate) fn transition(&self, next: ProcessStatus) -> Result<(), ProcessError> {
        self.transition_with_exit(next, None)
    }

    /// Moves the process to `next`, recording optional exit information.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::InvalidTransition`] if the transition is illegal.
    pub(crate) fn transition_with_exit(
        &self,
        next: ProcessStatus,
        exit: Option<ExitInfo>,
    ) -> Result<(), ProcessError> {
        let mut state = self.lock();
        let current = state.status;
        if !current.can_transition_to(next) {
            return Err(ProcessError::InvalidTransition {
                id: self.id.clone(),
                from: current,
                to: next,
            });
        }
        state.status = next;
        if let Some(exit) = exit {
            state.exit = Some(exit);
        }
        // Publish the status change while holding the lock so waiter order
        // matches transition order. `watch::send_replace` is synchronous and
        // never blocks.
        self.status_tx.send_replace(next);
        Ok(())
    }

    /// Records that the process has started with the given pid.
    pub(crate) fn mark_running(&self, pid: u32) -> Result<(), ProcessError> {
        let mut state = self.lock();
        let current = state.status;
        if !current.can_transition_to(ProcessStatus::Running) {
            return Err(ProcessError::InvalidTransition {
                id: self.id.clone(),
                from: current,
                to: ProcessStatus::Running,
            });
        }
        state.status = ProcessStatus::Running;
        state.pid = Some(pid);
        state.started_at = Some(Timestamp::now());
        self.status_tx.send_replace(ProcessStatus::Running);
        drop(state);
        self.events.publish(ProcessEvent::Started {
            id: self.id.clone(),
            pid,
        });
        Ok(())
    }

    /// Records that a graceful stop is in progress.
    pub(crate) fn mark_stopping(&self) {
        let mut state = self.lock();
        if state.status.can_transition_to(ProcessStatus::Stopping) {
            state.status = ProcessStatus::Stopping;
            self.status_tx.send_replace(ProcessStatus::Stopping);
            drop(state);
            self.events.publish(ProcessEvent::Stopping {
                id: self.id.clone(),
            });
        }
    }

    /// Prepares the entry for a fresh execution instance.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::InvalidTransition`] if a restart is not legal
    /// from the current status.
    pub(crate) fn begin_restart(&self) -> Result<(), ProcessError> {
        let mut state = self.lock();
        let current = state.status;
        if !current.can_transition_to(ProcessStatus::Starting) {
            return Err(ProcessError::InvalidTransition {
                id: self.id.clone(),
                from: current,
                to: ProcessStatus::Starting,
            });
        }
        state.status = ProcessStatus::Starting;
        state.pid = None;
        state.started_at = None;
        state.exit = None;
        self.status_tx.send_replace(ProcessStatus::Starting);
        self.shutdown_tx.send_replace(ShutdownRequest::None);
        Ok(())
    }

    /// Publishes an event for this process.
    pub(crate) fn publish(&self, event: ProcessEvent) {
        self.events.publish(event);
    }

    /// Publishes a captured output line as both a log line and an event.
    pub(crate) fn publish_log(&self, kind: LogStreamKind, line: String) {
        let line = LogLine {
            process: self.id.clone(),
            stream: kind,
            line,
            timestamp: Timestamp::now(),
        };
        let event = match kind {
            LogStreamKind::Stdout => ProcessEvent::StdoutLine(line),
            LogStreamKind::Stderr => ProcessEvent::StderrLine(line),
        };
        self.events.publish(event);
    }

    /// Records a terminal exit and publishes the matching event.
    pub(crate) fn complete(&self, exit: ExitInfo) {
        let status = exit.status();
        if self
            .transition_with_exit(status, Some(exit.clone()))
            .is_err()
        {
            // A terminal state was already recorded; never panic in the
            // supervisor because a stop raced a natural exit.
            return;
        }
        let event = match status {
            ProcessStatus::Stopped => ProcessEvent::Stopped {
                id: self.id.clone(),
                exit,
            },
            ProcessStatus::Exited => ProcessEvent::Exited {
                id: self.id.clone(),
                exit,
            },
            ProcessStatus::Killed => ProcessEvent::Killed {
                id: self.id.clone(),
                exit,
            },
            _ => ProcessEvent::Failed {
                id: self.id.clone(),
                exit,
            },
        };
        self.events.publish(event);
    }

    /// Records a spawn failure and marks the process failed.
    ///
    /// Returns the synthesized exit information so callers can include it in a
    /// [`ProcessError`].
    pub(crate) fn fail_spawn(&self, message: String) -> ExitInfo {
        let now = Timestamp::now();
        let exit = ExitInfo::new(None, None, ExitReason::SpawnFailed, now, now);
        {
            let mut state = self.lock();
            state.last_error = Some(message);
        }
        let _ = self.transition_with_exit(ProcessStatus::Failed, Some(exit.clone()));
        self.events.publish(ProcessEvent::Failed {
            id: self.id.clone(),
            exit: exit.clone(),
        });
        exit
    }

    /// Requests a shutdown, if the process is still active.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::AlreadyTerminal`] if the process has finished.
    pub(crate) fn request_shutdown(&self, request: ShutdownRequest) -> Result<(), ProcessError> {
        let status = self.status();
        if status.is_terminal() {
            return Err(ProcessError::AlreadyTerminal {
                id: self.id.clone(),
                status,
            });
        }
        self.shutdown_tx.send_replace(request);
        Ok(())
    }

    /// Returns the pending shutdown request, if one was made.
    pub(crate) fn pending_shutdown(&self) -> Option<ShutdownRequest> {
        let request = *self.shutdown_tx.borrow();
        (request != ShutdownRequest::None).then_some(request)
    }

    /// Subscribes to shutdown requests.
    pub(crate) fn shutdown_receiver(&self) -> watch::Receiver<ShutdownRequest> {
        self.shutdown_tx.subscribe()
    }

    /// Sends a signal to the current pid, if the process is alive.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::NotFound`] when there is no live pid, or a
    /// platform/I/O error when delivery fails.
    pub(crate) fn signal(&self, signal: TerminationSignal) -> Result<(), ProcessError> {
        let pid = self
            .pid()
            .ok_or_else(|| ProcessError::NotFound(self.id.clone()))?;
        send_signal(pid, signal)
    }

    /// Subscribes to this process's events.
    pub(crate) fn subscribe_events(&self) -> ProcessEventStream {
        ProcessEventStream::new(Some(self.id.clone()), self.events.subscribe())
    }

    /// Subscribes to this process's log lines.
    pub(crate) fn subscribe_logs(&self) -> LogStream {
        LogStream::new(Some(self.id.clone()), self.events.subscribe())
    }

    /// Waits until the process reaches a terminal state.
    pub(crate) async fn wait_terminal(&self) -> ProcessSnapshot {
        loop {
            let (status, mut revision) = self.status_and_revision();
            if status.is_terminal() {
                return self.snapshot();
            }
            if revision.changed().await.is_err() {
                // The entry was dropped; the snapshot is still a valid read.
                return self.snapshot();
            }
        }
    }

    /// Returns the configured timeout.
    pub(crate) fn timeout(&self) -> Option<Duration> {
        self.spec.timeout
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, EntryState> {
        self.inner.lock().expect("process entry mutex poisoned")
    }
}

impl std::fmt::Debug for ProcessEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessEntry")
            .field("id", &self.id)
            .field("status", &self.status())
            .field("pid", &self.pid())
            .finish()
    }
}
