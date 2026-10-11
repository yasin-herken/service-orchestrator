//! Events consumed by the interface.
//!
//! The event loop multiplexes three independent sources onto a single
//! [`AppEvent`] stream: terminal input, task-engine events, and the results of
//! asynchronous application queries. Keeping them in one enum means the state
//! machine has a single, deterministic entry point — which is also what makes it
//! testable without a terminal.

use service_orchestrator_application::{
    LibraryArtifactVerification, LibraryStatus, LogEntry, VersionAlignmentPlan,
};
use service_orchestrator_domain::{BranchInfo, LibraryId, RepositoryStatus, ServiceId};
use service_orchestrator_execution::TaskEvent;

use crate::input::InputEvent;

/// A request, produced by a user action, to perform an asynchronous query.
///
/// Effects are *descriptions*, not side effects: the state machine emits them,
/// and the event loop is the only component that actually runs work. Tests can
/// inspect the emitted effects without ever touching Git or Maven.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Inspect a service's repository status.
    GitStatus(ServiceId),
    /// List a service's repository branches.
    Branches(ServiceId),
    /// Inspect a library's status.
    LibraryStatus(LibraryId),
    /// Verify a library's artifact.
    VerifyArtifact(LibraryId),
    /// Inspect a library's consumer version alignment.
    Alignment(LibraryId),
    /// Begin streaming realtime logs for a service.
    StreamLogs(ServiceId),
    /// Stop any active realtime log stream.
    StopLogStream,
}

/// The result of an asynchronous query.
#[derive(Clone, Debug)]
pub enum AsyncOutcome {
    /// A repository status inspection finished.
    GitStatus {
        /// The inspected service.
        service: ServiceId,
        /// The status, or a structured failure message.
        result: Result<RepositoryStatus, String>,
    },
    /// A branch listing finished.
    Branches {
        /// The inspected service.
        service: ServiceId,
        /// The branches, or a structured failure message.
        result: Result<Vec<BranchInfo>, String>,
    },
    /// A library status inspection finished.
    LibraryStatus {
        /// The inspected library.
        library: LibraryId,
        /// The status, or a structured failure message.
        result: Result<LibraryStatus, String>,
    },
    /// An artifact verification finished.
    VerifyArtifact {
        /// The inspected library.
        library: LibraryId,
        /// The verification, or a structured failure message.
        result: Result<LibraryArtifactVerification, String>,
    },
    /// A version-alignment inspection finished.
    Alignment {
        /// The inspected library.
        library: LibraryId,
        /// The plan, or a structured failure message.
        result: Result<VersionAlignmentPlan, String>,
    },
}

/// Anything the state machine reacts to.
#[derive(Clone, Debug)]
pub enum AppEvent {
    /// Terminal input.
    Input(InputEvent),
    /// A task-engine event.
    Task(TaskEvent),
    /// The result of an asynchronous query.
    Async(Box<AsyncOutcome>),
    /// A realtime log entry delivered from a service log stream.
    Log(LogEntry),
    /// A periodic tick, used for refresh and toast expiry.
    Tick,
}
