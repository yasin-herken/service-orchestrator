//! The Service Manager.
//!
//! [`ServiceManager`] is the infrastructure coordinator for local service
//! lifecycle. It owns no business rules and no interface concerns: given a
//! [`ResolvedServiceTarget`] it resolves the service's runtime, validates the
//! environment, checks ports, prepares a structured start plan, and asks the
//! process manager to spawn it — then keeps owning the resulting process so it
//! outlives the task that started it (`tasks/010.md` §14, §15, §19).
//!
//! # Ownership and lifetime
//!
//! A start task is short; the process it starts is not. The manager therefore
//! records every process it spawns in its own registry *synchronously*, before
//! the first `await`, and spawns a small exit watcher that keeps the observed
//! state and the runtime event stream up to date. Cancelling a start that has not
//! yet spawned leaves nothing behind; cancelling one that has spawned leaves a
//! tracked process, never an orphan (`tasks/010.md` §26).
//!
//! # Concurrency
//!
//! Operations on the same service serialize behind a per-service lock
//! ([`ServiceCoordinator`]); operations on different services run concurrently.
//! All process work goes through the process manager, which is the only component
//! allowed to touch the operating system's process API (`AGENTS.md` §15).

use std::sync::Arc;

use service_orchestrator_application::{
    ResolvedServiceTarget, RuntimeStateStore, ServiceProcessInfo, ServiceRuntimeError,
    ServiceRuntimeIndex,
};
use service_orchestrator_config::{Config, ConfiguredWorkspace};
use service_orchestrator_domain::{
    AbsolutePath, CommandKind, Environment, ProcessState, Service, ServiceId, Timestamp,
    WorkspaceId,
};
use service_orchestrator_execution::Cancellation;
use service_orchestrator_process::{ExitReason, ProcessManager, ProcessSpec};

use crate::coordinator::ServiceCoordinator;
use crate::events::{RuntimeEvent, RuntimeEventBus, RuntimeEventStream};
use crate::managed::{ManagedProcessRegistry, ManagedService};
use crate::port_check::{PortChecker, SystemPortChecker};
use crate::registry::RuntimeRegistry;
use crate::runtime::{
    major_version, NullRuntimeInspector, RuntimeHost, RuntimeInspector, RuntimeType,
    SystemRuntimeHost,
};

/// The result of a successful start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartOutcome {
    /// The owning workspace.
    pub workspace_id: WorkspaceId,
    /// The service that was started.
    pub service_id: ServiceId,
    /// The managed process identifier.
    pub process_id: String,
    /// The operating-system process id, when known.
    pub pid: Option<u32>,
    /// The runtime that started it.
    pub runtime: RuntimeType,
    /// The redacted startup command.
    pub command: String,
    /// When the process started.
    pub started_at: Option<Timestamp>,
    /// Whether the service was already running and no new process was spawned.
    pub already_running: bool,
}

/// The result of a stop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopOutcome {
    /// The owning workspace.
    pub workspace_id: WorkspaceId,
    /// The service that was stopped.
    pub service_id: ServiceId,
    /// The managed process identifier, when one existed.
    pub process_id: Option<String>,
    /// The operating-system process id, when known.
    pub pid: Option<u32>,
    /// Whether nothing was running and no stop was needed.
    pub already_stopped: bool,
    /// The exit code, when the platform reported one.
    pub exit_code: Option<i32>,
    /// How the execution ended.
    pub exit_reason: Option<String>,
}

/// The result of a restart (a stop followed by a start).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestartOutcome {
    /// The owning workspace.
    pub workspace_id: WorkspaceId,
    /// The service that was restarted.
    pub service_id: ServiceId,
    /// The stop half of the restart.
    pub stopped: StopOutcome,
    /// The start half of the restart.
    pub started: StartOutcome,
}

/// Coordinates local service lifecycle over the process manager.
pub struct ServiceManager {
    processes: ProcessManager,
    config: Arc<Config>,
    runtime_state: Arc<dyn RuntimeStateStore>,
    host: Arc<dyn RuntimeHost>,
    runtimes: Arc<RuntimeRegistry>,
    inspector: Arc<dyn RuntimeInspector>,
    ports: Arc<dyn PortChecker>,
    managed: ManagedProcessRegistry,
    events: RuntimeEventBus,
    coordinator: ServiceCoordinator,
    check_ports: bool,
}

