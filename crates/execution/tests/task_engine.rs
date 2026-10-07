//! End-to-end tests for the task engine and scheduler.
//!
//! Everything here drives the *public* engine API with fake executors. No real
//! Git, Maven, npm, Liquibase, or process is ever invoked; the only dependency
//! on Tokio is the async runtime and its synchronization primitives, which the
//! tests use to coordinate deterministically instead of sleeping.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use service_orchestrator_domain::{
    StepId, TaskId, TaskKind, TaskStatus, WorkflowKind, WorkflowStep,
};
use service_orchestrator_execution::{
    DependencyPolicy, RetryPolicy, TaskContext, TaskDefinition, TaskEngine, TaskEngineError,
    TaskEvent, TaskExecutor, TaskExecutorError, TaskOutcome, WorkflowDefinition,
};
use tokio::sync::{broadcast, watch};

// ---------------------------------------------------------------------------
// Test executors
// ---------------------------------------------------------------------------

/// Counts how many times it ran and reports progress and a final message.
struct RecordingExecutor {
    runs: AtomicUsize,
}

impl RecordingExecutor {
    fn new() -> Self {
        Self {
            runs: AtomicUsize::new(0),
        }
    }

    fn runs(&self) -> usize {
        self.runs.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl TaskExecutor for RecordingExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        context
            .progress()
            .set_progress(1, Some(1), Some("working".to_owned()))
            .map_err(|error| TaskExecutorError::internal(error.to_string()))?;
        context
            .progress()
            .message("finished")
            .map_err(|error| TaskExecutorError::internal(error.to_string()))?;
        Ok(TaskOutcome::succeeded(None))
    }
}

/// Always fails with a fixed code.
struct FailingExecutor {
    code: &'static str,
}

impl FailingExecutor {
    const fn always(code: &'static str) -> Self {
        Self { code }
    }
}

#[async_trait]
impl TaskExecutor for FailingExecutor {
    async fn execute(&self, _context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        Err(TaskExecutorError::failed(self.code, "intentional failure"))
    }
}

/// Fails the first `fail_until` attempts, then succeeds.
struct FlakyExecutor {
    attempts: AtomicUsize,
    fail_until: usize,
}

impl FlakyExecutor {
    fn new(fail_until: usize) -> Self {
        Self {
            attempts: AtomicUsize::new(0),
            fail_until,
        }
    }

    fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl TaskExecutor for FlakyExecutor {
    async fn execute(&self, _context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst) + 1;
        if attempt <= self.fail_until {
            Err(TaskExecutorError::failed(
                "FLAKY",
                format!("attempt {attempt} failed"),
            ))
        } else {
            Ok(TaskOutcome::succeeded(None))
        }
    }
}

/// Never completes on its own; used to exercise timeouts.
struct HangingExecutor;

#[async_trait]
impl TaskExecutor for HangingExecutor {
    async fn execute(&self, _context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        tokio::time::sleep(Duration::from_secs(30)).await;
        Ok(TaskOutcome::succeeded(None))
    }
}

/// Records the order in which tasks of different kinds run.
struct OrderExecutor {
    order: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl TaskExecutor for OrderExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        self.order
            .lock()
            .expect("order mutex poisoned")
            .push(context.kind().label().to_owned());
        Ok(TaskOutcome::succeeded(None))
    }
}

// ---------------------------------------------------------------------------
// Test coordination primitives
// ---------------------------------------------------------------------------

/// A level-triggered one-way latch. Waiting is race-free: a waiter that
/// observes an unopened latch is guaranteed to be woken by the next `open`.
#[derive(Clone)]
struct Latch {
    tx: Arc<watch::Sender<bool>>,
}

impl Latch {
    fn new() -> Self {
        let (tx, _receiver) = watch::channel(false);
        Self { tx: Arc::new(tx) }
    }

    fn open(&self) {
        // `send_replace` always stores the value, even with no receivers, so a
        // later subscriber still observes the opened latch.
        self.tx.send_replace(true);
    }

    async fn wait(&self) {
        let mut receiver = self.tx.subscribe();
        loop {
            if *receiver.borrow() {
                return;
            }
            if receiver.changed().await.is_err() {
                return;
            }
        }
    }
}

