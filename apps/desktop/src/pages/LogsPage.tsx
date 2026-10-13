import { useEffect, useMemo, useRef, useState } from 'react';

import { UnifiedLogViewer, type LogScope } from '../components/logs/UnifiedLogViewer';
import { EmptyState, ErrorState, LoadingState } from '../components/StateViews';
import { useConnection } from '../context/ConnectionContext';
import { useTaskRevision } from '../context/TaskEventsContext';
import { useWorkspaces } from '../context/WorkspaceContext';
import { useAsync } from '../hooks/useAsync';
import { desktopApi } from '../services/desktopApi';
import type { ServiceSummary } from '../types/models';
import type { NavIntent } from '../types/nav';

/** The scope kinds the page offers, in the order they are shown. */
const SCOPE_KINDS = ['workspace', 'service', 'task'] as const;

type ScopeKind = (typeof SCOPE_KINDS)[number];

/** Derives the scope kind implied by a cross-screen navigation intent. */
function scopeKindFromIntent(intent: NavIntent | null | undefined): ScopeKind {
  if (intent?.taskId) {
    return 'task';
  }
  if (intent?.serviceId) {
    return 'service';
  }
  return 'workspace';
}

/**
 * The Realtime/Historical Logs screen (`tasks/021.md` §23–§28).
 *
 * It only picks a scope and hands it to the shared bounded viewer: there is
 * one log implementation in the application core, one query path through the
 * daemon, and one viewer component for every screen.
 */
export function LogsPage({ intent }: { intent?: NavIntent | null }) {
  const {
    activeWorkspace,
    activeWorkspaceId,
    loading: workspacesLoading,
    error: workspacesError,
    reload: reloadWorkspaces,
  } = useWorkspaces();
  const { snapshot } = useConnection();
  const connected = snapshot.state === 'connected';
  const terminalRevision = useTaskRevision();

  const services = useAsync<ServiceSummary[]>(
    () => {
      if (!activeWorkspace) {
        return Promise.resolve([]);
      }
      return desktopApi.listServices(activeWorkspace.id);
    },
    [activeWorkspace?.id, terminalRevision],
    { enabled: connected && activeWorkspace !== null },
  );

  const [kind, setKind] = useState<ScopeKind>(() => scopeKindFromIntent(intent));
  const [serviceId, setServiceId] = useState<string>('');
  const [taskId, setTaskId] = useState<string>('');

  // A cross-screen navigation pins the scope to what the caller asked for.
  useEffect(() => {
    if (intent?.taskId) {
      setKind('task');
      setTaskId(intent.taskId);
    } else if (intent?.serviceId) {
      setKind('service');
      setServiceId(intent.serviceId);
    }
  }, [intent]);

  // Leaving the service scope must detach the live subscription (the viewer
  // does that on unmount/scope change); switching workspaces resets it. The
  // very first render is skipped so a navigation intent survives mounting.
  const mountedWorkspace = useRef<string | null>(null);
  useEffect(() => {
    if (mountedWorkspace.current === null) {
      mountedWorkspace.current = activeWorkspaceId;
      return;
    }
    if (mountedWorkspace.current === activeWorkspaceId) {
      return;
    }
    mountedWorkspace.current = activeWorkspaceId;
    setServiceId('');
    setTaskId('');
    if (!intent?.taskId && !intent?.serviceId) {
      setKind('workspace');
    }
  }, [activeWorkspaceId, intent]);

  const serviceOptions = useMemo(
    () => (services.data ?? []).map((service) => service.id),
    [services.data],
  );

  // Keep the selected service valid for the current workspace.
  useEffect(() => {
    if (kind !== 'service') {
      return;
    }
    if (serviceId.length > 0 && serviceOptions.includes(serviceId)) {
      return;
    }
    if (serviceOptions.length > 0) {
      setServiceId(serviceOptions[0]);
    }
  }, [kind, serviceId, serviceOptions]);

  const scope: LogScope | null = useMemo(() => {
    if (!activeWorkspaceId) {
      return null;
    }
    switch (kind) {
      case 'workspace':
        return { kind: 'workspace', workspaceId: activeWorkspaceId };
      case 'service':
        return serviceId
          ? { kind: 'service', serviceId, workspaceId: activeWorkspaceId }
          : null;
      case 'task':
        return taskId ? { kind: 'task', taskId, workspaceId: activeWorkspaceId } : null;
    }
  }, [kind, activeWorkspaceId, serviceId, taskId]);

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
        description="Select a workspace to inspect its logs."
      />
    );
  }

  const waitingForService =
    kind === 'service' && serviceOptions.length === 0 && services.loading;

  return (
    <div className="page">
      <div className="toolbar" role="search">
        <span className="toolbar__title">Logs — {activeWorkspace.name}</span>
        <label className="toolbar__field toolbar__field-label">
          <span>Scope</span>
          <select
            className="input input--select"
            aria-label="Log scope kind"
            value={kind}
            onChange={(event) => setKind(event.target.value as ScopeKind)}
          >
            {SCOPE_KINDS.map((option) => (
              <option key={option} value={option}>
                {option === 'workspace'
                  ? 'Whole workspace'
                  : option === 'service'
                    ? 'One service'
                    : 'One task'}
              </option>
            ))}
          </select>
        </label>

        {kind === 'service' ? (
          <label className="toolbar__field toolbar__field-label">
            <span>Service</span>
            <select
              className="input input--select"
              aria-label="Log service"
              value={serviceId}
              disabled={services.loading}
              onChange={(event) => setServiceId(event.target.value)}
            >
              {serviceOptions.length === 0 ? (
                <option value="">No services</option>
              ) : (
                serviceOptions.map((id) => (
                  <option key={id} value={id}>
                    {id}
                  </option>
                ))
              )}
            </select>
          </label>
        ) : null}

        {kind === 'task' ? (
          <label className="toolbar__field toolbar__field-label">
            <span>Task id</span>
            <input
              type="text"
              className="input"
              aria-label="Log task id"
              placeholder="task id"
              value={taskId}
              onChange={(event) => setTaskId(event.target.value.trim())}
            />
          </label>
        ) : null}
      </div>

      {services.error && kind === 'service' ? (
        <ErrorState error={services.error} onRetry={services.reload} />
      ) : null}

      {!connected ? (
        <div className="page__notice" role="status">
          Disconnected from the daemon. Live follow is paused and no
          subscription is held; previously loaded lines stay visible and are
          marked as historical. Nothing is replayed automatically.
        </div>
      ) : null}

      {waitingForService ? <LoadingState label="Loading services…" /> : null}

      {scope ? (
        <UnifiedLogViewer scope={scope} disabled={!connected} />
      ) : !waitingForService ? (
        <EmptyState
          title={kind === 'task' ? 'Enter a task id' : 'No service selected'}
          description={
            kind === 'task'
              ? 'Logs can be scoped to one task by its id, for example from the Tasks screen.'
              : 'Pick a service to stream its logs.'
          }
        />
      ) : null}
    </div>
  );
}
