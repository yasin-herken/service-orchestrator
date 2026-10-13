import { StatusBadge } from '../StatusBadge';
import { TaskStatusIndicator } from '../tasks/TaskStatusIndicator';
import type { MigrationRowView, MigrationStatusView } from '../../types/models';
import { formatDateTime, formatMillis } from '../../utils/format';
import { environmentTone } from './MigrationTable';

/** One capability the backend exposes for a migration target. */
interface Capability {
  name: string;
  supported: boolean;
  /** Why the capability is unavailable, when it is. */
  reason?: string;
}

/** The migration operations the application core exposes (`tasks/021.md` §5). */
function capabilities(status: MigrationStatusView): Capability[] {
  const disabledReason = status.enabled
    ? undefined
    : 'migrations are disabled for this service in configuration';
  return [
    { name: 'Validate configuration', supported: true },
    { name: 'Inspect migration status', supported: true },
    { name: 'Inspect pending changesets', supported: true },
    { name: 'Generate SQL preview', supported: true },
    {
      name: 'Apply migration',
      supported: status.enabled,
      ...(disabledReason ? { reason: disabledReason } : {}),
    },
  ];
}

/**
 * The Migrations detail panel: the configuration snapshot, the capability list,
 * and the latest real migration task (`tasks/021.md` §6).
 *
 * The panel only ever renders display-safe data — the database identity comes
 * from the daemon's redacted snapshot, so no credential can appear here.
 */
