//! Deterministic scheduler stress and regression tests (`tasks/027.md` §11).
//!
//! These complement the focused behavioural tests in `task_engine.rs` and the
//! opt-in `#[ignore]`d scale measurements in `performance.rs`. Everything here
//! runs **by default**, is bounded, and uses fake in-process executors and
//! explicit synchronization — never a sleep race, never an external command.
//!
//! The invariants under test:
//!
//! - the configured concurrency limit is never exceeded;
//! - a task is never dispatched more than once;
//! - no task starts before every dependency has succeeded;
//! - a cancellation storm settles every task and leaves nothing pending;
//! - a wide graph completes without a deadlock.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use service_orchestrator_domain::{TaskId, TaskKind, TaskStatus};
use service_orchestrator_execution::{
    TaskContext, TaskDefinition, TaskEngine, TaskExecutor, TaskExecutorError, TaskOutcome,
};
use tokio::sync::watch;

/// How long any single stress test may take before it is treated as a hang.
const DEADLINE: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// A gate that blocks executors until released, tracking concurrency and runs.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct GateSnapshot {
    started: usize,
    finished: usize,
    active: usize,
    max_active: usize,
}

struct GateInner {
    snapshot: GateSnapshot,
    runs: HashMap<TaskId, usize>,
    tx: watch::Sender<GateSnapshot>,
}

/// Tracks concurrent executor occupancy and per-task invocation counts.
#[derive(Clone)]
struct Gate {
    inner: Arc<Mutex<GateInner>>,
}

impl Gate {
    fn new() -> Self {
        let (tx, _receiver) = watch::channel(GateSnapshot::default());
        Self {
            inner: Arc::new(Mutex::new(GateInner {
                snapshot: GateSnapshot::default(),
                runs: HashMap::new(),
                tx,
            })),
        }
    }

    fn subscribe(&self) -> watch::Receiver<GateSnapshot> {
        self.inner
            .lock()
            .expect("gate mutex poisoned")
            .tx
            .subscribe()
    }

    fn snapshot(&self) -> GateSnapshot {
        self.inner.lock().expect("gate mutex poisoned").snapshot
    }

    fn runs_for(&self, id: &TaskId) -> usize {
        self.inner
            .lock()
            .expect("gate mutex poisoned")
            .runs
            .get(id)
            .copied()
            .unwrap_or(0)
    }

    fn enter(&self, id: &TaskId) {
        let mut inner = self.inner.lock().expect("gate mutex poisoned");
        inner.snapshot.started += 1;
        inner.snapshot.active += 1;
        inner.snapshot.max_active = inner.snapshot.max_active.max(inner.snapshot.active);
        *inner.runs.entry(id.clone()).or_insert(0) += 1;
        let snapshot = inner.snapshot;
        inner.tx.send_replace(snapshot);
    }

    fn exit(&self) {
        let mut inner = self.inner.lock().expect("gate mutex poisoned");
        inner.snapshot.finished += 1;
        inner.snapshot.active -= 1;
        let snapshot = inner.snapshot;
        inner.tx.send_replace(snapshot);
    }

    /// Waits until `predicate` holds, using the watch channel so there is no
    /// poll-and-sleep race.
    async fn wait_for(&self, predicate: impl Fn(GateSnapshot) -> bool) {
        let mut receiver = self.subscribe();
        loop {
            if predicate(*receiver.borrow()) {
                return;
            }
            receiver.changed().await.expect("gate sender dropped");
        }
    }
}

/// Blocks in `execute` until the gate opens, observing cancellation first.
struct GatedExecutor {
    gate: Gate,
    release: watch::Receiver<bool>,
}

#[async_trait]
impl TaskExecutor for GatedExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        self.gate.enter(context.task_id());
        let mut release = self.release.clone();
        tokio::select! {
            () = async {
                loop {
                    if *release.borrow() {
                        return;
                    }
                    if release.changed().await.is_err() {
                        return;
                    }
                }
            } => {
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

/// Completes immediately, recording its occupancy and invocation in the gate.
struct ImmediateExecutor {
    gate: Gate,
}

#[async_trait]
impl TaskExecutor for ImmediateExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        self.gate.enter(context.task_id());
        // Yield so the scheduler can admit other ready tasks before this one
        // finishes, without ever sleeping.
        tokio::task::yield_now().await;
        self.gate.exit();
        Ok(TaskOutcome::succeeded(None))
    }
}

