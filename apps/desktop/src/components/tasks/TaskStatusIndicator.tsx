import { StatusBadge, type BadgeTone } from '../StatusBadge';
import { isTerminalTaskStatus, taskStatusLabel } from '../../utils/taskStatus';

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

export { isTerminalTaskStatus, taskStatusLabel };

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
