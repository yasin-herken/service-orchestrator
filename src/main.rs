//! Service Orchestrator binary — the composition root.
//!
//! This crate is intentionally thin. Its only responsibilities are to:
//!
//! 1. parse the command line;
//! 2. for the daemon, load and validate configuration, construct the concrete
//!    infrastructure adapters, register their task executors, build the shared
//!    task engine, and wire the application core over those adapters;
//! 3. for `tui` and `mcp`, connect to the shared local daemon and hand the
//!    interface a client over IPC.
//!
//! No business logic lives here. Every interface reaches the same application
//! core — the TUI and MCP through the daemon, which owns the single task engine
//! and process registry — and every long-running operation flows through that
//! one engine (`tasks/015.md` §4, §5). The interfaces remain strictly optional:
//! the CLI is fully functional without an LLM or an MCP client.
//!
//! See `docs/architecture.md`, `docs/local-daemon.md`, and
//! `docs/adr/015-shared-local-runtime.md`.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use service_orchestrator_application::{Application, ArtifactInspector, InMemoryRuntimeStateStore};
use service_orchestrator_build::{
    register_build_executors, register_library_executors, BuildManager, MavenArtifactInspector,
};
use service_orchestrator_client::{socket_path, ClientKind, OrchestratorClient, RemoteClient};
use service_orchestrator_config::{default_config_file, load_from_path, Config, ConfigError};
use service_orchestrator_daemon::{Daemon, DaemonOptions};
use service_orchestrator_execution::{ExecutorRegistry, TaskEngine};
use service_orchestrator_git::{register_git_executors, CliGitManager};
use service_orchestrator_health::{register_health_executors, HealthManager};
use service_orchestrator_liquibase::{register_migration_executors, MigrationManager};
use service_orchestrator_logging::{spawn_process_ingestor, spawn_task_ingestor, LogHub};
use service_orchestrator_mcp::{RemoteTaskMonitor, TaskMonitor};
use service_orchestrator_process::ProcessManager;
use service_orchestrator_runtime::{register_service_executors, ServiceManager};
use service_orchestrator_tui::{RemoteTaskObserver, TaskObserver};

mod probe;

use probe::SystemWorkspaceProbe;

/// The environment variable that overrides the configuration file location.
const CONFIG_ENV: &str = "SERVICE_ORCHESTRATOR_CONFIG";

/// The command-line usage text.
const USAGE: &str = "\
Usage: service-orchestrator [COMMAND] [OPTIONS]

Commands:
  tui                 Launch the terminal interface (default)
  mcp                 Serve the Model Context Protocol on stdin/stdout
  daemon              Run the shared local runtime in the foreground
  daemon status       Report whether the daemon is running
  daemon stop         Ask a running daemon to shut down

Options:
  -c, --config <path>  Path to the configuration file
                       (default: ~/.service-orchestrator/config.toml,
                        override: SERVICE_ORCHESTRATOR_CONFIG)
  -h, --help           Print this help
  -V, --version        Print the version

The TUI and MCP interfaces share one local daemon over a Unix domain socket.
Starting `tui` or `mcp` connects to a running daemon or starts one if needed;
closing an interface never stops the daemon. The TUI is fully functional
offline; no LLM and no MCP client is required for local operations. When run
with `mcp`, stdout carries MCP protocol messages only.";

/// Which interface the process was asked to launch.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Interface {
    /// The Ratatui terminal interface.
    Tui,
    /// The Model Context Protocol stdio server.
    Mcp,
    /// The shared local runtime daemon.
    Daemon,
}

/// What a `daemon` invocation should do.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DaemonAction {
    /// Run the daemon in the foreground until it shuts down.
    Run,
    /// Report the running daemon's status.
    Status,
    /// Ask a running daemon to shut down.
    Stop,
}

/// What the process was asked to do.
enum Command {
    /// Run the shared local runtime, optionally against an explicit config.
    Daemon {
        /// The daemon action to perform.
        action: DaemonAction,
        /// A configuration path supplied on the command line.
        config: Option<PathBuf>,
    },
    /// Launch the terminal interface, optionally against an explicit config.
    Tui {
        /// A configuration path supplied on the command line.
        config: Option<PathBuf>,
    },
    /// Serve MCP on stdio, optionally against an explicit config.
    Mcp {
        /// A configuration path supplied on the command line.
        config: Option<PathBuf>,
    },
    /// Print usage.
    Help,
    /// Print the version.
    Version,
}