export function MigrationDetailPanel({
  row,
  status,
  loading,
  loadError,
  busy,
  disabled,
  onClose,
  onValidate,
  onInspectStatus,
  onInspectPending,
  onPreview,
  onApply,
  onOpenTask,
  onViewLogs,
}: {
  /** The row the panel expands (for the latest task). */
  row: MigrationRowView | null;
  /** The freshest snapshot, or the row's own snapshot before the refresh lands. */
  status: MigrationStatusView | null;
  loading: boolean;
  loadError: string | null;
  busy: boolean;
  disabled: boolean;
  onClose: () => void;
  onValidate: () => void;
  onInspectStatus: () => void;
  onInspectPending: () => void;
  onPreview: () => void;
  onApply: () => void;
  onOpenTask: (taskId: string) => void;
  onViewLogs: () => void;
}) {
  if (!row || !status) {
    return null;
  }
  const latest = row.latest_task;
  const failure = latest?.failure ?? null;

  return (
    <section className="detail-panel" role="region" aria-label="Migration detail">
      <header className="detail-panel__block-header">
        <h2 className="detail-panel__block-title">
          Migrations — {status.service_id}
        </h2>
        <button type="button" className="button button--small" onClick={onClose}>
          Close
        </button>
      </header>

      <div className="detail-panel__block">
        <h3 className="detail-panel__block-title">Target</h3>
        {loading ? <p className="muted">Refreshing migration status…</p> : null}
        {loadError ? (
          <p className="detail-panel__state-item" role="alert">
            {loadError}
          </p>
        ) : null}
        <dl className="detail-grid">
          <div>
            <dt>Service</dt>
            <dd>{status.service_id}</dd>
          </div>
          <div>
            <dt>Workspace</dt>
            <dd>{status.workspace_id}</dd>
          </div>
          <div>
            <dt>Provider</dt>
            <dd>
              {status.provider} · {status.execution_mode}
            </dd>
          </div>
          <div>
            <dt>Environment</dt>
            <dd>
              <StatusBadge value={status.environment} tone={environmentTone(status.environment)} />
              {status.environment === 'local' ? (
                <span className="muted"> local target</span>
              ) : (
                <span className="muted"> non-local target — never applied automatically</span>
              )}
            </dd>
          </div>
          <div className="detail-grid__wide detail-grid__mono">
            <dt>Database identity</dt>
            <dd>{status.database_identity}</dd>
          </div>
          <div>
            <dt>Enabled</dt>
            <dd>{status.enabled ? 'Yes' : 'No'}</dd>
          </div>
          <div>
            <dt>Authorization</dt>
            <dd>
              {status.requires_authorization
                ? 'Required for apply'
                : 'Not required for this local target'}
            </dd>
          </div>
          <div>
            <dt>Timeout</dt>
            <dd>{status.timeout_seconds === null ? '—' : `${status.timeout_seconds}s`}</dd>
          </div>
          <div>
            <dt>Pending changesets</dt>
            <dd>
              {status.pending === null ? (
                <span className="muted" title="Run an inspection to learn the pending count">
                  Unknown — inspect pending changes
                </span>
              ) : (
                status.pending
              )}
            </dd>
          </div>
          <div>
            <dt>Target lock</dt>
            <dd>
              {status.locked === null ? (
                <span className="muted">Unknown — never inferred</span>
              ) : status.locked ? (
                'Locked'
              ) : (
                'Not locked'
              )}
            </dd>
          </div>
        </dl>
      </div>

      <div className="detail-panel__block">
        <h3 className="detail-panel__block-title">Operations</h3>
        <ul className="capability-list" aria-label="Migration capabilities">
          {capabilities(status).map((capability) => (
            <li
              key={capability.name}
              className={`capability${capability.supported ? ' capability--exposed' : ''}`}
            >
              <span className="capability__name">{capability.name}</span>
              <span className="capability__reason">
                {capability.supported
                  ? 'available'
                  : (capability.reason ?? 'unavailable for this service')}
              </span>
            </li>
          ))}
        </ul>
        <div className="detail-actions">
          <button
            type="button"
            className="button button--small"
            onClick={onValidate}
            disabled={disabled || busy}
          >
            Validate configuration
          </button>
          <button
            type="button"
            className="button button--small"
            onClick={onInspectStatus}
            disabled={disabled || busy}
          >
            Inspect status
          </button>
          <button
            type="button"
            className="button button--small"
            onClick={onInspectPending}
            disabled={disabled || busy}
          >
            Inspect pending
          </button>
          <button
            type="button"
            className="button button--small"
            onClick={onPreview}
            disabled={disabled || busy}
          >
            SQL preview…
          </button>
          <button
            type="button"
            className="button button--small button--primary"
            onClick={onApply}
            disabled={disabled || busy || !status.enabled}
            title={
              status.enabled
                ? 'Apply the pending changesets through the daemon'
                : 'Migrations are disabled for this service'
            }
          >
            Apply migration…
          </button>
          <button
            type="button"
            className="button button--small"
            onClick={onViewLogs}
            disabled={disabled}
          >
            View migration logs
          </button>
        </div>
      </div>

      <div className="detail-panel__block">
        <h3 className="detail-panel__block-title">Latest task</h3>
        {latest ? (
          <dl className="detail-grid">
            <div>
              <dt>Task</dt>
              <dd className="detail-grid__mono">{latest.id}</dd>
            </div>
            <div>
              <dt>Kind</dt>
              <dd>{latest.kind}</dd>
            </div>
            <div>
              <dt>Status</dt>
              <dd>
                <TaskStatusIndicator status={latest.status} />
              </dd>
            </div>
            <div>
              <dt>Started</dt>
              <dd>{formatDateTime(latest.started_at)}</dd>
            </div>
            <div>
              <dt>Finished</dt>
              <dd>{formatDateTime(latest.finished_at)}</dd>
            </div>
            <div>
              <dt>Duration</dt>
              <dd>{formatMillis(latest.duration_ms)}</dd>
            </div>
            <div className="detail-grid__wide">
              <dt>Progress</dt>
              <dd>
                {latest.progress.message ??
                  (latest.progress.total === null
                    ? `step ${latest.progress.current}`
                    : `${latest.progress.current} of ${latest.progress.total}`)}
              </dd>
            </div>
          </dl>
        ) : (
          <p className="muted">
            No migration task has run for this service yet. Nothing is shown
            because nothing has been observed.
          </p>
        )}
        {failure ? (
          <div className="error-details" role="alert">
            <p className="error-details__label">Latest error</p>
            <p className="error-details__message">{failure.message}</p>
            <p className="error-details__code">{failure.code}</p>
            {failure.detail ? <p className="muted">{failure.detail}</p> : null}
          </div>
        ) : null}
        {latest ? (
          <div className="detail-actions">
            <button
              type="button"
              className="button button--small"
              onClick={() => onOpenTask(latest.id)}
            >
              Open task details
            </button>
          </div>
        ) : null}
      </div>

      <p className="detail-panel__state-item muted">
        Credentials are never rendered: the daemon redacts connection details
        before they reach the GUI, and the frontend performs no redaction of its
        own.
      </p>
    </section>
  );
}