/// A snapshot of gate activity, broadcast through a watch channel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct GateProgress {
    started: usize,
    finished: usize,
    active: usize,
    max_active: usize,
}

struct GateInner {
    progress: GateProgress,
    started_order: Vec<String>,
    tx: watch::Sender<GateProgress>,
}

/// Tracks how many executors are inside `execute` at once.
#[derive(Clone)]
struct Gate {
    inner: Arc<Mutex<GateInner>>,
}

impl Gate {
    fn new() -> Self {
        let (tx, _receiver) = watch::channel(GateProgress::default());
        Self {
            inner: Arc::new(Mutex::new(GateInner {
                progress: GateProgress::default(),
                started_order: Vec::new(),
                tx,
            })),
        }
    }

    fn subscribe(&self) -> watch::Receiver<GateProgress> {
        self.inner
            .lock()
            .expect("gate mutex poisoned")
            .tx
            .subscribe()
    }

    fn snapshot(&self) -> GateProgress {
        self.inner.lock().expect("gate mutex poisoned").progress
    }

    fn started_order(&self) -> Vec<String> {
        self.inner
            .lock()
            .expect("gate mutex poisoned")
            .started_order
            .clone()
    }

    fn enter(&self, id: &TaskId) {
        let mut inner = self.inner.lock().expect("gate mutex poisoned");
        inner.progress.started += 1;
        inner.progress.active += 1;
        inner.progress.max_active = inner.progress.max_active.max(inner.progress.active);
        inner.started_order.push(id.to_string());
        let snapshot = inner.progress;
        inner.tx.send_replace(snapshot);
    }

    fn exit(&self) {
        let mut inner = self.inner.lock().expect("gate mutex poisoned");
        inner.progress.finished += 1;
        inner.progress.active -= 1;
        let snapshot = inner.progress;
        inner.tx.send_replace(snapshot);
    }
}

/// Blocks inside `execute` until the latch opens, then succeeds. Observes
/// cancellation cooperatively.
struct GateExecutor {
    gate: Gate,
    latch: Latch,
}

#[async_trait]
impl TaskExecutor for GateExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        self.gate.enter(context.task_id());
        tokio::select! {
            () = self.latch.wait() => {
                self.gate.exit();
                Ok(TaskOutcome::succeeded(None))
            }
            () = context.cancelled() => {
                self.gate.exit();
                Err(TaskExecutorError::Cancelled)
            }
        }
    }
}

async fn wait_for_gate(gate: &Gate, predicate: impl Fn(GateProgress) -> bool) {
    let mut receiver = gate.subscribe();
    loop {
        if predicate(*receiver.borrow()) {
            return;
        }
        receiver.changed().await.expect("gate sender dropped");
    }
}

// ---------------------------------------------------------------------------
// Event helpers
// ---------------------------------------------------------------------------

fn is_terminal_for(event: &TaskEvent, id: &TaskId) -> bool {
    match event {
        TaskEvent::TaskSucceeded { id: task }
        | TaskEvent::TaskFailed { id: task, .. }
        | TaskEvent::TaskCancelled { id: task }
        | TaskEvent::TaskSkipped { id: task } => task == id,
        _ => false,
    }
}

/// Collects events until `id` reaches a terminal state.
async fn drain_until_terminal(
    receiver: &mut broadcast::Receiver<TaskEvent>,
    id: &TaskId,
) -> Vec<TaskEvent> {
    let mut events = Vec::new();
    let collect = async {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let terminal = is_terminal_for(&event, id);
                    events.push(event);
                    if terminal {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), collect)
        .await
        .expect("terminal event should be emitted");
    events
}

// ---------------------------------------------------------------------------
// Lifecycle and state
// ---------------------------------------------------------------------------

