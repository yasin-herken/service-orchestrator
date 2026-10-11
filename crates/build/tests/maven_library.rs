//! Gated Maven integration tests.
//!
//! These are `#[ignore]`d by default because they require a real Maven
//! installation and can download plugins. Run them explicitly:
//!
//! ```text
//! cargo test -p service-orchestrator-build --test maven_library -- --ignored
//! ```
//!
//! Every run installs into an **isolated temporary local repository** via
//! `-Dmaven.repo.local=...`, so the developer's `~/.m2/repository` is never
//! modified (`tasks/009.md` §28). Only the read-only local-repository probe uses
//! the real settings, and it writes nothing.

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use service_orchestrator_application::{
    ArtifactInspector, ArtifactPresence, ResolvedArtifact, ResolvedBuildCommand,
    ResolvedBuildTarget,
};
use service_orchestrator_build::{
    BuildManager, LocalRepositoryProvider, MavenArtifactInspector, MavenLocalRepositoryProvider,
};
use service_orchestrator_domain::{
    AbsolutePath, ArtifactCoordinate, BuildOperation, BuildSystem, LibraryId, Version, WorkspaceId,
};
use service_orchestrator_process::ProcessManager;
use support::TempDir;

/// Writes `pom.xml` inside `dir`, creating `dir`.
fn write_pom(dir: &Path, xml: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("pom.xml"), xml).unwrap();
}

/// Writes an isolated Maven `settings.xml` with **no mirrors** and an explicit
/// local repository, returning its path.
///
/// This is what keeps the tests hermetic: without it, Maven would read the
/// developer's own settings (which may mirror every repository) and would install
/// into the real `~/.m2/repository`.
fn write_settings(dir: &Path, local_repository: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("settings.xml");
    let xml = format!(
        "<settings xmlns=\"http://maven.apache.org/SETTINGS/1.0.0\">\
         <localRepository>{}</localRepository></settings>",
        local_repository.display()
    );
    std::fs::write(&path, xml).unwrap();
    path
}

/// An absolute path for a fixture path.
fn absolute(path: &Path) -> AbsolutePath {
    AbsolutePath::new(path.to_string_lossy().into_owned()).unwrap()
}

/// A Maven install target running a configured `mvn` command.
fn install_target(project: &Path, workspace_root: &Path, settings: &Path) -> ResolvedBuildTarget {
    ResolvedBuildTarget {
        workspace_id: WorkspaceId::new("acme").unwrap(),
        name: "demo".to_owned(),
        build_system: BuildSystem::Maven,
        operation: BuildOperation::PublishLocalArtifact,
        path: absolute(project),
        workspace_root: absolute(workspace_root),
        command: Some(ResolvedBuildCommand {
            program: "mvn".to_owned(),
            args: vec![
                "-B".to_owned(),
                "-q".to_owned(),
                "-s".to_owned(),
                settings.display().to_string(),
                "install".to_owned(),
            ],
            working_dir: None,
        }),
    }
}

