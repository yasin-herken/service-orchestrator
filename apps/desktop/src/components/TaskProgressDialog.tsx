import { OperationProgressDialog } from './tasks/OperationProgressDialog';
import type { TrackedTask } from '../context/TaskEventsContext';
import { ACTION_LABELS, type ServiceOperation } from '../hooks/useServiceActions';

/**
 * Shows real task-engine progress for a submitted service lifecycle operation.
 *
 * This is a thin adapter over the shared [`OperationProgressDialog`] so the
 * services screen and the Git/Build/Library screens share one task-monitoring
 * implementation (`tasks/019.md` §25; `tasks/020.md` §27).
 */
export function TaskProgressDialog({
  operation,
  tasks,
  onClose,
}: {
  operation: ServiceOperation;
  tasks: readonly TrackedTask[];
  onClose: () => void;
}) {
  const targetLabel =
    operation.labels.length === 1
      ? operation.labels[0]
      : `${operation.labels.length} services`;

  return (
    <OperationProgressDialog
      title={`${ACTION_LABELS[operation.action]} — ${targetLabel}`}
      outcome={operation.outcome}
      tasks={tasks}
      at={operation.at}
      onClose={onClose}
    />
  );
}
