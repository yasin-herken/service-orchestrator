import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { ErrorDialog } from '../components/ErrorDialog';
import { GitSyncDialog } from '../components/git/GitSyncDialog';
import { RepositoryDetailPanel } from '../components/git/RepositoryDetailPanel';
import {
  RepositoryTable,
  type RepositoryTableSort,
} from '../components/git/RepositoryTable';
import { EmptyState, ErrorState, LoadingState } from '../components/StateViews';
import { OperationProgressDialog } from '../components/tasks/OperationProgressDialog';
import { useConnection } from '../context/ConnectionContext';
import { useTaskEvents } from '../context/TaskEventsContext';
import { useWorkspaces } from '../context/WorkspaceContext';
import { useAsync } from '../hooks/useAsync';
import type { OperationTarget, SubmittedOperation } from '../hooks/useEntityOperations';
import {
  GIT_ACTION_LABELS,
  useGitOperations,
  type GitAction,
} from '../hooks/useGitOperations';
import { desktopApi } from '../services/desktopApi';
import { toDesktopError } from '../services/errors';
import type {
  BranchView,
  DesktopError,
  GitState,
  GitSyncMode,
  GitSyncPreview,
  RepositoryInspectionView,
  RepositorySummary,
} from '../types/models';
import { pluralize } from '../utils/format';
import {
  distinctBranches,
  distinctProviders,
  filterRepositories,
  sortRepositories,
  type RepositorySortKey,
} from '../utils/repositoryQuery';
import {
  allSelected,
  emptySelection,
  pruneTo,
  setIds,
  someSelected,
} from '../utils/serviceSelection';
import { busyServiceIds, lastTaskByService } from '../utils/taskProgress';

/** The Git working-copy states offered by the filter. */
const GIT_STATES: readonly GitState[] = [
  'clean',
  'modified',
  'ahead',
  'behind',
  'diverged',
  'unknown',
];

/** A repository selection expressed as operation targets. */
function targetsOf(repositories: readonly RepositorySummary[]): OperationTarget[] {
  return repositories.map((repository) => ({
    id: repository.service_id,
    label: repository.name,
  }));
}

/**
 * The Git management screen (`tasks/020.md` §4–§11).
 *
 * The repository list is configuration-derived and cheap; real working-tree
 * state and branches are inspected on demand. Bulk synchronization is previewed
 * read-only and then submitted as one coordinated daemon workflow — never as an
 * uncoordinated loop — and it never silently discards local changes.
 */
