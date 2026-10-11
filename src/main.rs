//! Service Orchestrator binary — the composition root.
//!
//! This crate is intentionally thin. Its only responsibilities are to:
//!
//! 1. parse the command line;
//! 2. load and validate configuration;
//! 3. construct the concrete infrastructure adapters;
//! 4. register their task executors and build the shared task engine;
//! 5. wire the application core over those adapters; and
//! 6. hand the application core to an interface adapter (`tui` or `mcp`).
//!
//! No business logic lives here. Every interface calls the same application
//! core, and every long-running operation flows through the same task engine.
//! The two interfaces remain strictly optional: the CLI is fully functional
//! without an LLM or an MCP client, and `mcp` adds a transport without adding
//! behavior. See `docs/architecture.md` and `docs/adr/001-core-architecture.md`.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use service_orchestrator_application::{Application, ArtifactInspector, InMemoryRuntimeStateStore};
use service_orchestrator_build::{
    register_build_executors, register_library_executors, BuildManager, MavenArtifactInspector,
};
use service_orchestrator_config::{default_config_file, load_from_path, Config, ConfigError};
use service_orchestrator_execution::{ExecutorRegistry, TaskEngine};
use service_orchestrator_git::{register_git_executors, CliGitManager};
use service_orchestrator_health::{register_health_executors, HealthManager};
use service_orchestrator_liquibase::{register_migration_executors, MigrationManager};
use service_orchestrator_logging::{spawn_process_ingestor, spawn_task_ingestor, LogHub};
use service_orchestrator_mcp::TaskMonitor;
use service_orchestrator_process::ProcessManager;
use service_orchestrator_runtime::{register_service_executors, ServiceManager};
use service_orchestrator_tui::TaskObserver;

/// The environment variable that overrides the configuration file location.
const CONFIG_ENV: &str = "SERVICE_ORCHESTRATOR_CONFIG";

/// The command-line usage text.
const USAGE: &str = "\
Usage: service-orchestrator [COMMAND] [OPTIONS]

Commands:
  tui                 Launch the terminal interface (default)
  mcp                 Serve the Model Context Protocol on stdin/stdout

Options:
  -c, --config <path>  Path to the configuration file
                       (default: ~/.service-orchestrator/config.toml,
                        override: SERVICE_ORCHESTRATOR_CONFIG)
  -h, --help           Print this help
  -V, --version        Print the version

Both interfaces share the same application core. The TUI is fully functional
offline; no LLM and no MCP client is required for local operations. When run
with `mcp`, stdout carries MCP protocol messages only.";

/// Which interface the process was asked to launch.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Interface {
    /// The Ratatui terminal interface.
    Tui,
    /// The Model Context Protocol stdio server.
    Mcp,
}

/// What the process was asked to do.
enum Command {
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
    let mut interface = Interface::Tui;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let value = arg.to_string_lossy();
        match value.as_ref() {
            "tui" => interface = Interface::Tui,
            "mcp" => interface = Interface::Mcp,
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
    Ok(match interface {
        Interface::Tui => Command::Tui { config },
        Interface::Mcp => Command::Mcp { config },
    })
}

/// Resolves, loads, and validates the configuration.
///
/// The command line wins, then [`CONFIG_ENV`], then the default location.
/// The resolved path is returned alongside the configuration so the interface
/// can display it on the settings screen.
///
/// # Errors
///
/// Returns a [`ConfigError`] when the file cannot be read, parsed, or
/// validated.
fn resolve_config(explicit: Option<PathBuf>) -> Result<(Config, PathBuf), ConfigError> {
    let path = explicit
        .or_else(|| std::env::var_os(CONFIG_ENV).map(PathBuf::from))
        .unwrap_or_else(default_config_file);
    let config = load_from_path(&path)?;
    Ok((config, path))
}

/// The wired, running core shared by every interface.
///
/// It owns the task engine, the process manager, and the log ingestors so an
/// interface can hand the application core to its adapter and then shut the
/// core down in the correct order.
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
/// and the log ingestors must be spawned once it exists. Both interfaces call
/// this so they share identical wiring (`AGENTS.md` §1).
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

    let engine = Arc::new(
        TaskEngine::builder()
            .max_parallel_tasks(config.execution.max_parallel_tasks)
            .registry(registry)
            .build()?,
    );

    // -- observability -------------------------------------------------------
    let process_ingestor = spawn_process_ingestor(Arc::clone(&logs), processes.clone(), None);
    let task_ingestor = spawn_task_ingestor(Arc::clone(&logs), engine.subscribe());

    // -- application core ----------------------------------------------------
    // MCP and the TUI both run with `Authorization::standard`; a destructive
    // operation is denied unless an interactive interface confirms it.
    let application = Application::new(
        (*config).clone(),
        engine.clone(),
        runtime_state.clone(),
        logs.clone(),
    )
    .with_git_manager(git.clone())
    .with_artifact_inspector(Arc::clone(&artifacts))
    .with_service_runtime(services.clone())
    .with_health_index(health.store());

    Ok(Runtime {
        application,
        engine,
        processes,
        config_path,
        process_ingestor,
        task_ingestor,
    })
}

/// Boots the core and runs the terminal interface.
///
/// # Errors
///
/// Returns an error when startup or the terminal interface fails.
async fn run_tui(explicit_config: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = bootstrap(explicit_config).await?;
    let observer: Arc<dyn TaskObserver> = runtime.engine.clone();
    let outcome = service_orchestrator_tui::run(
        runtime.application.clone(),
        observer,
        runtime.config_path.display().to_string(),
    )
    .await;
    runtime.shutdown().await;
    outcome.map_err(Into::into)
}

/// Boots the core and serves the Model Context Protocol on stdin/stdout.
///
/// The application core is the same one the TUI would use: the same task
/// engine, runtime state, and log service. Nothing about the core changes for
/// MCP — it is purely a second transport (`AGENTS.md` §1, §21).
///
/// # Errors
///
/// Returns an error when startup or the MCP transport fails.
async fn run_mcp(explicit_config: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = bootstrap(explicit_config).await?;
    let monitor: Arc<dyn TaskMonitor> = runtime.engine.clone();
    let outcome = service_orchestrator_mcp::serve_stdio(runtime.application.clone(), monitor).await;
    runtime.shutdown().await;
    outcome.map_err(Into::into)
}
