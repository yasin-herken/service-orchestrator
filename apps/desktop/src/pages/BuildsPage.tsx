import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { BuildTargetDetailPanel } from '../components/build/BuildTargetDetailPanel';
import { BuildTargetTable, canBuild } from '../components/build/BuildTargetTable';
import { ErrorDialog } from '../components/ErrorDialog';
import { EmptyState, ErrorState, LoadingState } from '../components/StateViews';
import { LogViewer } from '../components/tasks/LogViewer';
import { OperationProgressDialog } from '../components/tasks/OperationProgressDialog';
import { isTerminalTaskStatus } from '../components/tasks/TaskStatusIndicator';
import { useConnection } from '../context/ConnectionContext';
import { useTaskEvents } from '../context/TaskEventsContext';
import { useWorkspaces } from '../context/WorkspaceContext';
import { useAsync } from '../hooks/useAsync';
import type { OperationTarget, SubmittedOperation } from '../hooks/useEntityOperations';
import {
  BUILD_ACTION_LABELS,
  useBuildOperations,
  type BuildAction,
} from '../hooks/useBuildOperations';
import { desktopApi } from '../services/desktopApi';
import type { BuildTargetSummary } from '../types/models';
import { pluralize } from '../utils/format';
import { busyServiceIds, lastTaskByService, tasksForWorkflow } from '../utils/taskProgress';

/** Applies the search text to a build-target list. */
function filterTargets(
  targets: readonly BuildTargetSummary[],
  search: string,
): BuildTargetSummary[] {
  const needle = search.trim().toLowerCase();
  if (needle.length === 0) {
    return [...targets];
  }
  return targets.filter(
    (target) =>
      target.service_id.toLowerCase().includes(needle) ||
      target.display_name.toLowerCase().includes(needle) ||
      target.kind.toLowerCase().includes(needle) ||
      (target.build_system ?? '').toLowerCase().includes(needle),
  );
}

/**
 * The build management screen (`tasks/020.md` §12–§19).
 *
 * It is build-system agnostic: it renders whatever provider capabilities the
 * Application Core reports, disables what is unsupported with the core's own
 * explanation, and submits one coordinated workflow for a bulk selection.
 */
