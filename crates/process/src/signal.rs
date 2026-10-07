//! Operating-system signal delivery.
//!
//! The process manager prefers a graceful signal (`SIGTERM` by default) and
//! escalates to `SIGKILL` only when a process does not exit within its grace
//! period. On Unix this is implemented with the safe [`nix`] wrappers; on other
//! platforms signalling is unsupported and the caller falls back to the Tokio
//! child's own kill path.
//!
//! Every process is spawned into its own process group (see
//! [`build_command`](crate::ProcessManager)), and signals are delivered to the
//! whole group. This is what prevents a shell that spawned children from
//! leaving those children behind: terminating the group reaps the descendants
//! too, so the manager never orphans a child process (`AGENTS.md` §15).
//!
//! Signalling is best-effort: a process that has already exited, or whose pid
//! has been recycled in the tiny window between observation and delivery, may
//! cause the signal call to fail. The supervisor treats a failed signal as a
//! non-fatal condition and still waits for the child to be reaped.

use crate::error::ProcessError;
use crate::spec::TerminationSignal;

/// Sends `signal` to the process group led by `pid`.
///
/// The negative pid form targets the whole process group, so descendants
/// spawned by the process are signalled as well.
///
/// # Errors
///
/// Returns [`ProcessError::UnsupportedSignal`] on platforms without Unix
/// signalling support, or [`ProcessError::Io`] when the operating system
/// rejects the signal.
#[cfg(unix)]
pub(crate) fn send_signal(pid: u32, signal: TerminationSignal) -> Result<(), ProcessError> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    let signal = match signal {
        TerminationSignal::Term => Signal::SIGTERM,
        TerminationSignal::Int => Signal::SIGINT,
        TerminationSignal::Kill => Signal::SIGKILL,
    };
    // A negative pid addresses the process group whose id is `pid`.
    let group = Pid::from_raw(-(pid as i32));
    kill(group, signal)
        .map_err(|error| ProcessError::io(format!("could not signal process group {pid}: {error}")))
}

/// Sends `signal` to the process group led by `pid`.
///
/// # Errors
///
/// Always returns [`ProcessError::UnsupportedSignal`] on non-Unix platforms.
#[cfg(not(unix))]
pub(crate) fn send_signal(_pid: u32, _signal: TerminationSignal) -> Result<(), ProcessError> {
    Err(ProcessError::UnsupportedSignal)
}
