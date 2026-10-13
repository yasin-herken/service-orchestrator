//! Ignored scale/performance measurements for the task engine (`tasks/025.md`
//! §12, §13, §31, §37, §38).
//!
//! Everything here uses fake, in-process executors: no Git, Maven, npm, or
//! process is invoked, so the numbers reflect the engine's own scheduling and
//! bookkeeping cost, never network or build speed.
//!
//! Run with:
//!
//! ```bash
//! cargo test --release -p service-orchestrator-execution --test performance \
//!   -- --ignored --nocapture --test-threads=1
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use async_trait::async_trait;
use service_orchestrator_domain::{StepId, TaskKind, TaskStatus, WorkflowKind, WorkflowStep};
use service_orchestrator_execution::{
    TaskContext, TaskDefinition, TaskEngine, TaskExecutor, TaskExecutorError, TaskOutcome,
    WorkflowDefinition,
};

/// Counts invocations and completes immediately.
struct CountingExecutor {
    runs: AtomicUsize,
}

impl CountingExecutor {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            runs: AtomicUsize::new(0),
        })
    }

    fn runs(&self) -> usize {
        self.runs.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl TaskExecutor for CountingExecutor {
    async fn execute(&self, _context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        self.runs.fetch_add(1, Ordering::Relaxed);
        Ok(TaskOutcome::succeeded(None))
    }
}

/// Builds an engine with the fake executor and a fixed concurrency limit.
fn engine_with(exec: Arc<CountingExecutor>, concurrency: usize) -> TaskEngine {
    TaskEngine::builder()
        .max_parallel_tasks(concurrency)
        .executor(TaskKind::GitFetch, exec)
        .build()
        .expect("the engine builds inside the runtime")
}

/// Submits `count` independent tasks and waits for them all to settle,
/// returning (submission time, drain time).
async fn submit_and_drain(engine: &TaskEngine, count: usize) -> (Duration, Duration) {
    let submit_started = Instant::now();
    let mut ids = Vec::with_capacity(count);
    for _ in 0..count {
        ids.push(
            engine
                .submit(TaskDefinition::new(TaskKind::GitFetch))
                .expect("the engine accepts the task"),
        );
    }
    let submit = submit_started.elapsed();

    let drain_started = Instant::now();
    for id in &ids {
        let task = engine.wait_for(id).await.expect("the task settles");
        debug_assert_eq!(task.status, TaskStatus::Succeeded);
    }
    (submit, drain_started.elapsed())
}

/// Drains throughput over three rounds while terminal history accumulates.
///
/// If scheduling or the cancellation scan are linear in retained history, the
/// drain time per round grows with the round number even though each round does
/// identical work.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "measurement; run with --ignored --nocapture"]
async fn measure_drain_throughput_with_growing_history() {
    const ROUND: usize = 2_000;
    let exec = CountingExecutor::new();
    let engine = engine_with(Arc::clone(&exec), 4);

    for round in 0..3usize {
        let history = round * ROUND;
        let (submit, drain) = submit_and_drain(&engine, ROUND).await;
        let per_task = drain / u32::try_from(ROUND).unwrap();
        eprintln!(
            "drain: history={history:>6} tasks, submit={submit:?}, drain={drain:?} \
             ({per_task:?}/task)"
        );
    }
    assert_eq!(exec.runs(), 3 * ROUND);
}

/// Submission throughput in isolation (execution is throttled by concurrency).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "measurement; run with --ignored --nocapture"]
async fn measure_submission_throughput() {
    const COUNT: usize = 20_000;
    let exec = CountingExecutor::new();
    let engine = engine_with(Arc::clone(&exec), 4);

    let started = Instant::now();
    let mut ids = Vec::with_capacity(COUNT);
    for _ in 0..COUNT {
        ids.push(
            engine
                .submit(TaskDefinition::new(TaskKind::GitFetch))
                .expect("the engine accepts the task"),
        );
    }
    let submit = started.elapsed();
    eprintln!(
        "submit: {COUNT} tasks in {submit:?} ({:?}/task)",
        submit / u32::try_from(COUNT).unwrap()
    );

    for id in &ids {
        let _ = engine.wait_for(id).await;
    }
    assert_eq!(exec.runs(), COUNT);
}

/// Plans and runs a deep (chain) workflow, where each step depends on the
/// previous one, at increasing widths.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "measurement; run with --ignored --nocapture"]
async fn measure_deep_workflow_planning() {
    for steps in [100usize, 500, 1_000] {
        let exec = CountingExecutor::new();
        let engine = engine_with(Arc::clone(&exec), 4);

        let mut definition = WorkflowDefinition::new(WorkflowKind::PrepareEnvironment);
        let mut previous: Option<StepId> = None;
        for index in 0..steps {
            let step_id = StepId::new(format!("s{index}")).expect("valid step id");
            let mut step = WorkflowStep::new(step_id.clone(), TaskKind::GitFetch);
            if let Some(previous) = previous.take() {
                step = step.after(previous);
            }
            definition = definition.with_step(step);
            previous = Some(step_id);
        }

        let started = Instant::now();
        let workflow = engine
            .submit_workflow(definition)
            .expect("the workflow is accepted");
        let plan = started.elapsed();

        let run_started = Instant::now();
        loop {
            let tasks = engine
                .get_workflow_tasks(&workflow)
                .expect("the workflow exists");
            if tasks.iter().all(|task| task.status.is_terminal()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let run = run_started.elapsed();
        eprintln!("workflow: steps={steps:>5}, plan={plan:?}, run={run:?}");
        assert_eq!(exec.runs(), steps);
    }
}
