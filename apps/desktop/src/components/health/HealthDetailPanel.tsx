import { HealthBadge, ProcessBadge } from '../StatusBadge';
import { EmptyState, ErrorState, LoadingState } from '../StateViews';
import type { DesktopError, HealthDetailView, HealthOutcomeView } from '../../types/models';
import { formatDateTime, formatMillis } from '../../utils/format';

/** One observed check with its type, requirement, result, and reason. */
function checkLine(outcome: HealthOutcomeView) {
  return (
    <li key={`${outcome.check_type}:${outcome.target ?? ''}`}>
      <span className="service-table__name">{outcome.check_type}</span>{' '}
      <span className="muted">({outcome.required ? 'required' : 'optional'})</span> —{' '}
      <HealthBadge state={outcome.state} />{' '}
      <span className="muted">{formatMillis(outcome.duration_ms)}</span>
      {outcome.target ? <span className="muted"> · {outcome.target}</span> : null}
      {outcome.message ? <div className="muted">{outcome.message}</div> : null}
      <div className="muted">checked {formatDateTime(outcome.checked_at)}</div>
    </li>
  );
}

/**
 * The Health detail panel (`tasks/021.md` §19, §22).
 *
 * It keeps runtime state and health state visibly separate, lists every
 * observed check with its duration and failure reason, and reports history
 * explicitly as unavailable instead of inventing it when nothing was observed.
 */
export function HealthDetailPanel({
  detail,
  loading,
  error,
  checking,
  disabled,
  onClose,
  onCheck,
  onReadiness,
  onViewLogs,
  onRetry,
}: {
  detail: HealthDetailView | null;
  loading: boolean;
  error: DesktopError | null;
  checking: boolean;
  disabled: boolean;
  onClose: () => void;
  onCheck: () => void;
  onReadiness: () => void;
  onViewLogs: () => void;
  onRetry: () => void;
}) {
  if (!detail) {
    // Never render a blank panel: while the first observation loads show a
    // spinner, on failure show the structured error with a retry, and only
    // then an explicit empty state (`tasks/024.md` §13, §15).
    return (
      <section className="detail-panel" role="region" aria-label="Health detail">
        <header className="detail-panel__block-header">
          <h2 className="detail-panel__block-title">Health detail</h2>
          <button type="button" className="button button--small" onClick={onClose}>
            Close
          </button>
        </header>
        {loading ? <LoadingState label="Loading health detail…" /> : null}
        {!loading && error ? <ErrorState error={error} onRetry={onRetry} /> : null}
        {!loading && !error ? (
          <EmptyState
            title="No health detail"
            description="The daemon did not report an observation for this service."
          />
        ) : null}
      </section>
    );
  }
  const { row } = detail;
  const historyKnown =
    row.last_healthy_at !== null ||
    row.last_unhealthy_at !== null ||
    row.consecutive_failures > 0;

  return (
    <section className="detail-panel" role="region" aria-label="Health detail">
      <header className="detail-panel__block-header">
        <h2 className="detail-panel__block-title">Health — {row.display_name}</h2>
        <button type="button" className="button button--small" onClick={onClose}>
          Close
        </button>
      </header>

      <div className="detail-panel__block">
        <h3 className="detail-panel__block-title">Runtime and health</h3>
        {loading ? <p className="muted">Refreshing health detail…</p> : null}
        {error ? (
          <p className="detail-panel__state-item" role="alert">
            {error.message} <span className="muted">({error.code})</span>
          </p>
        ) : null}
        <div className="detail-panel__state">
          <div className="detail-panel__state-item">
            <span className="muted">Runtime</span> <ProcessBadge state={row.process_state} />
          </div>
          <div className="detail-panel__state-item">
            <span className="muted">Health</span> <HealthBadge state={row.health_state} />
          </div>
          <div className="detail-panel__state-item">
            <span className="muted">Last checked</span> {formatDateTime(row.checked_at)}
          </div>
        </div>
        <p className="muted">
          Runtime and health are independent observations: a running process can
          be unhealthy, and a starting process reports checking rather than a
          verdict.
        </p>
      </div>

      <div className="detail-panel__block">
        <h3 className="detail-panel__block-title">Configured checks</h3>
        {detail.checks.length === 0 ? (
          <p className="muted" role="status">
            No check has been observed for this service yet. Run a health check
            to populate real results.
          </p>
        ) : (
          <ul className="detail-list">{detail.checks.map(checkLine)}</ul>
        )}
      </div>

      <div className="detail-panel__block">
        <h3 className="detail-panel__block-title">History</h3>
        {historyKnown ? (
          <dl className="detail-grid">
            <div>
              <dt>Last healthy</dt>
              <dd>{formatDateTime(row.last_healthy_at)}</dd>
            </div>
            <div>
              <dt>Last unhealthy</dt>
              <dd>{formatDateTime(row.last_unhealthy_at)}</dd>
            </div>
            <div>
              <dt>Consecutive failures</dt>
              <dd>{row.consecutive_failures}</dd>
            </div>
          </dl>
        ) : (
          <p className="muted" role="status">
            Health history is unavailable: no transition has been observed for
            this service. Nothing is fabricated.
          </p>
        )}
      </div>

      <div className="detail-panel__block">
        <h3 className="detail-panel__block-title">Diagnostics</h3>
        <dl className="detail-grid">
          <div>
            <dt>Managed process</dt>
            <dd>{detail.managed ? 'Yes' : 'No'}</dd>
          </div>
          <div>
            <dt>PID</dt>
            <dd className="detail-grid__mono">
              {detail.pid ?? <span className="muted">—</span>}
            </dd>
          </div>
          <div className="detail-grid__wide">
            <dt>Ports</dt>
            <dd className="detail-grid__mono">
              {detail.ports.length > 0 ? detail.ports.join(', ') : <span className="muted">—</span>}
            </dd>
          </div>
        </dl>
        {detail.last_failure ? (
          <div className="error-details" role="alert">
            <p className="error-details__label">Last failure ({detail.last_failure.source})</p>
            <p className="error-details__message">{detail.last_failure.message}</p>
            <p className="error-details__code">{detail.last_failure.code}</p>
          </div>
        ) : null}
        {detail.recent_errors.length > 0 ? (
          <>
            <p className="detail-panel__block-title">Recent errors</p>
            <ul className="detail-list detail-list--mono">
              {detail.recent_errors.map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
          </>
        ) : null}
      </div>

      <div className="detail-panel__block">
        <h3 className="detail-panel__block-title">Actions</h3>
        <div className="detail-actions">
          <button
            type="button"
            className="button button--primary"
            onClick={onCheck}
            disabled={disabled || checking}
          >
            {checking ? 'Checking…' : 'Run health check'}
          </button>
          <button
            type="button"
            className="button"
            onClick={onReadiness}
            disabled={disabled || checking}
          >
            Run readiness check
          </button>
          <button type="button" className="button" onClick={onViewLogs} disabled={disabled}>
            View service logs
          </button>
        </div>
        <p className="muted">
          Checks run inside the daemon&rsquo;s task engine; React never probes an
          endpoint and no second scheduler exists.
        </p>
      </div>
    </section>
  );
}