impl ServiceManager {
    /// Creates a manager over a process manager and validated configuration.
    ///
    /// The built-in Java and Node runtimes are registered; the runtime inspector
    /// is a [`NullRuntimeInspector`] (no executable/version probing), port
    /// checking uses the real [`SystemPortChecker`], and the environment host is
    /// the real [`SystemRuntimeHost`]. Call the `with_*` builders to override
    /// any of these.
    #[must_use]
    pub fn new(
        processes: ProcessManager,
        config: Arc<Config>,
        runtime_state: Arc<dyn RuntimeStateStore>,
    ) -> Self {
        Self {
            processes,
            config,
            runtime_state,
            host: Arc::new(SystemRuntimeHost),
            runtimes: Arc::new(RuntimeRegistry::standard()),
            inspector: Arc::new(NullRuntimeInspector),
            ports: Arc::new(SystemPortChecker::new()),
            managed: ManagedProcessRegistry::default(),
            events: RuntimeEventBus::new(256),
            coordinator: ServiceCoordinator::new(),
            check_ports: true,
        }
    }

    /// Replaces the runtime registry.
    #[must_use]
    pub fn with_runtimes(mut self, runtimes: Arc<RuntimeRegistry>) -> Self {
        self.runtimes = runtimes;
        self
    }

    /// Replaces the runtime inspector.
    #[must_use]
    pub fn with_inspector(mut self, inspector: Arc<dyn RuntimeInspector>) -> Self {
        self.inspector = inspector;
        self
    }

    /// Replaces the port checker.
    #[must_use]
    pub fn with_port_checker(mut self, ports: Arc<dyn PortChecker>) -> Self {
        self.ports = ports;
        self
    }

    /// Replaces the environment host.
    #[must_use]
    pub fn with_host(mut self, host: Arc<dyn RuntimeHost>) -> Self {
        self.host = host;
        self
    }

    /// Enables or disables port-conflict checking before a start.
    #[must_use]
    pub fn with_port_checking(mut self, enabled: bool) -> Self {
        self.check_ports = enabled;
        self
    }

    /// Returns the process manager, for observability.
    #[must_use]
    pub fn processes(&self) -> &ProcessManager {
        &self.processes
    }

    /// Returns the runtime registry, for observability.
    #[must_use]
    pub fn runtimes(&self) -> &Arc<RuntimeRegistry> {
        &self.runtimes
    }

    /// Subscribes to runtime lifecycle events.
    #[must_use]
    pub fn subscribe(&self) -> RuntimeEventStream {
        self.events.subscribe()
    }

    /// Starts a service, serializing with other operations on the same service.
    ///
    /// # Errors
    ///
    /// Returns a structured [`ServiceRuntimeError`] for every failure mode:
    /// unknown workspace or service, unsupported or unavailable runtime, version
    /// mismatch, missing start command, invalid working directory, port conflict,
    /// a conflicting in-flight operation, a spawn failure, or cancellation.
    pub async fn start(
        &self,
        target: &ResolvedServiceTarget,
        cancellation: Option<Cancellation>,
    ) -> Result<StartOutcome, ServiceRuntimeError> {
        let _guard = self.coordinator.acquire(&target.service_id).await;
        let result = self.start_locked(target, cancellation.as_ref()).await;
        if let Err(error) = &result {
            self.reconcile_failed_transition(&target.service_id, error);
        }
        result
    }

    /// Stops a service, serializing with other operations on the same service.
    ///
    /// Stopping a service that is not running is a successful no-op
    /// (`already_stopped`).
    ///
    /// # Errors
    ///
    /// Returns [`ServiceRuntimeError::ServiceNotFound`] if the service is not
    /// configured, or [`ServiceRuntimeError::ProcessStopFailed`] if the process
    /// manager refuses the stop.
    pub async fn stop(&self, service_id: &ServiceId) -> Result<StopOutcome, ServiceRuntimeError> {
        let _guard = self.coordinator.acquire(service_id).await;
        let result = self.stop_locked(service_id).await;
        if let Err(error) = &result {
            self.reconcile_failed_transition(service_id, error);
        }
        result
    }

