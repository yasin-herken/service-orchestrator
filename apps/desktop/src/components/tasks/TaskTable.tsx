import { TaskStatusIndicator } from './TaskStatusIndicator';
import type { TrackedTask } from '../../context/TaskEventsContext';
import type { TaskView } from '../../types/models';
import { formatDateTime, formatMillis } from '../../utils/format';

/** The live progress message for a task, when the engine reports one. */
function progressMessage(task: TaskView, tracked: TrackedTask | undefined): string {
  return (
    task.progress.message ??
    tracked?.progressMessage ??
    (task.progress.total === null
      ? `step ${task.progress.current}`
      : `${task.progress.current} of ${task.progress.total}`)
  );
}

/**
 * The Tasks table: real task-engine rows, newest first (`tasks/021.md` §10).
 *
 * Every terminal state keeps its own label — a cancelled task is never shown
 * as a generic “completed” — and progress is indeterminate whenever the engine
 * reports no reliable total.
 */
export function TaskTable({
  tasks,
  tracked,
  detailId,
  onOpenDetail,
  workspaceName,
}: {
  tasks: TaskView[];
  /** Live event-sourced records, keyed by task id, for freshest messages. */
  tracked: ReadonlyMap<string, TrackedTask>;
  detailId: string | null;
  onOpenDetail: (taskId: string) => void;
  workspaceName: string;
}) {
  return (
    <table className="service-table">
      <caption className="visually-hidden">Tasks in {workspaceName}</caption>
      <thead>
        <tr>
          <th scope="col">Task</th>
          <th scope="col">Type</th>
          <th scope="col">Workspace</th>
          <th scope="col">Service</th>
          <th scope="col">Workflow</th>
          <th scope="col">Status</th>
          <th scope="col">Progress</th>
          <th scope="col">Started</th>
          <th scope="col">Duration</th>
          <th scope="col">Current message</th>
        </tr>
      </thead>
      <tbody>
        {tasks.map((task) => {
          const open = task.id === detailId;
          const live = tracked.get(task.id);
          const status = live?.status ?? task.status;
          return (
            <tr
              key={task.id}
              tabIndex={0}
              aria-selected={open}
              className={`service-table__row${open ? ' service-table__row--detail' : ''}`}
              onClick={() => onOpenDetail(task.id)}
              onKeyDown={(event) => {
                if (event.target !== event.currentTarget) {
                  return;
                }
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  onOpenDetail(task.id);
                }
              }}
            >
              <td className="service-table__mono">{task.id}</td>
              <td>{task.kind}</td>
              <td>{task.workspace_id ?? <span className="muted">—</span>}</td>
              <td>{task.service_id ?? task.library_id ?? <span className="muted">—</span>}</td>
              <td className="service-table__mono">
                {task.workflow_id ?? <span className="muted">—</span>}
              </td>
              <td>
                <TaskStatusIndicator status={status} />
              </td>
              <td className="service-table__operation">
                <span
                  className="service-table__mono"
                  title={
                    task.progress.total === null
                      ? 'No reliable percentage is available'
                      : undefined
                  }
                >
                  {task.progress.fraction === null
                    ? 'Indeterminate'
                    : `${Math.round(task.progress.fraction * 100)}%`}
                </span>
              </td>
              <td className="service-table__mono">{formatDateTime(task.started_at)}</td>
              <td className="service-table__mono">{formatMillis(task.duration_ms)}</td>
              <td className="service-table__operation">{progressMessage(task, live)}</td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
