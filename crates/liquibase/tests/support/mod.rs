//! Shared fixtures for the migration adapter integration tests.
//!
//! Every fixture is local and offline: a temporary workspace directory and a
//! configuration whose migration commands are tiny local utilities. No
//! Liquibase, Maven, database, or network access is involved (`tasks/011.md`
//! §30, §31).
//!
//! Integration tests in a Cargo crate are separate binaries, so this module is
//! compiled into each of them; not every helper is used by every test file,
//! hence the module-level `dead_code` allowance.

#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use service_orchestrator_application::ResolvedMigrationTarget;
use service_orchestrator_config::{load_from_str, Config};
use service_orchestrator_domain::{AbsolutePath, MigrationOperation, ServiceId, WorkspaceId};
use service_orchestrator_liquibase::MigrationManager;
use service_orchestrator_process::ProcessManager;

/// The service id every fixture uses.
pub(crate) const SERVICE: &str = "auth-service";

/// A temporary directory removed when the fixture is dropped.
pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates a fresh, uniquely named temporary directory.
    pub(crate) fn new(label: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "so-migration-{label}-{}-{unique}",
            std::process::id()
        ));
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

/// A temporary workspace with one service directory.
pub(crate) struct Workspace {
    _root: TempDir,
    root: PathBuf,
    entity: PathBuf,
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

    /// The workspace root.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// The service's workspace-relative path.
    pub(crate) fn relative(&self) -> String {
        self.entity
            .strip_prefix(&self.root)
            .expect("the entity is inside the root")
            .to_string_lossy()
            .into_owned()
    }

    /// A migration target for this service.
    pub(crate) fn target(&self, operation: MigrationOperation) -> ResolvedMigrationTarget {
        ResolvedMigrationTarget {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            service_id: ServiceId::new(SERVICE).unwrap(),
            operation,
            preset: None,
            path: AbsolutePath::new(self.entity.to_string_lossy().into_owned()).unwrap(),
            workspace_root: AbsolutePath::new(self.root.to_string_lossy().into_owned()).unwrap(),
            non_local_authorized: false,
        }
    }

    /// The same target with an explicit non-local authorization.
    pub(crate) fn authorized_target(
        &self,
        operation: MigrationOperation,
    ) -> ResolvedMigrationTarget {
        ResolvedMigrationTarget {
            non_local_authorized: true,
            ..self.target(operation)
        }
    }
}

/// Builds a single-service configuration whose migration commands are the given
/// TOML table body (for example `update = { program = "/bin/echo", args = ["ok"] }`).
pub(crate) fn config(
    workspace: &Workspace,
    database_environment: &str,
    commands: &str,
) -> Arc<Config> {
    let text = format!(
        r#"
config_version = 1

[[workspaces]]
id = "acme"
root = "{root}"

[[services]]
id = "{service}"
type = "backend"
workspace = "acme"
path = "{relative}"

[services.commands]
build = "./mvnw clean install"
start = "./mvnw spring-boot:run"

[services.liquibase]
enabled = true

[services.liquibase.commands]
{commands}

[services.liquibase.database]
environment = "{environment}"
"#,
        root = workspace.root().display(),
        service = SERVICE,
        relative = workspace.relative(),
        commands = commands,
        environment = database_environment,
    );
    Arc::new(load_from_str(&text, "<migration-test>").expect("configuration should load"))
}

/// Creates a migration manager with the standard providers and system host.
pub(crate) fn manager(config: Arc<Config>) -> MigrationManager {
    MigrationManager::new(ProcessManager::new().unwrap(), config)
}

/// Writes an executable script into `workspace` and returns its absolute path.
///
/// A configured command is a structured program plus arguments and may never be
/// a `sh -c` command line (`tasks/026.md` §7). A project frequently reaches for
/// a small wrapper script instead, so the fixtures use one too: the script is
/// invoked directly as its own program.
pub(crate) fn script(workspace: &Workspace, name: &str, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt as _;

    let path = workspace.entity.join(name);
    let mut file = std::fs::File::create(&path).expect("the script is creatable");
    file.write_all(format!("#!/bin/sh\n{body}\n").as_bytes())
        .expect("the script is written");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("the script is executable");
    path.to_string_lossy().into_owned()
}
