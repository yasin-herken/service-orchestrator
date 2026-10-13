import type { ReactNode } from 'react';

import { Dialog } from '../Dialog';
import type { TrackedTask } from '../../context/TaskEventsContext';
import type { BulkSummaryView, OperationOutcome, TaskView } from '../../types/models';
import { formatClock } from '../../utils/format';
import { OperationSummary } from './OperationSummary';

/**
 * A modal that shows real task-engine progress for a submitted operation.
 *
 * Shared by the Git, Build, and Library screens so there is one task-monitoring
 * implementation, not three (`tasks/020.md` §27).
 */
export function OperationProgressDialog({
  title,
  outcome,
  tasks,
  task,
  summary,
  at,
  targetLabel,
  logs,
  onCancelTask,
  onCancelWorkflow,
  onClose,
}: {
  title: string;
  outcome: OperationOutcome;
  tasks: readonly TrackedTask[];
  task?: TaskView | null;
  summary?: BulkSummaryView | null;
  /** When the operation was submitted, seconds since the Unix epoch. */
  at: number;
  targetLabel?: string;
  /** Optional structured task logs (for example a shared `LogViewer`). */
  logs?: ReactNode;
  onCancelTask?: (taskId: string) => void;
  onCancelWorkflow?: (workflowId: string) => void;
  onClose: () => void;
}) {
  return (
    <Dialog
      title={title}
      onClose={onClose}
      footer={
        <button type="button" className="button" onClick={onClose}>
          Close
        </button>
      }
    >
      <OperationSummary
        outcome={outcome}
        tasks={tasks}
        task={task}
        summary={summary}
        targetLabel={targetLabel}
        onCancelTask={onCancelTask}
        onCancelWorkflow={onCancelWorkflow}
      />
      {logs}
      <p className="dialog__note muted">
        Started at {formatClock(at)}. Progress comes from the daemon&rsquo;s task
        events; nothing here is fabricated.
      </p>
    </Dialog>
  );
}
