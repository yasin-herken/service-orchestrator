import { StatusBadge } from '../StatusBadge';
import { TaskStatusIndicator } from '../tasks/TaskStatusIndicator';
import type { MigrationRowView } from '../../types/models';
import { useIndeterminateCheckbox } from '../../hooks/useIndeterminateCheckbox';
import { formatDateTime } from '../../utils/format';

/** Badge tone for a declared target environment. */
export function environmentTone(environment: string) {
  switch (environment) {
    case 'local':
      return 'positive' as const;
    case 'staging':
    case 'test':
      return 'warning' as const;
    case 'production':
    case 'prod':
      return 'negative' as const;
    default:
      return 'neutral' as const;
  }
}

/** When the latest migration task last ran, or `null` when it never did. */
function lastRunAt(row: MigrationRowView): number | null {
  const task = row.latest_task;
  if (!task) {
    return null;
  }
  return task.finished_at ?? task.started_at ?? task.created_at;
}

/**
 * The Migrations table: one row per migration-configured service of the active
 * workspace, built from the configuration snapshot plus the real latest
 * migration task (`tasks/021.md` §4).
 *
 * Unknown facts stay unknown: `pending` renders as “Unknown” until an
 * inspection task answers, and a service without any task shows “Never run”
 * rather than an invented result.
 */
export function MigrationTable({
  rows,
  workspaceName,
  selectedIds,
  onToggleSelect,
  onToggleSelectAll,
  allVisibleSelected,
  someVisibleSelected,
  detailId,
  onOpenDetail,
  busy,
  disabled,
}: {
  rows: MigrationRowView[];
  /** The workspace every row belongs to (the table is workspace-scoped). */
  workspaceName: string;
  selectedIds: ReadonlySet<string>;
  onToggleSelect: (serviceId: string, selected: boolean) => void;
  onToggleSelectAll: (selected: boolean) => void;
  allVisibleSelected: boolean;
  someVisibleSelected: boolean;
  detailId: string | null;
  onOpenDetail: (serviceId: string) => void;
  busy: ReadonlySet<string>;
  disabled: boolean;
}) {
  const headerCheckbox = useIndeterminateCheckbox(someVisibleSelected);

  return (
    <table className="service-table">
      <caption className="visually-hidden">Migration targets in {workspaceName}</caption>
      <thead>
        <tr>
          <th scope="col" className="service-table__col--select">
            <input
              ref={headerCheckbox}
              type="checkbox"
              aria-label="Select all visible services"
              checked={allVisibleSelected}
              onChange={(event) => onToggleSelectAll(event.target.checked)}
            />
          </th>
          <th scope="col">Service</th>
          <th scope="col">Environment</th>
          <th scope="col">Provider</th>
          <th scope="col">Database target</th>
          <th scope="col">Enabled</th>
          <th scope="col">Pending</th>
          <th scope="col">Latest result</th>
          <th scope="col">Last run</th>
          <th scope="col">Actions</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((row) => {
          const { status, latest_task: latest } = row;
          const selected = selectedIds.has(status.service_id);
          const isBusy = busy.has(status.service_id);
          const open = status.service_id === detailId;
          const ranAt = lastRunAt(row);
          return (
            <tr
              key={status.service_id}
              tabIndex={0}
              aria-selected={selected}
              aria-busy={isBusy || undefined}
              className={`service-table__row${selected ? ' service-table__row--selected' : ''}${
                open ? ' service-table__row--detail' : ''
              }${isBusy ? ' service-table__row--busy' : ''}`}
              onClick={() => onOpenDetail(status.service_id)}
              onKeyDown={(event) => {
                if (event.target !== event.currentTarget) {
                  return;
                }
                if (event.key === 'Enter') {
                  event.preventDefault();
                  onOpenDetail(status.service_id);
                } else if (event.key === ' ') {
                  event.preventDefault();
                  onToggleSelect(status.service_id, !selected);
                }
              }}
            >
              <td className="service-table__col--select">
                <input
                  type="checkbox"
                  aria-label={`Select ${status.service_id}`}
                  checked={selected}
                  disabled={disabled || !status.enabled}
                  onClick={(event) => event.stopPropagation()}
                  onChange={(event) =>
                    onToggleSelect(status.service_id, event.target.checked)
                  }
                />
              </td>
              <td>
                <span className="service-table__name">{status.service_id}</span>
                <span className="service-table__id">{status.workspace_id}</span>
              </td>
              <td>
                <StatusBadge
                  value={status.environment}
                  tone={environmentTone(status.environment)}
                />
                {status.requires_authorization ? (
                  <span className="tag tag--warning" title="Applying to this target requires explicit authorization">
                    authorization required
                  </span>
                ) : null}
              </td>
              <td>
                {status.provider}
                <span className="muted"> · {status.execution_mode}</span>
              </td>
              <td className="service-table__mono" title={status.database_identity}>
                {status.database_identity}
              </td>
              <td>{status.enabled ? 'Yes' : 'No'}</td>
              <td className="service-table__col--numeric">
                {status.pending === null ? (
                  <span className="muted" title="No inspection task has answered yet">
                    Unknown
                  </span>
                ) : (
                  status.pending
                )}
              </td>
              <td className="service-table__operation">
                {isBusy ? <span className="service-table__busy">Working…</span> : null}
                {latest ? (
                  <span>
                    <TaskStatusIndicator status={latest.status} /> {latest.kind}
                  </span>
                ) : (
                  <span className="muted">Never run</span>
                )}
              </td>
              <td className="service-table__mono">{formatDateTime(ranAt)}</td>
              <td className="service-table__operation">
                <button
                  type="button"
                  className="button button--small"
                  onClick={(event) => {
                    event.stopPropagation();
                    onOpenDetail(status.service_id);
                  }}
                >
                  Inspect
                </button>
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