#[tokio::test]
async fn single_task_runs_to_success_with_progress_and_events() {
    let exec = Arc::new(RecordingExecutor::new());
    let engine = TaskEngine::builder()
        .max_parallel_tasks(2)
        .executor(TaskKind::Validate, exec.clone())
        .build()
        .unwrap();
    let mut events = engine.subscribe();

    let id = engine
        .submit(TaskDefinition::new(TaskKind::Validate).with_description("validate config"))
        .unwrap();

    let task = engine.wait_for(&id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Succeeded);
    assert_eq!(exec.runs(), 1);
    assert_eq!(task.progress.current, 1);
    assert_eq!(task.progress.total, Some(1));
    assert_eq!(task.progress.message.as_deref(), Some("finished"));
    assert!(task.started_at.is_some());
    assert!(task.finished_at.is_some());

    let drained = drain_until_terminal(&mut events, &id).await;
    assert!(drained
        .iter()
        .any(|event| matches!(event, TaskEvent::TaskCreated { .. })));
    assert!(drained
        .iter()
        .any(|event| matches!(event, TaskEvent::TaskStarted { .. })));
    assert!(drained
        .iter()
        .any(|event| matches!(event, TaskEvent::TaskProgress { .. })));
    assert!(drained
        .iter()
        .any(|event| matches!(event, TaskEvent::TaskSucceeded { id: task } if task == &id)));
}

#[tokio::test]
async fn missing_executor_fails_the_task() {
    let engine = TaskEngine::builder().build().unwrap();
    let id = engine
        .submit(TaskDefinition::new(TaskKind::MavenBuild))
        .unwrap();
    let task = engine.wait_for(&id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(task.error.unwrap().code, "EXECUTOR_NOT_FOUND");
}

#[tokio::test]
async fn task_queries_report_state() {
    let engine = TaskEngine::builder()
        .max_parallel_tasks(1)
        .executor(TaskKind::Validate, Arc::new(RecordingExecutor::new()))
        .build()
        .unwrap();
    let id = engine
        .submit(TaskDefinition::new(TaskKind::Validate))
        .unwrap();
    engine.wait_for(&id).await.unwrap();

    assert_eq!(engine.get_task(&id).unwrap().status, TaskStatus::Succeeded);
    assert_eq!(engine.list_tasks().len(), 1);
    assert!(engine.list_running_tasks().is_empty());
    assert_eq!(engine.list_recent_tasks(10).len(), 1);
}

#[tokio::test]
async fn unknown_task_operations_return_not_found() {
    let engine = TaskEngine::builder().build().unwrap();
    let missing = TaskId::new("missing").unwrap();
    assert!(matches!(
        engine.get_task(&missing),
        Err(TaskEngineError::TaskNotFound(_))
    ));
    assert!(matches!(
        engine.cancel(&missing),
        Err(TaskEngineError::TaskNotFound(_))
    ));
    assert!(matches!(
        engine.wait_for(&missing).await,
        Err(TaskEngineError::TaskNotFound(_))
    ));
}

#[tokio::test]
async fn submit_rejects_unknown_dependency() {
    let engine = TaskEngine::builder().build().unwrap();
    let missing = TaskId::new("nope").unwrap();
    let error = engine
        .submit(TaskDefinition::new(TaskKind::Validate).depends_on(missing))
        .unwrap_err();
    assert!(matches!(error, TaskEngineError::DependencyNotFound { .. }));
}

// ---------------------------------------------------------------------------
// Dependencies and failure propagation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dependent_task_starts_blocked_and_runs_after_its_dependency() {
    let gate = Gate::new();
    let latch = Latch::new();
    let engine = TaskEngine::builder()
        .max_parallel_tasks(1)
        .executor(
            TaskKind::GitFetch,
            Arc::new(GateExecutor {
                gate: gate.clone(),
                latch: latch.clone(),
            }),
        )
        .build()
        .unwrap();

    let first = engine
        .submit(TaskDefinition::new(TaskKind::GitFetch))
        .unwrap();
    let second = engine
        .submit(TaskDefinition::new(TaskKind::GitFetch).depends_on(first.clone()))
        .unwrap();

    wait_for_gate(&gate, |progress| progress.started == 1).await;
    assert_eq!(
        engine.get_task(&second).unwrap().status,
        TaskStatus::Blocked
    );

    latch.open();
    assert_eq!(
        engine.wait_for(&first).await.unwrap().status,
        TaskStatus::Succeeded
    );
    assert_eq!(
        engine.wait_for(&second).await.unwrap().status,
        TaskStatus::Succeeded
    );
}

