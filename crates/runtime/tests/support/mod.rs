//! Shared fixtures for the runtime adapter integration tests.
//!
//! Everything is local and offline. Services are planned to run tiny local
//! utilities (`/bin/sleep`), so no Java, Node, Maven, or npm is invoked: the
//! configured start command is authoritative, which is exactly the seam
//! runtime-agnostic orchestration relies on (`tasks/010.md` §8).
//!
//! Integration tests in a Cargo crate are separate binaries, so this module is
//! compiled into each of them; not every helper is used by every test file, hence
//! the module-level `dead_code` allowance.

#![allow(dead_code)]
// This module is compiled into each integration-test binary as a private `mod
// support;`, so its helpers are intentionally reachable only from the test
// binary rather than through a public API.
#![allow(unreachable_pub)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use service_orchestrator_application::{
    InMemoryRuntimeStateStore, ResolvedServiceTarget, ServiceRuntimeError,
};
use service_orchestrator_config::{
    Config, ConfiguredWorkspace, DiscoverySettings, ExecutionSettings,
};
use service_orchestrator_domain::{
    AbsolutePath, CommandKind, CommandSet, CommandSpec, Environment, Port, Runtime, RuntimeKind,
    Service, ServiceId, ServiceType, Version, Workspace, WorkspaceId,
};
use service_orchestrator_process::{ProcessManager, ShutdownStrategy};
use service_orchestrator_runtime::{
    FreePortChecker, NullRuntimeInspector, PortChecker, RuntimeInfo, RuntimeInspector,
    RuntimeRegistry, RuntimeType, ServiceManager, ServiceRuntime, StartPlan,
};

/// The workspace id used by every fixture.
pub const WORKSPACE: &str = "acme";
/// The service id used by every fixture.
pub const SERVICE: &str = "auth-service";

/// A tiny long-running process used instead of a real service.
#[cfg(unix)]
pub const SLEEP: &str = "/bin/sleep";

/// A temporary directory removed when the fixture is dropped.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates a fresh, uniquely named temporary directory.
    pub fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "so-runtime-{tag}-{}-{sequence}-{}",
            std::process::id(),
            unique()
        ));
        std::fs::create_dir_all(&path).expect("the temporary directory is created");
        Self { path }
    }

    /// Returns the absolute directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the directory as a domain absolute path.
    pub fn absolute(&self) -> AbsolutePath {
        AbsolutePath::new(self.path.to_string_lossy().into_owned())
            .expect("the temporary path is absolute")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}

fn unique() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

/// A runtime that plans a tiny local process instead of a real service.
///
/// It is registered as [`RuntimeType::Java`] so the manager resolves it for a
/// service that declares a Java runtime; the planned program is irrelevant to
/// the manager, which is the point of the abstraction.
pub struct SleepRuntime {
    /// How many seconds the process should run.
    pub seconds: u32,
}

impl SleepRuntime {
    /// Creates a runtime that plans a long `sleep`.
    pub fn new() -> Self {
        Self { seconds: 30 }
    }
}

impl Default for SleepRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceRuntime for SleepRuntime {
    fn runtime_type(&self) -> RuntimeType {
        RuntimeType::Java
    }

    fn validate(&self, _service: &Service) -> Result<(), ServiceRuntimeError> {
        Ok(())
    }

    fn prepare_start(
        &self,
        _service: &Service,
        working_dir: &AbsolutePath,
        environment: Environment,
    ) -> Result<StartPlan, ServiceRuntimeError> {
        Ok(StartPlan {
            runtime: RuntimeType::Java,
            program: SLEEP.to_owned(),
            args: vec![self.seconds.to_string()],
            working_dir: working_dir.clone(),
            environment,
            shutdown: ShutdownStrategy::graceful(),
        })
    }
}

/// A registry containing only the [`SleepRuntime`].
pub fn sleep_registry() -> RuntimeRegistry {
    let mut registry = RuntimeRegistry::new();
    registry.register(Arc::new(SleepRuntime::new()));
    registry
}

/// A runtime that plans an arbitrary local program with structured arguments.
///
/// Used to drive short-lived processes (for example a command that exits with a
/// specific code) through the same orchestration as a real service.
pub struct ScriptRuntime {
    /// The executable to plan.
    pub program: String,
    /// The structured argument vector.
    pub args: Vec<String>,
}

impl ScriptRuntime {
    /// Creates a script runtime.
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }
}

impl ServiceRuntime for ScriptRuntime {
    fn runtime_type(&self) -> RuntimeType {
        RuntimeType::Java
    }

    fn validate(&self, _service: &Service) -> Result<(), ServiceRuntimeError> {
        Ok(())
    }

    fn prepare_start(
        &self,
        _service: &Service,
        working_dir: &AbsolutePath,
        environment: Environment,
    ) -> Result<StartPlan, ServiceRuntimeError> {
        Ok(StartPlan {
            runtime: RuntimeType::Java,
            program: self.program.clone(),
            args: self.args.clone(),
            working_dir: working_dir.clone(),
            environment,
            shutdown: ShutdownStrategy::graceful(),
        })
    }
}

