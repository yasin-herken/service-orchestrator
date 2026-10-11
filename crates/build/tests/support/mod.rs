//! Shared fixtures for the build adapter integration tests.
//!
//! Every fixture is local and offline: a temporary workspace directory and, for
//! planning tests, a [`FakeHost`] whose filesystem and environment answers the
//! test controls. No Maven, npm, or network access is involved (`tasks/008.md`).
//!
//! Integration tests in a Cargo crate are separate binaries, so this module is
//! compiled into each of them; not every helper is used by every test file,
//! hence the module-level `dead_code` allowance.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use service_orchestrator_application::{ResolvedBuildCommand, ResolvedBuildTarget};
use service_orchestrator_build::{BuildHost, BuildManager};
use service_orchestrator_domain::{AbsolutePath, BuildOperation, BuildSystem, WorkspaceId};
use service_orchestrator_process::ProcessManager;

/// A temporary directory removed when the fixture is dropped.
pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates a fresh, uniquely named temporary directory.
    pub(crate) fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("so-build-{label}-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temporary directory is creatable");
        Self { path }
    }

    /// Returns the temporary directory path.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A host whose answers a test controls.
pub(crate) struct FakeHost {
    files: Vec<String>,
    environment: Vec<String>,
}

impl FakeHost {
    /// A host that reports no files and no environment variables.
    pub(crate) fn new() -> Self {
        Self {
            files: Vec::new(),
            environment: Vec::new(),
        }
    }

    /// Reports that `name` exists in every directory.
    #[must_use]
    pub(crate) fn with_file(mut self, name: &str) -> Self {
        self.files.push(name.to_owned());
        self
    }

    /// Reports that the named environment variable is set.
    #[must_use]
    pub(crate) fn with_environment(mut self, name: &str) -> Self {
        self.environment.push(name.to_owned());
        self
    }
}

impl Default for FakeHost {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildHost for FakeHost {
    fn file_exists(&self, _directory: &AbsolutePath, name: &str) -> bool {
        self.files.iter().any(|file| file == name)
    }

    fn environment_has(&self, name: &str) -> bool {
        self.environment.iter().any(|variable| variable == name)
    }
}

/// A temporary workspace with one entity directory.
pub(crate) struct Workspace {
    _root: TempDir,
    /// The workspace root.
    pub(crate) root: PathBuf,
    /// The entity's absolute directory, inside the root.
    pub(crate) entity: PathBuf,
}

impl Workspace {
    /// Creates `root/backend/<name>` and returns the fixture.
    pub(crate) fn new(name: &str) -> Self {
        let root = TempDir::new("workspace");
        let root_path = root.path().to_path_buf();
        let entity = root_path.join("backend").join(name);
        std::fs::create_dir_all(&entity).expect("the entity directory is creatable");
        Self {
            _root: root,
            root: root_path,
            entity,
        }
    }

    /// A target for this entity, with no configured command.
    pub(crate) fn target(
        &self,
        system: BuildSystem,
        operation: BuildOperation,
    ) -> ResolvedBuildTarget {
        self.target_with(system, operation, None)
    }

    /// A target for this entity that carries a configured command.
    pub(crate) fn target_with_command(
        &self,
        system: BuildSystem,
        operation: BuildOperation,
        program: &str,
        args: &[&str],
    ) -> ResolvedBuildTarget {
        self.target_with(
            system,
            operation,
            Some(ResolvedBuildCommand {
                program: program.to_owned(),
                args: args.iter().map(|arg| (*arg).to_owned()).collect(),
                working_dir: None,
            }),
        )
    }

    fn target_with(
        &self,
        system: BuildSystem,
        operation: BuildOperation,
        command: Option<ResolvedBuildCommand>,
    ) -> ResolvedBuildTarget {
        ResolvedBuildTarget {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            name: "entity".to_owned(),
            build_system: system,
            operation,
            path: AbsolutePath::new(self.entity.to_string_lossy().into_owned()).unwrap(),
            workspace_root: AbsolutePath::new(self.root.to_string_lossy().into_owned()).unwrap(),
            command,
        }
    }
}

/// Creates a build manager with the standard providers and the system host.
pub(crate) fn manager() -> BuildManager {
    BuildManager::new(ProcessManager::new().unwrap())
}
