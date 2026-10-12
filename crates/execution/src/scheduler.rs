//! The scheduler.
//!
//! The scheduler answers *when* a task runs; executors answer *how*. It is a
//! single event-driven loop that wakes whenever work might have changed,
//! selects the oldest runnable task, acquires a concurrency permit, marks the
//! task `running`, and spawns its execution. It contains no knowledge of Git,
//! Maven, npm, Liquibase, or processes.
//!
//! # Concurrency
//!
//! A [`Semaphore`](tokio::sync::Semaphore) sized to `max_parallel_tasks` is the
//! single source of truth for concurrency: the scheduler can never hold more
//! permits than the limit, and shutdown can wait on the same semaphore.
//!
//! # Determinism
//!
//! Runnable selection is FIFO by submission order, and the scheduler never polls
//! — it is woken by [`Notify`](tokio::sync::Notify) when a task is submitted,
//! cancelled, or settles. This keeps scheduling predictable and avoids idle CPU
//! churn.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use service_orchestrator_domain::{TaskFailure, TaskId, TaskKind, TaskStatus};
use tokio::sync::OwnedSemaphorePermit;

use crate::cancellation::Cancellation;
use crate::context::{ProgressReporter, ProgressSink, TaskContext};
use crate::engine::EngineInner;
use crate::events::TaskEvent;
use crate::executor::{TaskExecutor, TaskExecutorError, TaskOutcome};
use crate::store::ExecutionSpec;
/// Runs the scheduler loop until the engine is shut down.
pub(crate) async fn run(inner: Arc<EngineInner>) {
    loop {
        tokio::select! {
            biased;
            _ = inner.scheduler_cancel.cancelled() => break,
            _ = inner.notify.notified() => {}
        }
        pump(&inner);
        if inner.is_shutdown() {
            break;
        }
    }
}

/// Starts as many runnable tasks as the concurrency limit allows.
fn pump(inner: &Arc<EngineInner>) {
    // Honour cancellation before scheduling: a queued task whose cancellation
    // was requested is settled as cancelled without taking an execution slot,
    // and its dependents settle as skipped (`tasks/016.md` §25).
    for change in inner.store.cancel_queued_signalled() {
        inner.publish_status_change(&change);
        for settled in inner.store.settle_dependents(&change.id) {
            inner.publish_status_change(&settled);
        }
    }

    loop {
        if inner.is_shutdown() {
            break;
        }
        let Some(id) = inner.store.next_ready() else {
            break;
        };
        let permit = match inner.semaphore.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => break,
        };
        match inner.store.transition(&id, TaskStatus::Running) {
            Ok(change) => inner.publish_status_change(&change),
            // The task left `Queued` between selection and the transition
            // (e.g. it was cancelled); skip it and keep dispatching other
            // ready tasks (`tasks/017.md` §19, defect D11). If it is somehow
            // still queued, stop dispatching rather than spinning.
            Err(_) => {
                if inner
                    .store
                    .task(&id)
                    .is_some_and(|task| task.status == TaskStatus::Queued)
                {
                    break;
                }
                continue;
            }
        }
        inner.handle.spawn(run_task(inner.clone(), id, permit));
    }
}

/// The terminal resolution of a task's execution.
enum Resolution {
    Succeeded,
    Failed(TaskFailure),
    Cancelled,
}

/// The result of a single executor invocation.
enum Attempt {
    Outcome(TaskOutcome),
    Failure(TaskFailure),
    Cancelled,
}

/// Executes a task to completion, applying retries, in the spawned task.
async fn run_task(inner: Arc<EngineInner>, id: TaskId, permit: OwnedSemaphorePermit) {
    let Some(spec) = inner.store.execution_spec(&id) else {
        // The task reached `Running` but its execution record is gone. Resolve
        // it as failed instead of leaving it (and its dependents) stuck
        // forever (`tasks/017.md` §19).
        if let Ok(change) = inner.store.fail(
            &id,
            failure(
                "EXECUTION_SPEC_MISSING",
                "the task's execution record was missing; the task was failed",
            ),
        ) {
            inner.publish_status_change(&change);
        }
        for change in inner.store.settle_dependents(&id) {
            inner.publish_status_change(&change);
        }
        drop(permit);
        inner.notify.notify_one();
        return;
    };

    // A panicking executor must not wedge the task in `Running`: the task would
    // hold its semaphore permit forever and every dependent would stay
    // `Blocked` (`tasks/017.md` §19, defect D2). Catch the panic and resolve
    // the task as a structured failure so dependents settle normally.
    let resolution = match AssertUnwindSafe(execute_with_retry(&inner, &spec))
        .catch_unwind()
        .await
    {
        Ok(resolution) => resolution,
        Err(_) => Resolution::Failed(failure(
            "TASK_EXECUTOR_PANICKED",
            "the task executor panicked; the task was failed so dependents are not blocked forever",
        )),
    };

    let change = match resolution {
        Resolution::Succeeded => inner.store.transition(&id, TaskStatus::Succeeded),
        Resolution::Failed(failure) => inner.store.fail(&id, failure),
        Resolution::Cancelled => inner.store.transition(&id, TaskStatus::Cancelled),
    };
    if let Ok(change) = change {
        inner.publish_status_change(&change);
    }

    for change in inner.store.settle_dependents(&id) {
        inner.publish_status_change(&change);
    }

    // Release the concurrency permit *before* waking the scheduler. Otherwise
    // the scheduler could observe the notification, fail to acquire the still
    // held permit, and then sleep forever with ready tasks queued.
    drop(permit);
    inner.notify.notify_one();
}

