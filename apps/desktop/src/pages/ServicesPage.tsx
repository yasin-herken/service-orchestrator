import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { BulkActionToolbar } from '../components/BulkActionToolbar';
import { ConfirmationDialog } from '../components/ConfirmationDialog';
import { ErrorDialog } from '../components/ErrorDialog';
import { ServiceDetailPanel } from '../components/ServiceDetailPanel';
import { ServiceTable, type TableSort } from '../components/ServiceTable';
import { EmptyState, ErrorState, LoadingState } from '../components/StateViews';
import { TaskProgressDialog } from '../components/TaskProgressDialog';
import { useConnection } from '../context/ConnectionContext';
import { useTaskEvents } from '../context/TaskEventsContext';
import { useWorkspaces } from '../context/WorkspaceContext';
import { useAsync } from '../hooks/useAsync';
import {
  ACTION_LABELS,
  useServiceActions,
  type LifecycleAction,
  type ServiceOperation,
  type ServiceTarget,
} from '../hooks/useServiceActions';
import { desktopApi } from '../services/desktopApi';
import type { ServiceDetail, ServiceSummary } from '../types/models';
import type { NavIntent, PageId } from '../types/nav';
import { pluralize } from '../utils/format';
import {
  filterServices,
  sortServices,
  type ServiceSortKey,
} from '../utils/serviceQuery';
import {
  allSelected,
  emptySelection,
  pruneTo,
  setIds,
  someSelected,
} from '../utils/serviceSelection';
import { busyServiceIds, lastTaskByService } from '../utils/taskProgress';

/** Runtime states offered by the filter and the "select by state" control. */
const PROCESS_STATES = [
  'not_running',
  'starting',
  'running',
  'stopping',
  'stopped',
  'failed',
  'unknown',
] as const;

/** Health states offered by the filter. */
const HEALTH_STATES = [
  'unknown',
  'checking',
  'healthy',
  'unhealthy',
  'degraded',
  'unreachable',
] as const;

/** An operation awaiting the user's confirmation. */
interface PendingConfirmation {
  action: LifecycleAction;
  targets: ServiceTarget[];
}

/**
 * The services screen: filtering, sorting, stable-id multi-selection, bulk and
 * single lifecycle actions, service details, confirmations, and real task
 * progress — all through the shared daemon (`tasks/019.md` §6–§19, §25–§30).
 */
