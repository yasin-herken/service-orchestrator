//! Integration tests for running Git work through the task engine.
//!
//! These exercise the whole adapter flow required by `tasks/007.md` §22:
//!
//! ```text
//! TaskDefinition (repository parameters)
//!     -> TaskEngine -> GitTaskExecutor -> CliGitManager -> process manager
//! ```
//!
//! They assert on real task outcomes (succeeded, failed with a structured code,
//! cancelled) and on the per-repository aggregation the bulk summary provides,
//! not on the manager in isolation.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use service_orchestrator_application::{parameter, BulkOperationSummary, GitManager};
use service_orchestrator_domain::{
    Branch, StepId, TaskKind, TaskStatus, WorkflowKind, WorkflowStep,
};
use service_orchestrator_execution::{
    ExecutorRegistry, TaskContext, TaskDefinition, TaskEngine, TaskExecutor, TaskExecutorError,
    TaskOutcome, WorkflowDefinition,
};
use service_orchestrator_git::{register_git_executors, CliGitManager};
use support::{manager, options, Fixture};

/// Builds an engine with the Git executors registered and `max_parallel` slots.
fn engine(max_parallel: usize) -> (TaskEngine, Arc<CliGitManager>) {
    let git = Arc::new(manager());
    let mut registry = ExecutorRegistry::new();
    register_git_executors(&mut registry, git.clone());
    let engine = TaskEngine::builder()
        .max_parallel_tasks(max_parallel)
        .registry(registry)
        .build()
        .expect("the engine builds inside a Tokio runtime");
    (engine, git)
}

/// Submits a Git task carrying a resolved repository and waits for it.
async fn run_git_task(
    engine: &TaskEngine,
    definition: TaskDefinition,
) -> service_orchestrator_domain::Task {
    let id = engine.submit(definition).expect("task is accepted");
    engine.wait_for(&id).await.expect("the task exists")
}

fn fetch_definition(
    repository: &service_orchestrator_application::ResolvedRepository,
) -> TaskDefinition {
    TaskDefinition::new(TaskKind::GitFetch).with_parameters(repository.to_parameters())
}

// -- single tasks ------------------------------------------------------------

#[tokio::test]
async fn fetch_task_succeeds_through_the_engine() {
    let fixture = Fixture::new("main");
    let (engine, _git) = engine(2);

    let task = run_git_task(&engine, fetch_definition(&fixture.repository())).await;
    assert_eq!(task.status, TaskStatus::Succeeded);
    engine.shutdown().await;
}