export function BuildsPage() {
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

  const targets = useAsync<BuildTargetSummary[]>(
    () => {
      if (!activeWorkspace) {
        return Promise.resolve([]);
      }
      return desktopApi.listBuildTargets(activeWorkspace.id);
    },
    [activeWorkspace?.id, terminalRevision],
    { enabled: connected && activeWorkspace !== null },
  );
  const reloadTargets = targets.reload;

  const [search, setSearch] = useState('');
  const [selectedIds, setSelectedIds] = useState<ReadonlySet<string>>(new Set());
  const [detailId, setDetailId] = useState<string | null>(null);

  const builds = useBuildOperations(activeWorkspaceId, reloadTargets);
  const [progress, setProgress] = useState<SubmittedOperation<BuildAction> | null>(null);

  const rows = targets.data ?? [];
  const filtered = useMemo(() => filterTargets(rows, search), [rows, search]);
  const visibleIds = useMemo(() => filtered.map((row) => row.service_id), [filtered]);

  // Keep the selection stable as the list changes.
  useEffect(() => {
    setSelectedIds((current) => {
      const keep = new Set(rows.map((row) => row.service_id));
      const next = new Set<string>();
      for (const id of current) {
        if (keep.has(id)) {
          next.add(id);
        }
      }
      return next.size === current.size ? current : next;
    });
  }, [rows]);

  // Scope to the active workspace (`tasks/020.md` §33).
  useEffect(() => {
    setSearch('');
    setSelectedIds(new Set());
    setDetailId(null);
    builds.clearError();
  }, [activeWorkspaceId, builds.clearError]);

  const busy = useMemo(() => {
    const set = busyServiceIds(tasks);
    for (const id of builds.pending) {
      set.add(id);
    }
    return set;
  }, [tasks, builds.pending]);

  const lastOperations = useMemo(() => lastTaskByService(tasks), [tasks]);

  const selectedTargets = useMemo(
    () =>
      rows
        .filter((row) => selectedIds.has(row.service_id))
        .sort((a, b) => a.service_id.localeCompare(b.service_id)),
    [rows, selectedIds],
  );
  const selectedCount = selectedTargets.length;
  const busySelectedCount = selectedTargets.filter((row) => busy.has(row.service_id)).length;
  const unbuildableSelected = selectedTargets.filter((row) => !canBuild(row));
  const allVisible =
    visibleIds.length > 0 && visibleIds.every((id) => selectedIds.has(id));
  const someVisible = visibleIds.some((id) => selectedIds.has(id)) && !allVisible;

  const detailTarget = useMemo(
    () => rows.find((row) => row.service_id === detailId) ?? null,
    [rows, detailId],
  );
  const detailLastTask = detailTarget
    ? lastOperations.get(detailTarget.service_id) ?? null
    : null;

  // Open the progress dialog once per submitted operation.
  const lastOpenedKey = useRef(0);
  useEffect(() => {
    const latest = builds.operations[0];
    if (latest && latest.key !== lastOpenedKey.current) {
      lastOpenedKey.current = latest.key;
      setProgress(latest);
    }
  }, [builds.operations]);

  const toggleSelect = (id: string, selected: boolean) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (selected) {
        next.add(id);
      } else {
        next.delete(id);
      }
      return next;
    });
  };
  const toggleSelectAllVisible = (selected: boolean) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      for (const id of visibleIds) {
        if (selected) {
          next.add(id);
        } else {
          next.delete(id);
        }
      }
      return next;
    });
  };

  const runBuild = useCallback(
    (targetList: readonly BuildTargetSummary[]) => {
      if (targetList.length === 0) {
        return;
      }
      const operationTargets: OperationTarget[] = targetList.map((target) => ({
        id: target.service_id,
        label: target.display_name,
      }));
      void builds.runBuild(operationTargets);
    },
    [builds],
  );

  const clearFilters = () => setSearch('');

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
        description="Configure a workspace in ~/.service-orchestrator/config.toml to see its build targets."
      />
    );
  }

  // Resolve a task to show live logs for the current operation.
  const logTaskId = progress
    ? progress.outcome.task_id ??
      (progress.outcome.workflow_id
        ? tasksForWorkflow(tasks, progress.outcome.workflow_id)[0]?.id ?? null
        : null)
    : null;
  const logTask = logTaskId ? tasks.find((task) => task.id === logTaskId) : undefined;
  const progressTargetLabel =
    progress && progress.labels.length === 1
      ? progress.labels[0]
      : progress
        ? `${progress.labels.length} services`
        : '';

  return (
    <div className="page">
      <div className="toolbar" role="search">
        <label className="toolbar__field">
          <span className="visually-hidden">Search build targets</span>
          <input
            type="search"
            className="input"
            placeholder="Search by name, id, or provider…"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            aria-label="Search build targets"
          />
        </label>
        <span className="toolbar__count" aria-live="polite">
          {pluralize(filtered.length, 'build target')}
          {filtered.length !== rows.length ? ` of ${rows.length}` : ''}
        </span>
        {search.trim().length > 0 ? (
          <button type="button" className="button button--small" onClick={clearFilters}>
            Clear filters
          </button>
        ) : null}
      </div>

      {!connected ? (
        <div className="page__notice" role="status">
          Disconnected from the daemon. Build actions are disabled until the
          connection returns; loaded state is kept.
        </div>
      ) : null}

      <div className="bulk-bar" role="toolbar" aria-label="Build selection actions">
        <span className="bulk-bar__count" aria-live="polite">
          <strong>{selectedCount}</strong> selected
          <span className="muted">
            {' '}
            · {filtered.length} visible · {rows.length} total
          </span>
        </span>
        <div className="bulk-bar__select">
          <button
            type="button"
            className="button button--small"
            onClick={() => toggleSelectAllVisible(true)}
            disabled={visibleIds.length === 0 || allVisible}
          >
            Select all visible
          </button>
          <button
            type="button"
            className="button button--small"
            onClick={() => setSelectedIds(new Set())}
            disabled={selectedCount === 0}
          >
            Clear
          </button>
        </div>
        <div className="bulk-bar__actions">
          <button
            type="button"
            className="button button--small button--primary"
            onClick={() => runBuild(selectedTargets)}
            disabled={
              !connected ||
              selectedCount === 0 ||
              busySelectedCount > 0 ||
              unbuildableSelected.length > 0
            }
            title={
              unbuildableSelected.length > 0
                ? 'one or more selected services expose no build operation'
                : undefined
            }
          >
            Build selected
          </button>
        </div>
        {unbuildableSelected.length > 0 ? (
          <span className="bulk-bar__hint muted" role="note">
            {unbuildableSelected.length} selected service
            {unbuildableSelected.length === 1 ? '' : 's'} expose no build operation
            through the application core: {unbuildableSelected.map((t) => t.display_name).join(', ')}.
          </span>
        ) : null}
        {busySelectedCount > 0 ? (
          <span className="bulk-bar__hint muted" role="note">
            {busySelectedCount} selected service{busySelectedCount === 1 ? ' is' : 's are'} busy —
            wait for the current operation to settle.
          </span>
        ) : null}
      </div>

      <div className="card card--table">
        {targets.loading ? <LoadingState label="Loading build targets…" /> : null}
        {targets.error ? <ErrorState error={targets.error} onRetry={targets.reload} /> : null}
        {!targets.loading && !targets.error ? (
          targets.data === null ? (
            <LoadingState
              label={connected ? 'Loading build targets…' : 'Waiting for the daemon connection…'}
            />
          ) : rows.length === 0 ? (
            <EmptyState
              title="No build targets configured"
              description={`The workspace “${activeWorkspace.name}” has no services with a build system.`}
            />
          ) : filtered.length === 0 ? (
            <EmptyState
              title="No build targets match your search"
              description="Adjust the search text."
              action={
                <button type="button" className="button" onClick={clearFilters}>
                  Clear filters
                </button>
              }
            />
          ) : (
            <BuildTargetTable
              targets={filtered}
              workspaceName={activeWorkspace.name}
              selectedIds={selectedIds}
              onToggleSelect={toggleSelect}
              onToggleSelectAll={toggleSelectAllVisible}
              allVisibleSelected={allVisible}
              someVisibleSelected={someVisible}
              detailId={detailId}
              onOpenDetail={setDetailId}
              onBuild={(target) => runBuild([target])}
              busy={busy}
              lastOperations={lastOperations}
            />
          )
        ) : null}
      </div>

      {detailId ? (
        <BuildTargetDetailPanel
          target={detailTarget}
          lastTask={detailLastTask}
          busy={detailTarget ? busy.has(detailTarget.service_id) : false}
          disabled={!connected}
          onClose={() => setDetailId(null)}
          onBuild={(target) => runBuild([target])}
        />
      ) : null}

      {detailTarget && detailLastTask ? (
        <section className="card" aria-label="Build logs">
          <div className="card__header">
            <h2 className="card__title">Build logs · {detailTarget.display_name}</h2>
          </div>
          <LogViewer
            taskId={detailLastTask.id}
            live={!isTerminalTaskStatus(detailLastTask.status)}
          />
        </section>
      ) : null}

      {progress ? (
        <OperationProgressDialog
          title={`${BUILD_ACTION_LABELS[progress.action]} — ${progressTargetLabel}`}
          outcome={progress.outcome}
          tasks={tasks}
          at={progress.at}
          targetLabel="services"
          logs={
            logTaskId ? (
              <LogViewer taskId={logTaskId} live={logTask ? !isTerminalTaskStatus(logTask.status) : false} />
            ) : null
          }
          onCancelTask={(taskId) => void desktopApi.cancelTask(taskId)}
          onCancelWorkflow={(workflowId) => void desktopApi.cancelWorkflow(workflowId)}
          onClose={() => setProgress(null)}
        />
      ) : null}

      {builds.error ? (
        <ErrorDialog
          title={`Unable to ${BUILD_ACTION_LABELS[builds.error.action].toLowerCase()} service${
            builds.error.labels.length === 1 ? '' : 's'
          }`}
          error={builds.error.error}
          labels={builds.error.labels}
          onClose={builds.clearError}
        />
      ) : null}
    </div>
  );
}
