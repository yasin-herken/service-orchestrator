import { StatusBadge, type BadgeTone } from '../StatusBadge';
import type { TaskView, WorkflowStatusView } from '../../types/models';
import { WORKFLOW_RUN_STATE_LABELS, type WorkflowRunState } from '../../types/models';
import { formatMillis, formatDuration } from '../../utils/format';

/** Badge tone for a workflow aggregate state. */
function stateTone(state: string): BadgeTone {
  switch (state) {
    case 'succeeded':
      return 'positive';
    case 'running':
      return 'info';
    case 'queued':
      return 'neutral';
    case 'partially_succeeded':
      return 'warning';
    default:
      return 'negative';
  }
}

/** Renders a workflow state as a badge with a human-readable label. */
export function WorkflowStateBadge({ state }: { state: string }) {
  const known = WORKFLOW_RUN_STATE_LABELS[state as WorkflowRunState];
  return <StatusBadge value={state} tone={stateTone(state)} label={known ?? state} />;
}

/** Aggregated task counts by status, from real task rows. */
function countByStatus(tasks: readonly TaskView[]): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const task of tasks) {
    counts[task.status] = (counts[task.status] ?? 0) + 1;
  }
  return counts;
}

/**
 * Realtime workflow progress (`tasks/022.md` §19–§22).
 *
 * Phase progress and counts come from the daemon's aggregated workflow
 * status; per-service rows come from the task engine's own task records for
 * this workflow id. Percentages are never fabricated: a phase without a known
 * total shows an indeterminate bar.
 */
export function WorkflowProgress({
  status,
  tasks,
  startedAt,
  onOpenTask,
}: {
  status: WorkflowStatusView;
  tasks: readonly TaskView[];
  /** The wall-clock second the GUI started observing, for elapsed time. */
  startedAt: number;
  /** Opens a task's bounded logs without leaving the current screen. */
  onOpenTask?: (taskId: string) => void;
}) {
  const counts = countByStatus(tasks);
  const elapsed =
    (status.finished_at ?? Math.floor(Date.now() / 1000)) - (startedAt || 0);

  return (
    <section className="workflow-progress" aria-label="Workflow progress">
      <header className="workflow-progress__header">
        <div>
          <h2 className="card__title">Workflow {status.workflow_id}</h2>
          <span className="muted">{status.kind}</span>
        </div>
        <WorkflowStateBadge state={status.state} />
      </header>

      <dl className="detail-grid">
        <div>
          <dt>State</dt>
          <dd>{WORKFLOW_RUN_STATE_LABELS[status.state as WorkflowRunState] ?? status.state}</dd>
        </div>
        <div>
          <dt>Tasks</dt>
          <dd>
            {status.succeeded} succeeded · {status.failed} failed · {status.cancelled}{' '}
            cancelled · {status.skipped} skipped · {status.pending} pending / {status.total}
          </dd>
        </div>
        <div>
          <dt>Started</dt>
          <dd>{status.started_at ? new Date(status.started_at * 1000).toLocaleString() : '—'}</dd>
        </div>
        <div>
          <dt>Elapsed</dt>
          <dd>{elapsed > 0 ? formatDuration(elapsed) : '—'}</dd>
        </div>
      </dl>

      <div className="workflow-progress__phases">
        <h3 className="dependency-view__title">Phases</h3>
        <ul>
          {status.phases.map((phase) => {
            const known = phase.total > 0;
            return (
              <li key={phase.phase} className="workflow-progress__phase">
                <span className="workflow-progress__phase-name">{phase.phase}</span>
                <span className="workflow-progress__phase-counts" aria-live="polite">
                  {known
                    ? `${phase.succeeded}/${phase.total} succeeded · ${phase.failed} failed · ${phase.pending} pending`
                    : 'no tasks scheduled'}
                </span>
                <span
                  className={`progress__status progress__status--${
                    phase.failed > 0
                      ? 'failed'
                      : phase.pending > 0
                        ? 'running'
                        : 'succeeded'
                  }`}
                >
                  {phase.failed > 0
                    ? 'failed'
                    : phase.pending > 0
                      ? 'running'
                      : 'succeeded'}
                </span>
              </li>
            );
          })}
        </ul>
      </div>

      <div className="workflow-progress__tasks">
        <h3 className="dependency-view__title">Per-service progress</h3>
        <p className="muted">
          Task states:{' '}
          {Object.entries(counts)
            .map(([statusKey, count]) => `${count} ${statusKey}`)
            .join(' · ') || 'no tasks yet'}
        </p>
        <ul className="workflow-progress__task-list">
          {tasks.map((task) => (
            <li key={task.id} className="workflow-progress__task">
              <span className="workflow-progress__service">
                {task.service_id ?? task.library_id ?? '—'}
              </span>
              <span className="workflow-progress__kind">{task.kind}</span>
              <span
                className={`progress__status progress__status--${task.status}`}
                data-status={task.status}
              >
                {task.status}
              </span>
              <span className="muted">{task.progress.message ?? ''}</span>
              <span className="muted">
                {task.duration_ms !== null ? formatMillis(task.duration_ms) : ''}
              </span>
              {onOpenTask ? (
                <button
                  type="button"
                  className="button button--small"
                  onClick={() => onOpenTask(task.id)}
                  aria-label={`Open logs for ${task.id}`}
                >
                  Logs
                </button>
              ) : null}
              {task.failure ? (
                <span className="workflow-progress__failure">
                  {task.failure.code}: {task.failure.message}
                </span>
              ) : null}
            </li>
          ))}
        </ul>
        {tasks.length === 0 ? (
          <p className="muted">Waiting for the daemon to schedule tasks…</p>
        ) : null}
      </div>
    </section>
  );
}
