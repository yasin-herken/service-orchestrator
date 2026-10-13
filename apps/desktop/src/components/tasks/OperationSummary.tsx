import type { TrackedTask } from '../../context/TaskEventsContext';
import type { BulkSummaryView, OperationOutcome, TaskView } from '../../types/models';
import { summarizeTasks, tasksForWorkflow } from '../../utils/taskProgress';
import { BulkOperationSummary } from './BulkOperationSummary';
import { ErrorDetailsPanel } from './ErrorDetailsPanel';
import { TaskStatusIndicator, isTerminalTaskStatus } from './TaskStatusIndicator';

/**
 * The shared, real task/workflow progress body.
 *
 * Every field comes from the daemon's task engine — either a tracked task event
 * or a task read back by id — never a fabricated percentage
 * (`tasks/020.md` §10, §17, §27). Cancellation is offered only while work is
 * still active, and it never claims a rollback.
 */
export function OperationSummary({
  outcome,
  tasks,
  task,
  summary,
  targetLabel,
  onCancelTask,
  onCancelWorkflow,
}: {
  outcome: OperationOutcome;
  /** Live tracked tasks from the shared task-event stream. */
  tasks: readonly TrackedTask[];
  /** The single task read back by id, when available. */
  task?: TaskView | null;
  /** The workflow aggregate read back from the daemon, when available. */
  summary?: BulkSummaryView | null;
  /** A noun for the bulk targets (`repositories`, `services`, `libraries`). */
  targetLabel?: string;
  /** Requests cancellation of a single task. */
  onCancelTask?: (taskId: string) => void;
  /** Requests cancellation of a workflow's active tasks. */
  onCancelWorkflow?: (workflowId: string) => void;
}) {
  if (outcome.status === 'already_in_desired_state') {
    return (
      <p className="dialog__message">
        Nothing to do: the selection is already in the desired state. The daemon
        scheduled no task.
      </p>
    );
  }

  if (outcome.status === 'task' && outcome.task_id) {
    const tracked = tasks.find((candidate) => candidate.id === outcome.task_id);
    const status = task?.status ?? tracked?.status ?? 'queued';
    const failure = task?.failure ?? tracked?.failure ?? null;
    const progressMessage = task?.progress.message ?? tracked?.progressMessage ?? null;
    const current = task?.progress.current ?? tracked?.progressCurrent ?? 0;
    const total = task?.progress.total ?? tracked?.progressTotal ?? null;
    const kind = task?.kind ?? tracked?.kind ?? outcome.kind;
    const active = !isTerminalTaskStatus(status);

    return (
      <div className="operation-summary">
        <dl className="dialog__facts">
          <div>
            <dt>Task</dt>
            <dd className="dialog__code">{outcome.task_id}</dd>
          </div>
          <div>
            <dt>Operation</dt>
            <dd>{kind}</dd>
          </div>
          <div>
            <dt>Status</dt>
            <dd>
              <TaskStatusIndicator status={status} />
            </dd>
          </div>
          {total !== null ? (
            <div>
              <dt>Progress</dt>
              <dd>
                {current} of {total}
              </dd>
            </div>
          ) : null}
          {task?.duration_ms != null ? (
            <div>
              <dt>Duration</dt>
              <dd>{(task.duration_ms / 1000).toFixed(1)}s</dd>
            </div>
          ) : null}
          {progressMessage ? (
            <div className="dialog__facts-wide">
              <dt>Detail</dt>
              <dd>{progressMessage}</dd>
            </div>
          ) : null}
        </dl>
        {failure ? (
          <ErrorDetailsPanel
            errors={[
              {
                code: failure.code,
                message: failure.message,
                detail: task?.failure?.detail ?? null,
              },
            ]}
          />
        ) : null}
        {active && onCancelTask ? (
          <button
            type="button"
            className="button button--small"
            onClick={() => onCancelTask(outcome.task_id as string)}
          >
            Cancel task
          </button>
        ) : null}
      </div>
    );
  }

  if (outcome.status === 'workflow' && outcome.workflow_id) {
    const workflowId = outcome.workflow_id;
    const workflowTasks = tasksForWorkflow(tasks, workflowId);
    const progress = summarizeTasks(workflowTasks);
    const active = !progress.complete;

    return (
      <div className="operation-summary">
        <p className="dialog__message">
          Workflow <code className="dialog__code">{workflowId}</code> scheduled.
        </p>
        {workflowTasks.length === 0 && !summary ? (
          <p className="muted">
            Waiting for the daemon to report the workflow&rsquo;s targets…
          </p>
        ) : (
          <BulkOperationSummary
            progress={progress}
            server={summary}
            targetLabel={targetLabel ?? 'targets'}
          />
        )}
        {workflowTasks.length > 0 ? (
          <ul className="progress__services">
            {workflowTasks.map((workflowTask) => (
              <li key={workflowTask.id}>
                <span className="progress__service">
                  {workflowTask.serviceId ?? 'workflow'}
                </span>
                <span className="progress__kind">{workflowTask.kind}</span>
                <TaskStatusIndicator status={workflowTask.status} />
                {workflowTask.failure ? (
                  <span className="progress__failure">{workflowTask.failure.message}</span>
                ) : null}
              </li>
            ))}
          </ul>
        ) : null}
        {active && onCancelWorkflow ? (
          <button
            type="button"
            className="button button--small"
            onClick={() => onCancelWorkflow(workflowId)}
          >
            Cancel workflow
          </button>
        ) : null}
      </div>
    );
  }

  return null;
}
