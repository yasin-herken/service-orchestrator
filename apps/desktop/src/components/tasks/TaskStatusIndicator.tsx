import { StatusBadge, type BadgeTone } from '../StatusBadge';

/** Badge tones for the task lifecycle statuses. */
const TASK_TONES: Record<string, BadgeTone> = {
  queued: 'neutral',
  blocked: 'warning',
  running: 'info',
  succeeded: 'positive',
  failed: 'negative',
  cancelled: 'warning',
  skipped: 'neutral',
  unknown: 'neutral',
};

/** Whether a task status is terminal (will not change again). */
const TERMINAL = new Set(['succeeded', 'failed', 'cancelled', 'skipped']);

/** Whether a task status means the task will not change again. */
export function isTerminalTaskStatus(status: string): boolean {
  return TERMINAL.has(status);
}

/** Human-readable label for a task status. */
export function taskStatusLabel(status: string): string {
  return status.charAt(0).toUpperCase() + status.slice(1);
}

/**
 * A status pill for a task lifecycle status.
 *
 * Status is conveyed by text as well as color (`tasks/020.md` §36).
 */
export function TaskStatusIndicator({
  status,
  label,
}: {
  status: string;
  label?: string;
}) {
  return (
    <StatusBadge
      value={status}
      tone={TASK_TONES[status] ?? 'neutral'}
      label={label ?? taskStatusLabel(status)}
    />
  );
}