#[tokio::test]
async fn failed_dependency_skips_the_dependent_task() {
    let engine = TaskEngine::builder()
        .executor(
            TaskKind::Validate,
            Arc::new(FailingExecutor::always("BOOM")),
        )
        .build()
        .unwrap();

    let first = engine
        .submit(TaskDefinition::new(TaskKind::Validate))
        .unwrap();
    let second = engine
        .submit(TaskDefinition::new(TaskKind::Validate).depends_on(first.clone()))
        .unwrap();

    assert_eq!(
        engine.wait_for(&first).await.unwrap().status,
        TaskStatus::Failed
    );
    assert_eq!(
        engine.wait_for(&second).await.unwrap().status,
        TaskStatus::Skipped
    );
}

#[tokio::test]
async fn any_completed_policy_runs_after_a_failure() {
    let downstream = Arc::new(RecordingExecutor::new());
    let engine = TaskEngine::builder()
        .executor(
            TaskKind::Validate,
            Arc::new(FailingExecutor::always("BOOM")),
        )
        .executor(TaskKind::Custom("after".to_owned()), downstream.clone())
        .build()
        .unwrap();

    let first = engine
        .submit(TaskDefinition::new(TaskKind::Validate))
        .unwrap();
    let second = engine
        .submit(
            TaskDefinition::new(TaskKind::Custom("after".to_owned()))
                .depends_on(first.clone())
                .with_dependency_policy(DependencyPolicy::AnyCompleted),
        )
        .unwrap();

    assert_eq!(
        engine.wait_for(&first).await.unwrap().status,
        TaskStatus::Failed
    );
    assert_eq!(
        engine.wait_for(&second).await.unwrap().status,
        TaskStatus::Succeeded
    );
    assert_eq!(downstream.runs(), 1);
}

// ---------------------------------------------------------------------------
// Concurrency
// ---------------------------------------------------------------------------

#[tokio::test]
async fn independent_tasks_run_concurrently_up_to_the_limit() {
    let gate = Gate::new();
    let latch = Latch::new();
    let engine = TaskEngine::builder()
        .max_parallel_tasks(2)
        .executor(
            TaskKind::GitFetch,
            Arc::new(GateExecutor {
                gate: gate.clone(),
                latch: latch.clone(),
            }),
        )
        .build()
        .unwrap();

    let ids: Vec<TaskId> = (0..3)
        .map(|_| {
            engine
                .submit(TaskDefinition::new(TaskKind::GitFetch))
                .unwrap()
        })
        .collect();

    wait_for_gate(&gate, |progress| progress.started == 2).await;
    assert_eq!(gate.snapshot().max_active, 2);
    assert_eq!(gate.snapshot().finished, 0);

    latch.open();
    for id in &ids {
        assert_eq!(
            engine.wait_for(id).await.unwrap().status,
            TaskStatus::Succeeded
        );
    }
    assert_eq!(gate.snapshot().max_active, 2);
    assert_eq!(gate.snapshot().started, 3);
    assert_eq!(gate.snapshot().finished, 3);
}

