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

use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use service_orchestrator_config::default_config_dir;
use service_orchestrator_domain::redact;

/// The environment variable that overrides the socket path.
pub const SOCKET_ENV: &str = "SERVICE_ORCHESTRATOR_SOCKET";

/// The environment variable that disables automatic daemon startup when set to
/// a truthy value (`1`, `true`, `yes`).
pub const NO_AUTOSTART_ENV: &str = "SERVICE_ORCHESTRATOR_NO_AUTOSTART";

/// The environment variable that overrides the executable spawned as the
/// daemon.
///
/// Automatic startup re-executes the current program (`daemon --config …`),
/// which is correct for the single `service-orchestrator` binary but not for a
/// host process that embeds the client without a `daemon` subcommand — the
/// Tauri desktop app, for example (`tasks/018.md` §8). Setting this variable
/// points the spawn at the real `service-orchestrator` executable while the
/// rest of the Task 015 auto-start mechanism (process group, log redirect,
/// readiness poll, advisory-lock race resolution) stays shared and unchanged.
///
/// An unset or invalid value falls back to the previous behaviour.
pub const DAEMON_BIN_ENV: &str = "SERVICE_ORCHESTRATOR_DAEMON_BIN";

/// How long a client waits for a freshly started daemon to accept connections.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

/// The interval between readiness probes.
const STARTUP_POLL: Duration = Duration::from_millis(50);

/// How long a start attempt keeps probing after its own child exited.
///
/// Two interfaces may race to start the daemon at the same time. Only one wins
/// the runtime directory's single-instance lock; the loser exits. Before
/// reporting that exit as a failure, the client gives the *winner* a short grace
/// period to bind, so a concurrent start adopts the surviving daemon instead of
/// surfacing a spurious failure (`tasks/031.md` §14).
const DAEMON_EXIT_GRACE: Duration = Duration::from_secs(2);

/// How many bytes of the daemon's own output a failure reports.
const LOG_EXCERPT_BYTES: u64 = 8 * 1024;

/// How many trailing log lines a failure reports.
const LOG_EXCERPT_LINES: usize = 20;

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
/// The directory is created with `0700` atomically (the creation mode can only
/// be narrowed by the umask), and a pre-existing directory is **never**
/// re-chmodded: `SERVICE_ORCHESTRATOR_SOCKET` may point at a directory the
/// daemon does not own, and tightening a shared directory such as `/tmp` would
/// break the whole system (`tasks/017.md` §27, finding H-IPC-3).
///
/// # Errors
///
/// Returns an [`io::Error`] when the directory cannot be created.
pub fn ensure_run_dir(socket_path: &Path) -> io::Result<()> {
    let dir = socket_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
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

/// Returns the executable that automatic startup will spawn as the daemon.
///
/// [`DAEMON_BIN_ENV`] wins when it names an existing file; otherwise the
/// current program is re-executed, which is the correct behaviour for the
/// single `service-orchestrator` binary.
///
/// # Errors
///
/// Returns an [`io::Error`] when the override is set but does not name an
/// existing file, or when the current executable cannot be determined.
pub fn daemon_executable() -> io::Result<PathBuf> {
    if let Some(override_path) = std::env::var_os(DAEMON_BIN_ENV) {
        let path = PathBuf::from(override_path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(io::Error::other(format!(
            "{DAEMON_BIN_ENV} points at {}, which is not an existing file",
            path.display()
        )));
    }
    std::env::current_exe()
}

/// Builds the detached daemon command for `config_path`.
///
/// The child runs in its own process group so a terminal hang-up on the parent
/// does not signal it. Its `PATH` is augmented with the discovered developer
/// tool directories, because a GUI launched from Finder has a bare system
/// `PATH` and the daemon (plus the managed services it later spawns) must still
/// find `git`, `java`, `mvn`, and `node` (`tasks/031.md` §19). The inherited
/// entries keep precedence.
fn daemon_command(exe: &Path, config_path: &Path, socket_path: &Path) -> Command {
    let mut command = Command::new(exe);
    command
        .arg("daemon")
        .arg("--config")
        .arg(config_path)
        .env(SOCKET_ENV, socket_path)
        .env("PATH", crate::path_env::effective_search_path());
    // Put the daemon in its own process group so it survives the terminal that
    // launched the client (`tasks/015.md` §16).
    use std::os::unix::process::CommandExt;
    command.process_group(0);
    command
}

/// Starts a detached daemon for `config_path` and waits until it is reachable.
///
/// The child runs in its own process group so a terminal hang-up on the parent
/// does not signal it, and its standard streams are redirected away from the
/// caller's terminal. The wait is bounded by [`STARTUP_TIMEOUT`].
///
/// # Errors
///
/// Returns an [`io::Error`] when the daemon cannot be spawned, exits before
/// becoming reachable, or does not become reachable before the timeout. The
/// message names the executable, its arguments, the IPC endpoint, the exit
/// status (when it exited), and a bounded, redacted excerpt of the daemon's own
/// output, so a failed auto-start is actionable without scraping the log by hand
/// (`tasks/031.md` §40).
pub fn start_daemon(config_path: &Path, socket_path: &Path) -> io::Result<UnixStream> {
    ensure_run_dir(socket_path)?;
    let exe = daemon_executable()?;
    let log_path = log_path(socket_path);
    let log = open_daemon_log(&log_path)?;
    let log_err = log.try_clone()?;

    let mut command = daemon_command(&exe, config_path, socket_path);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    // The diagnostic arguments are the command's own, so they can never drift
    // from what is actually spawned.
    let arguments = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    let startup = DaemonStartup {
        executable: exe,
        arguments,
        socket_path: socket_path.to_path_buf(),
        log_path,
        timeout: STARTUP_TIMEOUT,
        exit_grace: DAEMON_EXIT_GRACE,
    };
    let mut child = command.spawn().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "could not start the daemon `{}` at {}: {error}",
                startup.command(),
                startup.socket_path.display()
            ),
        )
    })?;
    wait_for_socket(&startup, &mut child)
}

