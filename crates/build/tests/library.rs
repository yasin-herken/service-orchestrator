//! Integration tests for library operations through the task engine.
//!
//! These exercise the library side of the adapter flow:
//!
//! ```text
//! TaskDefinition (target / artifact parameters)
//!     -> TaskEngine -> BuildTaskExecutor / ArtifactVerificationExecutor
//!     -> BuildManager / ArtifactInspector
//! ```
//!
//! The install tasks use a configured command (a tiny local utility) so no Maven
//! runs, and the verification tasks use a fake inspector so no repository is
//! touched. What is asserted is the engine-level wiring: the right task kinds are
//! registered and failures carry the stable library codes.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use service_orchestrator_application::{
    ArtifactInspectionError, ArtifactInspector, ArtifactPresence, DependencyVersionInspection,
    ResolvedArtifact, ResolvedDependency,
};
use service_orchestrator_build::{
    register_build_executors, register_library_executors, BuildManager,
};
use service_orchestrator_domain::{
    AbsolutePath, ArtifactCoordinate, BuildOperation, BuildSystem, LibraryId, TaskKind, TaskStatus,
    Version, WorkspaceId,
};
use service_orchestrator_execution::{ExecutorRegistry, TaskDefinition, TaskEngine};
use service_orchestrator_process::ProcessManager;
use support::Workspace;

#[cfg(unix)]
const ECHO: &str = "/bin/echo";
#[cfg(unix)]
const FALSE: &str = "/usr/bin/false";

/// An inspector whose presence answer the test controls.
struct FakeInspector {
    presence: ArtifactPresence,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl ArtifactInspector for FakeInspector {
    async fn locate(
        &self,
        _artifact: &ResolvedArtifact,
    ) -> Result<ArtifactPresence, ArtifactInspectionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.presence.clone())
    }

    async fn dependency_version(
        &self,
        _dependency: &ResolvedDependency,
    ) -> Result<DependencyVersionInspection, ArtifactInspectionError> {
        Ok(DependencyVersionInspection::NotDeclared)
    }
}

fn artifact() -> ResolvedArtifact {
    ResolvedArtifact {
        workspace_id: WorkspaceId::new("acme").unwrap(),
        library: LibraryId::new("common-core").unwrap(),
        path: AbsolutePath::new("/w/acme/libraries/common-core").unwrap(),
        coordinate: ArtifactCoordinate::new(
            "com.acme",
            "common-core",
            Version::new("1.8.0-SNAPSHOT").unwrap(),
        )
        .unwrap(),
    }
}

/// Builds an engine with both the build and library executors registered.
fn engine(inspector: Arc<FakeInspector>) -> TaskEngine {
    let manager = Arc::new(BuildManager::new(ProcessManager::new().unwrap()));
    let mut registry = ExecutorRegistry::new();
    register_build_executors(&mut registry, manager);
    register_library_executors(&mut registry, inspector);
    TaskEngine::builder()
        .max_parallel_tasks(2)
        .registry(registry)
        .build()
        .expect("the engine builds inside a Tokio runtime")
}

async fn run(engine: &TaskEngine, definition: TaskDefinition) -> service_orchestrator_domain::Task {
    let id = engine.submit(definition).expect("task is accepted");
    engine.wait_for(&id).await.expect("the task exists")
}

#[tokio::test]
#[cfg(unix)]
async fn a_library_install_succeeds_through_the_engine() {
    let workspace = Workspace::new("common-core");
    let target = workspace.target_with_command(
        BuildSystem::Maven,
        BuildOperation::PublishLocalArtifact,
        ECHO,
        &["installed"],
    );
    let inspector = Arc::new(FakeInspector {
        presence: ArtifactPresence::Missing {
            expected_path: None,
        },
        calls: AtomicUsize::new(0),
    });
    let engine = engine(inspector);

    let task = run(
        &engine,
        TaskDefinition::new(TaskKind::LibraryInstall)
            .for_library(LibraryId::new("common-core").unwrap())
            .with_parameters(target.to_parameters()),
    )
    .await;
    assert_eq!(task.status, TaskStatus::Succeeded);
    engine.shutdown().await;
}

#[tokio::test]
#[cfg(unix)]
async fn a_failed_library_install_carries_the_installation_code() {
    let workspace = Workspace::new("common-core");
    let target = workspace.target_with_command(
        BuildSystem::Maven,
        BuildOperation::PublishLocalArtifact,
        FALSE,
        &[],
    );
    let inspector = Arc::new(FakeInspector {
        presence: ArtifactPresence::Missing {
            expected_path: None,
        },
        calls: AtomicUsize::new(0),
    });
    let engine = engine(inspector);

    let task = run(
        &engine,
        TaskDefinition::new(TaskKind::LibraryInstall)
            .for_library(LibraryId::new("common-core").unwrap())
            .with_parameters(target.to_parameters()),
    )
    .await;
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(
        task.error.expect("failed tasks carry an error").code,
        "ARTIFACT_INSTALLATION_FAILED"
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn artifact_verification_succeeds_when_the_artifact_is_present() {
    let inspector = Arc::new(FakeInspector {
        presence: ArtifactPresence::Found {
            path: AbsolutePath::new("/repo/com/acme/common-core/1.8.0-SNAPSHOT/core.jar").unwrap(),
            size_bytes: Some(10),
        },
        calls: AtomicUsize::new(0),
    });
    let engine = engine(inspector.clone());

    let task = run(
        &engine,
        TaskDefinition::new(TaskKind::ArtifactVerification)
            .for_library(LibraryId::new("common-core").unwrap())
            .with_parameters(artifact().to_parameters()),
    )
    .await;
    assert_eq!(task.status, TaskStatus::Succeeded);
    assert_eq!(inspector.calls.load(Ordering::SeqCst), 1);
    engine.shutdown().await;
}

#[tokio::test]
async fn artifact_verification_fails_with_a_stable_code_when_missing() {
    let inspector = Arc::new(FakeInspector {
        presence: ArtifactPresence::Missing {
            expected_path: None,
        },
        calls: AtomicUsize::new(0),
    });
    let engine = engine(inspector);

    let task = run(
        &engine,
        TaskDefinition::new(TaskKind::ArtifactVerification)
            .for_library(LibraryId::new("common-core").unwrap())
            .with_parameters(artifact().to_parameters()),
    )
    .await;
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(
        task.error.expect("failed tasks carry an error").code,
        "ARTIFACT_NOT_FOUND"
    );
    engine.shutdown().await;
}