export function GitPage() {
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

  const repositories = useAsync<RepositorySummary[]>(
    () => {
      if (!activeWorkspace) {
        return Promise.resolve([]);
      }
      return desktopApi.listRepositories(activeWorkspace.id);
    },
    [activeWorkspace?.id, terminalRevision],
    { enabled: connected && activeWorkspace !== null },
  );
  const reloadRepositories = repositories.reload;

  const [search, setSearch] = useState('');
  const [provider, setProvider] = useState('all');
  const [branch, setBranch] = useState('all');
  const [gitState, setGitState] = useState('all');
  const [sort, setSort] = useState<RepositoryTableSort>({ key: 'name', direction: 'asc' });
  const [selectedIds, setSelectedIds] = useState<ReadonlySet<string>>(emptySelection);
  const [detailId, setDetailId] = useState<string | null>(null);

  const [inspections, setInspections] = useState<Map<string, RepositoryInspectionView>>(
    new Map(),
  );
  const [branches, setBranches] = useState<Map<string, BranchView[]>>(new Map());
  const [inspecting, setInspecting] = useState<ReadonlySet<string>>(new Set());
  const [branchesLoading, setBranchesLoading] = useState<ReadonlySet<string>>(new Set());
  const [inspectionError, setInspectionError] = useState<DesktopError | null>(null);
  const [branchesError, setBranchesError] = useState<DesktopError | null>(null);

  const [syncDialog, setSyncDialog] = useState<{ targets: RepositorySummary[] } | null>(null);
  const [preview, setPreview] = useState<GitSyncPreview | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [previewError, setPreviewError] = useState<DesktopError | null>(null);

  const git = useGitOperations(activeWorkspaceId, reloadRepositories);
  const [progress, setProgress] = useState<SubmittedOperation<GitAction> | null>(null);

  // Repositories whose status should be refreshed once the next Git task settles.
  const pendingInspect = useRef<Set<string>>(new Set());

  const rows = repositories.data ?? [];
  const rowsRef = useRef<RepositorySummary[]>(rows);
  useEffect(() => {
    rowsRef.current = rows;
  }, [rows]);

  const providerOptions = useMemo(() => distinctProviders(rows), [rows]);
  const branchOptions = useMemo(() => distinctBranches(rows), [rows]);

  const filtered = useMemo(
    () =>
      sortRepositories(
        filterRepositories(rows, { search, provider, branch, gitState }),
        sort.key,
        sort.direction,
      ),
    [rows, search, provider, branch, gitState, sort],
  );
  const visibleIds = useMemo(() => filtered.map((row) => row.id), [filtered]);

  // Keep the selection stable as the list changes.
  useEffect(() => {
    setSelectedIds((current) => {
      const pruned = pruneTo(current, rows.map((row) => row.id));
      return pruned.size === current.size ? current : pruned;
    });
  }, [rows]);

  // Scope everything to the active workspace: no cross-workspace selection,
  // detail, dialog, or filter bleed (`tasks/020.md` §33).
  useEffect(() => {
    setSelectedIds(emptySelection());
    setDetailId(null);
    setSyncDialog(null);
    setPreview(null);
    setPreviewError(null);
    setInspections(new Map());
    setBranches(new Map());
    setInspectionError(null);
    setBranchesError(null);
    setSearch('');
    setProvider('all');
    setBranch('all');
    setGitState('all');
    pendingInspect.current = new Set();
    git.clearError();
  }, [activeWorkspaceId, git.clearError]);

  const busy = useMemo(() => {
    const set = busyServiceIds(tasks);
    for (const id of git.pending) {
      set.add(id);
    }
    return set;
  }, [tasks, git.pending]);

  const lastOperations = useMemo(() => lastTaskByService(tasks), [tasks]);

  const selectedRepositories = useMemo(
    () =>
      rows
        .filter((row) => selectedIds.has(row.id))
        .sort((a, b) => a.id.localeCompare(b.id)),
    [rows, selectedIds],
  );
  const selectedCount = selectedRepositories.length;
  const busySelectedCount = selectedRepositories.filter((row) =>
    busy.has(row.service_id),
  ).length;
  const allVisible = allSelected(selectedIds, visibleIds);
  const someVisible = someSelected(selectedIds, visibleIds);

  const detailRepository = useMemo(
    () => rows.find((row) => row.id === detailId) ?? null,
    [rows, detailId],
  );

  // Open the progress dialog once per submitted operation.
  const lastOpenedKey = useRef(0);
  useEffect(() => {
    const latest = git.operations[0];
    if (latest && latest.key !== lastOpenedKey.current) {
      lastOpenedKey.current = latest.key;
      setProgress(latest);
    }
  }, [git.operations]);

  const inspectRepositories = useCallback(
    async (targets: readonly RepositorySummary[]) => {
      if (!activeWorkspace || targets.length === 0) {
        return;
      }
      const ids = targets.map((target) => target.id);
      setInspecting((current) => new Set([...current, ...ids]));
      setInspectionError(null);
      try {
        const results = await desktopApi.inspectRepositories(
          activeWorkspace.id,
          targets.map((target) => target.service_id),
        );
        setInspections((current) => {
          const next = new Map(current);
          for (const result of results) {
            next.set(result.repository_id, result);
          }
          return next;
        });
      } catch (cause) {
        setInspectionError(toDesktopError(cause));
      } finally {
        setInspecting((current) => {
          const next = new Set(current);
          for (const id of ids) {
            next.delete(id);
          }
          return next;
        });
      }
    },
    [activeWorkspace],
  );

  const loadBranches = useCallback(async (repository: RepositorySummary) => {
    setBranchesLoading((current) => new Set([...current, repository.id]));
    setBranchesError(null);
    try {
      const list = await desktopApi.listBranches(repository.service_id);
      setBranches((current) => new Map(current).set(repository.id, list));
    } catch (cause) {
      setBranchesError(toDesktopError(cause));
    } finally {
      setBranchesLoading((current) => {
        const next = new Set(current);
        next.delete(repository.id);
        return next;
      });
    }
  }, []);

  // Inspect a repository as soon as its detail opens (never all repositories).
  useEffect(() => {
    if (!connected || !detailRepository) {
      return;
    }
    if (!inspections.has(detailRepository.id)) {
      void inspectRepositories([detailRepository]);
    }
    if (!branches.has(detailRepository.id)) {
      void loadBranches(detailRepository);
    }
    // Intentionally keyed on the open repository id.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connected, detailRepository?.id]);

  // After a submitted Git operation settles, refresh the status of the
  // repositories it touched — bounded to the explicit selection.
  useEffect(() => {
    const pending = pendingInspect.current;
    if (pending.size === 0) {
      return;
    }
    const targets = rowsRef.current.filter((row) => pending.has(row.service_id));
    pending.clear();
    if (targets.length > 0) {
      void inspectRepositories(targets);
    }
    // Intentionally keyed on the terminal revision only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [terminalRevision]);

  const toggleSelect = (id: string, selected: boolean) => {
    setSelectedIds((current) => setIds(current, [id], selected));
  };
  const toggleSelectAllVisible = (selected: boolean) => {
    setSelectedIds((current) => setIds(current, visibleIds, selected));
  };
  const toggleSort = (key: RepositorySortKey) => {
    setSort((current) =>
      current.key === key
        ? { key, direction: current.direction === 'asc' ? 'desc' : 'asc' }
        : { key, direction: 'asc' },
    );
  };
  const clearFilters = () => {
    setSearch('');
    setProvider('all');
    setBranch('all');
    setGitState('all');
  };

  /** Records the touched service ids for a post-settle refresh. */
  const remember = (targets: readonly RepositorySummary[]) => {
    for (const target of targets) {
      pendingInspect.current.add(target.service_id);
    }
  };

  const runFetch = (targets: readonly RepositorySummary[]) => {
    if (targets.length === 0) return;
    remember(targets);
    void git.runFetch(targetsOf(targets));
  };
  const runSync = (targets: readonly RepositorySummary[]) => {
    if (targets.length === 0) return;
    remember(targets);
    void git.runSync(targetsOf(targets));
  };
  const runCheckout = (repository: RepositorySummary, target: string) => {
    remember([repository]);
    void git.runCheckout(targetsOf([repository]), target);
  };
  const runPull = (repository: RepositorySummary) => {
    remember([repository]);
    void git.runPull(targetsOf([repository]));
  };

  const openSyncDialog = () => {
    if (selectedRepositories.length === 0) {
      return;
    }
    setPreview(null);
    setPreviewError(null);
    setSyncDialog({ targets: selectedRepositories });
  };

  const previewSync = async (target: string, mode: GitSyncMode) => {
    if (!syncDialog || !activeWorkspace) {
      return;
    }
    setPreviewing(true);
    setPreviewError(null);
    try {
      const result = await desktopApi.previewGitSync(
        activeWorkspace.id,
        syncDialog.targets.map((repository) => repository.service_id),
        target,
        mode,
      );
      setPreview(result);
    } catch (cause) {
      setPreviewError(toDesktopError(cause));
    } finally {
      setPreviewing(false);
    }
  };

  const executeSync = (target: string, mode: GitSyncMode) => {
    const targets = syncDialog?.targets ?? [];
    setSyncDialog(null);
    if (targets.length === 0) {
      return;
    }
    if (mode === 'fetch') {
      runFetch(targets);
    } else if (mode === 'checkout') {
      remember(targets);
      void git.runCheckout(targetsOf(targets), target);
    } else {
      runSync(targets);
    }
  };

  const filtersActive =
    search.trim().length > 0 ||
    provider !== 'all' ||
    branch !== 'all' ||
    gitState !== 'all';

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
        description="Configure a workspace in ~/.service-orchestrator/config.toml to see its repositories."
      />
    );
  }

  const progressTargetLabel =
    progress && progress.labels.length === 1
      ? progress.labels[0]
      : progress
        ? `${progress.labels.length} repositories`
        : '';

  return (
    <div className="page">
      <div className="toolbar" role="search">
        <label className="toolbar__field">
          <span className="visually-hidden">Search repositories</span>
          <input
            type="search"
            className="input"
            placeholder="Search by name, id, path, or branch…"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            aria-label="Search repositories"
          />
        </label>
        <label className="toolbar__field">
          <span className="toolbar__field-label">Type</span>
          <select
            className="input input--select"
            value={provider}
            onChange={(event) => setProvider(event.target.value)}
            aria-label="Filter by repository type"
          >
            <option value="all">All types</option>
            {providerOptions.map((option) => (
              <option key={option} value={option}>
                {option}
              </option>
            ))}
          </select>
        </label>
        <label className="toolbar__field">
          <span className="toolbar__field-label">Branch</span>
          <select
            className="input input--select"
            value={branch}
            onChange={(event) => setBranch(event.target.value)}
            aria-label="Filter by branch"
          >
            <option value="all">All branches</option>
            {branchOptions.map((option) => (
              <option key={option} value={option}>
                {option}
              </option>
            ))}
          </select>
        </label>
        <label className="toolbar__field">
          <span className="toolbar__field-label">State</span>
          <select
            className="input input--select"
            value={gitState}
            onChange={(event) => setGitState(event.target.value)}
            aria-label="Filter by Git state"
          >
            <option value="all">All states</option>
            {GIT_STATES.map((option) => (
              <option key={option} value={option}>
                {option}
              </option>
            ))}
          </select>
        </label>
        <span className="toolbar__count" aria-live="polite">
          {pluralize(filtered.length, 'repository')}
          {filtered.length !== rows.length ? ` of ${rows.length}` : ''}
        </span>
        {filtersActive ? (
          <button type="button" className="button button--small" onClick={clearFilters}>
            Clear filters
          </button>
        ) : null}
      </div>

      {!connected ? (
        <div className="page__notice" role="status">
          Disconnected from the daemon. Git actions are disabled until the
          connection returns; loaded state is kept.
        </div>
      ) : null}

      <div className="bulk-bar" role="toolbar" aria-label="Repository selection actions">
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
            onClick={() => setSelectedIds(emptySelection())}
            disabled={selectedCount === 0}
          >
            Clear
          </button>
        </div>
        <div className="bulk-bar__actions">
          <button
            type="button"
            className="button button--small"
            onClick={() => inspectRepositories(selectedRepositories)}
            disabled={!connected || selectedCount === 0}
          >
            Inspect selected
          </button>
          <button
            type="button"
            className="button button--small"
            onClick={() => runFetch(selectedRepositories)}
            disabled={!connected || selectedCount === 0 || busySelectedCount > 0}
          >
            Fetch selected
          </button>
          <button
            type="button"
            className="button button--small button--primary"
            onClick={openSyncDialog}
            disabled={!connected || selectedCount === 0 || busySelectedCount > 0}
          >
            Sync selected…
          </button>
        </div>
        {busySelectedCount > 0 ? (
          <span className="bulk-bar__hint muted" role="note">
            {busySelectedCount} selected repositor{busySelectedCount === 1 ? 'y is' : 'ies are'} busy —
            wait for the current operation to settle.
          </span>
        ) : null}
      </div>

      <div className="card card--table">
        {repositories.loading ? <LoadingState label="Loading repositories…" /> : null}
        {repositories.error ? (
          <ErrorState error={repositories.error} onRetry={repositories.reload} />
        ) : null}
        {!repositories.loading && !repositories.error ? (
          repositories.data === null ? (
            <LoadingState
              label={connected ? 'Loading repositories…' : 'Waiting for the daemon connection…'}
            />
          ) : rows.length === 0 ? (
            <EmptyState
              title="No repositories configured"
              description={`The workspace “${activeWorkspace.name}” has no services with a repository.`}
            />
          ) : filtered.length === 0 ? (
            <EmptyState
              title="No repositories match your filters"
              description="Adjust the search text or filters."
              action={
                <button type="button" className="button" onClick={clearFilters}>
                  Clear filters
                </button>
              }
            />
          ) : (
            <RepositoryTable
              repositories={filtered}
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

      {detailRepository ? (
        <RepositoryDetailPanel
          repository={detailRepository}
          inspection={inspections.get(detailRepository.id) ?? null}
          branches={branches.get(detailRepository.id) ?? []}
          inspecting={inspecting.has(detailRepository.id)}
          branchesLoading={branchesLoading.has(detailRepository.id)}
          inspectionError={inspectionError}
          branchesError={branchesError}
          lastTask={lastOperations.get(detailRepository.service_id) ?? null}
          busy={busy.has(detailRepository.service_id)}
          disabled={!connected}
          onClose={() => setDetailId(null)}
          onInspect={() => void inspectRepositories([detailRepository])}
          onLoadBranches={() => void loadBranches(detailRepository)}
          onFetch={() => runFetch([detailRepository])}
          onPull={() => runPull(detailRepository)}
          onSync={() => runSync([detailRepository])}
          onCheckout={(target) => runCheckout(detailRepository, target)}
        />
      ) : null}

      {syncDialog ? (
        <GitSyncDialog
          workspaceName={activeWorkspace.name}
          targets={syncDialog.targets}
          preview={preview}
          previewing={previewing}
          previewError={previewError}
          onPreview={previewSync}
          onExecute={executeSync}
          executing={false}
          onClose={() => setSyncDialog(null)}
        />
      ) : null}

      {progress ? (
        <OperationProgressDialog
          title={`${GIT_ACTION_LABELS[progress.action]} — ${progressTargetLabel}`}
          outcome={progress.outcome}
          tasks={tasks}
          at={progress.at}
          targetLabel="repositories"
          onCancelTask={(taskId) => void desktopApi.cancelTask(taskId)}
          onCancelWorkflow={(workflowId) => void desktopApi.cancelWorkflow(workflowId)}
          onClose={() => setProgress(null)}
        />
      ) : null}

      {git.error ? (
        <ErrorDialog
          title={`Unable to ${GIT_ACTION_LABELS[git.error.action].toLowerCase()} repositor${
            git.error.labels.length === 1 ? 'y' : 'ies'
          }`}
          error={git.error.error}
          labels={git.error.labels}
          onClose={git.clearError}
        />
      ) : null}
    </div>
  );
}
