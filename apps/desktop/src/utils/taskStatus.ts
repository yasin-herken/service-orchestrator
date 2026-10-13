/**
 * Single source of truth for task-lifecycle status classification.
 *
 * Task status is one of the seven engine states (`queued`, `running`,
 * `succeeded`, `failed`, `cancelled`, `skipped`, `blocked`). Terminal states
 * never change again, which is what lets the task-event reducer refuse a late
 * or duplicate stream update and lets views stop polling a settled task
 * (`tasks/024.md` §20, §24).
 */

/** The task statuses that will not change again. */
export const TERMINAL_TASK_STATUSES: ReadonlySet<string> = new Set([
  'succeeded',
  'failed',
  'cancelled',
  'skipped',
]);

/** Whether a task status is terminal (will not change again). */
export function isTerminalTaskStatus(status: string): boolean {
  return TERMINAL_TASK_STATUSES.has(status);
}

/** A human-readable label for a task status (`succeeded` → `Succeeded`). */
export function taskStatusLabel(status: string): string {
  return status.charAt(0).toUpperCase() + status.slice(1);
}