/// Runs a task's executor, retrying failed attempts while the policy allows.
async fn execute_with_retry(inner: &Arc<EngineInner>, spec: &ExecutionSpec) -> Resolution {
    loop {
        if spec.cancellation.is_cancelled() {
            return Resolution::Cancelled;
        }

        let attempt = match inner.store.bump_attempt(&spec.id) {
            Ok(attempt) => attempt,
            Err(_) => return Resolution::Cancelled,
        };
        if attempt > 1 {
            inner.publish(TaskEvent::TaskRetrying {
                id: spec.id.clone(),
                attempt,
            });
        }

        let Some(executor) = inner.executors.get(&spec.kind) else {
            return Resolution::Failed(executor_not_found(&spec.kind));
        };

        let context = TaskContext::new(
            spec.id.clone(),
            spec.kind.clone(),
            spec.target.clone(),
            spec.parameters.clone(),
            spec.timeout,
            spec.cancellation.clone(),
            ProgressReporter::new(inner.clone(), spec.id.clone()),
        );

        let attempt_result =
            run_attempt(executor, context, spec.timeout, spec.cancellation.clone()).await;

        let failure = match attempt_result {
            Attempt::Cancelled => return Resolution::Cancelled,
            Attempt::Outcome(outcome) => {
                if let Some(message) = outcome.message {
                    inner.set_message(&spec.id, message);
                }
                if outcome.success {
                    return Resolution::Succeeded;
                }
                outcome.failure.unwrap_or_else(|| {
                    failure("TASK_FAILED", "task reported an unsuccessful outcome")
                })
            }
            Attempt::Failure(failure) => failure,
        };

        if spec.retry.should_retry(attempt) {
            if !sleep_backoff(spec.retry.backoff, &spec.cancellation).await {
                return Resolution::Cancelled;
            }
            continue;
        }
        return Resolution::Failed(failure);
    }
}

/// Runs one executor invocation with a timeout.
///
/// Cancellation is *cooperative*: the engine does not drop the executor future
/// out from under it. The executor receives the task's [`Cancellation`] in its
/// [`TaskContext`] and is responsible for observing it and returning
/// [`TaskExecutorError::Cancelled`] after releasing resources. On timeout the
/// engine signals the task's cancellation *before* dropping the future, so an
/// executor that owns a child process can terminate it instead of leaking it
/// (`tasks/017.md` §21, defect D6).
async fn run_attempt(
    executor: Arc<dyn TaskExecutor>,
    context: TaskContext,
    timeout: Option<Duration>,
    cancellation: Cancellation,
) -> Attempt {
    let future = executor.execute(context);
    match with_timeout(future, timeout).await {
        Ok(Ok(outcome)) => Attempt::Outcome(outcome),
        Ok(Err(TaskExecutorError::Cancelled)) => Attempt::Cancelled,
        Ok(Err(error)) => Attempt::Failure(error.to_failure()),
        Err(_elapsed) => {
            cancellation.cancel();
            Attempt::Failure(failure("TASK_TIMEOUT", "task timed out"))
        }
    }
}

async fn with_timeout<F>(
    future: F,
    timeout: Option<Duration>,
) -> Result<F::Output, tokio::time::error::Elapsed>
where
    F: Future,
{
    match timeout {
        Some(timeout) => tokio::time::timeout(timeout, future).await,
        None => Ok(future.await),
    }
}

/// Sleeps for `backoff`, returning `false` if cancellation was observed first.
async fn sleep_backoff(backoff: Duration, cancellation: &Cancellation) -> bool {
    if backoff.is_zero() {
        return !cancellation.is_cancelled();
    }
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => false,
        _ = tokio::time::sleep(backoff) => true,
    }
}

fn failure(code: &str, message: &str) -> TaskFailure {
    TaskFailure::new(code, message, None).expect("static failure codes and messages are valid")
}

fn executor_not_found(kind: &TaskKind) -> TaskFailure {
    TaskFailure::new(
        "EXECUTOR_NOT_FOUND",
        format!("no executor is registered for task kind '{kind}'"),
        None,
    )
    .expect("static failure code and non-empty message")
}