/// What a start attempt needs to explain a failure.
struct DaemonStartup {
    /// The daemon executable that was spawned.
    executable: PathBuf,
    /// The arguments the daemon was spawned with.
    arguments: String,
    /// The IPC endpoint the daemon was expected to bind.
    socket_path: PathBuf,
    /// The daemon's own output log.
    log_path: PathBuf,
    /// How long to wait for readiness before giving up.
    timeout: Duration,
    /// How long to keep probing after the child exited, for a concurrent winner.
    exit_grace: Duration,
}

impl DaemonStartup {
    /// The full command line, for diagnostics.
    fn command(&self) -> String {
        format!("{} {}", self.executable.display(), self.arguments)
    }

    /// A bounded, redacted excerpt of the daemon's own output.
    fn excerpt(&self) -> String {
        match read_log_tail(&self.log_path) {
            Some(lines) if !lines.trim().is_empty() => {
                format!("\n--- daemon output (last lines) ---\n{lines}")
            }
            _ => String::new(),
        }
    }

    /// The error for a daemon that exited before becoming reachable.
    fn failure_message(&self, status: ExitStatus) -> String {
        format!(
            "the daemon `{}` exited during startup with {status} before becoming reachable at {}\n\
             daemon log: {}{}",
            self.command(),
            self.socket_path.display(),
            self.log_path.display(),
            self.excerpt()
        )
    }

    /// The error for a daemon that did not become reachable in time.
    fn timeout_message(&self) -> String {
        format!(
            "the daemon `{}` did not become reachable at {} within {:?}\n\
             daemon log: {}{}",
            self.command(),
            self.socket_path.display(),
            self.timeout,
            self.log_path.display(),
            self.excerpt()
        )
    }
}

/// Reads the last few lines of the daemon log, redacted, for a failure message.
fn read_log_tail(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    if length > LOG_EXCERPT_BYTES {
        file.seek(SeekFrom::End(-(LOG_EXCERPT_BYTES as i64))).ok()?;
    }
    let mut bytes = Vec::new();
    file.take(LOG_EXCERPT_BYTES).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(LOG_EXCERPT_LINES);
    Some(redact(&lines[start..].join("\n")))
}

/// Polls `socket_path` until a connection succeeds, the child exits, or the
/// startup timeout elapses.
///
/// The socket is probed *first* on every iteration, so a daemon started
/// concurrently by another interface is adopted as soon as it binds, even if
/// this attempt's own child lost the single-instance race and exited.
fn wait_for_socket(startup: &DaemonStartup, child: &mut Child) -> io::Result<UnixStream> {
    let deadline = Instant::now() + startup.timeout;
    let mut exit_status: Option<ExitStatus> = None;
    let mut exit_deadline: Option<Instant> = None;
    loop {
        match UnixStream::connect(&startup.socket_path) {
            Ok(stream) => return Ok(stream),
            Err(error) => {
                if exit_status.is_none() {
                    if let Some(status) = child.try_wait()? {
                        exit_status = Some(status);
                        // Give a concurrent winner a short grace to bind before
                        // declaring this attempt failed, bounded by the timeout.
                        exit_deadline = Some((Instant::now() + startup.exit_grace).min(deadline));
                    }
                }
                match (exit_status, exit_deadline) {
                    (Some(status), Some(exit_deadline)) => {
                        if Instant::now() >= exit_deadline {
                            return Err(io::Error::other(startup.failure_message(status)));
                        }
                    }
                    _ if Instant::now() >= deadline => {
                        return Err(io::Error::new(error.kind(), startup.timeout_message()));
                    }
                    _ => {}
                }
                std::thread::sleep(STARTUP_POLL);
            }
        }
    }
}

