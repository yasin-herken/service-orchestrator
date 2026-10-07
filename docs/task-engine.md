# The Task Engine

The task engine is the generic execution core of Service Orchestrator. Every
meaningful operation — a Git sync, a Maven build, an npm install, a Liquibase
migration, starting or stopping a process, a health check, or a composed
workflow — is a **task**. The engine owns task state, dependency-aware
scheduling, bounded concurrency, cancellation, retry, timeout, progress
reporting, and an in-memory task registry. It lives in the
`service-orchestrator-execution` crate.

The engine is deliberately independent of:

- the TUI (`ratatui`) and the MCP server (`rmcp`),
- Git, Maven, npm, Liquibase, and OS process APIs,
- the application layer and all concrete infrastructure.

It depends only on `service-orchestrator-domain` (plus `tokio` and
`async-trait` for asynchronous execution). Concrete work is supplied from the
outside through the [executor boundary](#executor-model).

```text
TUI ─────┐
         │
MCP ─────┼──> Application use case
         │
CLI ─────┘
              ↓
        TaskDefinition / WorkflowDefinition
              ↓
          TaskEngine  ──> TaskStore (in-memory registry)
              ↓              ↑
          Scheduler ─────────┘
              ↓
       TaskExecutor (registry lookup)
              ↓
        Infrastructure
```

The single most important rule is the **scheduler/executor separation**: the
scheduler answers *when* a task runs; an executor answers *how*. The scheduler
never contains a `match task.kind { ... }` with one branch per operation.

---

## 1. Task model

Two values describe a unit of work, and they are intentionally different
(`AGENTS.md` §9, task 005 §5):

| Value | Meaning | Lives in |
| --- | --- | --- |
| `TaskDefinition` | *What should execute.* The application's intent: a `TaskKind`, an optional target, dependencies, timeout, description, retry policy, and dependency policy. | `execution::task_manager` |
| `Task` | *The runtime state of that execution.* Status, timestamps, progress, failure, dependencies, and the assigned identity. | `domain::task` |

A `TaskDefinition` carries only domain identifiers and plain data — never
process handles, shell commands, or interface types. The engine assigns a
`TaskId` and owns the `Task` thereafter.

```rust
let definition = TaskDefinition::new(TaskKind::BuildService)
    .for_service(ServiceId::new("auth-service")?)
    .depends_on(previous_task_id)
    .with_timeout(600)
    .with_retry(RetryPolicy::attempts(2));
let task_id = engine.submit(definition)?;
```

### Identity

Every task has a unique `domain::TaskId`. The engine generates monotonic
identifiers (`task-0`, `task-1`, …); a caller may also submit an explicit id by
inserting a record through the store. Array indices, timestamps, and positions
are never identities.

### Task kinds

`TaskKind` is the shared, extensible vocabulary
(`Validate`, `GitSync`, `GitFetch`, `GitCheckout`, `GitPull`, `BuildLibrary`,
`MavenBuild`, `NpmInstall`, `NpmBuild`, `Liquibase`, `BuildService`,
`StartService`, `StopService`, `RestartService`, `HealthCheck`, and
`Custom(String)`). Because executors are registered against a kind, adding a new
kind of work is a **registration**, not a change to the scheduling core.

### Progress and messages

`TaskProgress` supports both determinate and indeterminate work:

- `current` — completed units;
- `total` — the total, or `None` when unknown;
- `message` — an optional phase such as `"Downloading Maven dependencies"`.

A task's message may also be updated without touching counters
(`ProgressReporter::message`). The description supplied on the definition seeds
the initial message, so a queued task already explains itself. `current` greater
than `total` is rejected as a domain violation.

### Results and failures

An executor returns a `TaskOutcome`:

```text
TaskOutcome
├── success: bool
├── message: Option<String>
├── output: Option<String>      // a reference, not raw stdout
├── failure: Option<TaskFailure>
└── metadata: BTreeMap<String, String>
```

Large raw output is **not** embedded in task state; the executor writes it to
the logging subsystem and records a reference in `output`
(`AGENTS.md` §31). A `TaskFailure` is a structured `{ code, message, detail }`
pair such as `TASK_TIMEOUT` or `EXECUTOR_NOT_FOUND` — never a formatted string.

---

## 2. State machine

The engine uses exactly these states (defined by the domain and enforced by the
domain `Task` state machine):

```text
queued
  ├──> running ──> succeeded
  │        ├─────> failed
  │        └─────> cancelled
  ├──> blocked ──> queued | running | failed | cancelled | skipped
  ├──> cancelled
  └──> skipped

terminal: succeeded | failed | cancelled | skipped   (no outgoing edges)
```

`TaskStatus::can_transition_to` is the single source of truth for legality. The
engine never invents an edge: an illegal transition is rejected with
`TaskEngineError::InvalidTransition`.

### Initial status

When a task is inserted, its initial status is computed from its dependencies:

| Situation | Initial status |
| --- | --- |
| No dependencies | `queued` |
| Dependencies not yet terminal | `blocked` |
| Dependencies terminal and policy satisfied | `queued` |
| Dependencies terminal and policy **not** satisfied | `skipped` |

### Failure propagation

When a task becomes terminal, `settle_dependents` examines each direct
dependent:

- still has non-terminal dependencies → stays `blocked`;
- policy satisfied → `queued` (it will be scheduled);
- policy not satisfied → `skipped`.

A dependent is **never** started after a required dependency failed. With the
default `AllSucceeded` policy, a failed, cancelled, or skipped dependency skips
the dependent.

### Retries do not add state-machine edges

Retry is modelled **inside** one `running` episode: the domain has no
`failed -> queued` edge, and the engine deliberately does not create one. An
attempt that fails is retried while the policy allows; only the final failure
moves the task to `failed`.

---

## 3. Scheduler

The scheduler is a single event-driven loop. It never polls and never
busy-waits: it sleeps on a `tokio::sync::Notify` and is woken when a task is
submitted, cancelled, or settles.

```text
loop:
  wait for notify (or scheduler cancellation)
  pump:
    while a ready task exists and a concurrency permit is free:
      select the oldest ready task (FIFO by insertion sequence)
      mark it running, publish TaskStarted
      spawn its execution on the runtime
```

- **Ready** means `queued` with all dependency conditions satisfied.
- **Selection is FIFO** by insertion sequence, which is predictable and
  starvation-free for the MVP. Priorities are not implemented (task 005 §18
  permits a FIFO model).
- **When** a task runs is the only question the scheduler answers. It knows
  nothing about Git, Maven, npm, Liquibase, or processes.

Long-running operations never block the caller: `submit` returns a `TaskId`
immediately, and the caller observes progress through the query API or the event
stream (task 005 §49, §50).

### Graph validation

Before a workflow is scheduled, its shape is validated:

- the domain validates step ids (unique), prerequisite resolution, and
  acyclicity;
- the engine's `graph::validate` re-checks the mapped task graph for duplicate
  nodes, unknown dependencies, and cycles.

A cycle is a *static* error detected before scheduling, so the scheduler can
never deadlock on `A waits for B, B waits for A` (task 005 §30, §31). The error
is returned as `TaskEngineError::DependencyCycle`.

---

## 4. Dependencies and dependency policies

A task may declare any number of `TaskId` dependencies, which may point at
earlier submissions or at sibling tasks in a workflow. Dependencies form a DAG.

Two explicit policies cover the required behaviour without a policy engine
(task 005 §14):

| Policy | Runs when | On a failed dependency |
| --- | --- | --- |
| `AllSucceeded` *(default)* | every dependency succeeded | dependent is `skipped` |
| `AnyCompleted` | every dependency reached a terminal state | dependent still runs |

`AllSucceeded` is equivalent to "run only if all dependencies succeed" and "do
not run if any dependency fails". `AnyCompleted` is "run once dependencies have
finished", used for cleanup or reporting steps.

```text
common-core
     ├──> auth      ┐
     └──> payment   ├── run concurrently
                   │
auth ──────────────┴──> users   (waits for auth)
```

### Workflows

A `WorkflowDefinition` is a `WorkflowKind` plus `WorkflowStep`s, each carrying a
`TaskKind` and prerequisite `StepId`s. On submission the engine:

1. validates the workflow through the domain;
2. allocates a `TaskId` per step;
3. translates step prerequisites into task dependencies;
4. validates the resulting task graph;
5. inserts every task, records the workflow, and publishes
   `WorkflowCreated` and `TaskCreated` events.

The same dependency-aware scheduler then runs the workflow, so retry, timeout,
cancellation, and failure propagation are defined exactly once. The application
layer still owns the *business* meaning of a workflow; the engine only executes
its graph (task 005 §29).

---

## 5. Concurrency

Concurrency is bounded by a single `tokio::sync::Semaphore` sized to
`EngineConfig::max_parallel_tasks` (default `4`). A permit is acquired before a
task is marked `running` and released when its execution ends, so:

```text
running_tasks <= max_parallel_tasks
```

holds at all times. Independent tasks run concurrently; dependent tasks wait.
The scheduler never starts one Tokio worker per repository, so a workspace of
40–100+ services (task 005 §52) is handled with a small, fixed pool.

Because the permit is released *before* the scheduler is notified, the scheduler
can never wake, fail to acquire a permit, and then sleep with ready work queued.

### Backpressure

`EngineConfig::max_queued_tasks` optionally caps the number of non-terminal
tasks. Submissions beyond the cap are rejected with
`TaskEngineError::QueueFull { limit }`, so an AI agent or client cannot enqueue
unbounded work. The MVP limit is opt-in; retention/eviction policy for completed
tasks is future work.

---

## 6. Cancellation

Cancellation is **cooperative**. Every task owns a `Cancellation` — a one-way,
cheaply cloneable `tokio::sync::watch<bool>`. The scheduler and engine have their
own tokens too.

- `TaskEngine::cancel(task_id)` signals the task's token and wakes the
  scheduler.
- A `queued` task is checked when it is admitted; it transitions
  `running -> cancelled` without invoking its executor.
- A `running` executor receives the token in its `TaskContext` and is expected
  to observe it (`TaskContext::cancelled().await` or `is_cancelled()`) and
  return `TaskExecutorError::Cancelled` after releasing resources (a child
  process, a file handle).
- Cancelling an already-terminal task is a no-op that returns `Ok`.

The engine does **not** drop the executor's future out from under it, so an
executor can perform cleanup. The trade-off is explicit: an executor that never
observes cancellation runs to completion. Timeouts still bound runaway work, and
`shutdown` waits a bounded time for cooperation. Documented semantics for a
cancelled workflow: running tasks receive cancellation, queued tasks are
cancelled when admitted, and dependents never start (task 005 §32).

Process-level termination is not the engine's concern. The future
`process`/executor layer implements `Drop`/cancellation guards so that a dropped
or cancelled task kills its child process; the engine only defines the contract
(task 005 §12).

---

## 7. Retry

Retry is **opt-in** and defaults to a single attempt:

```rust
pub struct RetryPolicy {
    pub max_attempts: u32,   // total attempts, including the first
    pub backoff: Duration,   // delay between attempts
}
```

`RetryPolicy::none()` (the default) runs a task exactly once, so destructive
operations are never retried automatically. When a policy is set, a failed
*attempt* is retried until `max_attempts` is reached, all within the same
`running` episode. Between attempts the engine sleeps for `backoff`, aborting
the sleep if cancellation is observed first. Each retry publishes
`TaskRetrying { id, attempt }`. Only the final failure moves the task to
`failed`.

---

## 8. Timeout

A per-task timeout comes from `TaskDefinition::timeout_seconds`; if absent, the
engine uses `EngineConfig::default_timeout`. When an attempt exceeds its
timeout, the executor future is dropped and the attempt fails with the
structured failure `TASK_TIMEOUT`. A timeout is an ordinary failure, so the
retry policy applies to it like any other.

---

## 9. Event model

The engine publishes a `TaskEvent` for every meaningful change over a bounded
`tokio::sync::broadcast` channel. The model is interface-neutral: it carries
domain identifiers and values, never terminal rows or MCP response shapes.

```text
TaskCreated   TaskStarted   TaskProgress   TaskMessage
TaskRetrying  TaskSucceeded TaskFailed     TaskCancelled
TaskSkipped   TaskBlocked   WorkflowCreated
```

Consumers (`subscribe()`):

```text
TaskEvent
   ├──> TUI       (render, cancellation prompts)
   ├──> MCP       (tool results, progress)
   ├──> Logging   (structured task logs)
   └──> Metrics   (future telemetry)
```

Delivery is in-process only — there is no broker. A subscriber that falls behind
observes a `Lagged` error and resynchronises from the task store. Events are a
convenience for interfaces; the store is always authoritative.

---

## 10. Executor model

An executor answers *how* a task is performed:

```rust
#[async_trait]
pub trait TaskExecutor: Send + Sync {
    async fn execute(&self, context: TaskContext)
        -> Result<TaskOutcome, TaskExecutorError>;
}
```

An executor receives a deliberately narrow `TaskContext` — the task id, kind,
target, progress reporter, and cancellation token — and nothing else. It cannot
reach the scheduler, the store, or the application container (task 005 §27).
Executors that perform genuinely blocking work run it on a blocking thread pool
internally.

### Registry

Concrete executors are registered against a `TaskKind` and looked up at run
time. The registry is populated by the composition root and is immutable once
the engine is built.

```rust
let engine = TaskEngine::builder()
    .max_parallel_tasks(4)
    .executor(TaskKind::GitFetch, Arc::new(GitFetchExecutor::new()))
    .executor(TaskKind::MavenBuild, Arc::new(MavenExecutor::new()))
    .build()?;
```

A task whose kind has no registered executor fails with the structured code
`EXECUTOR_NOT_FOUND`; it does not panic. This keeps the scheduler free of
task-specific branching and makes new task kinds additive.

---

## 11. Shutdown

`TaskEngine::shutdown()` is the graceful path (task 005 §40):

1. stop accepting new work — `submit`/`submit_workflow` return
   `TaskEngineError::ShuttingDown`;
2. cancel the scheduler loop;
3. signal cancellation to every running task;
4. wait, bounded to 5 seconds, on the concurrency semaphore for tasks to release
   their permits;
5. return. Executors that ignore cancellation are not forcibly killed.

`TaskEngine` also implements `Drop`, which sets the shutdown flag, cancels the
scheduler, and signals running tasks, so dropping the engine never leaves the
background scheduler running.

---

## 12. Query and management API

The `TaskEngine` implements the application-facing `TaskManager` trait
(`submit`, `submit_workflow`, `task`, `workflow`, `cancel`) and adds a richer,
interface-neutral API:

| Method | Purpose |
| --- | --- |
| `get_task` / `list_tasks` | inspect one / all tasks (oldest first) |
| `list_running_tasks` | tasks currently `running` |
| `list_recent_tasks(n)` | up to `n` most recently finished tasks |
| `get_workflow` | inspect a submitted workflow |
| `subscribe` | receive the `TaskEvent` stream |
| `wait_for` | await a task's terminal state |
| `shutdown` / `is_shutdown` | lifecycle |

`get_task`, `list_tasks`, and the rest return domain values — no
interface-specific result types.

---

## 13. Thread safety and locking

The engine is shared concurrently by the TUI event loop, MCP handlers, the
background scheduler, executors, and event subscribers (task 005 §39).

- The task store is a single `std::sync::Mutex<Inner>`. Critical sections are
  short, and the lock is **never** held across an `.await`.
- A `tokio::sync::watch<u64>` revision counter lets `wait_for` observe status
  changes without polling; reading a task and subscribing to the revision happen
  under one lock, so a waiter that sees a non-terminal task is guaranteed to be
  woken by its next change.
- Concurrency is a `tokio::sync::Semaphore`; wake-ups use `tokio::sync::Notify`.
- `TaskEngine` is `Send + Sync` and is shared behind `Arc<dyn TaskManager>` (or
  `Arc<TaskEngine>`). It is intentionally **not** `Clone`: a single instance owns
  the engine's lifetime.

---

## 14. Integration with the application layer

The application already depends on the `TaskManager` trait. `TaskEngine`
implements it, so wiring the real engine in is a single composition-root line:

```rust
let engine: Arc<dyn TaskManager> = Arc::new(TaskEngine::builder()…build()?);
let application = Application::new(config, engine, runtime_state, logs, policy);
```

The application keeps ownership of *what* has to happen (preconditions, policy,
idempotency, business workflows); the engine owns *how and when* the tasks
execute. No business workflow logic moves into the engine, and the engine never
depends on the application (task 005 §48).

---

## 15. Testing

The engine has two layers of tests:

- **Unit tests** (inside the crate): the store's state machine, dependency
  settlement, FIFO selection, progress validation, retry policy, cancellation
  primitive, executor registry, and graph validation.
- **Integration tests** (`crates/execution/tests/task_engine.rs`) drive the
  public API with fake executors and a deterministic gate/latch primitive — no
  sleeps for correctness. They cover: lifecycle and events, missing executors,
  blocked/queued/skipped dependents, both dependency policies, concurrency
  limits, FIFO order, running/queued cancellation, timeout, retry success and
  exhaustion, backpressure, shutdown, workflow ordering, and cycle rejection.

A test-only executor records concurrency and start order; a watch-based latch
(not a semaphore) ensures blocked tasks stay blocked until the test opens it.
No real Git, Maven, npm, Liquibase, or process is ever invoked.

---

## 16. Related documents

- `docs/adr/005-task-engine.md` — the decision record for the engine.
- `docs/architecture.md` §10 — where the task engine belongs.
- `docs/application-core.md` — the use cases that submit tasks.
- `AGENTS.md` §9 (task engine), §10 (workflow engine), §21–§30 (MCP).
