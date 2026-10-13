import type { TrackedTask } from '../context/TaskEventsContext';
import { isTerminalTaskStatus } from './taskStatus';

/** Whether a task status is terminal. */
export function isTerminalStatus(status: string): boolean {
  return isTerminalTaskStatus(status);
}

/** The service ids that currently have a non-terminal (busy) task. */
export function busyServiceIds(tasks: readonly TrackedTask[]): Set<string> {
  const busy = new Set<string>();
  for (const task of tasks) {
    if (task.serviceId && !isTerminalStatus(task.status)) {
      busy.add(task.serviceId);
    }
  }
  return busy;
}

/**
 * The most recent task per service (by `updatedAt`).
 *
 * Used for the table's "Last operation" column; it reflects tasks from every
 * interface sharing the daemon, not just the GUI's own submissions.
 */
export function lastTaskByService(
  tasks: readonly TrackedTask[],
): Map<string, TrackedTask> {
  const latest = new Map<string, TrackedTask>();
  for (const task of tasks) {
    if (!task.serviceId) {
      continue;
    }
    const existing = latest.get(task.serviceId);
    if (!existing || task.updatedAt > existing.updatedAt) {
      latest.set(task.serviceId, task);
    }
  }
  return latest;
}

/** The tasks belonging to a workflow, in the order they were tracked. */
export function tasksForWorkflow(
  tasks: readonly TrackedTask[],
  workflowId: string,
): TrackedTask[] {
  return tasks.filter((task) => task.workflowId === workflowId);
}

/** Aggregated real progress for a set of workflow tasks (never fabricated). */
export interface WorkflowProgress {
  total: number;
  running: number;
  pending: number;
  succeeded: number;
  failed: number;
  cancelled: number;
  skipped: number;
  /** Whether every task has settled. */
  complete: boolean;
}

/** Aggregates the real per-task outcomes of a bulk operation. */
export function summarizeTasks(tasks: readonly TrackedTask[]): WorkflowProgress {
  const progress: WorkflowProgress = {
    total: tasks.length,
    running: 0,
    pending: 0,
    succeeded: 0,
    failed: 0,
    cancelled: 0,
    skipped: 0,
    complete: false,
  };
  for (const task of tasks) {
    switch (task.status) {
      case 'running':
        progress.running += 1;
        break;
      case 'queued':
      case 'blocked':
        progress.pending += 1;
        break;
      case 'succeeded':
        progress.succeeded += 1;
        break;
      case 'failed':
        progress.failed += 1;
        break;
      case 'cancelled':
        progress.cancelled += 1;
        break;
      case 'skipped':
        progress.skipped += 1;
        break;
      default:
        progress.pending += 1;
        break;
    }
  }
  progress.complete = progress.total > 0 && progress.running + progress.pending === 0;
  return progress;
}
