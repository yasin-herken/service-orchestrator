import { StatCard } from '../components/StatCard';
import { ErrorState, LoadingState, EmptyState } from '../components/StateViews';
import { useConnection } from '../context/ConnectionContext';
import { useTaskEvents } from '../context/TaskEventsContext';
import { useWorkspaces } from '../context/WorkspaceContext';
import { useAsync } from '../hooks/useAsync';
import { desktopApi } from '../services/desktopApi';
import { countServices } from '../utils/serviceQuery';
import { formatClock, pluralize } from '../utils/format';

/**
 * The read-only dashboard: real daemon data only (`tasks/018.md` §16).
 *
 * Counts are derived from the active workspace's service statuses — nothing is
 * hard-coded.
 */
export function DashboardPage() {
  const { workspaces, activeWorkspace, loading, error, reload } = useWorkspaces();
  const { snapshot } = useConnection();
  const connected = snapshot.state === 'connected';
  const { recent, terminalRevision } = useTaskEvents();

  const daemon = useAsync(() => desktopApi.getDaemonStatus(), [terminalRevision], {
    enabled: connected,
  });
  const services = useAsync(
    () => {
      if (!activeWorkspace) {
        return Promise.resolve(null);
      }
      return desktopApi.getWorkspaceStatus(activeWorkspace.id);
    },
    [activeWorkspace?.id, terminalRevision],
    { enabled: connected && activeWorkspace !== null },
  );

  if (error) {
    return <ErrorState error={error} onRetry={reload} />;
  }
  if (loading) {
    return <LoadingState label="Loading workspaces…" />;
  }
  if (!connected && workspaces.length === 0) {
    return <LoadingState label="Waiting for the daemon connection…" />;
  }
  if (workspaces.length === 0) {
    return (
      <EmptyState
        title="No workspaces configured"
        description="Add a workspace to ~/.service-orchestrator/config.toml (or set SERVICE_ORCHESTRATOR_CONFIG) and reconnect."
      />
    );
  }
  if (!activeWorkspace) {
    return <LoadingState label="Select a workspace…" />;
  }

  const statuses = services.data?.services ?? [];
  const counts = countServices(statuses);

  return (
    <div className="page">
      <section className="page__section" aria-label="Workspace overview">
        <div className="stat-grid">
          <StatCard
            label="Active workspace"
            value={activeWorkspace.name}
            hint={activeWorkspace.root}
          />
          <StatCard label="Total services" value={counts.total} />
          <StatCard label="Running" value={counts.running} tone="positive" />
          <StatCard label="Starting" value={counts.starting} tone="warning" />
          <StatCard label="Stopped" value={counts.stopped} />
          <StatCard
            label="Failed"
            value={counts.failed}
            tone={counts.failed > 0 ? 'negative' : 'default'}
          />
          <StatCard
            label="Unhealthy"
            value={counts.unhealthy}
            tone={counts.unhealthy > 0 ? 'negative' : 'default'}
          />
          <StatCard label="Active tasks" value={daemon.data?.active_tasks ?? '—'} />
        </div>
      </section>

      <section className="page__section card" aria-label="Daemon status">
        <div className="card__header">
          <h2 className="card__title">Daemon</h2>
        </div>
        {daemon.loading ? <LoadingState label="Loading daemon status…" /> : null}
        {!daemon.loading && !daemon.error && !daemon.data && !connected ? (
          <LoadingState label="Waiting for the daemon connection…" />
        ) : null}
        {daemon.error ? (
          <ErrorState error={daemon.error} onRetry={daemon.reload} />
        ) : null}
        {daemon.data ? (
          <dl className="detail-grid">
            <div>
              <dt>Process id</dt>
              <dd>{daemon.data.pid}</dd>
            </div>
            <div>
              <dt>Clients</dt>
              <dd>{daemon.data.clients}</dd>
            </div>
            <div>
              <dt>Active tasks</dt>
              <dd>{daemon.data.active_tasks}</dd>
            </div>
            <div>
              <dt>Managed services</dt>
              <dd>{daemon.data.managed_services}</dd>
            </div>
            <div>
              <dt>Protocol</dt>
              <dd>v{daemon.data.protocol_version}</dd>
            </div>
            <div>
              <dt>Started</dt>
              <dd>{formatClock(daemon.data.started_at_unix)}</dd>
            </div>
            <div className="detail-grid__wide">
              <dt>Socket</dt>
              <dd className="detail-grid__mono">{daemon.data.socket_path}</dd>
            </div>
          </dl>
        ) : null}
      </section>

      <section className="page__section card" aria-label="Service status">
        <div className="card__header">
          <h2 className="card__title">Service status</h2>
          {services.loading ? <span className="muted">Refreshing…</span> : null}
        </div>
        {services.error ? (
          <ErrorState error={services.error} onRetry={services.reload} />
        ) : !connected && services.data === null ? (
          <LoadingState label="Waiting for the daemon connection…" />
        ) : (
          <dl className="detail-grid">
            <div>
              <dt>Configured</dt>
              <dd>{pluralize(counts.total, 'service')}</dd>
            </div>
            <div>
              <dt>Running</dt>
              <dd>{counts.running}</dd>
            </div>
            <div>
              <dt>Starting</dt>
              <dd>{counts.starting}</dd>
            </div>
            <div>
              <dt>Stopped</dt>
              <dd>{counts.stopped}</dd>
            </div>
            <div>
              <dt>Failed</dt>
              <dd>{counts.failed}</dd>
            </div>
            <div>
              <dt>Unhealthy</dt>
              <dd>{counts.unhealthy}</dd>
            </div>
          </dl>
        )}
      </section>

      <section className="page__section card" aria-label="Task activity">
        <div className="card__header">
          <h2 className="card__title">Task activity</h2>
          <span className="muted">Live from the daemon</span>
        </div>
        {recent.length === 0 ? (
          <p className="muted">No task activity yet. Start an operation to see events here.</p>
        ) : (
          <ul className="activity-feed">
            {recent.slice(0, 5).map((entry) => (
              <li key={entry.key} className="activity-feed__item">
                <span className="activity-feed__variant">{entry.variant}</span>
                <span className="activity-feed__summary">{entry.summary}</span>
                <span className="activity-feed__time">{formatClock(entry.at)}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
