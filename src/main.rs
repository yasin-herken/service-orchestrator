//! Service Orchestrator binary — the composition root.
//!
//! This crate is intentionally thin. Its only responsibilities are to:
//!
//! 1. parse the command line;
//! 2. load and validate configuration;
//! 3. construct the concrete infrastructure adapters;
//! 4. register their task executors and build the shared task engine;
//! 5. wire the application core over those adapters; and
//! 6. hand the application core to an interface adapter (`tui`, and later
//!    `mcp` / `doctor`).
//!
//! No business logic lives here. Every interface calls the same application
//! core, and every long-running operation flows through the same task engine.
//! See `docs/architecture.md` and `docs/adr/001-core-architecture.md`.

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

Options:
  -c, --config <path>  Path to the configuration file
                       (default: ~/.service-orchestrator/config.toml,
                        override: SERVICE_ORCHESTRATOR_CONFIG)
  -h, --help           Print this help
  -V, --version        Print the version

The interface is fully functional offline. No LLM and no MCP client is
required for local operations.";

/// What the process was asked to do.
enum Command {
    /// Launch the terminal interface, optionally against an explicit config.
    Tui {
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
        Command::Tui { config } => match run_tui(config).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("service-orchestrator: {error}");
                ExitCode::FAILURE
            }
        },
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
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let value = arg.to_string_lossy();
        match value.as_ref() {
            "tui" => {}
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
    Ok(Command::Tui { config })
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

/// Loads configuration, wires the infrastructure adapters, builds the task
/// engine and application core, and runs the terminal interface.
///
/// The order matters: the task engine must be built inside the Tokio runtime,
/// the log ingestors must be spawned once it exists, and the process manager is
/// shut down last so every managed process is reaped before the process exits.
///
/// # Errors
///
/// Returns an error when configuration loading, infrastructure construction, or
/// the terminal interface fails.
async fn run_tui(explicit_config: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
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

    // -- interface -----------------------------------------------------------
    let observer: Arc<dyn TaskObserver> = engine.clone();
    let outcome =
        service_orchestrator_tui::run(application, observer, config_path.display().to_string())
            .await;

    // -- orderly shutdown ----------------------------------------------------
    process_ingestor.abort();
    task_ingestor.abort();
    engine.shutdown().await;
    processes.shutdown().await;

    outcome.map_err(Into::into)
}
