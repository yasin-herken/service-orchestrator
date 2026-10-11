//! Single-instance coordination and runtime-file management.
//!
//! Only one daemon may own a runtime directory. The guarantee comes from an
//! advisory lock on a lock file beside the socket: the kernel releases it
//! automatically when the process dies, so a crash never leaves the directory
//! permanently locked. The socket file alone is *not* used as the lock, because
//! a stale socket file outlives the process that created it
//! (`tasks/015.md` §15).
//!
//! The lock file, PID file, and socket all live in the same per-user runtime
//! directory, created with `0700` permissions (`tasks/015.md` §24).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use nix::fcntl::{Flock, FlockArg};
use service_orchestrator_client::pid_path;

use crate::error::DaemonError;

/// A held exclusive lock on a runtime directory.
///
/// Dropping the guard releases the lock and removes the PID file. The socket
/// file is removed separately by [`SocketGuard`].
pub(crate) struct InstanceLock {
    _lock: Flock<File>,
}

impl InstanceLock {
    /// Acquires the exclusive lock for the daemon owning `socket_path`.
    ///
    /// # Errors
    ///
    /// Returns [`DaemonError::AlreadyRunning`] when another process holds the
    /// lock, or [`DaemonError::Io`] when the lock file cannot be opened.
    pub(crate) fn acquire(socket_path: &Path) -> Result<Self, DaemonError> {
        let path = lock_path(socket_path);
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| DaemonError::io("open the lock file", &path, error))?;
        let lock = Flock::lock(file, FlockArg::LockExclusiveNonblock)
            .map_err(|_| DaemonError::AlreadyRunning(path))?;
        Ok(Self { _lock: lock })
    }
}

/// Removes the socket and PID files on drop.
pub(crate) struct SocketGuard {
    socket: PathBuf,
    pid: PathBuf,
}

impl SocketGuard {
    /// Arms cleanup for `socket_path`.
    #[must_use]
    pub(crate) fn new(socket_path: &Path) -> Self {
        Self {
            socket: socket_path.to_path_buf(),
            pid: pid_path(socket_path),
        }
    }

    /// Removes the PID file eagerly; the socket is removed on drop.
    pub(crate) fn remove_pid(&self) {
        let _ = std::fs::remove_file(&self.pid);
    }
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
        let _ = std::fs::remove_file(&self.pid);
    }
}

/// Returns the lock file path beside `socket_path`.
#[must_use]
pub(crate) fn lock_path(socket_path: &Path) -> PathBuf {
    socket_path.with_extension("lock")
}

/// Writes the current process id to the PID file beside `socket_path`.
///
/// # Errors
///
/// Returns [`DaemonError::Io`] when the file cannot be written.
pub(crate) fn write_pid_file(socket_path: &Path, pid: u32) -> Result<(), DaemonError> {
    let path = pid_path(socket_path);
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)
        .map_err(|error| DaemonError::io("write the pid file", &path, error))?;
    writeln!(file, "{pid}").map_err(|error| DaemonError::io("write the pid file", &path, error))
}
