//! The `ServiceRuntime` abstraction.
//!
//! A [`ServiceRuntime`] knows *how* one kind of service starts. It validates its
//! own preconditions and turns a validated service into a [`StartPlan`]. It
//! deliberately does **not** spawn, monitor, or signal a process: the process
//! manager owns execution, and the task engine owns scheduling (`tasks/010.md`
//! §3, §5).
//!
//! # Why an abstraction at all
//!
//! Java and Node are the current stacks, but the domain and the application name
//! runtimes neutrally. A runtime is the single place where runtime-specific
//! knowledge lives, so adding Python, Go, Docker, or Kubernetes is a new runtime
//! and a registry entry — not a change to orchestration, the task engine, the
//! process manager, or the interfaces (`AGENTS.md` §11`).
//!
//! # Environment observation
//!
//! A runtime must not touch the filesystem or the environment directly. The few
//! observations the manager needs — does a directory exist, where is an
//! executable, what is in the environment — are asked through the tiny
//! [`RuntimeHost`] trait, which keeps runtimes and the manager testable
//! (`tasks/010.md` §12). Version probing is a separate seam
//! ([`RuntimeInspector`]) because it runs a process, and only the process
//! manager may do that.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use service_orchestrator_application::ServiceRuntimeError;
use service_orchestrator_domain::{
    AbsolutePath, CommandKind, CommandSpec, Environment, RuntimeKind, Service, Version,
};
use service_orchestrator_process::{ProcessEvent, ProcessManager, ProcessSpec, ShutdownStrategy};
use tokio::sync::broadcast::error::RecvError;

/// The runtimes this adapter can run.
///
/// The set is intentionally closed and mirrors the two stacks in use today. A
/// future runtime is a new variant, a new [`ServiceRuntime`] implementation, and
/// one registry line (`AGENTS.md` §11).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RuntimeType {
    /// A Java virtual machine (also selected by a Maven build system).
    Java,
    /// A Node.js runtime (also selected by an npm build system).
    Node,
}

impl RuntimeType {
    /// Returns the runtime kind this type corresponds to.
    #[must_use]
    pub fn kind(self) -> RuntimeKind {
        match self {
            Self::Java => RuntimeKind::Java,
            Self::Node => RuntimeKind::Node,
        }
    }

    /// Maps a domain runtime kind onto a supported runtime type.
    ///
    /// `maven` maps to Java and `npm` to Node so a service that only declares a
    /// build toolchain still resolves; any other kind is unsupported.
    #[must_use]
    pub fn from_kind(kind: &RuntimeKind) -> Option<Self> {
        match kind {
            RuntimeKind::Java | RuntimeKind::Maven => Some(Self::Java),
            RuntimeKind::Node | RuntimeKind::Npm => Some(Self::Node),
            RuntimeKind::Other(_) => None,
        }
    }

    /// Returns a short, stable runtime label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Java => "java",
            Self::Node => "node",
        }
    }

    /// Returns the toolchain executable this runtime probes for a version.
    #[must_use]
    pub const fn executable_name(self) -> &'static str {
        match self {
            Self::Java => "java",
            Self::Node => "node",
        }
    }

    /// Returns the arguments that make the toolchain print its version.
    #[must_use]
    pub const fn version_args(self) -> &'static [&'static str] {
        match self {
            Self::Java => &["-version"],
            Self::Node => &["--version"],
        }
    }

    /// Returns the version a service pins for this runtime, if any.
    ///
    /// Only an exact kind match counts: a pinned *Maven* version is not the
    /// version of the *Java* toolchain the inspector probes, so it is ignored
    /// rather than compared against the wrong executable.
    #[must_use]
    pub fn declared_version(self, service: &Service) -> Option<Version> {
        let kind = self.kind();
        service
            .runtimes
            .iter()
            .find(|requirement| requirement.kind == kind)
            .and_then(|requirement| requirement.version.clone())
    }
}

impl std::fmt::Display for RuntimeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// A runtime implementation.
///
/// Implementations must be `Send + Sync` because one instance serves every task;
/// they must be stateless or internally synchronized.
pub trait ServiceRuntime: Send + Sync {
    /// Returns the runtime type this implementation handles.
    fn runtime_type(&self) -> RuntimeType;

