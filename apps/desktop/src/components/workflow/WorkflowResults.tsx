import type { TaskView, WorkflowStatusView } from '../../types/models';
import { WORKFLOW_RUN_STATE_LABELS, type WorkflowRunState } from '../../types/models';
import { formatMillis } from '../../utils/format';
import { WorkflowStateBadge } from './WorkflowProgress';

/**
 * The workflow results view (`tasks/022.md` §25, §27).
 *
 * The overall status is the daemon's own aggregate: a partially succeeded
 * workflow is reported as such — never as success — and failed, cancelled, and
 * blocked tasks are listed with their structured errors. Retry is offered only
 * as "plan again" from the same configuration: nothing reruns automatically.
 */
export function WorkflowResults({
  status,
  tasks,
  onPlanAgain,
  onOpenTask,
  onViewLogs,
}: {
  status: WorkflowStatusView;
  tasks: readonly TaskView[];
  onPlanAgain: () => void;
  onOpenTask: (taskId: string) => void;
  onViewLogs: (serviceId: string) => void;
}) {
  const stateLabel =
    WORKFLOW_RUN_STATE_LABELS[status.state as WorkflowRunState] ?? status.state;
  const duration =
    status.started_at !== null && status.finished_at !== null
      ? status.finished_at - status.started_at
      : null;

  return (
    <section className="workflow-results" aria-label="Workflow results">
      <header className="workflow-results__header">
        <div>
          <h2 className="card__title">Results — workflow {status.workflow_id}</h2>
          <span className="muted">{status.kind}</span>
        </div>
        <WorkflowStateBadge state={status.state} />
      </header>

      <p className="workflow-results__headline" role="status">
        {stateLabel}
        {duration !== null ? ` · ${formatMillis(duration * 1000)}` : ''}
      </p>

      <dl className="detail-grid">
        <div>
          <dt>Total tasks</dt>
          <dd>{status.total}</dd>
        </div>
        <div>
          <dt>Succeeded</dt>
          <dd>{status.succeeded}</dd>
        </div>
        <div>
          <dt>Failed</dt>
          <dd>{status.failed}</dd>
        </div>
        <div>
          <dt>Cancelled</dt>
          <dd>{status.cancelled}</dd>
        </div>
        <div>
          <dt>Skipped</dt>
          <dd>{status.skipped}</dd>
        </div>
        <div>
          <dt>Blocked / pending</dt>
          <dd>{status.pending}</dd>
        </div>
      </dl>

      <div className="workflow-results__phases">
        <h3 className="dependency-view__title">Per-phase results</h3>
        <ul>
          {status.phases.map((phase) => (
            <li key={phase.phase} className="workflow-progress__phase">
              <span className="workflow-progress__phase-name">{phase.phase}</span>
              <span className="workflow-progress__phase-counts">
                {phase.succeeded} succeeded · {phase.failed} failed · {phase.pending} pending
                {phase.total > 0 ? ` of ${phase.total}` : ''}
              </span>
            </li>
          ))}
        </ul>
      </div>

      {status.failures.length > 0 ? (
        <div className="workflow-results__failures" aria-label="Failed tasks">
          <h3 className="dependency-view__title">Failed tasks</h3>
          <ul>
            {status.failures.map((failure) => (
              <li key={failure.task_id} className="workflow-results__failure">
                <span className="error-details__code">{failure.code}</span>
                <span className="detail-grid__mono">{failure.task_id}</span>
                <span>
                  {failure.service ?? failure.library ?? '—'} · {failure.kind}
                </span>
                <span>{failure.message}</span>
                <button
                  type="button"
                  className="button button--small"
                  onClick={() => onOpenTask(failure.task_id)}
                >
                  Open task
                </button>
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      <div className="workflow-results__tasks">
        <h3 className="dependency-view__title">Per-service results</h3>
        <ul className="workflow-progress__task-list">
          {tasks.map((task) => (
            <li key={task.id} className="workflow-progress__task">
              <span className="workflow-progress__service">
                {task.service_id ?? task.library_id ?? '—'}
              </span>
              <span className="workflow-progress__kind">{task.kind}</span>
              <span className="detail-grid__mono">{task.id}</span>
              <span
                className={`progress__status progress__status--${task.status}`}
                data-status={task.status}
              >
                {task.status}
              </span>
              <span className="muted">
                {task.duration_ms !== null ? formatMillis(task.duration_ms) : ''}
              </span>
              <button
                type="button"
                className="button button--small"
                onClick={() => onOpenTask(task.id)}
              >
                Task
              </button>
              {task.service_id ? (
                <button
                  type="button"
                  className="button button--small"
                  onClick={() => onViewLogs(task.service_id!)}
                >
                  Logs
                </button>
              ) : null}
            </li>
          ))}
        </ul>
        {tasks.length === 0 ? <p className="muted">No task details retained.</p> : null}
      </div>

      <p className="dialog__note muted">
        Nothing is retried automatically. Planning again builds a fresh plan from
        the same configuration; the daemon re-validates every precondition —
        including migrations — before executing it.
      </p>

      <div className="detail-actions">
        <button type="button" className="button button--primary" onClick={onPlanAgain}>
          Plan again from this configuration
        </button>
      </div>
    </section>
  );
}