/// Opens (creating if needed) the daemon's log file, owner-only.
///
/// The daemon log captures the child's standard output and error, which can
/// include configuration-derived detail, so it is created `0600` and an
/// existing file is re-tightened to `0600`. The containing runtime directory is
/// normally already `0700`, but this holds even when
/// `SERVICE_ORCHESTRATOR_SOCKET` points at a shared directory such as `/tmp`
/// (`AGENTS.md` §31, §33).
fn open_daemon_log(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(file)
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
    use std::ffi::{OsStr, OsString};
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
    fn the_daemon_executable_defaults_to_the_current_program() {
        with_env(DAEMON_BIN_ENV, None, || {
            assert_eq!(
                daemon_executable().expect("the current program resolves"),
                std::env::current_exe().expect("the current program is known")
            );
        });
    }

    #[test]
    fn the_daemon_executable_override_wins() {
        let dir = std::env::temp_dir().join(format!("so-daemon-bin-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let binary = dir.join("service-orchestrator");
        std::fs::write(&binary, b"#!/bin/sh\n").expect("the stand-in binary is written");
        with_env(
            DAEMON_BIN_ENV,
            Some(binary.to_str().expect("utf-8 path")),
            || {
                assert_eq!(
                    daemon_executable().expect("the override resolves"),
                    binary,
                    "the override names the daemon executable"
                );
            },
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_daemon_executable_override_is_rejected() {
        with_env(
            DAEMON_BIN_ENV,
            Some("/nonexistent/service-orchestrator"),
            || {
                let error = daemon_executable().expect_err("the override must name a file");
                assert!(
                    error.to_string().contains(DAEMON_BIN_ENV),
                    "the error names the variable: {error}"
                );
            },
        );
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
    fn a_pre_existing_directory_is_never_rechmodded() {
        // Regression test for `tasks/017.md` finding H-IPC-3: with
        // SERVICE_ORCHESTRATOR_SOCKET pointing inside a shared directory,
        // ensure_run_dir must not tighten that directory's permissions.
        let root = std::env::temp_dir().join(format!("so-client-shared-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the shared directory is created");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755))
            .expect("the shared directory is world-readable");
        let socket = root.join("daemon.sock");
        ensure_run_dir(&socket).expect("the shared directory is acceptable");
        let mode = std::fs::metadata(&root)
            .expect("the directory exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o755,
            "a pre-existing directory keeps its permissions"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_spawned_daemon_receives_the_socket_and_an_augmented_path() {
        // A GUI launched from Finder starts the daemon through this command, so
        // it must carry the discovered developer tool directories on PATH
        // (tasks/031.md §19).
        let command = daemon_command(
            Path::new("/bin/true"),
            Path::new("/tmp/example/config.toml"),
            Path::new("/tmp/example/run/daemon.sock"),
        );
        let envs: Vec<(OsString, Option<OsString>)> = command
            .get_envs()
            .map(|(key, value)| (key.to_owned(), value.map(OsStr::to_owned)))
            .collect();

        let socket = envs
            .iter()
            .find(|(key, _)| key == SOCKET_ENV)
            .expect("the socket override is exported");
        assert_eq!(
            socket.1.as_deref(),
            Some(OsStr::new("/tmp/example/run/daemon.sock"))
        );

        let path = envs
            .iter()
            .find(|(key, _)| key == OsStr::new("PATH"))
            .expect("the PATH is exported");
        assert_eq!(
            path.1.as_deref(),
            Some(crate::path_env::effective_search_path().as_os_str())
        );
    }

    #[test]
    fn a_missing_endpoint_is_not_reachable() {
        let missing = std::env::temp_dir().join("so-client-does-not-exist.sock");
        let _ = std::fs::remove_file(&missing);
        assert!(!is_reachable(&missing));
        assert!(connect_existing(&missing).is_err());
    }

    /// A scratch directory removed when the test finishes.
    struct TempRoot(PathBuf);

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Builds a start-attempt context with short, deterministic bounds.
    fn startup(name: &str, timeout: Duration, exit_grace: Duration) -> (DaemonStartup, TempRoot) {
        let dir =
            std::env::temp_dir().join(format!("so-client-startup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the scratch directory is created");
        let root = TempRoot(dir);
        let socket = root.0.join("run/daemon.sock");
        let log_path = root.0.join("run/daemon.log");
        std::fs::create_dir_all(socket.parent().expect("run dir")).expect("run dir is created");
        let context = DaemonStartup {
            executable: PathBuf::from("/opt/service-orchestrator"),
            arguments: "daemon --config /tmp/example/config.toml".to_owned(),
            socket_path: socket,
            log_path,
            timeout,
            exit_grace,
        };
        (context, root)
    }

    #[test]
    fn a_daemon_that_never_binds_times_out_with_actionable_diagnostics() {
        let (context, _root) = startup(
            "timeout",
            Duration::from_millis(150),
            Duration::from_millis(150),
        );
        std::fs::write(
            &context.log_path,
            b"booting the daemon\nDB_PASSWORD=hunter2\n",
        )
        .expect("the daemon log exists");
        let mut child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("a stand-in daemon is spawned");

        let error = wait_for_socket(&context, &mut child).expect_err("readiness times out");
        let message = error.to_string();
        assert!(message.contains("did not become reachable"), "{message}");
        assert!(message.contains("/opt/service-orchestrator"), "{message}");
        assert!(
            message.contains("daemon --config /tmp/example/config.toml"),
            "names the arguments: {message}"
        );
        assert!(
            message.contains("daemon.sock"),
            "names the endpoint: {message}"
        );
        assert!(
            !message.contains("hunter2"),
            "the excerpt is redacted: {message}"
        );

        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn a_daemon_that_exits_is_reported_with_its_exit_status() {
        let (context, _root) = startup("exits", Duration::from_secs(2), Duration::from_millis(150));
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("exit 7")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("a failing stand-in daemon is spawned");

        let error = wait_for_socket(&context, &mut child).expect_err("the daemon exits");
        let message = error.to_string();
        assert!(message.contains("exited during startup"), "{message}");
        assert!(
            message.contains("exit status: 7"),
            "reports the status: {message}"
        );
        assert!(message.contains("/opt/service-orchestrator"), "{message}");
        assert!(message.contains("daemon.sock"), "{message}");
    }

    #[test]
    fn a_losing_start_adopts_the_concurrent_winner() {
        // Our own child exits, but another interface's daemon binds the socket
        // within the grace period, so the attempt must succeed rather than
        // report a spurious failure.
        let (context, _root) = startup("race", Duration::from_secs(3), Duration::from_secs(3));
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("exit 1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("a losing stand-in daemon is spawned");

        let socket = context.socket_path.clone();
        let winner = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(75));
            let listener = std::os::unix::net::UnixListener::bind(&socket)
                .expect("the concurrent winner binds the socket");
            // Keep serving until the attempt has connected.
            std::thread::sleep(Duration::from_millis(500));
            drop(listener);
        });

        let stream = wait_for_socket(&context, &mut child).expect("adopts the winner");
        drop(stream);
        winner.join().expect("the winner thread finishes");
    }

    #[test]
    fn the_log_excerpt_is_bounded_to_the_tail() {
        let (context, _root) = startup("tail", Duration::from_secs(1), Duration::from_secs(1));
        let mut body = String::new();
        for index in 0..(LOG_EXCERPT_LINES * 2) {
            body.push_str(&format!("line {index}\n"));
        }
        std::fs::write(&context.log_path, body).expect("the log is written");

        let excerpt = context.excerpt();
        assert!(excerpt.contains(&format!("line {}", LOG_EXCERPT_LINES * 2 - 1)));
        assert!(
            !excerpt.contains("line 0\n"),
            "only the tail is included: {excerpt}"
        );
    }

    #[test]
    fn the_daemon_log_is_created_owner_only() {
        let root = std::env::temp_dir().join(format!("so-client-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the directory is created");
        let log = root.join("daemon.log");
        open_daemon_log(&log).expect("the log file is created");
        let mode = std::fs::metadata(&log)
            .expect("the log exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the daemon log is owner-only");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pre_existing_world_readable_daemon_log_is_tightened() {
        let root = std::env::temp_dir().join(format!("so-client-logfix-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the directory is created");
        let log = root.join("daemon.log");
        std::fs::write(&log, b"old").expect("the log exists");
        std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o644))
            .expect("the log is world-readable");
        open_daemon_log(&log).expect("the log is reopened");
        let mode = std::fs::metadata(&log)
            .expect("the log exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o600,
            "an existing loose log is tightened to owner-only"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
