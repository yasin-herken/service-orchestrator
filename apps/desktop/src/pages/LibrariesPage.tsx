import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { ErrorDialog } from '../components/ErrorDialog';
import { LibraryDetailPanel } from '../components/library/LibraryDetailPanel';
import { LibraryTable } from '../components/library/LibraryTable';
import { PrepareLibraryDialog } from '../components/library/PrepareLibraryDialog';
import { EmptyState, ErrorState, LoadingState } from '../components/StateViews';
import { OperationProgressDialog } from '../components/tasks/OperationProgressDialog';
import { useConnection } from '../context/ConnectionContext';
import { useTaskEvents } from '../context/TaskEventsContext';
import { useWorkspaces } from '../context/WorkspaceContext';
import { useAsync } from '../hooks/useAsync';
import type { OperationTarget, SubmittedOperation } from '../hooks/useEntityOperations';
import {
  LIBRARY_ACTION_LABELS,
  useLibraryOperations,
  type LibraryAction,
} from '../hooks/useLibraryOperations';
import { desktopApi, type PrepareOptions } from '../services/desktopApi';
import { toDesktopError } from '../services/errors';
import type {
  ArtifactVerificationView,
  DesktopError,
  LibraryDependentView,
  LibrarySummary,
  VersionAlignmentView,
} from '../types/models';
import { pluralize } from '../utils/format';
import { busyServiceIds, lastTaskByService } from '../utils/taskProgress';

/** Applies the search text to a library list. */
function filterLibraries(
  libraries: readonly LibrarySummary[],
  search: string,
): LibrarySummary[] {
  const needle = search.trim().toLowerCase();
  if (needle.length === 0) {
    return [...libraries];
  }
  return libraries.filter(
    (library) =>
      library.id.toLowerCase().includes(needle) ||
      library.display_name.toLowerCase().includes(needle) ||
      (library.artifact?.coordinate ?? '').toLowerCase().includes(needle) ||
      (library.configured_version ?? '').toLowerCase().includes(needle),
  );
}

/** A library selection expressed as operation targets. */
function targetsOf(libraries: readonly LibrarySummary[]): OperationTarget[] {
  return libraries.map((library) => ({
    id: library.id,
    label: library.display_name,
  }));
}

/**
 * The library management screen (`tasks/020.md` §20–§26).
 *
 * It shows real artifact coordinates, local availability, dependents, and
 * consumer version alignment, and it submits builds, installs, syncs, and
 * prepare workflows through the shared daemon. Git synchronization is optional
 * and no version is ever rewritten.
 */