/// A registry containing only the given [`ScriptRuntime`].
pub fn script_registry(runtime: ScriptRuntime) -> RuntimeRegistry {
    let mut registry = RuntimeRegistry::new();
    registry.register(Arc::new(runtime));
    registry
}

/// A port checker that reports exactly one port as busy.
pub struct BusyPort(pub u16);

impl PortChecker for BusyPort {
    fn is_in_use(&self, port: u16) -> bool {
        port == self.0
    }
}

/// An inspector that always reports a fixed installed version.
pub struct FixedInspector(pub Option<Version>);

#[async_trait]
impl RuntimeInspector for FixedInspector {
    async fn inspect(&self, _runtime: RuntimeType) -> Result<RuntimeInfo, ServiceRuntimeError> {
        Ok(RuntimeInfo {
            version: self.0.clone(),
            executable: None,
        })
    }
}

/// A configured workspace, its validated configuration, and a start target.
pub struct Fixture {
    /// The temporary workspace root; kept alive for the test's duration.
    pub temp: TempDir,
    /// The validated configuration handed to the manager.
    pub config: Arc<Config>,
    /// The lifecycle target for the fixture service.
    pub target: ResolvedServiceTarget,
}

/// Builds a fixture with the given ports and optional runtime requirement.
pub fn fixture(ports: &[u16], runtime: Option<Runtime>) -> Fixture {
    let temp = TempDir::new("ws");
    let root = temp.absolute();

    let mut service = Service::new(
        ServiceId::new(SERVICE).unwrap(),
        "Auth service",
        ServiceType::Backend,
    )
    .unwrap();
    let mut commands = CommandSet::new();
    commands.set(
        CommandKind::Start,
        CommandSpec::new("./mvnw", ["spring-boot:run"]).unwrap(),
    );
    service.commands = commands;
    service.runtimes = runtime.into_iter().collect();
    service.ports = ports
        .iter()
        .filter_map(|port| Port::new(*port).ok())
        .collect();

    let workspace = Workspace::new(WorkspaceId::new(WORKSPACE).unwrap(), "Acme", root.clone())
        .unwrap()
        .with_service(service);

    let config = Config {
        version: 1,
        execution: ExecutionSettings::default(),
        discovery: DiscoverySettings::default(),
        templates: Vec::new(),
        migration_presets: Vec::new(),
        operation_presets: Vec::new(),
        workspaces: vec![ConfiguredWorkspace {
            workspace,
            environment: Environment::new(),
            liquibase: BTreeMap::new(),
        }],
    };

    let target = ResolvedServiceTarget {
        workspace_id: WorkspaceId::new(WORKSPACE).unwrap(),
        service_id: ServiceId::new(SERVICE).unwrap(),
        path: Some(root.clone()),
        workspace_root: root,
    };

    Fixture {
        temp,
        config: Arc::new(config),
        target,
    }
}

/// Builds a manager with an explicit registry, port checker, and inspector.
pub fn manager_with(
    config: Arc<Config>,
    runtimes: RuntimeRegistry,
    ports: Arc<dyn PortChecker>,
    inspector: Arc<dyn RuntimeInspector>,
) -> ServiceManager {
    ServiceManager::new(
        ProcessManager::new().expect("the process manager builds inside a Tokio runtime"),
        config,
        Arc::new(InMemoryRuntimeStateStore::new()),
    )
    .with_runtimes(Arc::new(runtimes))
    .with_port_checker(ports)
    .with_inspector(inspector)
}

/// Builds a manager using the [`SleepRuntime`], free ports, and no inspector.
pub fn manager(config: Arc<Config>) -> ServiceManager {
    manager_with(
        config,
        sleep_registry(),
        Arc::new(FreePortChecker),
        Arc::new(NullRuntimeInspector),
    )
}

/// Builds a manager that shares its runtime-state store with the caller, so a
/// test can seed the pre-task transition state an application records and
/// observe how a failed transition is reconciled (`tasks/017.md` defect D1).
pub fn manager_with_state(
    config: Arc<Config>,
    runtimes: RuntimeRegistry,
    ports: Arc<dyn PortChecker>,
    inspector: Arc<dyn RuntimeInspector>,
    state: Arc<InMemoryRuntimeStateStore>,
) -> ServiceManager {
    ServiceManager::new(
        ProcessManager::new().expect("the process manager builds inside a Tokio runtime"),
        config,
        state,
    )
    .with_runtimes(Arc::new(runtimes))
    .with_port_checker(ports)
    .with_inspector(inspector)
}

/// Convenience accessor for the fixture's service id.
pub fn service_id() -> ServiceId {
    ServiceId::new(SERVICE).unwrap()
}

/// Convenience accessor for a Java runtime requirement.
pub fn java_runtime() -> Runtime {
    Runtime::any(RuntimeKind::Java)
}
