import { useCallback, useEffect, useMemo, useState } from 'react';

import { ErrorDialog } from '../components/ErrorDialog';
import { HealthDetailPanel } from '../components/health/HealthDetailPanel';
import { HealthTable } from '../components/health/HealthTable';
import { EmptyState, ErrorState, LoadingState } from '../components/StateViews';
import { OperationProgressDialog } from '../components/tasks/OperationProgressDialog';
import { useConnection } from '../context/ConnectionContext';
import { useTaskEvents } from '../context/TaskEventsContext';
import { useWorkspaces } from '../context/WorkspaceContext';
import { useAsync } from '../hooks/useAsync';
import { desktopApi } from '../services/desktopApi';
import { toDesktopError } from '../services/errors';
import type {
  DesktopError,
  HealthDetailView,
  HealthRowView,
  OperationOutcome,
} from '../types/models';
import type { NavIntent, PageId } from '../types/nav';
import { busyServiceIds } from '../utils/taskProgress';

/** A submitted health operation, kept for the progress dialog. */
interface SubmittedHealth {
  outcome: OperationOutcome;
  serviceId: string;
  action: 'check' | 'readiness';
  at: number;
}

/**
 * The Health Monitoring screen (`tasks/021.md` §17–§22).
 *
 * Every state on this screen is an observation produced by the daemon's task
 * engine: React never probes an endpoint itself, runtime and health stay
 * separate columns, and an unobserved check shows an explicit unknown rather
 * than a fabricated healthy verdict.
 */
