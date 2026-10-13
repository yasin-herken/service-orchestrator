import { useEffect, useMemo, useState } from 'react';

import { EmptyState, ErrorState, LoadingState } from '../components/StateViews';
import { TaskDetailPanel } from '../components/tasks/TaskDetailPanel';
import { TaskTable } from '../components/tasks/TaskTable';
import { useConnection } from '../context/ConnectionContext';
import { useTaskEvents } from '../context/TaskEventsContext';
import { useWorkspaces } from '../context/WorkspaceContext';
import { useAsync } from '../hooks/useAsync';
import { desktopApi } from '../services/desktopApi';
import { toDesktopError } from '../services/errors';
import type { TaskView, WorkflowView } from '../types/models';
import type { NavIntent, PageId } from '../types/nav';
import { pluralize } from '../utils/format';

/** How many tasks one listing requests (the daemon bounds it further). */
const TASK_LIMIT = 200;

/** The task states offered by the status filter, in engine order. */
const STATUSES = [
  'all',
  'queued',
  'running',
  'succeeded',
  'failed',
  'cancelled',
  'skipped',
  'blocked',
] as const;

/** The time windows offered by the time filter. */
const WINDOWS = [
  { value: 'all', label: 'All time', seconds: null },
  { value: '15m', label: 'Last 15 minutes', seconds: 15 * 60 },
  { value: '1h', label: 'Last hour', seconds: 60 * 60 },
  { value: '24h', label: 'Last 24 hours', seconds: 24 * 60 * 60 },
] as const;

/** Applies every active filter to the bounded listing. */
function applyFilters(
  rows: readonly TaskView[],
  filters: {
    status: string;
    workspace: string;
    kind: string;
    service: string;
    workflow: string;
    window: string;
    search: string;
  },
  now: number,
): TaskView[] {
  const needle = filters.search.trim().toLowerCase();
  const windowSeconds = WINDOWS.find((entry) => entry.value === filters.window)?.seconds ?? null;
  return rows.filter((task) => {
    if (filters.status !== 'all' && task.status !== filters.status) {
      return false;
    }
    if (filters.workspace !== 'all') {
      const workspace = task.workspace_id ?? 'unknown';
      if (workspace !== filters.workspace) {
        return false;
      }
    }
    if (filters.kind !== 'all' && task.kind !== filters.kind) {
      return false;
    }
    if (filters.service !== 'all') {
      const service = task.service_id ?? task.library_id ?? 'unknown';
      if (service !== filters.service) {
        return false;
      }
    }
    if (filters.workflow !== 'all') {
      if ((task.workflow_id ?? 'unknown') !== filters.workflow) {
        return false;
      }
    }
    if (windowSeconds !== null && now - task.created_at > windowSeconds) {
      return false;
    }
    if (needle.length > 0) {
      const haystack = [
        task.id,
        task.kind,
        task.service_id ?? '',
        task.library_id ?? '',
        task.workflow_id ?? '',
        task.workspace_id ?? '',
        task.progress.message ?? '',
        task.failure?.message ?? '',
      ]
        .join(' ')
        .toLowerCase();
      if (!haystack.includes(needle)) {
        return false;
      }
    }
    return true;
  });
}

/**
 * The Task Management screen (`tasks/021.md` §10–§16).
 *
 * It renders the shared daemon's task engine directly: one bounded, newest
 * first listing plus live task events, all seven lifecycle states kept
 * distinct, filters applied client-side so they stay instant at history scale,
 * and cancellation delegated to the daemon (never decided in React).
 */