/// Locates `coordinate` against a fixed local repository.
async fn locate(local_repo: &Path, coordinate: ArtifactCoordinate) -> ArtifactPresence {
    let inspector = MavenArtifactInspector::with_fixed_repository(absolute(local_repo));
    inspector
        .locate(&ResolvedArtifact {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            library: LibraryId::new("demo").unwrap(),
            path: absolute(local_repo),
            coordinate,
        })
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires a Maven installation; run with --ignored"]
async fn installs_a_single_module_library_and_verifies_it() {
    let workspace = TempDir::new("m2-ws");
    let project = workspace.path().join("demo-lib");
    let local_repo = workspace.path().join("repository");
    let settings = write_settings(&workspace.path().join("settings"), &local_repo);
    write_pom(
        &project,
        "<project xmlns=\"http://maven.apache.org/POM/4.0.0\">\
         <modelVersion>4.0.0</modelVersion>\
         <groupId>com.acme</groupId><artifactId>demo-lib</artifactId>\
         <version>1.0.0</version><packaging>jar</packaging></project>",
    );

    let manager = BuildManager::new(ProcessManager::new().unwrap());
    let outcome = manager
        .execute(
            &install_target(&project, workspace.path(), &settings),
            Some(Duration::from_secs(600)),
            None,
        )
        .await
        .expect("the Maven install succeeds");
    assert_eq!(outcome.exit_code, Some(0));

    let coordinate =
        ArtifactCoordinate::new("com.acme", "demo-lib", Version::new("1.0.0").unwrap()).unwrap();
    assert!(locate(&local_repo, coordinate).await.is_found());
}

#[tokio::test]
#[ignore = "requires a Maven installation; run with --ignored"]
async fn a_missing_artifact_is_reported_as_missing() {
    let workspace = TempDir::new("m2-missing");
    let local_repo = workspace.path().join("repository");
    std::fs::create_dir_all(&local_repo).unwrap();

    let coordinate =
        ArtifactCoordinate::new("com.acme", "absent-lib", Version::new("9.9.9").unwrap()).unwrap();
    let presence = locate(&local_repo, coordinate).await;
    assert!(!presence.is_found());
    match presence {
        ArtifactPresence::Missing { expected_path } => assert!(expected_path.is_some()),
        other => panic!("expected a missing artifact, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires a Maven installation; run with --ignored"]
async fn installs_a_multi_module_reactor_and_verifies_every_artifact() {
    let workspace = TempDir::new("m2-reactor");
    let reactor = workspace.path().join("reactor");
    let local_repo = workspace.path().join("repository");
    let settings = write_settings(&workspace.path().join("settings"), &local_repo);

    write_pom(
        &reactor,
        "<project xmlns=\"http://maven.apache.org/POM/4.0.0\">\
         <modelVersion>4.0.0</modelVersion>\
         <groupId>com.acme</groupId><artifactId>reactor</artifactId>\
         <version>1.0.0</version><packaging>pom</packaging>\
         <modules><module>a</module><module>b</module></modules></project>",
    );
    write_pom(
        &reactor.join("a"),
        "<project xmlns=\"http://maven.apache.org/POM/4.0.0\">\
         <modelVersion>4.0.0</modelVersion>\
         <parent><groupId>com.acme</groupId><artifactId>reactor</artifactId>\
         <version>1.0.0</version></parent>\
         <artifactId>a-lib</artifactId><packaging>jar</packaging></project>",
    );
    write_pom(
        &reactor.join("b"),
        "<project xmlns=\"http://maven.apache.org/POM/4.0.0\">\
         <modelVersion>4.0.0</modelVersion>\
         <parent><groupId>com.acme</groupId><artifactId>reactor</artifactId>\
         <version>1.0.0</version></parent>\
         <artifactId>b-lib</artifactId><packaging>jar</packaging>\
         <dependencies><dependency><groupId>com.acme</groupId>\
         <artifactId>a-lib</artifactId><version>1.0.0</version></dependency>\
         </dependencies></project>",
    );

    let manager = BuildManager::new(ProcessManager::new().unwrap());
    manager
        .execute(
            &install_target(&reactor, workspace.path(), &settings),
            Some(Duration::from_secs(600)),
            None,
        )
        .await
        .expect("the reactor install succeeds");

    for artifact_id in ["a-lib", "b-lib"] {
        let coordinate =
            ArtifactCoordinate::new("com.acme", artifact_id, Version::new("1.0.0").unwrap())
                .unwrap();
        assert!(
            locate(&local_repo, coordinate).await.is_found(),
            "expected {artifact_id} to be installed"
        );
    }
}

#[tokio::test]
#[ignore = "requires a Maven installation; run with --ignored"]
async fn the_local_repository_provider_reports_a_path() {
    let workspace = TempDir::new("m2-provider");
    let project = workspace.path().join("demo");
    let local_repo = workspace.path().join("repository");
    let settings = write_settings(&workspace.path().join("settings"), &local_repo);
    write_pom(
        &project,
        "<project xmlns=\"http://maven.apache.org/POM/4.0.0\">\
         <modelVersion>4.0.0</modelVersion>\
         <groupId>com.acme</groupId><artifactId>demo</artifactId>\
         <version>1.0.0</version></project>",
    );

    let provider = MavenLocalRepositoryProvider::new(ProcessManager::new().unwrap())
        .with_settings(absolute(&settings));
    let root: PathBuf = provider
        .local_repository(&absolute(&project))
        .await
        .expect("Maven reports a local repository")
        .as_str()
        .into();
    assert!(
        root.is_absolute(),
        "reported path is not absolute: {root:?}"
    );
    assert!(
        root.ends_with("repository"),
        "expected the pinned local repository, got {root:?}"
    );
}
