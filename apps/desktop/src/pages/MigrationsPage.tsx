import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { ConfirmationDialog } from '../components/ConfirmationDialog';
import { ErrorDialog } from '../components/ErrorDialog';
import { MigrationDetailPanel } from '../components/migration/MigrationDetailPanel';
import { MigrationTable } from '../components/migration/MigrationTable';
import { SqlPreviewDialog } from '../components/migration/SqlPreviewDialog';
import { EmptyState, ErrorState, LoadingState } from '../components/StateViews';
import { OperationProgressDialog } from '../components/tasks/OperationProgressDialog';
import { isTerminalTaskStatus } from '../components/tasks/TaskStatusIndicator';
import { useConnection } from '../context/ConnectionContext';
import { useTaskEvents } from '../context/TaskEventsContext';
import { useWorkspaces } from '../context/WorkspaceContext';
import { useAsync } from '../hooks/useAsync';
import { desktopApi } from '../services/desktopApi';
import { toDesktopError } from '../services/errors';
import type {
  DesktopError,
  LogEntryView,
  MigrationRowView,
  MigrationStatusView,
  OperationOutcome,
  TaskView,
} from '../types/models';
import type { NavIntent, PageId } from '../types/nav';
import { pluralize } from '../utils/format';
import { busyServiceIds } from '../utils/taskProgress';

/** A submitted migration operation, kept for the progress dialog. */
interface SubmittedMigration {
  action: 'validate' | 'inspect' | 'pending' | 'apply' | 'applySelected';
  outcome: OperationOutcome;
  labels: string[];
  at: number;
}

/** The in-flight SQL preview. */
interface PreviewState {
  serviceId: string;
  loading: boolean;
  sql: string;
  error: DesktopError | null;
}