    /// Restarts a service: a stop followed by a start, under one lock.
    ///
    /// The process is awaited to a terminal state before the replacement starts,
    /// so two instances never overlap (`tasks/010.md` §31).
    ///
    /// # Errors
    ///
    /// Returns any error from the stop or start half.
    pub async fn restart(
        &self,
        target: &ResolvedServiceTarget,
        cancellation: Option<Cancellation>,
    ) -> Result<RestartOutcome, ServiceRuntimeError> {
        let _guard = self.coordinator.acquire(&target.service_id).await;
        let result = async {
            let stopped = self.stop_locked(&target.service_id).await?;
            let started = self.start_locked(target, cancellation.as_ref()).await?;
            Ok::<RestartOutcome, ServiceRuntimeError>(RestartOutcome {
                workspace_id: target.workspace_id.clone(),
                service_id: target.service_id.clone(),
                stopped,
                started,
            })
        }
        .await;
        if let Err(error) = &result {
            self.reconcile_failed_transition(&target.service_id, error);
        }
        result
    }

    /// Reconciles the observed runtime state after a failed lifecycle
    /// transition.
    ///
    /// The application marks a service `Starting`/`Stopping` *before* it
    /// submits the task, so every failure path inside the manager must clear
    /// that transition state — otherwise the service is reported as
    /// transitioning forever and every future start is refused (`tasks/017.md`
    /// §15, defect D1). Errors that belong to a *different* in-flight
    /// operation or to an unknown service are left untouched: clearing the
    /// state there would clobber a legitimate transition owned elsewhere.
    fn reconcile_failed_transition(&self, service_id: &ServiceId, error: &ServiceRuntimeError) {
        match error {
            ServiceRuntimeError::ServiceAlreadyStarting { .. }
            | ServiceRuntimeError::ServiceNotFound { .. }
            | ServiceRuntimeError::WorkspaceNotFound { .. } => return,
            _ => {}
        }
        let current = self
            .runtime_state
            .service_state(service_id)
            .unwrap_or_default();
        let next = match current.process {
            ProcessState::Starting | ProcessState::Stopping => {
                // A live managed process means the stop half failed and the
                // process is (probably) still alive; otherwise the transition
                // failed outright and the honest state is `Failed`.
                let still_alive = self
                    .managed
                    .get(service_id)
                    .is_some_and(|entry| entry.state() == ProcessState::Running);
                if still_alive {
                    ProcessState::Running
                } else {
                    ProcessState::Failed
                }
            }
            _ => return,
        };
        self.runtime_state
            .set_service_state(service_id, current.with_process(next));
    }

    /// Returns the configured workspace that owns a service, if any.
    fn workspace_of(&self, service_id: &ServiceId) -> Option<&ConfiguredWorkspace> {
        self.config.workspace_of_service(service_id)
    }

