import { ConfirmationDialog } from '../ConfirmationDialog';
import type { WorkflowStatusView } from '../../types/models';

/**
 * Cancellation confirmation for a running workflow (`tasks/022.md` §23, §24).
 *
 * The dialog states the workflow identity, its current aggregate state, the
 * pending/running tasks, and the consequences: completed work is preserved,
 * cancellation does not roll back Git operations, installed artifacts, or
 * database migrations, and the result may be partial. The actual cancellation
 * goes through the daemon's existing cancellation API.
 */
export function WorkflowCancellationDialog({
  status,
  busy = false,
  onConfirm,
  onCancel,
}: {
  status: WorkflowStatusView;
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const active = status.pending;
  return (
    <ConfirmationDialog
      title={`Cancel workflow ${status.workflow_id}`}
      message={
        <>
          The workflow is <strong>{status.state}</strong>: {status.succeeded} of{' '}
          {status.total} tasks have already completed and {active} are still
          pending or running.
        </>
      }
      confirmLabel="Cancel workflow"
      tone="danger"
      busy={busy}
      onConfirm={onConfirm}
      onCancel={onCancel}
    >
      <dl className="dialog__facts">
        <div>
          <dt>Workflow</dt>
          <dd>{status.workflow_id}</dd>
        </div>
        <div>
          <dt>Kind</dt>
          <dd>{status.kind}</dd>
        </div>
        <div>
          <dt>Current state</dt>
          <dd>{status.state}</dd>
        </div>
        <div>
          <dt>Still pending</dt>
          <dd>{active}</dd>
        </div>
      </dl>
      <p className="dialog__note">
        Cancelling stops scheduling pending tasks, cancels running tasks where the
        daemon supports it, and prevents dependent tasks from starting. Completed
        work is preserved: cancellation does <strong>not</strong> roll back Git
        operations, installed artifacts, or database migrations. The final result
        will be reported as partial where work already finished.
      </p>
    </ConfirmationDialog>
  );
}