#[tokio::test]
async fn scheduler_runs_ready_tasks_in_submission_order() {
    let gate = Gate::new();
    let latch = Latch::new();
    let engine = TaskEngine::builder()
        .max_parallel_tasks(1)
        .executor(
            TaskKind::GitFetch,
            Arc::new(GateExecutor {
                gate: gate.clone(),
                latch: latch.clone(),
            }),
        )
        .build()
        .unwrap();

    let ids: Vec<TaskId> = (0..3)
        .map(|_| {
            engine
                .submit(TaskDefinition::new(TaskKind::GitFetch))
                .unwrap()
        })
        .collect();

    wait_for_gate(&gate, |progress| progress.started == 1).await;
    assert_eq!(gate.started_order(), vec![ids[0].to_string()]);

    latch.open();
    wait_for_gate(&gate, |progress| progress.finished == 3).await;
    for id in &ids {
        assert_eq!(
            engine.wait_for(id).await.unwrap().status,
            TaskStatus::Succeeded
        );
    }
    assert_eq!(
        gate.started_order(),
        ids.iter().map(ToString::to_string).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Cancellation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancel_running_task_transitions_to_cancelled() {
    let gate = Gate::new();
    let latch = Latch::new();
    let engine = TaskEngine::builder()
        .max_parallel_tasks(1)
        .executor(
            TaskKind::GitFetch,
            Arc::new(GateExecutor {
                gate: gate.clone(),
                latch: latch.clone(),
            }),
        )
        .build()
        .unwrap();

    let id = engine
        .submit(TaskDefinition::new(TaskKind::GitFetch))
        .unwrap();
    wait_for_gate(&gate, |progress| progress.started == 1).await;

    engine.cancel(&id).unwrap();
    let task = engine.wait_for(&id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Cancelled);
    assert_eq!(gate.snapshot().finished, 1);
}

#[tokio::test]
async fn cancel_queued_task_is_observed_when_it_is_admitted() {
    let gate = Gate::new();
    let latch = Latch::new();
    let engine = TaskEngine::builder()
        .max_parallel_tasks(1)
        .executor(
            TaskKind::GitFetch,
            Arc::new(GateExecutor {
                gate: gate.clone(),
                latch: latch.clone(),
            }),
        )
        .build()
        .unwrap();

    let first = engine
        .submit(TaskDefinition::new(TaskKind::GitFetch))
        .unwrap();
    let second = engine
        .submit(TaskDefinition::new(TaskKind::GitFetch))
        .unwrap();

    wait_for_gate(&gate, |progress| progress.started == 1).await;
    engine.cancel(&second).unwrap();
    assert_eq!(engine.get_task(&second).unwrap().status, TaskStatus::Queued);

    latch.open();
    assert_eq!(
        engine.wait_for(&first).await.unwrap().status,
        TaskStatus::Succeeded
    );
    assert_eq!(
        engine.wait_for(&second).await.unwrap().status,
        TaskStatus::Cancelled
    );
    // The cancelled queued task never invoked its executor.
    assert_eq!(gate.snapshot().started, 1);
}

// ---------------------------------------------------------------------------
// Timeout and retry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn task_timeout_produces_a_structured_failure() {
    let engine = TaskEngine::builder()
        .default_timeout(Duration::from_millis(50))
        .executor(TaskKind::MavenBuild, Arc::new(HangingExecutor))
        .build()
        .unwrap();

    let id = engine
        .submit(TaskDefinition::new(TaskKind::MavenBuild))
        .unwrap();
    let task = engine.wait_for(&id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(task.error.unwrap().code, "TASK_TIMEOUT");
}

#[tokio::test]
async fn retry_policy_retries_until_success() {
    let flaky = Arc::new(FlakyExecutor::new(2));
    let engine = TaskEngine::builder()
        .executor(TaskKind::BuildService, flaky.clone())
        .build()
        .unwrap();
    let mut events = engine.subscribe();

    let id = engine
        .submit(TaskDefinition::new(TaskKind::BuildService).with_retry(RetryPolicy::attempts(3)))
        .unwrap();
    let task = engine.wait_for(&id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Succeeded);
    assert_eq!(flaky.attempts(), 3);

    let drained = drain_until_terminal(&mut events, &id).await;
    let retries: Vec<u32> = drained
        .iter()
        .filter_map(|event| match event {
            TaskEvent::TaskRetrying { attempt, .. } => Some(*attempt),
            _ => None,
        })
        .collect();
    assert_eq!(retries, vec![2, 3]);
}

#[tokio::test]
async fn retry_policy_gives_up_after_max_attempts() {
    let flaky = Arc::new(FlakyExecutor::new(100));
    let engine = TaskEngine::builder()
        .executor(TaskKind::BuildService, flaky.clone())
        .build()
        .unwrap();

    let id = engine
        .submit(TaskDefinition::new(TaskKind::BuildService).with_retry(RetryPolicy::attempts(2)))
        .unwrap();
    let task = engine.wait_for(&id).await.unwrap();
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(flaky.attempts(), 2);
    assert_eq!(task.error.unwrap().code, "FLAKY");
}

// ---------------------------------------------------------------------------
// Backpressure and shutdown
// ---------------------------------------------------------------------------

#[tokio::test]
async fn backpressure_rejects_work_beyond_max_queued_tasks() {
    let gate = Gate::new();
    let latch = Latch::new();
    let engine = TaskEngine::builder()
        .max_parallel_tasks(1)
        .max_queued_tasks(1)
        .executor(
            TaskKind::GitFetch,
            Arc::new(GateExecutor {
                gate: gate.clone(),
                latch: latch.clone(),
            }),
        )
        .build()
        .unwrap();

    let first = engine
        .submit(TaskDefinition::new(TaskKind::GitFetch))
        .unwrap();
    wait_for_gate(&gate, |progress| progress.started == 1).await;

    let error = engine
        .submit(TaskDefinition::new(TaskKind::GitFetch))
        .unwrap_err();
    assert!(matches!(error, TaskEngineError::QueueFull { limit: 1 }));

    latch.open();
    assert_eq!(
        engine.wait_for(&first).await.unwrap().status,
        TaskStatus::Succeeded
    );
}

#[tokio::test]
async fn shutdown_cancels_running_tasks_and_rejects_new_work() {
    let gate = Gate::new();
    let latch = Latch::new();
    let engine = TaskEngine::builder()
        .max_parallel_tasks(1)
        .executor(
            TaskKind::GitFetch,
            Arc::new(GateExecutor {
                gate: gate.clone(),
                latch: latch.clone(),
            }),
        )
        .build()
        .unwrap();

    let id = engine
        .submit(TaskDefinition::new(TaskKind::GitFetch))
        .unwrap();
    wait_for_gate(&gate, |progress| progress.started == 1).await;

    engine.shutdown().await;
    assert!(engine.is_shutdown());

    assert_eq!(
        engine.wait_for(&id).await.unwrap().status,
        TaskStatus::Cancelled
    );

    let error = engine
        .submit(TaskDefinition::new(TaskKind::GitFetch))
        .unwrap_err();
    assert!(matches!(error, TaskEngineError::ShuttingDown));
}

// ---------------------------------------------------------------------------
// Workflows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn workflow_executes_steps_in_dependency_order() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let exec = Arc::new(OrderExecutor {
        order: order.clone(),
    });

    let mut builder = TaskEngine::builder().max_parallel_tasks(2);
    for name in ["a", "b", "c"] {
        builder = builder.executor(TaskKind::Custom(name.to_owned()), exec.clone());
    }
    let engine = builder.build().unwrap();

    let step = |name: &str| {
        WorkflowStep::new(
            StepId::new(name).unwrap(),
            TaskKind::Custom(name.to_owned()),
        )
    };
    let workflow = WorkflowDefinition::new(WorkflowKind::PrepareEnvironment)
        .with_step(step("a"))
        .with_step(step("b").after(StepId::new("a").unwrap()))
        .with_step(step("c").after(StepId::new("b").unwrap()));

    let workflow_id = engine.submit_workflow(workflow).unwrap();

    let tasks: Vec<_> = engine
        .list_tasks()
        .into_iter()
        .filter(|task| task.workflow.as_ref() == Some(&workflow_id))
        .collect();
    assert_eq!(tasks.len(), 3);
    for task in &tasks {
        assert_eq!(
            engine.wait_for(&task.id).await.unwrap().status,
            TaskStatus::Succeeded
        );
    }

    assert_eq!(
        *order.lock().unwrap(),
        vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]
    );
}

#[tokio::test]
async fn workflow_with_a_cycle_is_rejected() {
    let engine = TaskEngine::builder().build().unwrap();
    let first = StepId::new("a").unwrap();
    let second = StepId::new("b").unwrap();
    let workflow = WorkflowDefinition::new(WorkflowKind::SyncWorkspace)
        .with_step(WorkflowStep::new(first.clone(), TaskKind::GitSync).after(second.clone()))
        .with_step(WorkflowStep::new(second, TaskKind::GitSync).after(first));

    let error = engine.submit_workflow(workflow).unwrap_err();
    assert!(matches!(error, TaskEngineError::DependencyCycle { .. }));
}

#[tokio::test]
async fn empty_workflow_is_rejected() {
    let engine = TaskEngine::builder().build().unwrap();
    let error = engine
        .submit_workflow(WorkflowDefinition::new(WorkflowKind::SyncWorkspace))
        .unwrap_err();
    assert!(matches!(error, TaskEngineError::InvalidWorkflow { .. }));
}