    /// Starts a service assuming the per-service lock is already held.
    async fn start_locked(
        &self,
        target: &ResolvedServiceTarget,
        cancellation: Option<&Cancellation>,
    ) -> Result<StartOutcome, ServiceRuntimeError> {
        let service_id = &target.service_id;

        let path =
            target
                .path
                .as_ref()
                .ok_or_else(|| ServiceRuntimeError::WorkingDirectoryInvalid {
                    service: service_id.clone(),
                    path: String::new(),
                    message: "the service has no configured working directory".to_owned(),
                })?;
        if !target.is_inside_workspace() {
            return Err(ServiceRuntimeError::WorkingDirectoryInvalid {
                service: service_id.clone(),
                path: path.to_string(),
                message: format!(
                    "the working directory escapes the workspace root '{}'",
                    target.workspace_root
                ),
            });
        }

        let workspace = self.config.workspace(&target.workspace_id).ok_or_else(|| {
            ServiceRuntimeError::WorkspaceNotFound {
                workspace: target.workspace_id.clone(),
            }
        })?;
        let service = workspace.workspace.service(service_id).ok_or_else(|| {
            ServiceRuntimeError::ServiceNotFound {
                service: service_id.clone(),
            }
        })?;

        // A live process is either already running (a no-op start) or in a
        // conflicting transition.
        if let Some(existing) = self.managed.get(service_id) {
            match existing.state() {
                ProcessState::Running => {
                    let info = existing.info();
                    return Ok(StartOutcome {
                        workspace_id: info.workspace_id,
                        service_id: service_id.clone(),
                        process_id: info.process_id.unwrap_or_default(),
                        pid: info.pid,
                        runtime: existing.runtime,
                        command: info.command.unwrap_or_default(),
                        started_at: info.started_at,
                        already_running: true,
                    });
                }
                ProcessState::Starting | ProcessState::Stopping => {
                    return Err(ServiceRuntimeError::ServiceAlreadyStarting {
                        service: service_id.clone(),
                    });
                }
                ProcessState::NotRunning
                | ProcessState::Stopped
                | ProcessState::Failed
                | ProcessState::Unknown => {}
            }
        }

        let runtime = self.runtimes.resolve(service).ok_or_else(|| {
            ServiceRuntimeError::UnsupportedRuntime {
                service: service_id.clone(),
                runtime: service
                    .runtimes
                    .first()
                    .map(|runtime| runtime.kind.to_string()),
            }
        })?;
        runtime.validate(service)?;

        let declared = runtime.runtime_type().declared_version(service);
        let info = self
            .inspector
            .inspect(runtime.runtime_type())
            .await
            .map_err(|error| remap_inspect_error(error, service_id, runtime.runtime_type()))?;
        check_version(
            service_id,
            runtime.runtime_type(),
            declared.as_ref(),
            info.version.as_ref(),
        )?;

        if self.check_ports {
            for port in &service.ports {
                if self.ports.is_in_use(port.value()) {
                    return Err(ServiceRuntimeError::PortAlreadyInUse {
                        service: service_id.clone(),
                        port: port.value(),
                    });
                }
            }
        }

        if cancellation.is_some_and(Cancellation::is_cancelled) {
            return Err(ServiceRuntimeError::OperationCancelled {
                service: service_id.clone(),
            });
        }

        let working_dir = self.resolve_working_dir(workspace, service, path)?;
        let environment = self.merged_environment(workspace, service);
        let plan = runtime.prepare_start(service, &working_dir, environment)?;

        self.events.publish(RuntimeEvent::ServiceStarting {
            workspace_id: workspace.workspace.id.clone(),
            service_id: service_id.clone(),
            runtime: plan.runtime.label().to_owned(),
            command: plan.command_identity(),
        });

        let spec = build_spec(&plan, service_id)?;
        let handle = self.processes.spawn(spec).map_err(|error| {
            ServiceRuntimeError::ProcessStartFailed {
                service: service_id.clone(),
                message: error.to_string(),
            }
        })?;

        let record = ManagedService {
            workspace_id: workspace.workspace.id.clone(),
            service_id: service_id.clone(),
            process_id: handle.id().clone(),
            runtime: plan.runtime,
            command: plan.command_identity(),
            handle: handle.clone(),
        };
        // Record synchronously, before any await, so a cancellation cannot orphan
        // the process.
        let pid = handle.pid();
        let started_at = handle.snapshot().started_at;
        self.managed.insert(record.clone());

        let current = self
            .runtime_state
            .service_state(service_id)
            .unwrap_or_default();
        self.runtime_state
            .set_service_state(service_id, current.with_process(ProcessState::Running));

        self.events.publish(RuntimeEvent::ServiceStarted {
            workspace_id: record.workspace_id.clone(),
            service_id: service_id.clone(),
            process_id: record.process_id.as_str().to_owned(),
            pid,
            runtime: record.runtime.label().to_owned(),
        });

        self.spawn_exit_watcher(record);

        Ok(StartOutcome {
            workspace_id: workspace.workspace.id.clone(),
            service_id: service_id.clone(),
            process_id: handle.id().as_str().to_owned(),
            pid,
            runtime: plan.runtime,
            command: plan.command_identity(),
            started_at,
            already_running: false,
        })
    }