    /// Validates that `service` can be started by this runtime.
    ///
    /// # Errors
    ///
    /// Returns a [`ServiceRuntimeError`] naming the missing precondition.
    fn validate(&self, service: &Service) -> Result<(), ServiceRuntimeError>;

    /// Builds the structured start plan for `service`.
    ///
    /// # Errors
    ///
    /// Returns a [`ServiceRuntimeError`] if the plan cannot be built.
    fn prepare_start(
        &self,
        service: &Service,
        working_dir: &AbsolutePath,
        environment: Environment,
    ) -> Result<StartPlan, ServiceRuntimeError>;
}

/// A runtime's fully resolved instruction to start one service process.
///
/// A plan is *data*, not behaviour: the runtime builds it and the manager hands
/// it to the process manager. Environment values are present because the child
/// needs them, but the domain [`Environment`] redacts secret values whenever the
/// plan is displayed or serialized (`AGENTS.md` §31).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartPlan {
    /// The runtime that produced the plan.
    pub runtime: RuntimeType,
    /// The executable to run, already resolved for this machine.
    pub program: String,
    /// The structured argument vector.
    pub args: Vec<String>,
    /// The absolute working directory the process starts in.
    pub working_dir: AbsolutePath,
    /// The full environment, including secret values.
    pub environment: Environment,
    /// How the process should be shut down.
    pub shutdown: ShutdownStrategy,
}

impl StartPlan {
    /// Returns a redacted, human-readable identity of the command.
    ///
    /// Only the program and its structured arguments are shown, never an
    /// environment value, so the identity is safe for an event or a task result
    /// (`AGENTS.md` §31).
    #[must_use]
    pub fn command_identity(&self) -> String {
        let mut identity = self.program.clone();
        for arg in &self.args {
            identity.push(' ');
            identity.push_str(arg);
        }
        identity
    }
}

/// Validates the preconditions every runtime shares.
///
/// # Errors
///
/// Returns [`ServiceRuntimeError::StartCommandMissing`] when the service has no
/// start command, or [`ServiceRuntimeError::InvalidConfiguration`] when the
/// command is unusable.
pub fn validate_service(service: &Service) -> Result<(), ServiceRuntimeError> {
    let command = service.commands.get(&CommandKind::Start).ok_or_else(|| {
        ServiceRuntimeError::StartCommandMissing {
            service: service.id.clone(),
        }
    })?;
    if command.program.trim().is_empty() {
        return Err(ServiceRuntimeError::InvalidConfiguration {
            message: format!("service '{}' has an empty start command", service.id),
        });
    }
    Ok(())
}

/// Builds a start plan from a service's configured start command.
///
/// The configured command is authoritative — nothing is hard-coded. The one
/// convenience is the Maven wrapper: when a Java service's command is `mvn` and a
/// `mvnw` wrapper exists in the working directory, the wrapper is preferred, so a
/// developer's commits pin their Maven version (`tasks/010.md` §10).
///
/// # Errors
///
/// Returns [`ServiceRuntimeError::StartCommandMissing`] when the service has no
/// start command.
pub fn plan_from_service(
    runtime: RuntimeType,
    service: &Service,
    working_dir: &AbsolutePath,
    environment: Environment,
) -> Result<StartPlan, ServiceRuntimeError> {
    let command = service.commands.get(&CommandKind::Start).ok_or_else(|| {
        ServiceRuntimeError::StartCommandMissing {
            service: service.id.clone(),
        }
    })?;
    Ok(StartPlan {
        runtime,
        program: resolve_program(runtime, command, working_dir),
        args: command.args.clone(),
        working_dir: working_dir.clone(),
        environment,
        shutdown: ShutdownStrategy::graceful(),
    })
}

/// Chooses the executable for a start command, preferring the Maven wrapper.
fn resolve_program(
    runtime: RuntimeType,
    command: &CommandSpec,
    working_dir: &AbsolutePath,
) -> String {
    if runtime == RuntimeType::Java
        && basename(&command.program) == "mvn"
        && Path::new(working_dir.as_str()).join("mvnw").is_file()
    {
        return "./mvnw".to_owned();
    }
    command.program.clone()
}

