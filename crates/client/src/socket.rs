//! Socket path resolution, daemon auto-start, and readiness waiting.
//!
//! The IPC endpoint is a Unix domain socket. Its path is configurable through
//! the `SERVICE_ORCHESTRATOR_SOCKET` environment variable and otherwise derived
//! from the configuration file's directory (`<config_dir>/run/daemon.sock`), so
//! two configurations never share a daemon (`tasks/015.md` §7, §14).
//!
//! The socket lives in a per-user runtime directory created with `0700`
//! permissions and the socket itself is restricted to the owner, so another
//! local user cannot connect (`tasks/015.md` §24).

use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use service_orchestrator_config::default_config_dir;

/// The environment variable that overrides the socket path.
pub const SOCKET_ENV: &str = "SERVICE_ORCHESTRATOR_SOCKET";

/// The environment variable that disables automatic daemon startup when set to
/// a truthy value (`1`, `true`, `yes`).
pub const NO_AUTOSTART_ENV: &str = "SERVICE_ORCHESTRATOR_NO_AUTOSTART";

/// How long a client waits for a freshly started daemon to accept connections.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

/// The interval between readiness probes.
const STARTUP_POLL: Duration = Duration::from_millis(50);

/// Returns the per-user runtime directory for the given configuration file.
#[must_use]
pub fn run_dir(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(
            || default_config_dir().join("run"),
            |parent| parent.join("run"),
        )
}

/// Returns the configured socket path for the given configuration file.
#[must_use]
pub fn socket_path(config_path: &Path) -> PathBuf {
    if let Some(override_path) = std::env::var_os(SOCKET_ENV) {
        return PathBuf::from(override_path);
    }
    run_dir(config_path).join("daemon.sock")
}

/// Returns the daemon PID file path that sits beside the socket.
#[must_use]
pub fn pid_path(socket_path: &Path) -> PathBuf {
    socket_path.with_extension("pid")
}

/// Returns the daemon log file path that sits beside the socket.
#[must_use]
pub fn log_path(socket_path: &Path) -> PathBuf {
    socket_path.with_extension("log")
}

/// Creates the runtime directory with owner-only permissions if it is missing.
///
/// # Errors
///
/// Returns an [`io::Error`] when the directory cannot be created or its
/// permissions cannot be tightened.
pub fn ensure_run_dir(socket_path: &Path) -> io::Result<()> {
    let dir = socket_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// Returns `true` when a process is accepting connections at `socket_path`.
#[must_use]
pub fn is_reachable(socket_path: &Path) -> bool {
    UnixStream::connect(socket_path).is_ok()
}

/// Returns `true` when automatic daemon startup is disabled by the environment.
#[must_use]
pub fn autostart_disabled() -> bool {
    std::env::var(NO_AUTOSTART_ENV).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

/// Starts a detached daemon for `config_path` and waits until it is reachable.
///
/// The child runs in its own process group so a terminal hang-up on the parent
/// does not signal it, and its standard streams are redirected away from the
/// caller's terminal. The wait is bounded by [`STARTUP_TIMEOUT`].
///
/// # Errors
///
/// Returns an [`io::Error`] when the daemon cannot be spawned or does not become
/// reachable before the timeout.
pub fn start_daemon(config_path: &Path, socket_path: &Path) -> io::Result<UnixStream> {
    ensure_run_dir(socket_path)?;
    let exe = std::env::current_exe()?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path(socket_path))?;
    let log_err = log.try_clone()?;

    let mut command = Command::new(exe);
    command
        .arg("daemon")
        .arg("--config")
        .arg(config_path)
        .env(SOCKET_ENV, socket_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    // Put the daemon in its own process group so it survives the terminal that
    // launched the client (`tasks/015.md` §16).
    use std::os::unix::process::CommandExt;
    command.process_group(0);
    let mut child = command.spawn()?;
    wait_for_socket(socket_path, &mut child)
}

/// Polls `socket_path` until a connection succeeds or `child` exits.
fn wait_for_socket(socket_path: &Path, child: &mut Child) -> io::Result<UnixStream> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        match UnixStream::connect(socket_path) {
            Ok(stream) => return Ok(stream),
            Err(error) => {
                if let Some(status) = child.try_wait()? {
                    return Err(io::Error::other(format!(
                        "daemon exited during startup with {status}"
                    )));
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        error.kind(),
                        format!(
                            "daemon did not become reachable at {} within {:?}",
                            socket_path.display(),
                            STARTUP_TIMEOUT
                        ),
                    ));
                }
                std::thread::sleep(STARTUP_POLL);
            }
        }
    }
}

