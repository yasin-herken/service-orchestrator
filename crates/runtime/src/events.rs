//! Service runtime events.
//!
//! The runtime adapter publishes a *higher-level* vocabulary than the process
//! manager: each event carries the workspace and service context an interface
//! needs to render "auth-service started" without joining a process id back to a
//! service. The process manager's bus is process-scoped and crate-private, so a
//! small, parallel broadcast bus is used here, mirroring its pattern
//! (`tasks/010.md` §27).
//!
//! Delivery is in-process only and bounded; the managed-process registry remains
//! authoritative for current state, so a subscriber that falls behind (a `Lagged`
//! error) can simply re-read the registry.

use tokio::sync::broadcast;

use service_orchestrator_domain::{ServiceId, WorkspaceId};

/// A notable change in a service's runtime lifecycle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeEvent {
    /// A service start is about to spawn a process.
    ServiceStarting {
        /// The owning workspace.
        workspace_id: WorkspaceId,
        /// The service being started.
        service_id: ServiceId,
        /// The runtime label.
        runtime: String,
        /// The redacted startup command.
        command: String,
    },
    /// A service process is alive and monitored.
    ServiceStarted {
        /// The owning workspace.
        workspace_id: WorkspaceId,
        /// The service that started.
        service_id: ServiceId,
        /// The managed process identifier.
        process_id: String,
        /// The operating-system process id.
        pid: Option<u32>,
        /// The runtime label.
        runtime: String,
    },
    /// A graceful stop has been requested.
    ServiceStopping {
        /// The owning workspace.
        workspace_id: WorkspaceId,
        /// The service being stopped.
        service_id: ServiceId,
        /// The managed process identifier.
        process_id: String,
    },
    /// A service process ended after a requested stop.
    ServiceStopped {
        /// The owning workspace.
        workspace_id: WorkspaceId,
        /// The service that stopped.
        service_id: ServiceId,
        /// The managed process identifier.
        process_id: String,
        /// The exit code, when the platform reported one.
        exit_code: Option<i32>,
        /// How the execution ended.
        exit_reason: String,
    },
    /// A service process ended on its own.
    ServiceExited {
        /// The owning workspace.
        workspace_id: WorkspaceId,
        /// The service that exited.
        service_id: ServiceId,
        /// The managed process identifier.
        process_id: String,
        /// The exit code, when the platform reported one.
        exit_code: Option<i32>,
        /// How the execution ended.
        exit_reason: String,
    },
    /// A service failed to start or crashed.
    ServiceFailed {
        /// The owning workspace.
        workspace_id: WorkspaceId,
        /// The service that failed.
        service_id: ServiceId,
        /// The managed process identifier, when one exists.
        process_id: Option<String>,
        /// A human-readable explanation.
        message: String,
    },
}

impl RuntimeEvent {
    /// Returns the service this event concerns.
    #[must_use]
    pub fn service_id(&self) -> &ServiceId {
        match self {
            Self::ServiceStarting { service_id, .. }
            | Self::ServiceStarted { service_id, .. }
            | Self::ServiceStopping { service_id, .. }
            | Self::ServiceStopped { service_id, .. }
            | Self::ServiceExited { service_id, .. }
            | Self::ServiceFailed { service_id, .. } => service_id,
        }
    }
}

/// The runtime adapter's bounded broadcast bus.
#[derive(Clone)]
pub(crate) struct RuntimeEventBus {
    sender: broadcast::Sender<RuntimeEvent>,
}

impl RuntimeEventBus {
    /// Creates a bus with the given channel capacity (at least one).
    pub(crate) fn new(capacity: usize) -> Self {
        let (sender, _receiver) = broadcast::channel(capacity.max(1));
        Self { sender }
    }

    /// Publishes an event, ignoring the absence of subscribers.
    pub(crate) fn publish(&self, event: RuntimeEvent) {
        let _ = self.sender.send(event);
    }

    /// Subscribes to future events.
    pub(crate) fn subscribe(&self) -> RuntimeEventStream {
        RuntimeEventStream {
            receiver: self.sender.subscribe(),
        }
    }
}

/// A bounded, async view of runtime events.
pub struct RuntimeEventStream {
    receiver: broadcast::Receiver<RuntimeEvent>,
}

impl RuntimeEventStream {
    /// Receives the next event.
    ///
    /// # Errors
    ///
    /// Returns [`broadcast::error::RecvError`] when the consumer fell behind or
    /// the manager has shut down.
    pub async fn recv(&mut self) -> Result<RuntimeEvent, broadcast::error::RecvError> {
        self.receiver.recv().await
    }
}

impl std::fmt::Debug for RuntimeEventStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeEventStream").finish_non_exhaustive()
    }
}