export function HealthPage({
  intent,
  onNavigate,
}: {
  intent?: NavIntent | null;
  onNavigate?: (page: PageId, intent?: NavIntent) => void;
}) {
  const {
    activeWorkspace,
    activeWorkspaceId,
    loading: workspacesLoading,
    error: workspacesError,
    reload: reloadWorkspaces,
  } = useWorkspaces();
  const { snapshot } = useConnection();
  const connected = snapshot.state === 'connected';
  const { tasks, terminalRevision } = useTaskEvents();

  // Health rows are refreshed from the daemon, and once more whenever a
  // health-kind task settles — no client-side fabrication of results.
  const health = useAsync<HealthRowView[]>(
    () => {
      if (!activeWorkspace) {
        return Promise.resolve([]);
      }
      return desktopApi.listHealth(activeWorkspace.id);
    },
    [activeWorkspace?.id, terminalRevision],
    { enabled: connected && activeWorkspace !== null },
  );

  const [detailId, setDetailId] = useState<string | null>(null);
  const [detail, setDetail] = useState<HealthDetailView | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState<DesktopError | null>(null);
  const [detailNonce, setDetailNonce] = useState(0);
  const [submitted, setSubmitted] = useState<SubmittedHealth | null>(null);
  const [failure, setFailure] = useState<{ title: string; error: DesktopError } | null>(
    null,
  );

  const rows = health.data ?? [];
  const busy = useMemo(() => busyServiceIds(tasks), [tasks]);

  // Everything on this screen is scoped to the active workspace.
  useEffect(() => {
    setDetailId(null);
    setDetail(null);
    setDetailError(null);
    setSubmitted(null);
    setFailure(null);
  }, [activeWorkspaceId]);

  // Cross-screen navigation can open a specific service's health detail.
  useEffect(() => {
    if (intent?.serviceId) {
      setDetailId(intent.serviceId);
    }
  }, [intent]);

  // Refresh the detail of the open service; stale replies are discarded.
  useEffect(() => {
    if (!connected || !detailId || !activeWorkspaceId) {
      setDetail(null);
      setDetailError(null);
      setDetailLoading(false);
      return undefined;
    }
    let cancelled = false;
    setDetailLoading(true);
    setDetailError(null);
    desktopApi.getHealth(activeWorkspaceId, detailId).then(
      (view) => {
        if (!cancelled) {
          setDetail(view);
          setDetailLoading(false);
        }
      },
      (cause: unknown) => {
        if (!cancelled) {
          setDetailError(toDesktopError(cause));
          setDetailLoading(false);
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, [detailId, connected, activeWorkspaceId, detailNonce, terminalRevision]);

  /** Schedules one health operation and tracks it in the progress dialog. */
  const runOperation = useCallback(
    async (serviceId: string, action: SubmittedHealth['action']) => {
      setFailure(null);
      try {
        const outcome =
          action === 'check'
            ? await desktopApi.checkServiceHealth(serviceId)
            : await desktopApi.checkServiceReadiness(serviceId);
        setSubmitted({
          outcome,
          serviceId,
          action,
          at: Math.floor(Date.now() / 1000),
        });
      } catch (cause) {
        setFailure({
          title: 'Unable to run the health operation',
          error: toDesktopError(cause),
        });
      }
    },
    [],
  );

  const authorizationHint =
    failure && failure.error.code === 'PermissionDenied'
      ? 'The daemon refused this operation because it could not verify an authorized interactive request. Health checks run inside the daemon’s task engine, and its policy decision is final; reconnect through the desktop application and try again.'
      : undefined;

  if (workspacesError) {
    return <ErrorState error={workspacesError} onRetry={reloadWorkspaces} />;
  }
  if (!connected && !activeWorkspace) {
    return <LoadingState label="Waiting for the daemon connection…" />;
  }
  if (workspacesLoading && !activeWorkspace) {
    return <LoadingState label="Loading workspaces…" />;
  }
  if (!activeWorkspace) {
    return (
      <EmptyState
        title="No workspace selected"
        description="Select a workspace to see its services’ observed health."
      />
    );
  }

  const detailRow = rows.find((row) => row.service_id === detailId) ?? null;

  return (
    <div className="page">
      <div className="toolbar" role="search">
        <span className="toolbar__title">
          Health — {activeWorkspace.name}
        </span>
        <span className="toolbar__count" aria-live="polite">
          {rows.length} {rows.length === 1 ? 'service' : 'services'}
        </span>
        <button
          type="button"
          className="button button--small"
          onClick={health.reload}
          disabled={!connected}
        >
          Refresh
        </button>
      </div>

      {!connected ? (
        <div className="page__notice" role="status">
          Disconnected from the daemon. Health checks are disabled until the
          connection returns; previously observed state is kept and clearly
          dated, and nothing is replayed automatically.
        </div>
      ) : null}

      <div className="card card--table">
        {health.loading ? <LoadingState label="Loading health state…" /> : null}
        {health.error ? <ErrorState error={health.error} onRetry={health.reload} /> : null}
        {!health.loading && !health.error ? (
          health.data === null ? (
            <LoadingState
              label={
                connected
                  ? 'Loading health state…'
                  : 'Waiting for the daemon connection…'
              }
            />
          ) : rows.length === 0 ? (
            <EmptyState
              title="No services in this workspace"
              description="Health rows appear once the daemon’s configuration lists services."
            />
          ) : (
            <HealthTable
              rows={rows}
              detailId={detailId}
              onOpenDetail={setDetailId}
              onCheck={(serviceId) => void runOperation(serviceId, 'check')}
              checking={busy}
              disabled={!connected}
            />
          )
        ) : null}
      </div>

      {detailId ? (
        <HealthDetailPanel
          detail={detail}
          loading={detailLoading}
          error={detailError}
          checking={busy.has(detailId)}
          disabled={!connected}
          onClose={() => {
            setDetailId(null);
            setDetail(null);
            setDetailError(null);
          }}
          onCheck={() => void runOperation(detailId, 'check')}
          onReadiness={() => void runOperation(detailId, 'readiness')}
          onViewLogs={() => onNavigate?.('logs', { serviceId: detailId })}
          onRetry={() => setDetailNonce((value) => value + 1)}
        />
      ) : null}

      {submitted ? (
        <OperationProgressDialog
          title={submitted.action === 'check' ? 'Health check' : 'Readiness check'}
          outcome={submitted.outcome}
          tasks={tasks}
          at={submitted.at}
          targetLabel={submitted.serviceId}
          onCancelTask={(taskId) => void desktopApi.cancelTask(taskId)}
          onClose={() => {
            setSubmitted(null);
            health.reload();
          }}
        />
      ) : null}

      {failure ? (
        <ErrorDialog
          title={failure.title}
          error={failure.error}
          labels={detailRow ? [detailRow.display_name] : []}
          hint={authorizationHint ?? undefined}
          onClose={() => setFailure(null)}
        />
      ) : null}
    </div>
  );
}
