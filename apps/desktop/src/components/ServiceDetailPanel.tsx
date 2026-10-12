import { EmptyState, ErrorState, LoadingState } from './StateViews';
import { HealthBadge, ProcessBadge } from './StatusBadge';
import type { TrackedTask } from '../context/TaskEventsContext';
import type { DesktopError, ServiceDetail } from '../types/models';
import { formatClock } from '../utils/format';

/**
 * The detail panel for one service.
 *
 * It shows only what the adapter can observe: configured facts plus the last
 * observed process/health/Git state and diagnostics. Values it cannot know yet
 * (for example the current commit, which needs a Git operation deferred to a
 * later task) are labelled as unavailable rather than invented. No secret is
 * ever rendered (`tasks/019.md` §18, §19, §21).
 */
export function ServiceDetailPanel({
  detail,
  loading,
  error,
  lastTask,
  busy,
  disabled,
  onClose,
  onStart,
  onStop,
  onRestart,
  onCheckHealth,
  onViewLogs,
  onViewTasks,
  onRetry,
}: {
  detail: ServiceDetail | null;
  loading: boolean;
  error: DesktopError | null;
  lastTask: TrackedTask | null;
  busy: boolean;
  disabled: boolean;
  onClose: () => void;
  onStart: () => void;
  onStop: () => void;
  onRestart: () => void;
  onCheckHealth: () => void;
  onViewLogs: () => void;
  onViewTasks: () => void;
  onRetry: () => void;
}) {
  const actionsDisabled = disabled || busy;

  return (
    <section className="card detail-panel" aria-label="Service detail">
      <div className="card__header">
        <h2 className="card__title">
          Service detail{detail ? ` · ${detail.display_name}` : ''}
        </h2>
        <button type="button" className="button button--small" onClick={onClose}>
          Close
        </button>
      </div>

      {loading && !detail ? <LoadingState label="Loading service detail…" /> : null}
      {error && !detail ? <ErrorState error={error} onRetry={onRetry} /> : null}
      {!loading && !error && !detail ? (
        <EmptyState title="Select a service to see its details." />
      ) : null}

      {detail ? (
        <>
          <div className="detail-actions" role="group" aria-label="Service actions">
            <button
              type="button"
              className="button button--small button--primary"
              onClick={onStart}
              disabled={actionsDisabled}
            >
              Start
            </button>
            <button
              type="button"
              className="button button--small"
              onClick={onStop}
              disabled={actionsDisabled}
            >
              Stop
            </button>
            <button
              type="button"
              className="button button--small"
              onClick={onRestart}
              disabled={actionsDisabled}
            >
              Restart
            </button>
            <button
              type="button"
              className="button button--small"
              onClick={onCheckHealth}
              disabled={actionsDisabled}
            >
              Check health
            </button>
            <button
              type="button"
              className="button button--small"
              onClick={onViewLogs}
              disabled={disabled}
            >
              View logs
            </button>
            <button
              type="button"
              className="button button--small"
              onClick={onViewTasks}
              disabled={disabled}
            >
              View tasks
            </button>
            {busy ? <span className="muted">Working…</span> : null}
          </div>

          <div className="detail-panel__state">
            <span className="detail-panel__state-item">
              Runtime <ProcessBadge state={detail.process_state} />
            </span>
            <span className="detail-panel__state-item">
              Health <HealthBadge state={detail.health_state} />
            </span>
          </div>

          <dl className="detail-grid">
            <div>
              <dt>Service id</dt>
              <dd className="detail-grid__mono">{detail.id}</dd>
            </div>
            <div>
              <dt>Type</dt>
              <dd>{detail.kind}</dd>
            </div>
            <div>
              <dt>Workspace</dt>
              <dd>{detail.workspace_id}</dd>
            </div>
            <div>
              <dt>Path</dt>
              <dd className="detail-grid__mono">{detail.path ?? '—'}</dd>
            </div>
            <div>
              <dt>Repository</dt>
              <dd className="detail-grid__mono">
                {detail.repository ? detail.repository.local_path : '—'}
              </dd>
            </div>
            <div>
              <dt>Branch</dt>
              <dd className="detail-grid__mono">{detail.branch ?? '—'}</dd>
            </div>
            <div>
              <dt>Commit</dt>
              <dd className="muted">Not inspected yet</dd>
            </div>
            <div>
              <dt>Ports</dt>
              <dd>{detail.ports.length > 0 ? detail.ports.join(', ') : '—'}</dd>
            </div>
            <div>
              <dt>Build system</dt>
              <dd>{detail.build_system ?? '—'}</dd>
            </div>
            <div>
              <dt>Managed process</dt>
              <dd>{detail.managed ? `Yes (pid ${detail.pid ?? '—'})` : 'No'}</dd>
            </div>
            <div>
              <dt>Last operation</dt>
              <dd>
                {lastTask
                  ? `${lastTask.kind} · ${lastTask.status}`
                  : '—'}
              </dd>
            </div>
            <div>
              <dt>Last checked</dt>
              <dd>{detail.last_checked_at ? formatClock(detail.last_checked_at) : '—'}</dd>
            </div>
          </dl>

          <div className="detail-panel__columns">
            <section className="detail-panel__block" aria-label="Runtime requirements">
              <h3 className="detail-panel__block-title">Runtimes</h3>
              {detail.runtimes.length === 0 ? (
                <p className="muted">None declared.</p>
              ) : (
                <ul className="tag-list">
                  {detail.runtimes.map((runtime) => (
                    <li key={`${runtime.kind}:${runtime.version ?? ''}`} className="tag">
                      {runtime.kind}
                      {runtime.version ? ` ${runtime.version}` : ''}
                    </li>
                  ))}
                </ul>
              )}
            </section>

            <section className="detail-panel__block" aria-label="Dependencies">
              <h3 className="detail-panel__block-title">Dependencies</h3>
              {detail.dependencies.length === 0 ? (
                <p className="muted">None declared.</p>
              ) : (
                <ul className="detail-list">
                  {detail.dependencies.map((dependency) => (
                    <li key={`${dependency.target_kind}:${dependency.target}:${dependency.kind}`}>
                      <span className="detail-grid__mono">{dependency.target}</span>
                      <span className="muted">
                        {' '}
                        ({dependency.target_kind}, {dependency.kind}
                        {dependency.required ? ', required' : ', optional'})
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </section>

            <section className="detail-panel__block" aria-label="Health checks">
              <h3 className="detail-panel__block-title">Health checks</h3>
              {detail.health_checks.length === 0 ? (
                <p className="muted">None configured.</p>
              ) : (
                <ul className="detail-list">
                  {detail.health_checks.map((check) => {
                    const outcome = detail.health_outcomes.find(
                      (candidate) =>
                        candidate.check_type === check.kind ||
                        check.kind.startsWith(candidate.check_type),
                    );
                    return (
                      <li key={check.kind}>
                        <span>{check.kind}</span>
                        {outcome?.target ? (
                          <span className="detail-grid__mono muted"> {outcome.target}</span>
                        ) : null}
                        {check.required ? <span className="muted"> · required</span> : null}
                        {outcome ? (
                          <span className="muted">
                            {' '}
                            · {outcome.state}
                            {outcome.message ? `: ${outcome.message}` : ''}
                          </span>
                        ) : (
                          <span className="muted"> · not observed yet</span>
                        )}
                      </li>
                    );
                  })}
                </ul>
              )}
            </section>
          </div>

          {detail.last_failure ? (
            <section className="detail-panel__block" aria-label="Last failure">
              <h3 className="detail-panel__block-title">Last failure</h3>
              <p>
                <span className="dialog__code">{detail.last_failure.code}</span>{' '}
                {detail.last_failure.message}
                {detail.last_failure.at
                  ? ` · ${formatClock(detail.last_failure.at)}`
                  : ''}
              </p>
            </section>
          ) : null}

          <section className="detail-panel__block" aria-label="Recent errors">
            <h3 className="detail-panel__block-title">Recent errors</h3>
            {detail.recent_errors.length === 0 ? (
              <p className="muted">No recent errors.</p>
            ) : (
              <ul className="detail-list detail-list--mono">
                {detail.recent_errors.map((line, index) => (
                  <li key={`${index}:${line}`}>{line}</li>
                ))}
              </ul>
            )}
          </section>
        </>
      ) : null}
    </section>
  );
}
