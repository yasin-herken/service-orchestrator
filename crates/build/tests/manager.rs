//! End-to-end build execution through the manager and the process manager.
//!
//! These tests run real processes (tiny, local utilities) to exercise the
//! manager's classification of success, non-zero exit, timeout, and
//! cancellation. They never invoke Maven or npm: a configured command is
//! authoritative, which is exactly the seam provider-independent orchestration
//! relies on.

mod support;

use std::time::Duration;

use service_orchestrator_build::{BuildError, BuildOutcome};
use service_orchestrator_domain::BuildOperation;
use service_orchestrator_execution::Cancellation;
use support::{manager, Workspace};

#[cfg(unix)]
const ECHO: &str = "/bin/echo";
#[cfg(unix)]
const FALSE: &str = "/usr/bin/false";
#[cfg(unix)]
const SLEEP: &str = "/bin/sleep";

#[tokio::test]
#[cfg(unix)]
async fn a_build_runs_and_reports_its_exit_code() {
    let workspace = Workspace::new("auth-service");
    let target = workspace.target_with_command(
        service_orchestrator_domain::BuildSystem::Maven,
        BuildOperation::Build,
        ECHO,
        &["built"],
    );

    let outcome: BuildOutcome = manager().execute(&target, None, None).await.unwrap();
    assert_eq!(outcome.exit_code, Some(0));
    assert_eq!(outcome.stdout, "built");
}

#[tokio::test]
#[cfg(unix)]
async fn a_failed_build_is_reported_with_a_structured_code() {
    let workspace = Workspace::new("auth-service");
    let target = workspace.target_with_command(
        service_orchestrator_domain::BuildSystem::Maven,
        BuildOperation::Build,
        FALSE,
        &[],
    );

    let error = manager().execute(&target, None, None).await.unwrap_err();
    assert_eq!(error.code(), "BUILD_COMMAND_FAILED");
    match error {
        BuildError::CommandFailed { exit_code, .. } => assert_eq!(exit_code, Some(1)),
        other => panic!("expected a command failure, got {other:?}"),
    }
}

#[tokio::test]
#[cfg(unix)]
async fn a_timed_out_build_is_reported() {
    let workspace = Workspace::new("auth-service");
    let target = workspace.target_with_command(
        service_orchestrator_domain::BuildSystem::Maven,
        BuildOperation::Build,
        SLEEP,
        &["30"],
    );

    let error = manager()
        .execute(&target, Some(Duration::from_millis(100)), None)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "BUILD_TIMEOUT");
}

#[tokio::test]
#[cfg(unix)]
async fn a_cancelled_build_is_reported() {
    let workspace = Workspace::new("auth-service");
    let target = workspace.target_with_command(
        service_orchestrator_domain::BuildSystem::Maven,
        BuildOperation::Build,
        SLEEP,
        &["30"],
    );

    let cancellation = Cancellation::new();
    let token = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        token.cancel();
    });

    let error = manager()
        .execute(&target, None, Some(cancellation))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "BUILD_CANCELLED");
}

#[tokio::test]
async fn an_unsupported_operation_is_rejected_before_execution() {
    let workspace = Workspace::new("auth-service");
    let target = workspace.target(
        service_orchestrator_domain::BuildSystem::Maven,
        BuildOperation::InstallDependencies,
    );
    let error = manager().execute(&target, None, None).await.unwrap_err();
    assert_eq!(error.code(), "BUILD_UNSUPPORTED_OPERATION");
}
