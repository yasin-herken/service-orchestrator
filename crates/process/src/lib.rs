//! # Process
//!
//! The local process manager: the one component in Service Orchestrator that is
//! allowed to create, monitor, and terminate operating-system processes.
//!
//! It tracks pids, commands, states, exit codes, child processes, and logs; it
//! captures `stdout` and `stderr` independently; it performs graceful and forced
//! termination; and it emits interface-neutral events. Every other subsystem —
//! Git, build tooling, Liquibase, health checks, and the service executors —
//! must ask the process manager to run a process rather than spawning one
//! directly (`AGENTS.md` §15, §32).
//!
//! # Architecture
//!
//! ```text
//! Git / Build / Liquibase / ServiceExecutor
//!                     │
//!                     ▼
//!              ProcessManager ──> ProcessRegistry (in-memory)
//!                     │                    ▲
//!                     ▼                    │
//!               supervisor task ───────────┘
//!                     │
//!        ┌────────────┼────────────┐
//!        ▼            ▼            ▼
//!      stdout       stderr       wait/exit
//!                     │
//!                     ▼
//!          events / log streams
//! ```
//!
//! # Lifecycle
//!
//! ```text
//! Created ──> Starting ──> Running ──> (Stopping) ──> Stopped | Exited | Failed | Killed
//!                              │
//!                              └──> Starting   (an automatic restart)
//! ```
//!
//! [`ProcessStatus::can_transition_to`] is the single source of truth for
//! legality. A requested stop is always graceful first (`SIGTERM` by default),
//! escalates to `SIGKILL` after a configurable grace period, and is recorded as
//! [`ExitReason::Terminated`], [`ExitReason::Cancelled`](crate::ExitReason::Cancelled),
//! or [`ExitReason::TimedOut`](crate::ExitReason::TimedOut) as appropriate.
//!
//! # Cancellation
//!
//! [`ProcessManager::spawn_with_cancellation`] spawns a process and stops it
//! when a caller-supplied future resolves. This is how the manager integrates
//! with the task engine's cooperative cancellation model without depending on
//! it: a task executor passes `context.cancellation().clone().cancelled()`.
//!
//! See `docs/process-manager.md` and `docs/adr/006-process-manager.md`.

#![deny(missing_docs)]

mod entry;
mod error;
mod event;
mod handle;
mod id;
mod log;
mod manager;
mod registry;
mod signal;
mod snapshot;
mod spec;
mod state;

pub use error::ProcessError;
pub use event::{ProcessEvent, ProcessEventStream};
pub use handle::ProcessHandle;
pub use id::{ProcessId, MAX_PROCESS_ID_LEN};
pub use log::{LogLine, LogStream, LogStreamKind};
pub use manager::{ProcessManager, ProcessManagerConfig};
pub use registry::ProcessRegistry;
pub use snapshot::ProcessSnapshot;
pub use spec::{ProcessSpec, RestartPolicy, ShutdownStrategy, TerminationSignal};
pub use state::{ExitInfo, ExitReason, ProcessStatus};
