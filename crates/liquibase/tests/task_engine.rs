//! Integration tests for running migrations through the task engine.
//!
//! These exercise the whole adapter flow required by `tasks/011.md` §20, §44 of
//! `AGENTS.md`, and the spirit of `tasks/011.md` §30:
//!
//! ```text
//! TaskDefinition (target parameters)
//!     -> TaskEngine -> MigrationTaskExecutor -> MigrationManager
//!     -> MigrationProvider -> MigrationPlan -> MigrationRunner -> process manager
//! ```
//!
//! They assert on real task outcomes (succeeded, failed with a structured code).
//! The configured command is a tiny local utility so no Liquibase or database is
//! invoked.

mod support;

use std::sync::Arc;

use service_orchestrator_domain::{MigrationOperation, Task, TaskKind, TaskStatus};
use service_orchestrator_execution::{ExecutorRegistry, TaskDefinition, TaskEngine};
use service_orchestrator_liquibase::{register_migration_executors, MigrationManager};
use service_orchestrator_process::ProcessManager;
use support::{config, Workspace};

const UPDATE_ECHO: &str = r#"update = { program = "/bin/echo", args = ["applied"] }"#;
const STATUS_ECHO: &str = r#"status = { program = "/bin/echo", args = ["status"] }"#;

/// Builds an engine with the migration executors registered.
fn engine(manager: MigrationManager, max_parallel: usize) -> TaskEngine {
    let manager = Arc::new(manager);
    let mut registry = ExecutorRegistry::new();
    register_migration_executors(&mut registry, manager);
    TaskEngine::builder()
        .max_parallel_tasks(max_parallel)
        .registry(registry)
        .build()
        .expect("the engine builds inside a Tokio runtime")
}

/// Submits a task and waits for it to reach a terminal state.
async fn run(engine: &TaskEngine, definition: TaskDefinition) -> Task {
    let id = engine.submit(definition).expect("task is accepted");
    engine.wait_for(&id).await.expect("the task exists")
}

fn manager_for(workspace: &Workspace, environment: &str, commands: &str) -> MigrationManager {
    let config = config(workspace, environment, commands);
    MigrationManager::new(ProcessManager::new().unwrap(), config)
}

#[tokio::test]
#[cfg(unix)]
async fn applying_a_migration_succeeds_through_the_engine() {
    let workspace = Workspace::new("auth-service");
    let target = workspace.target(MigrationOperation::Apply);
    let engine = engine(manager_for(&workspace, "local", UPDATE_ECHO), 2);

    let task = run(
        &engine,
        TaskDefinition::new(TaskKind::MigrationApply).with_parameters(target.to_parameters()),
    )
    .await;
    assert_eq!(task.status, TaskStatus::Succeeded);
    engine.shutdown().await;
}

#[tokio::test]
#[cfg(unix)]
async fn status_inspection_succeeds_through_the_engine() {
    let workspace = Workspace::new("auth-service");
    let target = workspace.target(MigrationOperation::Status);
    let engine = engine(manager_for(&workspace, "local", STATUS_ECHO), 2);

    let task = run(
        &engine,
        TaskDefinition::new(TaskKind::MigrationStatus).with_parameters(target.to_parameters()),
    )
    .await;
    assert_eq!(task.status, TaskStatus::Succeeded);
    engine.shutdown().await;
}

#[tokio::test]
#[cfg(unix)]
async fn the_legacy_liquibase_kind_applies_migrations() {
    let workspace = Workspace::new("auth-service");
    let target = workspace.target(MigrationOperation::Apply);
    let engine = engine(manager_for(&workspace, "local", UPDATE_ECHO), 2);

    let task = run(
        &engine,
        TaskDefinition::new(TaskKind::Liquibase).with_parameters(target.to_parameters()),
    )
    .await;
    assert_eq!(task.status, TaskStatus::Succeeded);
    engine.shutdown().await;
}

#[tokio::test]
#[cfg(unix)]
async fn a_failing_migration_is_reported_with_a_structured_code() {
    let workspace = Workspace::new("auth-service");
    let target = workspace.target(MigrationOperation::Apply);
    let engine = engine(
        manager_for(
            &workspace,
            "local",
            r#"update = { program = "/usr/bin/false", args = [] }"#,
        ),
        2,
    );

    let task = run(
        &engine,
        TaskDefinition::new(TaskKind::MigrationApply).with_parameters(target.to_parameters()),
    )
    .await;
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(
        task.error.expect("failed tasks carry an error").code,
        "MIGRATION_FAILED"
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn an_unauthorized_target_fails_the_task_with_a_structured_code() {
    let workspace = Workspace::new("auth-service");
    let target = workspace.target(MigrationOperation::Apply);
    let engine = engine(manager_for(&workspace, "production", UPDATE_ECHO), 2);

    let task = run(
        &engine,
        TaskDefinition::new(TaskKind::MigrationApply).with_parameters(target.to_parameters()),
    )
    .await;
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(
        task.error.expect("failed tasks carry an error").code,
        "MIGRATION_DATABASE_TARGET_NOT_AUTHORIZED"
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn a_task_without_target_parameters_fails_with_a_structured_code() {
    let workspace = Workspace::new("auth-service");
    let engine = engine(manager_for(&workspace, "local", UPDATE_ECHO), 2);

    // A migration task that carries no resolved target is malformed.
    let task = run(&engine, TaskDefinition::new(TaskKind::MigrationApply)).await;
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(
        task.error.expect("failed tasks carry an error").code,
        "MIGRATION_MISSING_PARAMETER"
    );
    engine.shutdown().await;
}
