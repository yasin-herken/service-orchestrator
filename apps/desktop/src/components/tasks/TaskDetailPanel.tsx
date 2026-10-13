import { LogViewer } from './LogViewer';
import { TaskStatusIndicator, isTerminalTaskStatus, taskStatusLabel } from './TaskStatusIndicator';
import type { TrackedTask } from '../../context/TaskEventsContext';
import type { TaskView, WorkflowView } from '../../types/models';
import { formatDateTime, formatMillis } from '../../utils/format';

/** A single workflow relationship, rendered from real ids — never names. */
function relationshipLine(step: WorkflowView['steps'][number]): string {
  const depends = step.depends_on.length > 0 ? ` after ${step.depends_on.join(', ')}` : '';
  return `${step.id} · ${step.task_kind}${depends}`;
}

/**
 * The task detail panel (`tasks/021.md` §13, §14, §16).
 *
 * It shows the engine's own identity, state, progress, timestamps, structured
 * failure, related logs, and — when the task belongs to a workflow — the real
 * step ids and dependency edges the daemon declared. Cancellation is a daemon
 * call; this panel never decides locally that a task stopped.
 */
export function TaskDetailPanel({
  task,
  tracked,
  workflow,
  workflowLoading,
  siblings,
  cancelling,
  cancelResult,
  cancelError,
  disabled,
  onCancel,
  onViewLogs,
  onClose,
}: {
  task: TaskView | null;
  tracked: TrackedTask | null;
  workflow: WorkflowView | null;
  workflowLoading: boolean;
  /** Other tasks of the same workflow, from the same bounded listing. */
  siblings: TaskView[];
  cancelling: boolean;
  /** Accurate description of the last cancellation request, when one was made. */
  cancelResult: string | null;
  cancelError: string | null;
  disabled: boolean;
  onCancel: () => void;
  onViewLogs: () => void;
  onClose: () => void;
}) {
  if (!task) {
    return null;
  }
  const status = tracked?.status ?? task.status;
  const terminal = isTerminalTaskStatus(status);
  const failure = task.failure;
  const progress = task.progress;

  return (
    <section className="detail-panel" role="region" aria-label="Task detail">
      <header className="detail-panel__block-header">
        <h2 className="detail-panel__block-title">Task {task.id}</h2>
        <button type="button" className="button button--small" onClick={onClose}>
          Close
        </button>
      </header>

      <div className="detail-panel__block">
        <h3 className="detail-panel__block-title">Identity and state</h3>
        <dl className="detail-grid">
          <div>
            <dt>Task id</dt>
            <dd className="detail-grid__mono">{task.id}</dd>
          </div>
          <div>
            <dt>Operation</dt>
            <dd>{task.kind}</dd>
          </div>
          <div>
            <dt>State</dt>
            <dd>
              <TaskStatusIndicator status={status} />
              <span className="muted"> {taskStatusLabel(status)}</span>
            </dd>
          </div>
          <div>
            <dt>Workspace</dt>
            <dd>{task.workspace_id ?? <span className="muted">unknown</span>}</dd>
          </div>
          <div>
            <dt>Service</dt>
            <dd>{task.service_id ?? <span className="muted">—</span>}</dd>
          </div>
          <div>
            <dt>Library</dt>
            <dd>{task.library_id ?? <span className="muted">—</span>}</dd>
          </div>
          <div className="detail-grid__wide">
            <dt>Workflow</dt>
            <dd className="detail-grid__mono">
              {task.workflow_id ?? <span className="muted">not part of a workflow</span>}
            </dd>
          </div>
        </dl>
      </div>

      <div className="detail-panel__block">
        <h3 className="detail-panel__block-title">Progress</h3>
        <div
          className="progress"
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={progress.total ?? undefined}
          aria-valuenow={progress.total === null ? undefined : progress.current}
          aria-valuetext={
            progress.total === null
              ? 'indeterminate progress'
              : `${progress.current} of ${progress.total}`
          }
        >
          <div className="progress__counts">
            {progress.total === null
              ? 'Indeterminate — the engine reports no reliable total'
              : `${progress.current} of ${progress.total}`}
            {progress.fraction !== null
              ? ` · ${Math.round(progress.fraction * 100)}%`
              : ''}
          </div>
          <div className="progress__status">Current phase: {progress.message ?? '—'}</div>
        </div>
      </div>

      <div className="detail-panel__block">
        <h3 className="detail-panel__block-title">Timestamps</h3>
        <dl className="detail-grid">
          <div>
            <dt>Created</dt>
            <dd>{formatDateTime(task.created_at)}</dd>
          </div>
          <div>
            <dt>Started</dt>
            <dd>{formatDateTime(task.started_at)}</dd>
          </div>
          <div>
            <dt>Completed</dt>
            <dd>{formatDateTime(task.finished_at)}</dd>
          </div>
          <div>
            <dt>Duration</dt>
            <dd>{formatMillis(task.duration_ms)}</dd>
          </div>
        </dl>
      </div>

      <div className="detail-panel__block">
        <h3 className="detail-panel__block-title">Structured result</h3>
        {failure ? (
          <div className="error-details" role="alert">
            <p className="error-details__label">Failure</p>
            <p className="error-details__message">{failure.message}</p>
            <p className="error-details__code">{failure.code}</p>
            {failure.detail ? <p className="muted">{failure.detail}</p> : null}
          </div>
        ) : terminal ? (
          <p className="muted">
            No structured error: the task settled as {taskStatusLabel(status).toLowerCase()}.
          </p>
        ) : (
          <p className="muted">The task has not settled yet.</p>
        )}
      </div>

      {task.workflow_id ? (
        <div className="detail-panel__block">
          <h3 className="detail-panel__block-title">Workflow relationships</h3>
          {workflowLoading ? <p className="muted">Loading workflow…</p> : null}
          {workflow ? (
            <ul className="detail-list detail-list--mono">
              {workflow.steps.map((step) => (
                <li key={step.id}>
                  {relationshipLine(step)}
                  {step.description ? <span className="muted"> — {step.description}</span> : null}
                </li>
              ))}
            </ul>
          ) : !workflowLoading ? (
            <p className="muted">The daemon did not report this workflow.</p>
          ) : null}
          {siblings.length > 0 ? (
            <>
              <p className="detail-panel__block-title">Tasks in this workflow</p>
              <ul className="detail-list detail-list--mono">
                {siblings.map((sibling) => (
                  <li key={sibling.id}>
                    {sibling.id} · {sibling.kind} · {taskStatusLabel(sibling.status)}
                  </li>
                ))}
              </ul>
            </>
          ) : null}
        </div>
      ) : null}

      <div className="detail-panel__block">
        <h3 className="detail-panel__block-title">Actions</h3>
        <div className="detail-actions">
          <button
            type="button"
            className="button button--danger"
            onClick={onCancel}
            disabled={disabled || cancelling || terminal}
            title={
              terminal
                ? 'Terminal tasks cannot be cancelled'
                : 'Ask the daemon task engine to cancel this task'
            }
          >
            {cancelling ? 'Cancelling…' : 'Cancel task'}
          </button>
          <button type="button" className="button" onClick={onViewLogs}>
            View task logs
          </button>
        </div>
        {terminal ? (
          <p className="muted">
            This task is {taskStatusLabel(status).toLowerCase()}, so cancellation is not offered.
          </p>
        ) : null}
        {cancelResult ? (
          <p className="detail-panel__state-item" role="status">
            {cancelResult}
          </p>
        ) : null}
        {cancelError ? (
          <p className="detail-panel__state-item" role="alert">
            {cancelError}
          </p>
        ) : null}
      </div>

      <div className="detail-panel__block">
        <h3 className="detail-panel__block-title">Related logs</h3>
        <LogViewer taskId={task.id} live={!terminal} />
      </div>
    </section>
  );
}