export function LibrariesPage() {
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

  const libraries = useAsync<LibrarySummary[]>(
    () => {
      if (!activeWorkspace) {
        return Promise.resolve([]);
      }
      return desktopApi.listLibraries(activeWorkspace.id);
    },
    [activeWorkspace?.id, terminalRevision],
    { enabled: connected && activeWorkspace !== null },
  );
  const reloadLibraries = libraries.reload;

  const [search, setSearch] = useState('');
  const [selectedIds, setSelectedIds] = useState<ReadonlySet<string>>(new Set());
  const [detailId, setDetailId] = useState<string | null>(null);

  const [prepareDialog, setPrepareDialog] = useState<{ targets: LibrarySummary[] } | null>(
    null,
  );

  const [dependents, setDependents] = useState<LibraryDependentView[] | null>(null);
  const [dependentsLoading, setDependentsLoading] = useState(false);
  const [dependentsError, setDependentsError] = useState<DesktopError | null>(null);

  const [verification, setVerification] = useState<ArtifactVerificationView | null>(null);
  const [verifying, setVerifying] = useState(false);
  const [verificationError, setVerificationError] = useState<DesktopError | null>(null);

  const [alignment, setAlignment] = useState<VersionAlignmentView | null>(null);
  const [aligning, setAligning] = useState(false);
  const [alignmentError, setAlignmentError] = useState<DesktopError | null>(null);

  const librariesOps = useLibraryOperations(activeWorkspaceId, reloadLibraries);
  const [progress, setProgress] = useState<SubmittedOperation<LibraryAction> | null>(null);

  const rows = libraries.data ?? [];
  const filtered = useMemo(() => filterLibraries(rows, search), [rows, search]);
  const visibleIds = useMemo(() => filtered.map((row) => row.id), [filtered]);

  // Keep the selection stable as the list changes.
  useEffect(() => {
    setSelectedIds((current) => {
      const keep = new Set(rows.map((row) => row.id));
      const next = new Set<string>();
      for (const id of current) {
        if (keep.has(id)) {
          next.add(id);
        }
      }
      return next.size === current.size ? current : next;
    });
  }, [rows]);

  // Scope everything to the active workspace (`tasks/020.md` §33).
  useEffect(() => {
    setSearch('');
    setSelectedIds(new Set());
    setDetailId(null);
    setPrepareDialog(null);
    setDependents(null);
    setVerification(null);
    setAlignment(null);
    setDependentsError(null);
    setVerificationError(null);
    setAlignmentError(null);
    librariesOps.clearError();
  }, [activeWorkspaceId, librariesOps.clearError]);

  const busy = useMemo(() => {
    const set = busyServiceIds(tasks);
    for (const id of librariesOps.pending) {
      set.add(id);
    }
    return set;
  }, [tasks, librariesOps.pending]);

  const lastOperations = useMemo(() => lastTaskByService(tasks), [tasks]);

  const selectedLibraries = useMemo(
    () =>
      rows
        .filter((row) => selectedIds.has(row.id))
        .sort((a, b) => a.id.localeCompare(b.id)),
    [rows, selectedIds],
  );
  const selectedCount = selectedLibraries.length;
  const busySelectedCount = selectedLibraries.filter((row) => busy.has(row.id)).length;
  const allVisible = visibleIds.length > 0 && visibleIds.every((id) => selectedIds.has(id));
  const someVisible = visibleIds.some((id) => selectedIds.has(id)) && !allVisible;

  const detailLibrary = useMemo(
    () => rows.find((row) => row.id === detailId) ?? null,
    [rows, detailId],
  );

  // Open the progress dialog once per submitted operation.
  const lastOpenedKey = useRef(0);
  useEffect(() => {
    const latest = librariesOps.operations[0];
    if (latest && latest.key !== lastOpenedKey.current) {
      lastOpenedKey.current = latest.key;
      setProgress(latest);
    }
  }, [librariesOps.operations]);

  const loadDependents = useCallback(async (libraryId: string) => {
    setDependentsLoading(true);
    setDependentsError(null);
    try {
      setDependents(await desktopApi.getLibraryDependents(libraryId));
    } catch (cause) {
      setDependentsError(toDesktopError(cause));
    } finally {
      setDependentsLoading(false);
    }
  }, []);

  const verifyArtifact = useCallback(async (libraryId: string) => {
    setVerifying(true);
    setVerificationError(null);
    try {
      setVerification(await desktopApi.verifyLibraryArtifact(libraryId));
    } catch (cause) {
      setVerificationError(toDesktopError(cause));
    } finally {
      setVerifying(false);
    }
  }, []);

  const inspectAlignment = useCallback(async (libraryId: string) => {
    setAligning(true);
    setAlignmentError(null);
    try {
      setAlignment(await desktopApi.inspectVersionAlignment(libraryId));
    } catch (cause) {
      setAlignmentError(toDesktopError(cause));
    } finally {
      setAligning(false);
    }
  }, []);

  // On opening a library, load its cheap inspection data (dependents and
  // artifact presence). Alignment is inspected on request, as it reads every
  // consumer's dependency (`tasks/020.md` §21).
  useEffect(() => {
    setDependents(null);
    setVerification(null);
    setAlignment(null);
    setDependentsError(null);
    setVerificationError(null);
    setAlignmentError(null);
    if (!connected || !detailLibrary) {
      return;
    }
    void loadDependents(detailLibrary.id);
    void verifyArtifact(detailLibrary.id);
    // Intentionally keyed on the open library id.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connected, detailLibrary?.id]);

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

  const executePrepare = (options: PrepareOptions) => {
    const targets = prepareDialog?.targets ?? [];
    setPrepareDialog(null);
    if (targets.length === 0) {
      return;
    }
    void librariesOps.runPrepare(targetsOf(targets), options);
  };

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
        description="Configure a workspace in ~/.service-orchestrator/config.toml to see its libraries."
      />
    );
  }

  const progressTargetLabel =
    progress && progress.labels.length === 1
      ? progress.labels[0]
      : progress
        ? `${progress.labels.length} libraries`
        : '';

  return (
    <div className="page">
      <div className="toolbar" role="search">
        <label className="toolbar__field">
          <span className="visually-hidden">Search libraries</span>
          <input
            type="search"
            className="input"
            placeholder="Search by name, id, or coordinate…"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            aria-label="Search libraries"
          />
        </label>
        <span className="toolbar__count" aria-live="polite">
          {pluralize(filtered.length, 'library')}
          {filtered.length !== rows.length ? ` of ${rows.length}` : ''}
        </span>
        {search.trim().length > 0 ? (
          <button type="button" className="button button--small" onClick={() => setSearch('')}>
            Clear filters
          </button>
        ) : null}
      </div>

      {!connected ? (
        <div className="page__notice" role="status">
          Disconnected from the daemon. Library actions are disabled until the
          connection returns; loaded state is kept.
        </div>
      ) : null}

      <div className="bulk-bar" role="toolbar" aria-label="Library selection actions">
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
            className="button button--small"
            onClick={() => void librariesOps.runBuild(targetsOf(selectedLibraries))}
            disabled={!connected || selectedCount === 0 || busySelectedCount > 0}
          >
            Build selected
          </button>
          <button
            type="button"
            className="button button--small"
            onClick={() => void librariesOps.runInstall(targetsOf(selectedLibraries))}
            disabled={!connected || selectedCount === 0 || busySelectedCount > 0}
          >
            Install selected
          </button>
          <button
            type="button"
            className="button button--small"
            onClick={() => void librariesOps.runSync(targetsOf(selectedLibraries))}
            disabled={!connected || selectedCount === 0 || busySelectedCount > 0}
          >
            Sync selected
          </button>
          <button
            type="button"
            className="button button--small button--primary"
            onClick={() => setPrepareDialog({ targets: selectedLibraries })}
            disabled={!connected || selectedCount === 0 || busySelectedCount > 0}
          >
            Prepare selected…
          </button>
        </div>
        {busySelectedCount > 0 ? (
          <span className="bulk-bar__hint muted" role="note">
            {busySelectedCount} selected librar{busySelectedCount === 1 ? 'y is' : 'ies are'} busy —
            wait for the current operation to settle.
          </span>
        ) : null}
      </div>

      <div className="card card--table">
        {libraries.loading ? <LoadingState label="Loading libraries…" /> : null}
        {libraries.error ? (
          <ErrorState error={libraries.error} onRetry={libraries.reload} />
        ) : null}
        {!libraries.loading && !libraries.error ? (
          libraries.data === null ? (
            <LoadingState
              label={connected ? 'Loading libraries…' : 'Waiting for the daemon connection…'}
            />
          ) : rows.length === 0 ? (
            <EmptyState
              title="No libraries configured"
              description={`The workspace “${activeWorkspace.name}” has no libraries.`}
            />
          ) : filtered.length === 0 ? (
            <EmptyState
              title="No libraries match your search"
              description="Adjust the search text."
              action={
                <button type="button" className="button" onClick={() => setSearch('')}>
                  Clear filters
                </button>
              }
            />
          ) : (
            <LibraryTable
              libraries={filtered}
              workspaceName={activeWorkspace.name}
              selectedIds={selectedIds}
              onToggleSelect={toggleSelect}
              onToggleSelectAll={toggleSelectAllVisible}
              allVisibleSelected={allVisible}
              someVisibleSelected={someVisible}
              detailId={detailId}
              onOpenDetail={setDetailId}
              busy={busy}
              lastOperations={lastOperations}
            />
          )
        ) : null}
      </div>

      {detailId ? (
        <LibraryDetailPanel
          library={detailLibrary}
          lastTask={detailLibrary ? lastOperations.get(detailLibrary.id) ?? null : null}
          busy={detailLibrary ? busy.has(detailLibrary.id) : false}
          disabled={!connected}
          dependents={dependents}
          dependentsLoading={dependentsLoading}
          dependentsError={dependentsError}
          verification={verification}
          verifying={verifying}
          verificationError={verificationError}
          alignment={alignment}
          aligning={aligning}
          alignmentError={alignmentError}
          onClose={() => setDetailId(null)}
          onBuild={() => detailLibrary && void librariesOps.runBuild(targetsOf([detailLibrary]))}
          onInstall={() =>
            detailLibrary && void librariesOps.runInstall(targetsOf([detailLibrary]))
          }
          onSync={() => detailLibrary && void librariesOps.runSync(targetsOf([detailLibrary]))}
          onPrepare={() => detailLibrary && setPrepareDialog({ targets: [detailLibrary] })}
          onLoadDependents={() =>
            detailLibrary && void loadDependents(detailLibrary.id)
          }
          onVerify={() => detailLibrary && void verifyArtifact(detailLibrary.id)}
          onInspectAlignment={() =>
            detailLibrary && void inspectAlignment(detailLibrary.id)
          }
        />
      ) : null}

      {prepareDialog ? (
        <PrepareLibraryDialog
          title={
            prepareDialog.targets.length === 1
              ? `Prepare ${prepareDialog.targets[0].display_name}`
              : `Prepare ${prepareDialog.targets.length} libraries`
          }
          targets={prepareDialog.targets.map((target) => target.display_name)}
          onExecute={executePrepare}
          onClose={() => setPrepareDialog(null)}
        />
      ) : null}

      {progress ? (
        <OperationProgressDialog
          title={`${LIBRARY_ACTION_LABELS[progress.action]} — ${progressTargetLabel}`}
          outcome={progress.outcome}
          tasks={tasks}
          at={progress.at}
          targetLabel="libraries"
          onCancelTask={(taskId) => void desktopApi.cancelTask(taskId)}
          onCancelWorkflow={(workflowId) => void desktopApi.cancelWorkflow(workflowId)}
          onClose={() => setProgress(null)}
        />
      ) : null}

      {librariesOps.error ? (
        <ErrorDialog
          title={`Unable to ${LIBRARY_ACTION_LABELS[librariesOps.error.action].toLowerCase()} librar${
            librariesOps.error.labels.length === 1 ? 'y' : 'ies'
          }`}
          error={librariesOps.error.error}
          labels={librariesOps.error.labels}
          onClose={librariesOps.clearError}
        />
      ) : null}
    </div>
  );
}
