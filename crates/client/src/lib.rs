//! # Service Orchestrator client
//!
//! The shared client boundary every interface — the Ratatui TUI, the MCP stdio
//! server, and the `daemon status` command — talks to.
//!
//! A [`OrchestratorClient`] mirrors the application core's use cases one-to-one:
//! an interface expresses intent (a method call) and never learns whether it is
//! running in-process or proxying to the local daemon. Two implementations
//! exist:
//!
//! - [`Application`] itself implements the trait, which keeps interface tests on
//!   the real core with no daemon; and
//! - [`RemoteClient`] proxies each call over the versioned local IPC protocol to
//!   the daemon (`tasks/015.md` §11, §13).
//!
//! The wire format, socket resolution, and daemon auto-start all live here so
//! the daemon and its clients agree on a single implementation of the transport.
//!
//! See `docs/local-daemon.md` and `docs/adr/015-shared-local-runtime.md`.

mod client;
pub mod ops;
mod protocol;
mod remote;
mod socket;

pub use client::{ConfigSummary, OrchestratorClient};
pub use protocol::{
    codes, read_frame, unavailable, write_frame, ClientKind, DiffParams, RequestEnvelope,
    ResponseEnvelope, ServerEvent, ServerMessage, MAX_FRAME_BYTES, PROTOCOL_VERSION,
};
pub use remote::{DaemonStatus, RemoteClient, RemoteLogSubscription, REQUEST_TIMEOUT};
pub use socket::{
    autostart_disabled, connect_existing, connect_or_start, ensure_run_dir, is_reachable, log_path,
    pid_path, run_dir, socket_path, start_daemon, NO_AUTOSTART_ENV, SOCKET_ENV,
};