    /// Stops a service assuming the per-service lock is already held.
    async fn stop_locked(
        &self,
        service_id: &ServiceId,
    ) -> Result<StopOutcome, ServiceRuntimeError> {
        let workspace =
            self.workspace_of(service_id)
                .ok_or_else(|| ServiceRuntimeError::ServiceNotFound {
                    service: service_id.clone(),
                })?;
        let workspace_id = workspace.workspace.id.clone();

        let Some(record) = self.managed.get(service_id) else {
            // The application marked the service `Stopping` before scheduling;
            // clear that transition state so the service is not stuck
            // (`tasks/017.md` §15, defect D1). The process is not ours, so the
            // honest observed state is `Unknown` — not `Stopped`.
            let current = self
                .runtime_state
                .service_state(service_id)
                .unwrap_or_default();
            if matches!(current.process, ProcessState::Stopping) {
                self.runtime_state
                    .set_service_state(service_id, current.with_process(ProcessState::Unknown));
            }
            return Ok(StopOutcome {
                workspace_id,
                service_id: service_id.clone(),
                process_id: None,
                pid: None,
                already_stopped: true,
                exit_code: None,
                exit_reason: None,
            });
        };

        if record.handle.is_terminal() {
            let snapshot = record.handle.snapshot();
            self.managed.remove(service_id);
            let current = self
                .runtime_state
                .service_state(service_id)
                .unwrap_or_default();
            self.runtime_state
                .set_service_state(service_id, current.with_process(snapshot.domain_state()));
            return Ok(StopOutcome {
                workspace_id,
                service_id: service_id.clone(),
                process_id: Some(record.process_id.as_str().to_owned()),
                pid: snapshot.pid,
                already_stopped: true,
                exit_code: snapshot.exit.as_ref().and_then(|exit| exit.code),
                exit_reason: snapshot
                    .exit
                    .as_ref()
                    .map(|exit| exit.reason.label().to_owned()),
            });
        }

        self.events.publish(RuntimeEvent::ServiceStopping {
            workspace_id: workspace_id.clone(),
            service_id: service_id.clone(),
            process_id: record.process_id.as_str().to_owned(),
        });

        record
            .handle
            .stop()
            .map_err(|error| ServiceRuntimeError::ProcessStopFailed {
                service: service_id.clone(),
                message: error.to_string(),
            })?;
        let snapshot =
            record
                .handle
                .wait()
                .await
                .map_err(|error| ServiceRuntimeError::ProcessStopFailed {
                    service: service_id.clone(),
                    message: error.to_string(),
                })?;

        let current = self
            .runtime_state
            .service_state(service_id)
            .unwrap_or_default();
        self.runtime_state
            .set_service_state(service_id, current.with_process(snapshot.domain_state()));

        self.managed.remove(service_id);

        Ok(StopOutcome {
            workspace_id,
            service_id: service_id.clone(),
            process_id: Some(record.process_id.as_str().to_owned()),
            pid: snapshot.pid,
            already_stopped: false,
            exit_code: snapshot.exit.as_ref().and_then(|exit| exit.code),
            exit_reason: snapshot
                .exit
                .as_ref()
                .map(|exit| exit.reason.label().to_owned()),
        })
    }

    /// Resolves the absolute working directory for a service start.
    ///
    /// A command-level relative working directory wins over the service
    /// directory; the caller has already validated containment. The directory
    /// must exist, or the start is rejected before a process is spawned
    /// (`tasks/010.md` §9).
    fn resolve_working_dir(
        &self,
        workspace: &ConfiguredWorkspace,
        service: &Service,
        target_path: &AbsolutePath,
    ) -> Result<AbsolutePath, ServiceRuntimeError> {
        let directory = match service
            .commands
            .get(&CommandKind::Start)
            .and_then(|command| command.working_dir.as_ref())
        {
            Some(relative) => AbsolutePath::new(format!(
                "{}/{}",
                workspace.workspace.root.as_str().trim_end_matches('/'),
                relative.as_str()
            ))
            .map_err(|error| ServiceRuntimeError::WorkingDirectoryInvalid {
                service: service.id.clone(),
                path: relative.to_string(),
                message: error.to_string(),
            })?,
            None => target_path.clone(),
        };
        if !self.host.directory_exists(&directory) {
            return Err(ServiceRuntimeError::WorkingDirectoryInvalid {
                service: service.id.clone(),
                path: directory.to_string(),
                message: "the working directory does not exist".to_owned(),
            });
        }
        Ok(directory)
    }

    /// Merges workspace, service, and start-command environment.
    ///
    /// Later layers override earlier ones. Secret values remain redacted
    /// whenever the environment is displayed or serialized (`AGENTS.md` §31).
    fn merged_environment(
        &self,
        workspace: &ConfiguredWorkspace,
        service: &Service,
    ) -> Environment {
        let mut environment = workspace.environment.clone();
        for variable in service.environment.iter() {
            let _ = environment.set(variable.clone());
        }
        if let Some(command) = service.commands.get(&CommandKind::Start) {
            for variable in command.environment.iter() {
                let _ = environment.set(variable.clone());
            }
        }
        environment
    }

