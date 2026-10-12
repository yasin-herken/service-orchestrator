import { Dialog } from './Dialog';
import type { TrackedTask } from '../context/TaskEventsContext';
import { ACTION_LABELS, type ServiceOperation } from '../hooks/useServiceActions';
import { summarizeTasks, tasksForWorkflow } from '../utils/taskProgress';
import { formatClock } from '../utils/format';

/** A concise, capitalized status label. */
function statusLabel(status: string): string {
  return status.charAt(0).toUpperCase() + status.slice(1);
}

/**
 * Shows real task-engine progress for a submitted operation.
 *
 * Nothing here is fabricated: a single operation follows its scheduled task id,
 * and a bulk operation aggregates the real per-task outcomes of its workflow's
 * tasks. Percentages are only shown when the daemon reports a total
 * (`tasks/019.md` §25).
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
  const title = `${ACTION_LABELS[operation.action]} — ${targetLabel}`;

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
      {operation.outcome.status === 'already_in_desired_state' ? (
        <p className="dialog__message">
          Nothing to do: the selection is already in the desired state. The
          daemon scheduled no task.
        </p>
      ) : null}

      {operation.outcome.status === 'task' && operation.outcome.task_id ? (
        <TaskProgressBody
          taskId={operation.outcome.task_id}
          tasks={tasks}
        />
      ) : null}

      {operation.outcome.status === 'workflow' && operation.outcome.workflow_id ? (
        <WorkflowProgressBody
          workflowId={operation.outcome.workflow_id}
          tasks={tasks}
        />
      ) : null}

      <p className="dialog__note muted">
        Started at {formatClock(operation.at)}. Progress comes from the daemon's
        task events.
      </p>
    </Dialog>
  );
}

function TaskProgressBody({
  taskId,
  tasks,
}: {
  taskId: string;
  tasks: readonly TrackedTask[];
}) {
  const task = tasks.find((candidate) => candidate.id === taskId);
  if (!task) {
    return (
      <p className="dialog__message">
        Task <code className="dialog__code">{taskId}</code> scheduled. Waiting
        for the daemon to report its progress…
      </p>
    );
  }
  return (
    <dl className="dialog__facts">
      <div>
        <dt>Task</dt>
        <dd className="dialog__code">{task.id}</dd>
      </div>
      <div>
        <dt>Operation</dt>
        <dd>{task.kind}</dd>
      </div>
      <div>
        <dt>Status</dt>
        <dd>{statusLabel(task.status)}</dd>
      </div>
      {task.progressTotal !== null ? (
        <div>
          <dt>Progress</dt>
          <dd>
            {task.progressCurrent} of {task.progressTotal}
          </dd>
        </div>
      ) : null}
      {task.progressMessage ? (
        <div className="dialog__facts-wide">
          <dt>Detail</dt>
          <dd>{task.progressMessage}</dd>
        </div>
      ) : null}
      {task.failure ? (
        <div className="dialog__facts-wide">
          <dt>Failure</dt>
          <dd>
            {task.failure.message} <span className="muted">({task.failure.code})</span>
          </dd>
        </div>
      ) : null}
    </dl>
  );
}

function WorkflowProgressBody({
  workflowId,
  tasks,
}: {
  workflowId: string;
  tasks: readonly TrackedTask[];
}) {
  const workflowTasks = tasksForWorkflow(tasks, workflowId);
  const progress = summarizeTasks(workflowTasks);

  return (
    <div className="progress">
      <p className="dialog__message">
        Workflow <code className="dialog__code">{workflowId}</code> scheduled.
      </p>
      {workflowTasks.length === 0 ? (
        <p className="muted">Waiting for the daemon to report the workflow's tasks…</p>
      ) : (
        <>
          <ul className="progress__counts">
            <li>
              Succeeded <strong>{progress.succeeded}</strong>
            </li>
            <li>
              Running <strong>{progress.running}</strong>
            </li>
            <li>
              Pending <strong>{progress.pending}</strong>
            </li>
            <li>
              Failed <strong>{progress.failed}</strong>
            </li>
            {progress.skipped > 0 ? (
              <li>
                Skipped <strong>{progress.skipped}</strong>
              </li>
            ) : null}
          </ul>
          <ul className="progress__services">
            {workflowTasks.map((task) => (
              <li key={task.id}>
                <span className="progress__service">
                  {task.serviceId ?? 'workflow'}
                </span>
                <span className="progress__kind">{task.kind}</span>
                <span className={`progress__status progress__status--${task.status}`}>
                  {statusLabel(task.status)}
                </span>
                {task.failure ? (
                  <span className="progress__failure">{task.failure.message}</span>
                ) : null}
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}