#[tokio::test]
async fn checkout_task_uses_the_branch_parameter() {
    let fixture = Fixture::new("main");
    fixture.remote_branch("feature/remote");
    let (engine, git) = engine(2);
    let repository = fixture.repository();

    run_git_task(&engine, fetch_definition(&repository)).await;

    let definition = TaskDefinition::new(TaskKind::GitCheckout)
        .with_parameters(repository.to_parameters())
        .with_parameter(parameter::BRANCH, "feature/remote");
    let task = run_git_task(&engine, definition).await;

    assert_eq!(task.status, TaskStatus::Succeeded);
    let status = git.status(&repository, &options()).await.unwrap();
    assert_eq!(
        status.branch.as_ref().map(Branch::as_str),
        Some("feature/remote")
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn a_missing_branch_parameter_fails_with_a_structured_code() {
    let fixture = Fixture::new("main");
    let (engine, _git) = engine(2);

    // A checkout without a branch parameter is a malformed task.
    let definition = TaskDefinition::new(TaskKind::GitCheckout)
        .with_parameters(fixture.repository().to_parameters());
    let task = run_git_task(&engine, definition).await;

    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(
        task.error.expect("failed tasks carry an error").code,
        "GIT_MISSING_PARAMETER"
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn a_git_failure_is_reported_with_its_domain_code() {
    let fixture = Fixture::new("main");
    let (engine, _git) = engine(2);

    let definition = TaskDefinition::new(TaskKind::GitCheckout)
        .with_parameters(fixture.repository().to_parameters())
        .with_parameter(parameter::BRANCH, "does-not-exist");
    let task = run_git_task(&engine, definition).await;

    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(
        task.error.expect("failed tasks carry an error").code,
        "GIT_BRANCH_NOT_FOUND"
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn a_not_cloned_repository_fails_with_a_structured_code() {
    let fixture = Fixture::new("main");
    std::fs::remove_dir_all(&fixture.checkout).unwrap();
    let (engine, _git) = engine(2);

    let task = run_git_task(&engine, fetch_definition(&fixture.repository())).await;
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(
        task.error.expect("failed tasks carry an error").code,
        "GIT_NOT_CLONED"
    );
    engine.shutdown().await;
}

// -- bulk workflows ----------------------------------------------------------

#[tokio::test]
async fn a_bulk_workflow_reports_per_repository_outcomes() {
    let first = Fixture::new("main");
    let second = Fixture::new("main");
    let broken = Fixture::new("main");
    // The third repository is not cloned locally, so its fetch must fail
    // without affecting the other two.
    std::fs::remove_dir_all(&broken.checkout).unwrap();

    let (engine, _git) = engine(4);

    let mut definition =
        WorkflowDefinition::new(WorkflowKind::Custom("fetch_repositories".to_owned()));
    for (position, fixture) in [&first, &second, &broken].iter().enumerate() {
        let mut step = WorkflowStep::new(
            StepId::new(format!("s{position}")).unwrap(),
            TaskKind::GitFetch,
        );
        step.parameters = fixture.repository().to_parameters();
        definition = definition.with_step(step);
    }

    let workflow = engine
        .submit_workflow(definition)
        .expect("workflow is accepted");
    let tasks = engine
        .get_workflow_tasks(&workflow)
        .expect("workflow exists");
    for task in &tasks {
        engine.wait_for(&task.id).await.expect("task exists");
    }

    let tasks = engine.get_workflow_tasks(&workflow).unwrap();
    let summary = BulkOperationSummary::from_tasks(&tasks);
    assert_eq!(summary.total, 3);
    assert_eq!(summary.succeeded, 2);
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.cancelled, 0);
    assert!(summary.is_complete());
    engine.shutdown().await;
}

#[tokio::test]
async fn shards_of_one_workflow_run_concurrently_and_all_succeed() {
    let fixtures: Vec<Fixture> = (0..4).map(|_| Fixture::new("main")).collect();
    let (engine, _git) = engine(4);

    let mut definition =
        WorkflowDefinition::new(WorkflowKind::Custom("fetch_repositories".to_owned()));
    for (position, fixture) in fixtures.iter().enumerate() {
        let mut step = WorkflowStep::new(
            StepId::new(format!("s{position}")).unwrap(),
            TaskKind::GitFetch,
        );
        step.parameters = fixture.repository().to_parameters();
        definition = definition.with_step(step);
    }

    let workflow = engine.submit_workflow(definition).unwrap();
    for task in engine.get_workflow_tasks(&workflow).unwrap() {
        let settled = engine.wait_for(&task.id).await.unwrap();
        assert_eq!(settled.status, TaskStatus::Succeeded);
    }
    engine.shutdown().await;
}

// -- concurrency and cancellation --------------------------------------------

/// An executor that records the peak number of concurrent invocations.
struct CountingExecutor {
    running: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
}

#[async_trait]
impl TaskExecutor for CountingExecutor {
    async fn execute(&self, _context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        let current = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(current, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(40)).await;
        self.running.fetch_sub(1, Ordering::SeqCst);
        Ok(TaskOutcome::succeeded(None))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_engine_honours_its_concurrency_limit() {
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let kind = TaskKind::Custom("count".to_owned());

    let engine = TaskEngine::builder()
        .max_parallel_tasks(2)
        .executor(
            kind.clone(),
            Arc::new(CountingExecutor {
                running: running.clone(),
                peak: peak.clone(),
            }),
        )
        .build()
        .unwrap();

    let mut ids = Vec::new();
    for _ in 0..5 {
        ids.push(engine.submit(TaskDefinition::new(kind.clone())).unwrap());
    }
    for id in &ids {
        engine.wait_for(id).await.unwrap();
    }

    assert_eq!(
        peak.load(Ordering::SeqCst),
        2,
        "concurrency is exactly bounded"
    );
    engine.shutdown().await;
}

/// An executor that only completes when it observes cancellation.
struct SlowExecutor;

#[async_trait]
impl TaskExecutor for SlowExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        tokio::select! {
            () = context.cancelled() => Err(TaskExecutorError::Cancelled),
            () = tokio::time::sleep(Duration::from_millis(250)) => Ok(TaskOutcome::succeeded(None)),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_queued_task_can_be_cancelled_before_it_runs() {
    let fixture = Fixture::new("main");
    let slow = TaskKind::Custom("slow".to_owned());

    let git = Arc::new(manager());
    let mut registry = ExecutorRegistry::new();
    register_git_executors(&mut registry, git);
    registry.register(slow.clone(), Arc::new(SlowExecutor));
    let engine = TaskEngine::builder()
        .max_parallel_tasks(1)
        .registry(registry)
        .build()
        .unwrap();

    let slow_id = engine
        .submit(TaskDefinition::new(slow))
        .expect("slow task is accepted");
    let queued_id = engine
        .submit(fetch_definition(&fixture.repository()))
        .expect("fetch task is accepted");

    // The fetch cannot start while the single slot is held by the slow task.
    engine.cancel(&queued_id).expect("cancellation is accepted");

    let cancelled = engine.wait_for(&queued_id).await.unwrap();
    assert_eq!(cancelled.status, TaskStatus::Cancelled);

    let finished = engine.wait_for(&slow_id).await.unwrap();
    assert_eq!(finished.status, TaskStatus::Succeeded);
    engine.shutdown().await;
}