/// Returns the final path segment of a program.
#[must_use]
pub fn basename(program: &str) -> &str {
    program.rsplit('/').next().unwrap_or(program)
}

/// Extracts the first version-like token (digits separated by dots) from command
/// output.
///
/// This parses `openjdk version "21.0.1"`, `java version "1.8.0_292"`, and
/// `v20.x` alike.
#[must_use]
pub fn extract_version(output: &str) -> Option<String> {
    let mut current = String::new();
    let mut started = false;
    for character in output.chars() {
        if character.is_ascii_digit() {
            current.push(character);
            started = true;
        } else if character == '.' && started {
            current.push(character);
        } else if started && (character == 'x' || character == 'X') && current.ends_with('.') {
            // A wildcard component such as `20.x` is part of the reported
            // version; it is the only non-digit allowed after a dot.
            current.push(character);
        } else if started {
            break;
        }
    }
    let trimmed = current.trim_end_matches('.');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Returns the major component of a version.
///
/// Legacy Java versions use a `1.<major>` scheme, so `1.8.0` yields `8`.
#[must_use]
pub fn major_version(version: &str) -> Option<u32> {
    let version = version.trim().trim_start_matches(['v', 'V']);
    let mut parts = version.split('.');
    let first = parts.next()?;
    if first == "1" {
        return parts.next()?.parse().ok();
    }
    first.parse().ok()
}

/// The read-only environment a runtime and the manager observe.
pub trait RuntimeHost: Send + Sync {
    /// Returns `true` when `directory` exists and is a directory.
    fn directory_exists(&self, directory: &AbsolutePath) -> bool;

    /// Resolves a bare program name to an executable path.
    ///
    /// A program that already contains a `/` is returned unchanged.
    fn resolve_executable(&self, program: &str) -> Option<String>;

    /// Returns the value of an environment variable, if it is set.
    fn environment_var(&self, name: &str) -> Option<String>;
}

/// The host backed by the real operating system.
///
/// It performs only cheap, synchronous observations; it never spawns a process.
/// Version probing goes through the process manager (see
/// [`ProcessRuntimeInspector`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemRuntimeHost;

impl SystemRuntimeHost {
    /// Creates a system host.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Returns the directories searched for a bare program name.
    fn search_directories() -> Vec<PathBuf> {
        let mut directories: Vec<PathBuf> = Vec::new();

        if let Ok(path) = std::env::var("PATH") {
            directories.extend(
                path.split(':')
                    .filter(|entry| !entry.is_empty())
                    .map(PathBuf::from),
            );
        }
        if let Ok(home) = std::env::var("JAVA_HOME") {
            directories.push(PathBuf::from(home).join("bin"));
        }
        let home = std::env::var("HOME").ok();
        let mut push_home = |suffix: &str| {
            if let Some(home) = &home {
                directories.push(Path::new(home).join(suffix));
            }
        };
        push_home(".volta/bin");
        push_home(".asdf/shims");
        push_home(".local/share/fnm/aliases/default/bin");
        push_home(".sdkman/candidates/java/current/bin");
        push_home(".jenv/shims");

        // nvm installs one directory per Node version; search newest first.
        if let Some(home) = &home {
            let versions = Path::new(home).join(".nvm/versions/node");
            if let Ok(entries) = std::fs::read_dir(&versions) {
                let mut bins: Vec<PathBuf> = entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path().join("bin"))
                    .collect();
                bins.sort();
                bins.reverse();
                directories.extend(bins);
            }
        }

        for system in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
            directories.push(PathBuf::from(system));
        }
        directories
    }
}

impl RuntimeHost for SystemRuntimeHost {
    fn directory_exists(&self, directory: &AbsolutePath) -> bool {
        Path::new(directory.as_str()).is_dir()
    }