/// Connects to `socket_path`, starting a daemon if none is running.
///
/// # Errors
///
/// Returns an [`io::Error`] when the daemon is unreachable and cannot be
/// started.
pub fn connect_or_start(config_path: &Path, socket_path: &Path) -> io::Result<UnixStream> {
    if let Ok(stream) = UnixStream::connect(socket_path) {
        return Ok(stream);
    }
    if autostart_disabled() {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!(
                "no daemon is running at {} and automatic startup is disabled",
                socket_path.display()
            ),
        ));
    }
    start_daemon(config_path, socket_path)
}

/// Connects to an already-running daemon at `socket_path`, never starting one.
///
/// This is what `daemon status` and `daemon stop` use: a diagnostic must report
/// a stopped daemon rather than start one (`tasks/015.md` §30).
///
/// # Errors
///
/// Returns an [`io::Error`] when no daemon is listening.
pub fn connect_existing(socket_path: &Path) -> io::Result<UnixStream> {
    UnixStream::connect(socket_path)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    use super::*;

    /// Serializes the tests that mutate the process environment.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Runs `body` with an environment variable set to `value`, restoring the
    /// previous value afterwards.
    fn with_env(name: &str, value: Option<&str>, body: impl FnOnce()) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let previous: Option<OsString> = std::env::var_os(name);
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
        body();
        match previous {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }

    #[test]
    fn the_socket_path_is_derived_from_the_config_directory() {
        with_env(SOCKET_ENV, None, || {
            let config = Path::new("/tmp/example/config.toml");
            let socket = PathBuf::from("/tmp/example/run/daemon.sock");
            assert_eq!(socket_path(config), socket);
            assert_eq!(
                pid_path(&socket),
                PathBuf::from("/tmp/example/run/daemon.pid")
            );
            assert_eq!(
                log_path(&socket),
                PathBuf::from("/tmp/example/run/daemon.log")
            );
            assert_eq!(run_dir(config), PathBuf::from("/tmp/example/run"));
        });
    }

    #[test]
    fn the_socket_environment_override_wins() {
        with_env(SOCKET_ENV, Some("/custom/endpoint.sock"), || {
            assert_eq!(
                socket_path(Path::new("/tmp/example/config.toml")),
                PathBuf::from("/custom/endpoint.sock")
            );
        });
    }

    #[test]
    fn autostart_can_be_disabled_by_the_environment() {
        with_env(NO_AUTOSTART_ENV, None, || {
            assert!(!autostart_disabled(), "autostart is enabled by default");
        });
        for value in ["1", "true", "YES", "On"] {
            with_env(NO_AUTOSTART_ENV, Some(value), || {
                assert!(autostart_disabled(), "value {value:?} disables autostart");
            });
        }
        with_env(NO_AUTOSTART_ENV, Some("no"), || {
            assert!(!autostart_disabled(), "a falsey value keeps autostart on");
        });
    }

    #[test]
    fn the_runtime_directory_is_created_owner_only() {
        let root = std::env::temp_dir().join(format!("so-client-socket-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let socket = root.join("run/daemon.sock");
        ensure_run_dir(&socket).expect("the run directory is created");
        let directory = socket.parent().expect("the socket has a parent");
        let mode = std::fs::metadata(directory)
            .expect("the directory exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700, "the runtime directory is owner-only");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_endpoint_is_not_reachable() {
        let missing = std::env::temp_dir().join("so-client-does-not-exist.sock");
        let _ = std::fs::remove_file(&missing);
        assert!(!is_reachable(&missing));
        assert!(connect_existing(&missing).is_err());
    }
}