    /// Spawns a detached watcher that keeps observed state and events current.
    fn spawn_exit_watcher(&self, record: ManagedService) {
        let handle = record.handle;
        let workspace_id = record.workspace_id;
        let service_id = record.service_id;
        let process_id = record.process_id.as_str().to_owned();
        let runtime_state = Arc::clone(&self.runtime_state);
        let events = self.events.clone();
        tokio::spawn(async move {
            let Ok(snapshot) = handle.wait().await else {
                return;
            };
            let current = runtime_state.service_state(&service_id).unwrap_or_default();
            runtime_state
                .set_service_state(&service_id, current.with_process(snapshot.domain_state()));
            let exit_code = snapshot.exit.as_ref().and_then(|exit| exit.code);
            let reason = snapshot.exit.as_ref().map(|exit| exit.reason);
            let exit_reason = reason
                .map(|reason| reason.label().to_owned())
                .unwrap_or_else(|| "unknown".to_owned());
            let event = match reason {
                Some(ExitReason::Terminated | ExitReason::Cancelled | ExitReason::Killed) => {
                    RuntimeEvent::ServiceStopped {
                        workspace_id,
                        service_id,
                        process_id,
                        exit_code,
                        exit_reason,
                    }
                }
                Some(ExitReason::Failed | ExitReason::SpawnFailed) => RuntimeEvent::ServiceFailed {
                    workspace_id,
                    service_id,
                    process_id: Some(process_id),
                    message: format!("process exited ({exit_reason})"),
                },
                _ => RuntimeEvent::ServiceExited {
                    workspace_id,
                    service_id,
                    process_id,
                    exit_code,
                    exit_reason,
                },
            };
            events.publish(event);
        });
    }
}

impl ServiceRuntimeIndex for ServiceManager {
    fn runtime_state(&self, service_id: &ServiceId) -> Option<ServiceProcessInfo> {
        self.managed.info(service_id)
    }

    fn managed_services(&self) -> Vec<ServiceProcessInfo> {
        self.managed.infos()
    }
}

impl std::fmt::Debug for ServiceManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceManager")
            .field("runtimes", &self.runtimes.types())
            .field("check_ports", &self.check_ports)
            .finish_non_exhaustive()
    }
}

/// Builds a structured [`ProcessSpec`] from a start plan.
fn build_spec(
    plan: &crate::runtime::StartPlan,
    service_id: &ServiceId,
) -> Result<ProcessSpec, ServiceRuntimeError> {
    let spec = ProcessSpec::new(plan.program.clone())
        .map_err(|error| ServiceRuntimeError::Internal {
            message: error.to_string(),
        })?
        .args(plan.args.clone())
        .working_dir(plan.working_dir.as_str())
        .with_environment(plan.environment.clone())
        .with_shutdown(plan.shutdown)
        .for_service(service_id.clone());
    spec.validate()
        .map_err(|error| ServiceRuntimeError::Internal {
            message: error.to_string(),
        })?;
    Ok(spec)
}

/// Verifies the installed runtime major version matches the pinned one.
fn check_version(
    service_id: &ServiceId,
    runtime: RuntimeType,
    expected: Option<&service_orchestrator_domain::Version>,
    found: Option<&service_orchestrator_domain::Version>,
) -> Result<(), ServiceRuntimeError> {
    let (Some(expected), Some(found)) = (expected, found) else {
        return Ok(());
    };
    let (Some(expected_major), Some(found_major)) = (
        major_version(expected.as_str()),
        major_version(found.as_str()),
    ) else {
        return Ok(());
    };
    if expected_major == found_major {
        return Ok(());
    }
    Err(ServiceRuntimeError::RuntimeVersionMismatch {
        service: service_id.clone(),
        runtime: runtime.label().to_owned(),
        expected: expected.as_str().to_owned(),
        found: found.as_str().to_owned(),
    })
}

/// Rewrites a probe's placeholder service id with the real one.
fn remap_inspect_error(
    error: ServiceRuntimeError,
    service_id: &ServiceId,
    runtime: RuntimeType,
) -> ServiceRuntimeError {
    match error {
        ServiceRuntimeError::RuntimeNotFound { message, .. } => {
            ServiceRuntimeError::RuntimeNotFound {
                service: service_id.clone(),
                runtime: runtime.label().to_owned(),
                message,
            }
        }
        other => other,
    }
}
