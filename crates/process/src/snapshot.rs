//! A point-in-time view of a managed process.
//!
//! [`ProcessSnapshot`] is the value returned by the registry and the status
//! queries. It is structured, `serde`-serializable, and never contains a raw
//! process handle. Secret environment values remain redacted because they are
//! carried in the domain [`Environment`] type, which redacts on display and
//! serialization.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use service_orchestrator_domain::{Environment, ProcessState, ServiceId, Timestamp};

use crate::id::ProcessId;
use crate::state::{ExitInfo, ProcessStatus};

/// An immutable snapshot of a managed process's identity, specification, and
/// observed state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessSnapshot {
    /// The process identifier.
    pub id: ProcessId,
    /// The service this process belongs to, when it is a service process.
    pub service: Option<ServiceId>,
    /// The current lifecycle status.
    pub status: ProcessStatus,
    /// The operating-system process id, while the process is alive.
    pub pid: Option<u32>,
    /// The executable.
    pub program: String,
    /// The arguments passed to the executable.
    pub args: Vec<String>,
    /// The working directory, when one was configured.
    pub working_dir: Option<PathBuf>,
    /// The configured environment; secret values are redacted when serialized.
    pub environment: Environment,
    /// When the current execution started.
    pub started_at: Option<Timestamp>,
    /// How the last execution ended, once it has.
    pub exit: Option<ExitInfo>,
    /// The last error the manager recorded, for example a spawn failure.
    pub last_error: Option<String>,
}

impl ProcessSnapshot {
    /// Returns the process identifier.
    #[must_use]
    pub fn id(&self) -> &ProcessId {
        &self.id
    }

    /// Returns `true` if the process has reached a terminal state.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }

    /// Returns `true` if the process is currently alive.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.status.is_running()
    }

    /// Maps the rich status into the interface-neutral domain vocabulary.
    #[must_use]
    pub fn domain_state(&self) -> ProcessState {
        self.status.domain_state()
    }

    /// Returns the redacted command line (`program` plus arguments).
    #[must_use]
    pub fn description(&self) -> String {
        let mut rendered = self.program.clone();
        for arg in &self.args {
            rendered.push(' ');
            rendered.push_str(arg);
        }
        rendered
    }
}