/// Records a globally ordered start and finish sequence for every task, so the
/// test can verify dependency edges after the graph settles.
#[derive(Default)]
struct SequenceRecorder {
    counter: AtomicU64,
    starts: Mutex<HashMap<TaskId, u64>>,
    finishes: Mutex<HashMap<TaskId, u64>>,
}

impl SequenceRecorder {
    fn start(&self, id: &TaskId) {
        let seq = self.counter.fetch_add(1, Ordering::SeqCst);
        self.starts
            .lock()
            .expect("starts mutex poisoned")
            .insert(id.clone(), seq);
    }

    fn finish(&self, id: &TaskId) {
        let seq = self.counter.fetch_add(1, Ordering::SeqCst);
        self.finishes
            .lock()
            .expect("finishes mutex poisoned")
            .insert(id.clone(), seq);
    }

    fn start_of(&self, id: &TaskId) -> u64 {
        *self
            .starts
            .lock()
            .expect("starts mutex poisoned")
            .get(id)
            .expect("the task started")
    }

    fn finish_of(&self, id: &TaskId) -> u64 {
        *self
            .finishes
            .lock()
            .expect("finishes mutex poisoned")
            .get(id)
            .expect("the task finished")
    }
}

/// Records start/finish order and runs each task's body exactly once.
struct SequencingExecutor {
    recorder: Arc<SequenceRecorder>,
    runs: AtomicUsize,
}

#[async_trait]
impl TaskExecutor for SequencingExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        let id = context.task_id().clone();
        self.runs.fetch_add(1, Ordering::SeqCst);
        self.recorder.start(&id);
        tokio::task::yield_now().await;
        self.recorder.finish(&id);
        Ok(TaskOutcome::succeeded(None))
    }
}

/// Waits for every task to reach a terminal state, bounded by [`DEADLINE`].
async fn settle_all(engine: &TaskEngine, ids: &[TaskId]) -> Vec<TaskStatus> {
    let wait = async {
        let mut statuses = Vec::with_capacity(ids.len());
        for id in ids {
            statuses.push(engine.wait_for(id).await.expect("the task settles").status);
        }
        statuses
    };
    tokio::time::timeout(DEADLINE, wait)
        .await
        .expect("every task reaches a terminal state before the deadline")
}

/// Asserts every task ran at most once.
fn assert_no_duplicate_execution(gate: &Gate, ids: &[TaskId]) {
    for id in ids {
        assert!(
            gate.runs_for(id) <= 1,
            "task {id} was dispatched more than once"
        );
    }
}

// ---------------------------------------------------------------------------
// Concurrency limit under load
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_hundred_tasks_never_exceed_the_concurrency_limit_or_run_twice() {
    const LIMIT: usize = 4;
    const TASKS: usize = 100;

    let gate = Gate::new();
    let (release_tx, release_rx) = watch::channel(false);
    let engine = TaskEngine::builder()
        .max_parallel_tasks(LIMIT)
        .executor(
            TaskKind::Validate,
            Arc::new(GatedExecutor {
                gate: gate.clone(),
                release: release_rx,
            }),
        )
        .build()
        .expect("a Tokio runtime is active");

    let ids: Vec<TaskId> = (0..TASKS)
        .map(|_| {
            engine
                .submit(TaskDefinition::new(TaskKind::Validate))
                .expect("the engine accepts the task")
        })
        .collect();

    // Hold the gate closed until exactly `LIMIT` executors are inside it. The
    // scheduler must not admit a further task while the limit is saturated.
    gate.wait_for(|snapshot| snapshot.active == LIMIT).await;
    assert_eq!(
        gate.snapshot().started,
        LIMIT,
        "the scheduler admits exactly the concurrency limit while blocked"
    );
    assert_eq!(
        gate.snapshot().max_active,
        LIMIT,
        "the limit is reached but never exceeded"
    );

    release_tx.send_replace(true);
    let statuses = settle_all(&engine, &ids).await;

    assert!(
        statuses
            .iter()
            .all(|status| *status == TaskStatus::Succeeded),
        "every task succeeded"
    );
    assert_eq!(gate.snapshot().started, TASKS);
    assert_eq!(gate.snapshot().finished, TASKS);
    assert!(
        gate.snapshot().max_active <= LIMIT,
        "the configured limit was never exceeded"
    );
    assert_no_duplicate_execution(&gate, &ids);
}