    fn resolve_executable(&self, program: &str) -> Option<String> {
        if program.contains('/') {
            return Some(program.to_owned());
        }
        for directory in Self::search_directories() {
            let candidate = directory.join(program);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
        None
    }

    fn environment_var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// What a runtime inspection learned about the host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeInfo {
    /// The installed version, when it could be determined.
    pub version: Option<Version>,
    /// The resolved toolchain executable, when one was found.
    pub executable: Option<String>,
}

/// Determines whether a runtime toolchain is available and which version it is.
///
/// This is a seam, not a rule: the manager asks the inspector for the installed
/// version and compares it against a pinned one. The default
/// [`NullRuntimeInspector`] answers "unknown" without spawning anything, which
/// keeps most starts free of an extra process; production wiring uses
/// [`ProcessRuntimeInspector`].
#[async_trait]
pub trait RuntimeInspector: Send + Sync {
    /// Inspects `runtime`, returning what is installed.
    ///
    /// # Errors
    ///
    /// A real inspector may return [`ServiceRuntimeError::RuntimeNotFound`]
    /// when the runtime is unavailable.
    async fn inspect(&self, runtime: RuntimeType) -> Result<RuntimeInfo, ServiceRuntimeError>;
}

/// A [`RuntimeInspector`] that never probes and never fails.
///
/// It is the default so a start only pays for probing when version compatibility
/// is actually configured and an inspector is wired.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullRuntimeInspector;

#[async_trait]
impl RuntimeInspector for NullRuntimeInspector {
    async fn inspect(&self, _runtime: RuntimeType) -> Result<RuntimeInfo, ServiceRuntimeError> {
        Ok(RuntimeInfo::default())
    }
}

/// How long a version probe may run before it is abandoned.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// A [`RuntimeInspector`] that runs the toolchain through the process manager.
///
/// This is the only way to learn an installed version: the process manager is
/// the sole owner of process execution (`AGENTS.md` §15). Output capture
/// subscribes to the manager's event stream *before* spawning so an early
/// version line is never missed.
pub struct ProcessRuntimeInspector {
    processes: ProcessManager,
    host: Arc<dyn RuntimeHost>,
}

impl ProcessRuntimeInspector {
    /// Creates an inspector over a process manager and environment host.
    #[must_use]
    pub fn new(processes: ProcessManager, host: Arc<dyn RuntimeHost>) -> Self {
        Self { processes, host }
    }