#[tokio::main]
async fn main() -> ExitCode {
    let command = match parse_args(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("service-orchestrator: {message}");
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    match command {
        Command::Help => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Command::Version => {
            println!("service-orchestrator {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Daemon { action, config } => report(run_daemon_command(action, config).await),
        Command::Tui { config } => report(run_tui(config).await),
        Command::Mcp { config } => report(run_mcp(config).await),
    }
}

/// Maps an interface outcome onto a process exit code, reporting the error on
/// stderr.
///
/// Reporting to stderr (never stdout) keeps the MCP stream pure even on failure.
fn report(outcome: Result<(), Box<dyn std::error::Error>>) -> ExitCode {
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("service-orchestrator: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Parses the command line.
///
/// # Errors
///
/// Returns a human-readable message for an unrecognized argument or a missing
/// option value.
fn parse_args(args: impl Iterator<Item = OsString>) -> Result<Command, String> {
    let mut config: Option<PathBuf> = None;
    let mut interface: Option<Interface> = None;
    let mut daemon_action: Option<DaemonAction> = None;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let value = arg.to_string_lossy();
        match value.as_ref() {
            "tui" => interface = Some(Interface::Tui),
            "mcp" => interface = Some(Interface::Mcp),
            "daemon" => {
                interface = Some(Interface::Daemon);
                // An optional `status`/`stop` verb follows `daemon`.
                if let Some(next) = args.peek() {
                    match next.to_string_lossy().as_ref() {
                        "status" => {
                            let _ = args.next();
                            daemon_action = Some(DaemonAction::Status);
                        }
                        "stop" => {
                            let _ = args.next();
                            daemon_action = Some(DaemonAction::Stop);
                        }
                        _ => {}
                    }
                }
            }
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "-c" | "--config" => {
                let path = args
                    .next()
                    .ok_or_else(|| format!("option '{value}' requires a path"))?;
                config = Some(PathBuf::from(path));
            }
            other if other.starts_with("--config=") => {
                config = Some(PathBuf::from(other.trim_start_matches("--config=")));
            }
            other => return Err(format!("unrecognized argument '{other}'")),
        }
    }
    Ok(match interface.unwrap_or(Interface::Tui) {
        Interface::Tui => Command::Tui { config },
        Interface::Mcp => Command::Mcp { config },
        Interface::Daemon => Command::Daemon {
            action: daemon_action.unwrap_or(DaemonAction::Run),
            config,
        },
    })
}

/// Resolves the configuration file path.
///
/// The command line wins, then [`CONFIG_ENV`], then the default location. The
/// daemon loads and validates it; a client only needs the path so it can derive
/// the socket location (`tasks/015.md` §7, §26).
fn resolve_config_path(explicit: Option<PathBuf>) -> PathBuf {
    explicit
        .or_else(|| std::env::var_os(CONFIG_ENV).map(PathBuf::from))
        .unwrap_or_else(default_config_file)
}

/// Resolves, loads, and validates the configuration.
///
/// # Errors
///
/// Returns a [`ConfigError`] when the file cannot be read, parsed, or
/// validated.
fn resolve_config(explicit: Option<PathBuf>) -> Result<(Config, PathBuf), ConfigError> {
    let path = resolve_config_path(explicit);
    let config = load_from_path(&path)?;
    Ok((config, path))
}

/// The wired, running core owned by the daemon.
///
/// It owns the task engine, the process manager, and the log ingestors so the
/// daemon can shut the core down in the correct order.
struct Runtime {
    application: Application,
    engine: Arc<TaskEngine>,
    processes: ProcessManager,
    config_path: PathBuf,
    process_ingestor: tokio::task::JoinHandle<()>,
    task_ingestor: tokio::task::JoinHandle<()>,
}

impl Runtime {
    /// Shuts the core down in the correct order.
    ///
    /// The ingestors are stopped first so no further log/session message is
    /// produced, then the task engine drains, then the process manager reaps
    /// every managed process before the process exits.
    async fn shutdown(self) {
        self.process_ingestor.abort();
        self.task_ingestor.abort();
        self.engine.shutdown().await;
        self.processes.shutdown().await;
    }
}

/// Loads configuration, wires the infrastructure adapters, and builds the task
/// engine and application core.
///
/// The order matters: the task engine must be built inside the Tokio runtime,
/// and the log ingestors must be spawned once it exists. Only the daemon calls
/// this; the interfaces reach this same core over IPC (`AGENTS.md` §1).
///
/// # Errors
///
/// Returns an error when configuration loading, infrastructure construction, or
/// task-engine construction fails.
async fn bootstrap(
    explicit_config: Option<PathBuf>,
) -> Result<Runtime, Box<dyn std::error::Error>> {
    let (config, config_path) = resolve_config(explicit_config)?;
    let config = Arc::new(config);

    // -- infrastructure adapters --------------------------------------------
    let processes = ProcessManager::new()?;
    let runtime_state = Arc::new(InMemoryRuntimeStateStore::new());
    let logs = Arc::new(LogHub::new());

    let git = Arc::new(CliGitManager::new(processes.clone()));
    let builds = Arc::new(BuildManager::new(processes.clone()));
    let artifacts: Arc<dyn ArtifactInspector> =
        Arc::new(MavenArtifactInspector::new(processes.clone()));
    let migrations = Arc::new(MigrationManager::new(
        processes.clone(),
        Arc::clone(&config),
    ));
    let services = Arc::new(ServiceManager::new(
        processes.clone(),
        Arc::clone(&config),
        runtime_state.clone(),
    ));
    let health = Arc::new(HealthManager::new(
        Arc::clone(&config),
        services.clone(),
        runtime_state.clone(),
    ));

    // -- task engine ---------------------------------------------------------
    let mut registry = ExecutorRegistry::new();
    register_git_executors(&mut registry, Arc::clone(&git));
    register_build_executors(&mut registry, Arc::clone(&builds));
    register_library_executors(&mut registry, Arc::clone(&artifacts));
    register_migration_executors(&mut registry, Arc::clone(&migrations));
    register_service_executors(&mut registry, Arc::clone(&services));
    register_health_executors(&mut registry, Arc::clone(&health));

    // The engine's default timeout is the configured `execution` default; it
    // covers tasks that carry no explicit timeout — most importantly workflow
    // steps, which previously had none and could hang a branch forever
    // (`tasks/017.md` §19, defects D3/D4).
    let engine = Arc::new(
        TaskEngine::builder()
            .max_parallel_tasks(config.execution.max_parallel_tasks)
            .default_timeout(Duration::from_secs(
                config.execution.default_timeout_seconds,
            ))
            .registry(registry)
            .build()?,
    );

    // -- observability -------------------------------------------------------
    let process_ingestor = spawn_process_ingestor(Arc::clone(&logs), processes.clone(), None);
    let task_ingestor = spawn_task_ingestor(Arc::clone(&logs), engine.subscribe());

    // -- application core ----------------------------------------------------
    // The daemon itself runs with `Authorization::standard`; the request router
    // rebuilds an authorized view per request so the shared policy engine
    // always runs. A destructive operation is denied unless an interactive
    // client asks for it (`tasks/015.md` §23).
    let application = Application::new(
        (*config).clone(),
        engine.clone(),
        runtime_state.clone(),
        logs.clone(),
    )
    .with_git_manager(git.clone())
    .with_artifact_inspector(Arc::clone(&artifacts))
    .with_service_runtime(services.clone())
    .with_health_index(health.store())
    // The real, read-only environment probe behind `validate_workspace`. The
    // application core never reaches the filesystem, `PATH`, or the network on
    // its own (`tasks/017.md` §7).
    .with_workspace_probe(Arc::new(SystemWorkspaceProbe));

    Ok(Runtime {
        application,
        engine,
        processes,
        config_path,
        process_ingestor,
        task_ingestor,
    })
}

/// Runs a `daemon` subcommand.
///
/// # Errors
///
/// Returns an error when running the daemon fails; the diagnostic `status` and
/// `stop` verbs never fail on a stopped daemon.
async fn run_daemon_command(
    action: DaemonAction,
    config: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        DaemonAction::Run => run_daemon(config).await,
        DaemonAction::Status => {
            report_daemon_status(config);
            Ok(())
        }
        DaemonAction::Stop => stop_daemon(config),
    }
}

/// Boots the core and runs the shared local runtime until it shuts down.
///
/// # Errors
///
/// Returns an error when startup, binding, or serving fails.
async fn run_daemon(explicit_config: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = bootstrap(explicit_config).await?;
    let socket = socket_path(&runtime.config_path);
    let daemon = Daemon::new(
        runtime.application.clone(),
        Arc::clone(&runtime.engine),
        DaemonOptions {
            socket_path: socket,
        },
    );
    let outcome = daemon.run().await;
    runtime.shutdown().await;
    outcome.map_err(Into::into)
}

/// Connects to the shared local runtime and runs the terminal interface.
///
/// The interface never owns the task engine or process registry: it drives the
/// daemon through the client, so state is shared with every other interface
/// (`tasks/015.md` §12, §17).
///
/// # Errors
///
/// Returns an error when the daemon cannot be reached or started, or the
/// terminal interface fails.
async fn run_tui(explicit_config: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let config_path = resolve_config_path(explicit_config);
    let client: Arc<dyn OrchestratorClient> =
        Arc::new(RemoteClient::connect(config_path.clone(), ClientKind::Tui)?);
    let observer: Arc<dyn TaskObserver> = Arc::new(RemoteTaskObserver::new(Arc::clone(&client)));
    let outcome =
        service_orchestrator_tui::run(client, observer, config_path.display().to_string()).await;
    outcome.map_err(Into::into)
}

/// Connects to the shared local runtime and serves MCP on stdin/stdout.
///
/// The MCP adapter is a thin client of the same daemon the TUI uses. Nothing
/// about the core changes for MCP — it is purely a second transport
/// (`AGENTS.md` §1, §21; `tasks/015.md` §13).
///
/// # Errors
///
/// Returns an error when the daemon cannot be reached or started, or the MCP
/// transport fails.
async fn run_mcp(explicit_config: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let config_path = resolve_config_path(explicit_config);
    let client: Arc<dyn OrchestratorClient> =
        Arc::new(RemoteClient::connect(config_path, ClientKind::Mcp)?);
    let monitor: Arc<dyn TaskMonitor> = Arc::new(RemoteTaskMonitor::new(Arc::clone(&client)));
    let outcome = service_orchestrator_mcp::serve_stdio(client, monitor).await;
    outcome.map_err(Into::into)
}

/// Reports whether a daemon is running, without starting one (`tasks/015.md`
/// §30).
///
/// A missing daemon is reported as `stopped`, not an error.
fn report_daemon_status(explicit_config: Option<PathBuf>) {
    let config_path = resolve_config_path(explicit_config);
    let socket = socket_path(&config_path);
    match RemoteClient::connect_existing(&config_path, ClientKind::Cli) {
        Ok(client) => match client.daemon_status() {
            Ok(status) => {
                println!("service-orchestrator daemon: running");
                println!("  pid:              {}", status.pid);
                println!("  endpoint:         {}", status.socket_path);
                println!("  protocol version: {}", status.protocol_version);
                println!("  clients:          {}", status.clients);
                println!("  active tasks:     {}", status.active_tasks);
                println!("  managed services: {}", status.managed_services);
                println!("  started at:       {}", status.started_at_unix);
            }
            Err(error) => {
                println!("service-orchestrator daemon: running but unreachable");
                println!(
                    "  endpoint: {socket_display}",
                    socket_display = socket.display()
                );
                println!("  error:    {error}");
            }
        },
        Err(_) => {
            println!("service-orchestrator daemon: stopped");
            println!("  endpoint: {}", socket.display());
        }
    }
}

/// Asks a running daemon to shut down, reporting a stopped daemon without
/// error (`tasks/015.md` §28, §29).
///
/// # Errors
///
/// Returns an error only if a reachable daemon refuses the shutdown request.
fn stop_daemon(explicit_config: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let config_path = resolve_config_path(explicit_config);
    match RemoteClient::connect_existing(&config_path, ClientKind::Cli) {
        Ok(client) => {
            client.shutdown_daemon()?;
            println!("service-orchestrator daemon: shutdown requested");
        }
        Err(_) => println!("service-orchestrator daemon: not running"),
    }
    Ok(())
}