/** Reads a task until it settles, or `null` when it never did in time. */
async function waitForTask(taskId: string): Promise<TaskView | null> {
  for (let attempt = 0; attempt < 40; attempt += 1) {
    const task = await desktopApi.getTask(taskId);
    if (isTerminalTaskStatus(task.status)) {
      return task;
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  return null;
}

/** Picks the SQL out of a preview task's own logs. */
function extractSql(entries: readonly LogEntryView[]): string {
  const fromProvider = entries
    .filter((entry) => entry.source === 'migration')
    .map((entry) => entry.message);
  if (fromProvider.length > 0) {
    return fromProvider.join('\n');
  }
  const looksLikeSql = /\b(create|alter|drop|insert|update|delete|select|begin|commit|--)\b/i;
  const sqlLines = entries
    .filter((entry) => looksLikeSql.test(entry.message))
    .map((entry) => entry.message);
  return (sqlLines.length > 0 ? sqlLines : entries.map((entry) => entry.message)).join('\n');
}

/** Applies the search text to a migration row list. */
function filterRows(
  rows: readonly MigrationRowView[],
  search: string,
): MigrationRowView[] {
  const needle = search.trim().toLowerCase();
  if (needle.length === 0) {
    return [...rows];
  }
  return rows.filter(
    (row) =>
      row.status.service_id.toLowerCase().includes(needle) ||
      row.status.workspace_id.toLowerCase().includes(needle) ||
      row.status.provider.toLowerCase().includes(needle) ||
      row.status.environment.toLowerCase().includes(needle) ||
      row.status.database_identity.toLowerCase().includes(needle),
  );
}

/**
 * The Migration Management screen (`tasks/021.md` §4–§9).
 *
 * Every value comes from the daemon's configuration snapshot and real task
 * engine: unknown pending counts stay unknown, apply always passes through an
 * explicit confirmation that the daemon independently re-verifies, and bulk
 * apply is a single coordinated backend workflow rather than a frontend loop.
 */
export function MigrationsPage({
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

  const migrations = useAsync<MigrationRowView[]>(
    () => {
      if (!activeWorkspace) {
        return Promise.resolve([]);
      }
      return desktopApi.listMigrations(activeWorkspace.id);
    },
    [activeWorkspace?.id, terminalRevision],
    { enabled: connected && activeWorkspace !== null },
  );

  const [search, setSearch] = useState('');
  const [selectedIds, setSelectedIds] = useState<ReadonlySet<string>>(new Set());
  const [detailId, setDetailId] = useState<string | null>(null);

  const [snapshotView, setSnapshotView] = useState<MigrationStatusView | null>(null);
  const [snapshotLoading, setSnapshotLoading] = useState(false);
  const [snapshotError, setSnapshotError] = useState<string | null>(null);

  const [submitted, setSubmitted] = useState<SubmittedMigration | null>(null);
  const [failure, setFailure] = useState<{
    title: string;
    error: DesktopError;
    labels: string[];
  } | null>(null);
  const [applyTarget, setApplyTarget] = useState<MigrationStatusView | null>(null);
  const [bulkApply, setBulkApply] = useState(false);
  const [applying, setApplying] = useState(false);
  const [preview, setPreview] = useState<PreviewState | null>(null);
  const previewSequence = useRef(0);

  const rows = migrations.data ?? [];
  const filtered = useMemo(() => filterRows(rows, search), [rows, search]);
  const visibleIds = useMemo(() => filtered.map((row) => row.status.service_id), [filtered]);
  const busy = useMemo(() => busyServiceIds(tasks), [tasks]);

  const selectedRows = useMemo(
    () =>
      rows
        .filter((row) => selectedIds.has(row.status.service_id))
        .sort((a, b) => a.status.service_id.localeCompare(b.status.service_id)),
    [rows, selectedIds],
  );
  const selectedCount = selectedRows.length;
  const allVisible =
    visibleIds.length > 0 && visibleIds.every((id) => selectedIds.has(id));
  const someVisible =
    visibleIds.some((id) => selectedIds.has(id)) && !allVisible;

  // Everything on this screen is scoped to the active workspace.
  useEffect(() => {
    setSearch('');
    setSelectedIds(new Set());
    setDetailId(null);
    setSnapshotView(null);
    setSnapshotError(null);
    setSubmitted(null);
    setFailure(null);
    setApplyTarget(null);
    setBulkApply(false);
    setPreview(null);
    previewSequence.current += 1;
  }, [activeWorkspaceId]);

  // Cross-screen navigation can open a specific service's detail.
  useEffect(() => {
    if (intent?.serviceId) {
      setDetailId(intent.serviceId);
    }
  }, [intent]);

  const detailRow = useMemo(
    () => rows.find((row) => row.status.service_id === detailId) ?? null,
    [rows, detailId],
  );
  const detailStatus = snapshotView ?? detailRow?.status ?? null;

  // Refresh the snapshot of the open service; stale replies are discarded.
  useEffect(() => {
    if (!connected || !detailId) {
      setSnapshotView(null);
      setSnapshotError(null);
      setSnapshotLoading(false);
      return undefined;
    }
    let cancelled = false;
    setSnapshotLoading(true);
    setSnapshotError(null);
    desktopApi.getMigrationStatus(detailId).then(
      (view) => {
        if (!cancelled) {
          setSnapshotView(view);
          setSnapshotLoading(false);
        }
      },
      (cause: unknown) => {
        if (!cancelled) {
          setSnapshotError(toDesktopError(cause).message);
          setSnapshotLoading(false);
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, [detailId, connected, terminalRevision]);

  const runSingle = useCallback(
    async (
      action: SubmittedMigration['action'],
      outcomeOf: (serviceId: string) => Promise<OperationOutcome>,
      labels: string[],
    ) => {
      setFailure(null);
      try {
        const outcome = await outcomeOf(labels[0]);
        setSubmitted({ action, outcome, labels, at: Math.floor(Date.now() / 1000) });
        migrations.reload();
      } catch (cause) {
        setFailure({
          title: 'Unable to run the migration operation',
          error: toDesktopError(cause),
          labels,
        });
      }
    },
    [migrations.reload],
  );

  const runPreview = useCallback(async (serviceId: string) => {
    previewSequence.current += 1;
    const sequence = previewSequence.current;
    setPreview({ serviceId, loading: true, sql: '', error: null });
    const fail = (error: DesktopError) => {
      if (previewSequence.current === sequence) {
        setPreview({ serviceId, loading: false, sql: '', error });
      }
    };
    try {
      const outcome = await desktopApi.previewMigration(serviceId);
      if (previewSequence.current !== sequence) {
        return;
      }
      if (outcome.status !== 'task' || !outcome.task_id) {
        fail({
          code: 'PREVIEW_NOT_SCHEDULED',
          message: 'The daemon scheduled no preview task, so there is no SQL to show.',
        });
        return;
      }
      const task = await waitForTask(outcome.task_id);
      if (previewSequence.current !== sequence) {
        return;
      }
      if (!task) {
        fail({
          code: 'PREVIEW_TIMEOUT',
          message: 'The preview task did not finish in time; nothing was executed.',
        });
        return;
      }
      if (task.status !== 'succeeded') {
        fail({
          code: task.failure?.code ?? 'PREVIEW_FAILED',
          message:
            task.failure?.message ??
            `The preview task settled as ${task.status}; no SQL was produced.`,
        });
        return;
      }
      const page = await desktopApi.getTaskLogs(outcome.task_id, { limit: 500 });
      if (previewSequence.current !== sequence) {
        return;
      }
      const sql = extractSql(page.entries);
      if (sql.trim().length === 0) {
        fail({
          code: 'PREVIEW_EMPTY',
          message: 'The preview task recorded no SQL for this target.',
        });
        return;
      }
      setPreview({ serviceId, loading: false, sql, error: null });
    } catch (cause) {
      fail(toDesktopError(cause));
    }
  }, []);

  const confirmApply = useCallback(async () => {
    const target = applyTarget;
    if (!target) {
      return;
    }
    setApplyTarget(null);
    setApplying(true);
    setFailure(null);
    try {
      // The confirmation is intent, not authorization: the daemon re-verifies
      // the client kind and the peer user before allowing a protected apply.
      const outcome = await desktopApi.applyMigration(target.service_id, true);
      setSubmitted({
        action: 'apply',
        outcome,
        labels: [target.service_id],
        at: Math.floor(Date.now() / 1000),
      });
      migrations.reload();
    } catch (cause) {
      setFailure({
        title: 'Unable to apply the migration',
        error: toDesktopError(cause),
        labels: [target.service_id],
      });
    } finally {
      setApplying(false);
    }
  }, [applyTarget, migrations.reload]);

  const confirmBulkApply = useCallback(async () => {
    const targets = selectedRows.map((row) => row.status.service_id);
    setBulkApply(false);
    if (!activeWorkspaceId || targets.length === 0) {
      return;
    }
    setApplying(true);
    setFailure(null);
    try {
      const outcome = await desktopApi.applySelectedMigrations(
        activeWorkspaceId,
        targets,
        true,
      );
      setSubmitted({ action: 'applySelected', outcome, labels: targets, at: Math.floor(Date.now() / 1000) });
      setSelectedIds(new Set());
      migrations.reload();
    } catch (cause) {
      setFailure({
        title: 'Unable to apply the selected migrations',
        error: toDesktopError(cause),
        labels: targets,
      });
    } finally {
      setApplying(false);
    }
  }, [activeWorkspaceId, selectedRows, migrations.reload]);

  const authorizationHint =
    failure && failure.error.code === 'PermissionDenied'
      ? 'The daemon refused this protected operation because it could not verify an authorized interactive request. The confirmation dialog alone is never treated as authorization: the daemon checks the client kind and the peer user on every request. Reconnect through the desktop application and confirm again; never retry a failed migration automatically.'
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
        description="Configure a workspace in ~/.service-orchestrator/config.toml to see migration targets."
      />
    );
  }

  return (
    <div className="page">
      <div className="toolbar" role="search">
        <label className="toolbar__field">
          <span className="visually-hidden">Search migrations</span>
          <input
            type="search"
            className="input"
            placeholder="Search by service, provider, environment, or target…"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            aria-label="Search migrations"
          />
        </label>
        <span className="toolbar__count" aria-live="polite">
          {pluralize(filtered.length, 'target')}
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
          Disconnected from the daemon. Migration actions are disabled until the
          connection returns; loaded state is kept and nothing is replayed.
        </div>
      ) : null}

      <div className="bulk-bar" role="toolbar" aria-label="Migration selection actions">
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
            onClick={() => setSelectedIds(new Set(visibleIds))}
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
            onClick={() => setBulkApply(true)}
            disabled={!connected || selectedCount === 0}
          >
            Apply selected…
          </button>
        </div>
        {selectedCount > 0 ? (
          <span className="bulk-bar__hint muted" role="note">
            Bulk apply runs as one coordinated backend workflow; the daemon owns
            database locking and ordering.
          </span>
        ) : null}
      </div>

      <div className="card card--table">
        {migrations.loading ? <LoadingState label="Loading migration targets…" /> : null}
        {migrations.error ? (
          <ErrorState error={migrations.error} onRetry={migrations.reload} />
        ) : null}
        {!migrations.loading && !migrations.error ? (
          migrations.data === null ? (
            <LoadingState
              label={connected ? 'Loading migration targets…' : 'Waiting for the daemon connection…'}
            />
          ) : rows.length === 0 ? (
            <EmptyState
              title="No migration-enabled services"
              description={`The workspace “${activeWorkspace.name}” configures no migration targets.`}
            />
          ) : filtered.length === 0 ? (
            <EmptyState
              title="No migration targets match your search"
              description="Adjust the search text."
              action={
                <button type="button" className="button" onClick={() => setSearch('')}>
                  Clear filters
                </button>
              }
            />
          ) : (
            <MigrationTable
              rows={filtered}
              workspaceName={activeWorkspace.name}
              selectedIds={selectedIds}
              onToggleSelect={(serviceId, selected) =>
                setSelectedIds((current) => {
                  const next = new Set(current);
                  if (selected) {
                    next.add(serviceId);
                  } else {
                    next.delete(serviceId);
                  }
                  return next;
                })
              }
              onToggleSelectAll={(selected) =>
                setSelectedIds(selected ? new Set(visibleIds) : new Set())
              }
              allVisibleSelected={allVisible}
              someVisibleSelected={someVisible}
              detailId={detailId}
              onOpenDetail={setDetailId}
              busy={busy}
              disabled={!connected}
            />
          )
        ) : null}
      </div>

      {detailRow && detailStatus ? (
        <MigrationDetailPanel
          row={detailRow}
          status={detailStatus}
          loading={snapshotLoading}
          loadError={snapshotError}
          busy={busy.has(detailRow.status.service_id)}
          disabled={!connected}
          onClose={() => {
            setDetailId(null);
            setSnapshotView(null);
            setSnapshotError(null);
          }}
          onValidate={() =>
            void runSingle(
              'validate',
              (serviceId) => desktopApi.validateMigration(serviceId),
              [detailRow.status.service_id],
            )
          }
          onInspectStatus={() =>
            void runSingle(
              'inspect',
              (serviceId) => desktopApi.inspectMigrationStatus(serviceId),
              [detailRow.status.service_id],
            )
          }
          onInspectPending={() =>
            void runSingle(
              'pending',
              (serviceId) => desktopApi.inspectMigrationPending(serviceId),
              [detailRow.status.service_id],
            )
          }
          onPreview={() => void runPreview(detailRow.status.service_id)}
          onApply={() => setApplyTarget(detailStatus)}
          onOpenTask={(taskId) => onNavigate?.('tasks', { taskId })}
          onViewLogs={() => onNavigate?.('logs', { serviceId: detailRow.status.service_id })}
        />
      ) : null}

      {applyTarget ? (
        <ConfirmationDialog
          title={`Apply migrations for ${applyTarget.service_id}`}
          message={
            <>
              Target <strong>{applyTarget.environment}</strong> (
              {applyTarget.database_identity}) through {applyTarget.provider} ·{' '}
              {applyTarget.execution_mode}. This can modify the database schema.
              Nothing runs automatically, nothing retries on failure, and
              cancelling a running task never claims to roll back database
              changes.
            </>
          }
          confirmLabel="Apply migration"
          tone="danger"
          busy={applying}
          onConfirm={() => void confirmApply()}
          onCancel={() => setApplyTarget(null)}
        >
          <p className="dialog__note">
            Your confirmation is forwarded as intent. The daemon independently
            verifies the client and the requesting user, and its decision is
            final.
          </p>
        </ConfirmationDialog>
      ) : null}

      {bulkApply ? (
        <ConfirmationDialog
          title={`Apply migrations for ${selectedCount} service${selectedCount === 1 ? '' : 's'}`}
          message={
            <>
              {selectedRows
                .map((row) => `${row.status.service_id} → ${row.status.environment}`)
                .join(', ')}
              . The daemon runs one coordinated workflow so database locking and
              ordering stay with the backend.
            </>
          }
          confirmLabel="Apply selected"
          tone="danger"
          busy={applying}
          onConfirm={() => void confirmBulkApply()}
          onCancel={() => setBulkApply(false)}
        >
          <p className="dialog__note">
            Non-local targets require the daemon&rsquo;s own authorization; a
            frontend confirmation is never sufficient on its own.
          </p>
        </ConfirmationDialog>
      ) : null}

      {preview ? (
        <SqlPreviewDialog
          title={`SQL preview — ${preview.serviceId}`}
          sql={preview.sql}
          loading={preview.loading}
          error={preview.error}
          onClose={() => {
            previewSequence.current += 1;
            setPreview(null);
          }}
        />
      ) : null}

      {submitted ? (
        <OperationProgressDialog
          title="Migration operation"
          outcome={submitted.outcome}
          tasks={tasks}
          at={submitted.at}
          targetLabel="services"
          onCancelTask={(taskId) => void desktopApi.cancelTask(taskId)}
          onCancelWorkflow={(workflowId) => void desktopApi.cancelWorkflow(workflowId)}
          onClose={() => setSubmitted(null)}
        />
      ) : null}

      {failure ? (
        <ErrorDialog
          title={failure.title}
          error={failure.error}
          labels={failure.labels}
          hint={authorizationHint ?? undefined}
          onClose={() => setFailure(null)}
        />
      ) : null}
    </div>
  );
}