    /// Resolves the toolchain executable for a runtime.
    fn executable(&self, runtime: RuntimeType) -> Option<String> {
        match runtime {
            RuntimeType::Java => {
                if let Some(home) = self.host.environment_var("JAVA_HOME") {
                    let candidate = format!("{}/bin/java", home.trim_end_matches('/'));
                    if Path::new(&candidate).is_file() {
                        return Some(candidate);
                    }
                }
                self.host.resolve_executable(runtime.executable_name())
            }
            RuntimeType::Node => self.host.resolve_executable(runtime.executable_name()),
        }
    }
}

impl std::fmt::Debug for ProcessRuntimeInspector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessRuntimeInspector")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl RuntimeInspector for ProcessRuntimeInspector {
    async fn inspect(&self, runtime: RuntimeType) -> Result<RuntimeInfo, ServiceRuntimeError> {
        let Some(executable) = self.executable(runtime) else {
            // Not finding the toolchain is not fatal here: the start itself will
            // surface a precise spawn failure, and a service that pins no version
            // must not be blocked by a probe.
            return Ok(RuntimeInfo::default());
        };
        let Ok(spec) = ProcessSpec::new(executable.clone()) else {
            return Ok(RuntimeInfo {
                version: None,
                executable: Some(executable),
            });
        };
        let spec = spec
            .args(runtime.version_args().iter().map(|arg| (*arg).to_owned()))
            .with_timeout(PROBE_TIMEOUT);

        // Subscribe before spawning so no early output line is missed.
        let mut events = self.processes.subscribe_events();
        let Ok(handle) = self.processes.spawn(spec) else {
            return Ok(RuntimeInfo {
                version: None,
                executable: Some(executable),
            });
        };
        let id = handle.id().clone();

        let mut output = String::new();
        loop {
            match events.recv().await {
                Ok(event) => {
                    if event.process_id() != &id {
                        continue;
                    }
                    match event {
                        ProcessEvent::StdoutLine(line) | ProcessEvent::StderrLine(line) => {
                            output.push_str(&line.line);
                            output.push('\n');
                        }
                        ProcessEvent::Stopped { .. }
                        | ProcessEvent::Exited { .. }
                        | ProcessEvent::Failed { .. }
                        | ProcessEvent::Killed { .. } => break,
                        _ => {}
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }

        let version = extract_version(&output).and_then(|raw| Version::new(raw).ok());
        Ok(RuntimeInfo {
            version,
            executable: Some(executable),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_types_map_to_domain_kinds() {
        assert_eq!(
            RuntimeType::from_kind(&RuntimeKind::Java),
            Some(RuntimeType::Java)
        );
        assert_eq!(
            RuntimeType::from_kind(&RuntimeKind::Maven),
            Some(RuntimeType::Java)
        );
        assert_eq!(
            RuntimeType::from_kind(&RuntimeKind::Node),
            Some(RuntimeType::Node)
        );
        assert_eq!(
            RuntimeType::from_kind(&RuntimeKind::Npm),
            Some(RuntimeType::Node)
        );
        assert_eq!(
            RuntimeType::from_kind(&RuntimeKind::Other("go".to_owned())),
            None
        );
    }

    #[test]
    fn extracts_versions_from_known_outputs() {
        assert_eq!(
            extract_version("openjdk version \"21.0.1\" 2023-10-17").as_deref(),
            Some("21.0.1")
        );
        assert_eq!(
            extract_version("java version \"1.8.0_292\"").as_deref(),
            Some("1.8.0")
        );
        assert_eq!(extract_version("v20.x\n").as_deref(), Some("20.x"));
        assert_eq!(extract_version("no digits here"), None);
    }

    #[test]
    fn major_version_handles_legacy_java() {
        assert_eq!(major_version("21.0.1"), Some(21));
        assert_eq!(major_version("1.8.0"), Some(8));
        assert_eq!(major_version("v20.x"), Some(20));
        assert_eq!(major_version("not-a-version"), None);
    }

    #[test]
    fn basename_returns_the_final_segment() {
        assert_eq!(basename("./mvnw"), "mvnw");
        assert_eq!(basename("java"), "java");
        assert_eq!(basename("/usr/bin/node"), "node");
    }

    #[test]
    fn a_java_command_prefers_a_present_wrapper() {
        let dir = std::env::temp_dir().join(format!("so-runtime-wrapper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mvnw"), "#!/bin/sh\n").unwrap();
        let working_dir = AbsolutePath::new(dir.to_string_lossy().into_owned()).unwrap();
        let command = CommandSpec::new("mvn", ["spring-boot:run"]).unwrap();
        assert_eq!(
            resolve_program(RuntimeType::Java, &command, &working_dir),
            "./mvnw"
        );
        // Node never rewrites its program.
        assert_eq!(
            resolve_program(RuntimeType::Node, &command, &working_dir),
            "mvn"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_java_command_keeps_mvn_without_a_wrapper() {
        let dir = std::env::temp_dir().join(format!("so-runtime-nowrapper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let working_dir = AbsolutePath::new(dir.to_string_lossy().into_owned()).unwrap();
        let command = CommandSpec::new("mvn", ["spring-boot:run"]).unwrap();
        assert_eq!(
            resolve_program(RuntimeType::Java, &command, &working_dir),
            "mvn"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn the_null_inspector_reports_nothing() {
        let info = NullRuntimeInspector
            .inspect(RuntimeType::Java)
            .await
            .unwrap();
        assert_eq!(info, RuntimeInfo::default());
    }

    #[test]
    fn the_system_host_treats_a_path_like_program_as_is() {
        let host = SystemRuntimeHost::new();
        assert_eq!(host.resolve_executable("./mvnw").as_deref(), Some("./mvnw"));
    }

    #[test]
    fn the_system_host_resolves_a_known_executable() {
        let host = SystemRuntimeHost::new();
        // `/bin` is always searched, and `sh` is always present.
        let resolved = host.resolve_executable("sh");
        assert!(resolved.is_some());
    }

    #[test]
    fn the_host_is_object_safe() {
        let host: Arc<dyn RuntimeHost> = Arc::new(SystemRuntimeHost::new());
        assert!(host.environment_var("PATH").is_some());
    }
}