export function ServicesPage({
  onNavigate,
}: {
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
  const reloadServices = services.reload;

  const [search, setSearch] = useState('');
  const [kind, setKind] = useState('all');
  const [processState, setProcessState] = useState('all');
  const [healthState, setHealthState] = useState('all');
  const [sort, setSort] = useState<TableSort>({ key: 'name', direction: 'asc' });
  const [selectedIds, setSelectedIds] = useState<ReadonlySet<string>>(emptySelection);
  const [detailId, setDetailId] = useState<string | null>(null);
  const [confirmation, setConfirmation] = useState<PendingConfirmation | null>(null);
  const [progress, setProgress] = useState<ServiceOperation | null>(null);

  const actions = useServiceActions(activeWorkspaceId, reloadServices);

  const detailResource = useAsync<ServiceDetail | null>(
    () => {
      if (!activeWorkspace || !detailId) {
        return Promise.resolve(null);
      }
      return desktopApi.getServiceDetail(activeWorkspace.id, detailId);
    },
    [activeWorkspace?.id, detailId, terminalRevision],
    { enabled: connected && activeWorkspace !== null && detailId !== null },
  );
  const detail =
    detailResource.data && detailResource.data.id === detailId ? detailResource.data : null;

  const rows = services.data ?? [];
  const kindOptions = useMemo(
    () => Array.from(new Set(rows.map((row) => row.kind))).sort(),
    [rows],
  );

  const filtered = useMemo(
    () =>
      sortServices(
        filterServices(rows, { search, kind, processState, healthState }),
        sort.key,
        sort.direction,
      ),
    [rows, search, kind, processState, healthState, sort],
  );
  const visibleIds = useMemo(() => filtered.map((row) => row.id), [filtered]);

  // Keep the selection stable: drop ids that are no longer present.
  useEffect(() => {
    setSelectedIds((current) => {
      const pruned = pruneTo(current, rows.map((row) => row.id));
      return pruned.size === current.size ? current : pruned;
    });
  }, [rows]);

  // Scope everything to the active workspace: no cross-workspace selection,
  // detail, dialog, or filter bleed (`tasks/019.md` §30).
  useEffect(() => {
    setSelectedIds(emptySelection());
    setDetailId(null);
    setConfirmation(null);
    setProgress(null);
    setSearch('');
    setKind('all');
    setProcessState('all');
    setHealthState('all');
    actions.clearError();
  }, [activeWorkspaceId, actions.clearError]);

  // Mark busy services from real in-flight tasks and in-flight commands.
  const busy = useMemo(() => {
    const set = busyServiceIds(tasks);
    for (const id of actions.pending) {
      set.add(id);
    }
    return set;
  }, [tasks, actions.pending]);

  const lastOperations = useMemo(() => lastTaskByService(tasks), [tasks]);

  // Selected services in a deterministic order (by id) so the daemon payload
  // and confirmation preview never depend on the list's incidental order.
  const selectedServices = useMemo(
    () =>
      rows
        .filter((row) => selectedIds.has(row.id))
        .sort((a, b) => a.id.localeCompare(b.id)),
    [rows, selectedIds],
  );
  const selectedCount = selectedServices.length;
  const busySelectedCount = selectedServices.filter((service) => busy.has(service.id)).length;
  const allVisible = allSelected(selectedIds, visibleIds);
  const someVisible = someSelected(selectedIds, visibleIds);

  // Open the progress dialog once per submitted operation.
  const lastOpenedKey = useRef(0);
  useEffect(() => {
    const latest = actions.operations[0];
    if (latest && latest.key !== lastOpenedKey.current) {
      lastOpenedKey.current = latest.key;
      setProgress(latest);
    }
  }, [actions.operations]);

  const requestAction = useCallback(
    (action: LifecycleAction, targets: ServiceTarget[]) => {
      if (targets.length === 0) {
        return;
      }
      const needsConfirmation =
        targets.length > 1 || action === 'stop' || action === 'restart';
      if (needsConfirmation) {
        setConfirmation({ action, targets });
      } else {
        void actions.execute(action, targets);
      }
    },
    [actions.execute],
  );

  const confirmPending = () => {
    const pending = confirmation;
    if (!pending) {
      return;
    }
    setConfirmation(null);
    void actions.execute(pending.action, pending.targets);
  };

  const toggleSelect = (id: string, selected: boolean) => {
    setSelectedIds((current) => setIds(current, [id], selected));
  };
  const toggleSelectAllVisible = (selected: boolean) => {
    setSelectedIds((current) => setIds(current, visibleIds, selected));
  };
  const selectKind = (value: string) => {
    const ids = rows.filter((row) => row.kind === value).map((row) => row.id);
    setSelectedIds((current) => setIds(current, ids, true));
  };
  const selectState = (value: string) => {
    const ids = rows.filter((row) => row.process_state === value).map((row) => row.id);
    setSelectedIds((current) => setIds(current, ids, true));
  };
  const clearFilters = () => {
    setSearch('');
    setKind('all');
    setProcessState('all');
    setHealthState('all');
  };

  const toggleSort = (key: ServiceSortKey) => {
    setSort((current) =>
      current.key === key
        ? { key, direction: current.direction === 'asc' ? 'desc' : 'asc' }
        : { key, direction: 'asc' },
    );
  };

  const filtersActive =
    search.trim().length > 0 ||
    kind !== 'all' ||
    processState !== 'all' ||
    healthState !== 'all';

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
        description="Configure a workspace in ~/.service-orchestrator/config.toml to see its services."
      />
    );
  }

  const detailTarget: ServiceTarget | null = detail
    ? { id: detail.id, display_name: detail.display_name }
    : null;

  return (
    <div className="page">
      <div className="toolbar" role="search">
        <label className="toolbar__field">
          <span className="visually-hidden">Search services</span>
          <input
            type="search"
            className="input"
            placeholder="Search by name, id, or branch…"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            aria-label="Search services"
          />
        </label>
        <label className="toolbar__field">
          <span className="toolbar__field-label">Type</span>
          <select
            className="input input--select"
            value={kind}
            onChange={(event) => setKind(event.target.value)}
            aria-label="Filter by service type"
          >
            {['all', ...kindOptions].map((option) => (
              <option key={option} value={option}>
                {option === 'all' ? 'All types' : option}
              </option>
            ))}
          </select>
        </label>
        <label className="toolbar__field">
          <span className="toolbar__field-label">State</span>
          <select
            className="input input--select"
            value={processState}
            onChange={(event) => setProcessState(event.target.value)}
            aria-label="Filter by runtime state"
          >
            <option value="all">All states</option>
            {PROCESS_STATES.map((option) => (
              <option key={option} value={option}>
                {option.replace(/_/g, ' ')}
              </option>
            ))}
          </select>
        </label>
        <label className="toolbar__field">
          <span className="toolbar__field-label">Health</span>
          <select
            className="input input--select"
            value={healthState}
            onChange={(event) => setHealthState(event.target.value)}
            aria-label="Filter by health state"
          >
            <option value="all">All health</option>
            {HEALTH_STATES.map((option) => (
              <option key={option} value={option}>
                {option}
              </option>
            ))}
          </select>
        </label>
        <span className="toolbar__count" aria-live="polite">
          {pluralize(filtered.length, 'service')}
          {filtered.length !== rows.length ? ` of ${rows.length}` : ''}
        </span>
        {filtersActive ? (
          <button type="button" className="button button--small" onClick={clearFilters}>
            Clear filters
          </button>
        ) : null}
        <div className="bulk-bar__actions">
          <button
            type="button"
            className="button button--small button--primary"
            disabled={!connected}
            onClick={() =>
              onNavigate?.('workflows', {
                workflowServiceIds: selectedServices.map((service) => service.id),
              })
            }
          >
            Prepare workflow…
          </button>
        </div>
      </div>

      {!connected ? (
        <div className="page__notice" role="status">
          Disconnected from the daemon. Service actions are disabled until the
          connection returns; loaded state is kept.
        </div>
      ) : null}

      <BulkActionToolbar
        selectedCount={selectedCount}
        visibleCount={filtered.length}
        totalCount={rows.length}
        busySelectedCount={busySelectedCount}
        allVisibleSelected={allVisible}
        disabled={!connected}
        kinds={kindOptions}
        states={PROCESS_STATES}
        onSelectAllVisible={() => toggleSelectAllVisible(true)}
        onClearSelection={() => setSelectedIds(emptySelection())}
        onSelectKind={selectKind}
        onSelectState={selectState}
        onStart={() => requestAction('start', selectedServices)}
        onStop={() => requestAction('stop', selectedServices)}
        onRestart={() => requestAction('restart', selectedServices)}
      />

      <div className="card card--table">
        {services.loading ? <LoadingState label="Loading services…" /> : null}
        {services.error ? <ErrorState error={services.error} onRetry={services.reload} /> : null}
        {!services.loading && !services.error ? (
          services.data === null ? (
            <LoadingState
              label={connected ? 'Loading services…' : 'Waiting for the daemon connection…'}
            />
          ) : rows.length === 0 ? (
            <EmptyState
              title="No services configured"
              description={`The workspace “${activeWorkspace.name}” has no services.`}
            />
          ) : filtered.length === 0 ? (
            <EmptyState
              title="No services match your filters"
              description="Adjust the search text or filters."
              action={
                <button type="button" className="button" onClick={clearFilters}>
                  Clear filters
                </button>
              }
            />
          ) : (
            <ServiceTable
              services={filtered}
              workspaceName={activeWorkspace.name}
              selectedIds={selectedIds}
              onToggleSelect={toggleSelect}
              onToggleSelectAll={toggleSelectAllVisible}
              allVisibleSelected={allVisible}
              someVisibleSelected={someVisible}
              detailId={detailId}
              onOpenDetail={setDetailId}
              sort={sort}
              onSortChange={toggleSort}
              busy={busy}
              lastOperations={lastOperations}
            />
          )
        ) : null}
      </div>

      {detailId ? (
        <ServiceDetailPanel
          detail={detail}
          loading={detailResource.loading}
          error={detailResource.error}
          lastTask={lastOperations.get(detailId) ?? null}
          busy={busy.has(detailId)}
          disabled={!connected}
          onClose={() => setDetailId(null)}
          onStart={() => detailTarget && requestAction('start', [detailTarget])}
          onStop={() => detailTarget && requestAction('stop', [detailTarget])}
          onRestart={() => detailTarget && requestAction('restart', [detailTarget])}
          onCheckHealth={() =>
            detailTarget && requestAction('check_health', [detailTarget])
          }
          onViewLogs={() => onNavigate?.('logs')}
          onViewTasks={() => onNavigate?.('tasks')}
          onRetry={detailResource.reload}
        />
      ) : null}

      {confirmation ? (
        <ConfirmationDialog
          title={`${ACTION_LABELS[confirmation.action]}${
            confirmation.targets.length === 1 ? ' service' : ` ${confirmation.targets.length} services`
          }`}
          message={
            confirmation.action === 'stop'
              ? 'Stopping interrupts the running process. The daemon coordinates a graceful stop.'
              : confirmation.action === 'restart'
                ? 'Restarting briefly stops and starts each selected service. The daemon coordinates the lifecycle.'
                : 'The daemon schedules the operation for the selected services.'
          }
          confirmLabel={`${ACTION_LABELS[confirmation.action]} ${
            confirmation.targets.length === 1 ? 'service' : `${confirmation.targets.length} services`
          }`}
          tone={confirmation.action === 'restart' || confirmation.action === 'stop' ? 'danger' : 'default'}
          onConfirm={confirmPending}
          onCancel={() => setConfirmation(null)}
        >
          <dl className="dialog__facts">
            <div>
              <dt>Operation</dt>
              <dd>{ACTION_LABELS[confirmation.action]}</dd>
            </div>
            <div>
              <dt>Workspace</dt>
              <dd>{activeWorkspace.name}</dd>
            </div>
            <div>
              <dt>Affected services</dt>
              <dd>{confirmation.targets.length}</dd>
            </div>
            <div className="dialog__facts-wide">
              <dt>Services</dt>
              <dd>{confirmation.targets.map((target) => target.display_name).join(', ')}</dd>
            </div>
          </dl>
          <p className="dialog__note muted">
            Dependencies and concurrency are enforced by the daemon, not here;
            this dialog shows only the explicit selection. The operation is
            authorized as a safe write through the daemon's policy engine.
          </p>
        </ConfirmationDialog>
      ) : null}

      {progress ? (
        <TaskProgressDialog
          operation={progress}
          tasks={tasks}
          onClose={() => setProgress(null)}
        />
      ) : null}

      {actions.error ? (
        <ErrorDialog
          title={`Unable to ${ACTION_LABELS[actions.error.action].toLowerCase()} service${
            actions.error.labels.length === 1 ? '' : 's'
          }`}
          error={actions.error.error}
          labels={actions.error.labels}
          onClose={actions.clearError}
        />
      ) : null}
    </div>
  );
}
