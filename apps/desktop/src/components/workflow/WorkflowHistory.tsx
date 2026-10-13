import type { DesktopError, WorkflowSummaryView } from '../../types/models';
import { ErrorState, LoadingState } from '../StateViews';
import { pluralize } from '../../utils/format';
import { WorkflowStateBadge } from './WorkflowProgress';

/**
 * Recent workflow history (`tasks/022.md` §26).
 *
 * The list is the daemon's bounded, workspace-scoped view derived from
 * retained tasks — the GUI renders it as-is and claims no persistence across
 * daemon restarts.
 */
export function WorkflowHistory({
  rows,
  loading,
  error,
  onRetry,
  onSelect,
}: {
  rows: readonly WorkflowSummaryView[];
  loading: boolean;
  error: DesktopError | null;
  onRetry: () => void;
  onSelect?: (workflowId: string) => void;
}) {
  return (
    <section className="card" aria-label="Recent workflows">
      <div className="card__header">
        <h2 className="card__title">Recent workflows</h2>
        <span className="muted">Bounded history from the daemon</span>
      </div>
      {loading ? <LoadingState label="Loading workflow history…" /> : null}
      {error ? <ErrorState error={error} onRetry={onRetry} /> : null}
      {!loading && !error ? (
        rows.length === 0 ? (
          <p className="muted">No workflows recorded for this workspace yet.</p>
        ) : (
          <table className="service-table">
            <thead>
              <tr>
                <th scope="col">Workflow</th>
                <th scope="col">Kind</th>
                <th scope="col">Status</th>
                <th scope="col">Tasks</th>
                <th scope="col">Started</th>
                <th scope="col" aria-label="Actions" />
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <tr key={row.workflow_id}>
                  <td>{row.workflow_id}</td>
                  <td>{row.kind}</td>
                  <td>
                    <WorkflowStateBadge state={row.state} />
                  </td>
                  <td>
                    {row.succeeded}/{row.total} succeeded
                    {row.failed > 0 ? ` · ${row.failed} failed` : ''}
                  </td>
                  <td>
                    {row.started_at
                      ? new Date(row.started_at * 1000).toLocaleString()
                      : '—'}
                  </td>
                  <td>
                    {onSelect ? (
                      <button
                        type="button"
                        className="button button--small"
                        onClick={() => onSelect(row.workflow_id)}
                      >
                        Inspect
                      </button>
                    ) : null}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )
      ) : null}
      {rows.length > 0 ? (
        <p className="muted">{pluralize(rows.length, 'workflow')} shown.</p>
      ) : null}
    </section>
  );
}