export function TasksPage({
  intent,
  onNavigate,
}: {
  intent?: NavIntent | null;
  onNavigate?: (page: PageId, intent?: NavIntent) => void;
}) {
  const {
    workspaces,
    activeWorkspaceId,
    loading: workspacesLoading,
    error: workspacesError,
    reload: reloadWorkspaces,
  } = useWorkspaces();
  const { snapshot } = useConnection();
  const connected = snapshot.state === 'connected';
  const { tasks: tracked, terminalRevision } = useTaskEvents();

  const listing = useAsync<TaskView[]>(
    () => desktopApi.listTasks(TASK_LIMIT),
    [terminalRevision],
    { enabled: connected },
  );

  const [status, setStatus] = useState<(typeof STATUSES)[number]>('all');
  const [workspace, setWorkspace] = useState<string>('all');
  const [kind, setKind] = useState('all');
  const [service, setService] = useState('all');
  const [workflowFilter, setWorkflowFilter] = useState('all');
  const [timeWindow, setTimeWindow] = useState<(typeof WINDOWS)[number]['value']>('all');
  const [search, setSearch] = useState('');
  const [detailId, setDetailId] = useState<string | null>(null);
  const [override, setOverride] = useState<TaskView | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const [cancelResult, setCancelResult] = useState<string | null>(null);
  const [cancelError, setCancelError] = useState<string | null>(null);

  // Scope the filters to the newly selected workspace (`tasks/021.md` §35).
  useEffect(() => {
    if (activeWorkspaceId) {
      setWorkspace(activeWorkspaceId);
    }
    setStatus('all');
    setKind('all');
    setService('all');
    setWorkflowFilter('all');
    setTimeWindow('all');
    setSearch('');
    setDetailId(null);
    setOverride(null);
    setCancelResult(null);
    setCancelError(null);
  }, [activeWorkspaceId]);

  // Cross-screen navigation can open a specific task.
  useEffect(() => {
    if (intent?.taskId) {
      setWorkspace('all');
      setStatus('all');
      setDetailId(intent.taskId);
      setOverride(null);
      setCancelResult(null);
      setCancelError(null);
    }
  }, [intent]);

  const rows = useMemo(() => {
    const base = listing.data ?? [];
    return override
      ? base.map((task) => (task.id === override.id ? override : task))
      : base;
  }, [listing.data, override]);

  const trackedById = useMemo(() => new Map(tracked.map((task) => [task.id, task])), [tracked]);

  const filtered = useMemo(
    () =>
      applyFilters(
        rows,
        { status, workspace, kind, service, workflow: workflowFilter, window: timeWindow, search },
        Math.floor(Date.now() / 1000),
      ),
    [rows, status, workspace, kind, service, workflowFilter, timeWindow, search],
  );

  const kinds = useMemo(() => [...new Set(rows.map((task) => task.kind))].sort(), [rows]);
  const services = useMemo(
    () =>
      [...new Set(rows.map((task) => task.service_id ?? task.library_id ?? 'unknown'))].sort(),
    [rows],
  );
  const workflows = useMemo(
    () => [...new Set(rows.map((task) => task.workflow_id ?? 'unknown'))].sort(),
    [rows],
  );

  const selected = useMemo(
    () => rows.find((task) => task.id === detailId) ?? null,
    [rows, detailId],
  );
  const siblings = useMemo(
    () =>
      selected?.workflow_id
        ? rows.filter((task) => task.workflow_id === selected.workflow_id)
        : [],
    [rows, selected],
  );

  const workflow = useAsync<WorkflowView | null>(
    () => {
      if (!selected?.workflow_id) {
        return Promise.resolve(null);
      }
      return desktopApi.getWorkflow(selected.workflow_id);
    },
    [selected?.workflow_id],
    { enabled: connected && Boolean(selected?.workflow_id) },
  );

  const cancel = async () => {
    if (!selected) {
      return;
    }
    setCancelling(true);
    setCancelError(null);
    setCancelResult(null);
    try {
      const updated = await desktopApi.cancelTask(selected.id);
      setOverride(updated);
      setCancelResult(
        `The daemon reports this task as ${updated.status}. Cancellation stops further work; it never claims that completed database or build work was rolled back.`,
      );
      listing.reload();
    } catch (cause) {
      setCancelError(toDesktopError(cause).message);
    } finally {
      setCancelling(false);
    }
  };

  if (workspacesError) {
    return <ErrorState error={workspacesError} onRetry={reloadWorkspaces} />;
  }
  if (workspacesLoading && workspaces.length === 0) {
    return <LoadingState label="Loading workspaces…" />;
  }

  const hasFilters =
    status !== 'all' ||
    workspace !== 'all' ||
    kind !== 'all' ||
    service !== 'all' ||
    workflowFilter !== 'all' ||
    timeWindow !== 'all' ||
    search.trim().length > 0;

  const clearFilters = () => {
    setStatus('all');
    setWorkspace('all');
    setKind('all');
    setService('all');
    setWorkflowFilter('all');
    setTimeWindow('all');
    setSearch('');
  };

  return (
    <div className="page">
      <div className="toolbar" role="search">
        <label className="toolbar__field">
          <span className="visually-hidden">Search tasks</span>
          <input
            type="search"
            className="input"
            placeholder="Search id, service, kind, or message…"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            aria-label="Search tasks"
          />
        </label>
        <label className="toolbar__field toolbar__field-label">
          <span>Status</span>
          <select
            className="input input--select"
            aria-label="Filter by status"
            value={status}
            onChange={(event) => setStatus(event.target.value as (typeof STATUSES)[number])}
          >
            {STATUSES.map((option) => (
              <option key={option} value={option}>
                {option === 'all' ? 'All statuses' : option}
              </option>
            ))}
          </select>
        </label>
        <label className="toolbar__field toolbar__field-label">
          <span>Workspace</span>
          <select
            className="input input--select"
            aria-label="Filter by workspace"
            value={workspace}
            onChange={(event) => setWorkspace(event.target.value)}
          >
            <option value="all">All workspaces</option>
            {workspaces.map((entry) => (
              <option key={entry.id} value={entry.id}>
                {entry.name}
              </option>
            ))}
            <option value="unknown">Unresolved workspace</option>
          </select>
        </label>
        <label className="toolbar__field toolbar__field-label">
          <span>Type</span>
          <select
            className="input input--select"
            aria-label="Filter by task type"
            value={kind}
            onChange={(event) => setKind(event.target.value)}
          >
            <option value="all">All types</option>
            {kinds.map((option) => (
              <option key={option} value={option}>
                {option}
              </option>
            ))}
          </select>
        </label>
        <label className="toolbar__field toolbar__field-label">
          <span>Service</span>
          <select
            className="input input--select"
            aria-label="Filter by service"
            value={service}
            onChange={(event) => setService(event.target.value)}
          >
            <option value="all">All services</option>
            {services.map((option) => (
              <option key={option} value={option}>
                {option}
              </option>
            ))}
          </select>
        </label>
        <label className="toolbar__field toolbar__field-label">
          <span>Workflow</span>
          <select
            className="input input--select"
            aria-label="Filter by workflow"
            value={workflowFilter}
            onChange={(event) => setWorkflowFilter(event.target.value)}
          >
            <option value="all">All workflows</option>
            {workflows.map((option) => (
              <option key={option} value={option}>
                {option}
              </option>
            ))}
          </select>
        </label>
        <label className="toolbar__field toolbar__field-label">
          <span>Time</span>
          <select
            className="input input--select"
            aria-label="Filter by time"
            value={timeWindow}
            onChange={(event) =>
              setTimeWindow(event.target.value as (typeof WINDOWS)[number]['value'])
            }
          >
            {WINDOWS.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
        <span className="toolbar__count" aria-live="polite">
          {pluralize(filtered.length, 'task')}
          {filtered.length !== rows.length ? ` of ${rows.length}` : ''}
        </span>
        {hasFilters ? (
          <button
            type="button"
            className="button button--small"
            onClick={clearFilters}
          >
            Clear filters
          </button>
        ) : null}
      </div>

      {!connected ? (
        <div className="page__notice" role="status">
          Disconnected from the daemon. Cancellation and other mutating actions
          are disabled until the connection returns; no operation is replayed
          automatically.
        </div>
      ) : null}

      <div className="card card--table">
        {listing.loading ? <LoadingState label="Loading tasks…" /> : null}
        {listing.error ? <ErrorState error={listing.error} onRetry={listing.reload} /> : null}
        {!listing.loading && !listing.error ? (
          listing.data === null ? (
            <LoadingState
              label={connected ? 'Loading tasks…' : 'Waiting for the daemon connection…'}
            />
          ) : rows.length === 0 ? (
            <EmptyState
              title="No tasks yet"
              description="Tasks appear here as soon as the daemon's task engine runs one."
            />
          ) : filtered.length === 0 ? (
            <EmptyState
              title="No tasks match your filters"
              description="Adjust or clear the filters to see the bounded history."
              action={
                <button
                  type="button"
                  className="button"
                  onClick={clearFilters}
                >
                  Clear filters
                </button>
              }
            />
          ) : (
            <TaskTable
              tasks={filtered}
              tracked={trackedById}
              detailId={detailId}
              onOpenDetail={(taskId) => {
                setDetailId(taskId);
                setCancelResult(null);
                setCancelError(null);
              }}
              workspaceName="all workspaces"
            />
          )
        ) : null}
      </div>

      {selected ? (
        <TaskDetailPanel
          task={selected}
          tracked={trackedById.get(selected.id) ?? null}
          workflow={workflow.data ?? null}
          workflowLoading={workflow.loading}
          workflowError={workflow.error}
          siblings={siblings}
          cancelling={cancelling}
          cancelResult={cancelResult}
          cancelError={cancelError}
          disabled={!connected}
          onCancel={() => void cancel()}
          onViewLogs={() =>
            onNavigate?.('logs', {
              taskId: selected.id,
              ...(selected.service_id ? { serviceId: selected.service_id } : {}),
            })
          }
          onClose={() => setDetailId(null)}
        />
      ) : null}
    </div>
  );
}
