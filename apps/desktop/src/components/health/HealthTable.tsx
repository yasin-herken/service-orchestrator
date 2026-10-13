import { HealthBadge, ProcessBadge } from '../StatusBadge';
import type { HealthOutcomeView, HealthRowView } from '../../types/models';
import { formatDateTime, formatMillis } from '../../utils/format';

/** Renders one per-check cell: its observed state, or an explicit unknown. */
function checkCell(outcome: HealthOutcomeView | null) {
  if (!outcome) {
    return (
      <span className="muted" title="No observation for this check yet">
        Not checked
      </span>
    );
  }
  return (
    <span title={outcome.message ?? undefined}>
      <HealthBadge state={outcome.state} />
      <span className="muted"> {formatMillis(outcome.duration_ms)}</span>
    </span>
  );
}

/** The most recent observed health transition, or `null` when none exists. */
function lastTransition(row: HealthRowView): number | null {
  const candidates = [row.last_healthy_at, row.last_unhealthy_at].filter(
    (value): value is number => value !== null,
  );
  if (candidates.length === 0) {
    return null;
  }
  return Math.max(...candidates);
}

/**
 * The Health table (`tasks/021.md` §17, §18).
 *
 * Runtime state and health state are separate columns on purpose: a `running`
 * process may be `unhealthy`, and a service that has never been checked shows
 * an explicit unknown rather than a fabricated healthy state.
 */
export function HealthTable({
  rows,
  detailId,
  onOpenDetail,
  onCheck,
  checking,
  disabled,
}: {
  rows: HealthRowView[];
  detailId: string | null;
  onOpenDetail: (serviceId: string) => void;
  onCheck: (serviceId: string) => void;
  /** Services with a health-check task in flight. */
  checking: ReadonlySet<string>;
  disabled: boolean;
}) {
  return (
    <table className="service-table">
      <caption className="visually-hidden">Service health</caption>
      <thead>
        <tr>
          <th scope="col">Service</th>
          <th scope="col">Runtime</th>
          <th scope="col">Overall health</th>
          <th scope="col">Process</th>
          <th scope="col">TCP</th>
          <th scope="col">HTTP</th>
          <th scope="col">Last check</th>
          <th scope="col">Last transition</th>
          <th scope="col">Failures</th>
          <th scope="col">Actions</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((row) => {
          const open = row.service_id === detailId;
          const busy = checking.has(row.service_id);
          return (
            <tr
              key={row.service_id}
              tabIndex={0}
              aria-selected={open}
              aria-busy={busy || undefined}
              className={`service-table__row${open ? ' service-table__row--detail' : ''}`}
              onClick={() => onOpenDetail(row.service_id)}
              onKeyDown={(event) => {
                if (event.target !== event.currentTarget) {
                  return;
                }
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  onOpenDetail(row.service_id);
                }
              }}
            >
              <td>
                <span className="service-table__name">{row.display_name}</span>
                <span className="service-table__id">{row.workspace_id}</span>
              </td>
              <td>
                <ProcessBadge state={row.process_state} />
              </td>
              <td>
                <HealthBadge state={row.health_state} />
              </td>
              <td>{checkCell(row.process_check)}</td>
              <td>{checkCell(row.tcp_check)}</td>
              <td>{checkCell(row.http_check)}</td>
              <td className="service-table__mono">{formatDateTime(row.checked_at)}</td>
              <td className="service-table__mono">{formatDateTime(lastTransition(row))}</td>
              <td className="service-table__col--numeric">
                {row.consecutive_failures}
              </td>
              <td className="service-table__operation">
                <button
                  type="button"
                  className="button button--small"
                  onClick={(event) => {
                    event.stopPropagation();
                    onOpenDetail(row.service_id);
                  }}
                >
                  Details
                </button>
                <button
                  type="button"
                  className="button button--small"
                  disabled={disabled || busy}
                  onClick={(event) => {
                    event.stopPropagation();
                    onCheck(row.service_id);
                  }}
                >
                  {busy ? 'Checking…' : 'Check now'}
                </button>
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