// ---------------------------------------------------------------------------
// Dependency-heavy graph
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dependency_heavy_graph_never_starts_a_task_early() {
    const LAYERS: usize = 10;
    const WIDTH: usize = 10;
    const LIMIT: usize = 8;

    let recorder = Arc::new(SequenceRecorder::default());
    let executor = Arc::new(SequencingExecutor {
        recorder: recorder.clone(),
        runs: AtomicUsize::new(0),
    });
    let engine = TaskEngine::builder()
        .max_parallel_tasks(LIMIT)
        .executor(TaskKind::Validate, executor.clone())
        .build()
        .expect("a Tokio runtime is active");

    // Build a layered DAG: every task in layer L depends on every task in
    // layer L-1. Submission order is deliberately topological, but the engine
    // must still honour the declared edges.
    let mut rows: Vec<Vec<TaskId>> = Vec::with_capacity(LAYERS);
    for layer in 0..LAYERS {
        let mut row = Vec::with_capacity(WIDTH);
        for _ in 0..WIDTH {
            let mut definition = TaskDefinition::new(TaskKind::Validate);
            if layer > 0 {
                for dep in &rows[layer - 1] {
                    definition = definition.depends_on(dep.clone());
                }
            }
            row.push(
                engine
                    .submit(definition)
                    .expect("the engine accepts the task"),
            );
        }
        rows.push(row);
    }

    let all: Vec<TaskId> = rows.iter().flatten().cloned().collect();
    let statuses = settle_all(&engine, &all).await;
    assert!(
        statuses
            .iter()
            .all(|status| *status == TaskStatus::Succeeded),
        "every task in the graph succeeded"
    );

    // Every dependency must have finished before its dependent started.
    for layer in 1..LAYERS {
        for task in &rows[layer] {
            for dep in &rows[layer - 1] {
                assert!(
                    recorder.finish_of(dep) < recorder.start_of(task),
                    "task {task} started before its dependency {dep} finished"
                );
            }
        }
    }
    assert_eq!(
        executor.runs.load(Ordering::SeqCst),
        LAYERS * WIDTH,
        "each task ran exactly once"
    );
}

// ---------------------------------------------------------------------------
// Cancellation storm
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancellation_storm_settles_every_task_without_a_hang() {
    const TASKS: usize = 200;

    let gate = Gate::new();
    let (_release_tx, release_rx) = watch::channel(false);
    let engine = TaskEngine::builder()
        .max_parallel_tasks(1)
        .executor(
            TaskKind::Validate,
            Arc::new(GatedExecutor {
                gate: gate.clone(),
                release: release_rx,
            }),
        )
        .build()
        .expect("a Tokio runtime is active");

    let ids: Vec<TaskId> = (0..TASKS)
        .map(|_| {
            engine
                .submit(TaskDefinition::new(TaskKind::Validate))
                .expect("the engine accepts the task")
        })
        .collect();

    // Let exactly one executor become active, then cancel everything at once.
    gate.wait_for(|snapshot| snapshot.active == 1).await;
    for id in &ids {
        engine
            .cancel(id)
            .expect("the engine accepts the cancellation");
    }

    let statuses = settle_all(&engine, &ids).await;
    assert!(
        statuses
            .iter()
            .all(|status| *status == TaskStatus::Cancelled),
        "every task in the storm settled as cancelled"
    );
    assert!(
        !engine.list_tasks().iter().any(|task| {
            matches!(
                task.status,
                TaskStatus::Queued | TaskStatus::Running | TaskStatus::Blocked
            )
        }),
        "no task is left pending after the storm"
    );
    assert!(
        gate.snapshot().max_active <= 1,
        "the concurrency limit held through the storm"
    );
    assert_no_duplicate_execution(&gate, &ids);
}

// ---------------------------------------------------------------------------
// Wide graph: 1,000 tasks must complete without a deadlock
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn one_thousand_independent_tasks_complete_without_deadlock() {
    const TASKS: usize = 1000;

    let gate = Gate::new();
    let engine = TaskEngine::builder()
        .max_parallel_tasks(8)
        .executor(
            TaskKind::Validate,
            Arc::new(ImmediateExecutor { gate: gate.clone() }),
        )
        .build()
        .expect("a Tokio runtime is active");

    let ids: Vec<TaskId> = (0..TASKS)
        .map(|_| {
            engine
                .submit(TaskDefinition::new(TaskKind::Validate))
                .expect("the engine accepts the task")
        })
        .collect();

    let statuses = settle_all(&engine, &ids).await;
    assert!(
        statuses
            .iter()
            .all(|status| *status == TaskStatus::Succeeded),
        "all {TASKS} tasks succeeded"
    );
    assert_eq!(gate.snapshot().started, TASKS);
    assert_eq!(gate.snapshot().finished, TASKS);
    assert_no_duplicate_execution(&gate, &ids);
}
