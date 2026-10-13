//! End-to-end migration execution through the manager and the process manager.
//!
//! These tests run real processes (tiny local utilities) to exercise the
//! manager's environment-safety policy and its classification of success,
//! non-zero exit, lock conflict, timeout, cancellation, and shared-database
//! coordination. They never invoke Liquibase or a database: a configured command
//! is authoritative, which is exactly the seam provider-independent
//! orchestration relies on (`tasks/011.md` §31).

mod support;

use std::sync::Arc;
use std::time::Duration;

use service_orchestrator_domain::{MigrationOperation, ServiceId};
use service_orchestrator_execution::Cancellation;
use service_orchestrator_liquibase::DatabaseCoordinator;
use support::{config, manager, script, Workspace, SERVICE};

const UPDATE_ECHO: &str = r#"update = { program = "/bin/echo", args = ["applied"] }"#;
const STATUS_ECHO: &str = r#"status = { program = "/bin/echo", args = ["status"] }"#;

#[tokio::test]
#[cfg(unix)]
async fn a_migration_runs_and_reports_its_exit_code() {
    let workspace = Workspace::new("auth-service");
    let config = config(&workspace, "local", UPDATE_ECHO);
    let outcome = manager(config)
        .execute(&workspace.target(MigrationOperation::Apply), None, None)
        .await
        .unwrap();
    assert_eq!(outcome.exit_code, Some(0));
    assert_eq!(outcome.stdout, "applied");
    assert!(outcome.database_identity.contains("local"));
}

#[tokio::test]
#[cfg(unix)]
async fn a_failed_migration_is_reported_with_a_structured_code() {
    let workspace = Workspace::new("auth-service");
    let commands = r#"update = { program = "/usr/bin/false", args = [] }"#;
    let config = config(&workspace, "local", commands);
    let error = manager(config)
        .execute(&workspace.target(MigrationOperation::Apply), None, None)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "MIGRATION_FAILED");
}

#[tokio::test]
#[cfg(unix)]
async fn a_lock_conflict_is_reported() {
    let workspace = Workspace::new("auth-service");
    let program = script(
        &workspace,
        "lock.sh",
        "echo 'Waiting for changelog lock....' >&2\nexit 1",
    );
    let commands = format!(r#"update = {{ program = "{program}", args = [] }}"#);
    let config = config(&workspace, "local", &commands);
    let error = manager(config)
        .execute(&workspace.target(MigrationOperation::Apply), None, None)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "MIGRATION_LOCK_CONFLICT");
}

#[tokio::test]
#[cfg(unix)]
async fn a_timed_out_migration_is_reported() {
    let workspace = Workspace::new("auth-service");
    let commands = r#"update = { program = "/bin/sleep", args = ["30"] }"#;
    let config = config(&workspace, "local", commands);
    let error = manager(config)
        .execute(
            &workspace.target(MigrationOperation::Apply),
            Some(Duration::from_millis(100)),
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), "MIGRATION_TIMEOUT");
}

#[tokio::test]
#[cfg(unix)]
async fn a_cancelled_migration_is_reported() {
    let workspace = Workspace::new("auth-service");
    let commands = r#"update = { program = "/bin/sleep", args = ["30"] }"#;
    let config = config(&workspace, "local", commands);

    let cancellation = Cancellation::new();
    let token = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        token.cancel();
    });

    let error = manager(config)
        .execute(
            &workspace.target(MigrationOperation::Apply),
            None,
            Some(cancellation),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), "MIGRATION_CANCELLED");
}

#[tokio::test]
async fn a_non_local_target_requires_explicit_authorization() {
    let workspace = Workspace::new("auth-service");
    let config = config(&workspace, "production", UPDATE_ECHO);
    let error = manager(config)
        .execute(&workspace.target(MigrationOperation::Apply), None, None)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "MIGRATION_DATABASE_TARGET_NOT_AUTHORIZED");
}

#[tokio::test]
#[cfg(unix)]
async fn an_authorized_non_local_target_runs() {
    let workspace = Workspace::new("auth-service");
    let config = config(&workspace, "production", UPDATE_ECHO);
    let outcome = manager(config)
        .execute(
            &workspace.authorized_target(MigrationOperation::Apply),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(outcome.exit_code, Some(0));
}

#[tokio::test]
#[cfg(unix)]
async fn a_read_only_operation_never_requires_authorization() {
    let workspace = Workspace::new("auth-service");
    let config = config(&workspace, "production", STATUS_ECHO);
    let outcome = manager(config)
        .execute(&workspace.target(MigrationOperation::Status), None, None)
        .await
        .unwrap();
    assert_eq!(outcome.stdout, "status");
}

#[tokio::test]
#[cfg(unix)]
async fn credentials_are_redacted_from_captured_output() {
    let workspace = Workspace::new("auth-service");
    let program = script(
        &workspace,
        "secret.sh",
        "echo 'jdbc:postgresql://db/app?password=hunter2'\necho 'AUTH_TOKEN=abc123'",
    );
    let commands = format!(r#"update = {{ program = "{program}", args = [] }}"#);
    let config = config(&workspace, "local", &commands);
    let outcome = manager(config)
        .execute(&workspace.target(MigrationOperation::Apply), None, None)
        .await
        .unwrap();
    // The JDBC line is dropped wholesale; the token line survives but its
    // value is masked by the shared redaction rule.
    assert!(!outcome.stdout.contains("hunter2"));
    assert!(!outcome.stderr.contains("hunter2"));
    assert!(!outcome.stdout.contains("abc123"));
    assert!(outcome.stdout.contains("AUTH_TOKEN=***"));
}

#[tokio::test]
#[cfg(unix)]
async fn migrations_to_the_same_database_target_are_serialized() {
    let workspace = Workspace::new("auth-service");
    let config = config(&workspace, "local", UPDATE_ECHO);
    let coordinator = DatabaseCoordinator::new();
    let manager = Arc::new(manager(config.clone()).with_coordinator(coordinator.clone()));

    // The coordination key is the declared database identity.
    let key = config.workspaces[0].liquibase[&ServiceId::new(SERVICE).unwrap()]
        .database
        .target_key();

    // Hold the lock, then confirm a migration for the same target waits.
    let guard = coordinator.acquire(&key).await;
    let target = workspace.target(MigrationOperation::Apply);
    let waiting = {
        let manager = manager.clone();
        tokio::spawn(async move { manager.execute(&target, None, None).await })
    };
    tokio::time::sleep(Duration::from_millis(75)).await;
    assert!(!waiting.is_finished(), "a same-target migration must wait");

    drop(guard);
    let outcome = waiting.await.unwrap().unwrap();
    assert_eq!(outcome.stdout, "applied");
}
