import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { BulkImportReview } from '../components/bulk-import/BulkImportReview';
import { BulkImportWizard } from '../components/bulk-import/BulkImportWizard';
import { BulkOverridesForm } from '../components/bulk-import/BulkOverridesForm';
import { DetectionReview } from '../components/bulk-import/DetectionReview';
import {
  RepositoryOverridesTable,
} from '../components/bulk-import/RepositoryOverridesTable';
import { RepositorySelection } from '../components/bulk-import/RepositorySelection';
import { TemplateAssignment } from '../components/bulk-import/TemplateAssignment';
import {
  buildImportRequest,
  draftProblems,
  emptyBulkOverrides,
  newRowDraft,
  parsePortBase,
  requestKey,
  type BulkOverridesDraft,
  type ImportRowDraft,
} from '../components/bulk-import/draft';
import { ConfirmationDialog } from '../components/ConfirmationDialog';
import { ErrorDialog } from '../components/ErrorDialog';
import { EmptyState, ErrorState, LoadingState } from '../components/StateViews';
import { useConnection } from '../context/ConnectionContext';
import { useWorkspaces } from '../context/WorkspaceContext';
import { useAsync } from '../hooks/useAsync';
import { desktopApi } from '../services/desktopApi';
import { toDesktopError } from '../services/errors';
import type {
  BulkImportCommitView,
  BulkImportPlanView,
  ConfigurationSnapshotView,
  DesktopError,
  MigrationPresetSummaryView,
  RepositoryDiscoveryReportView,
} from '../types/models';
import type { NavIntent, PageId } from '../types/nav';

/** The summary card shown after a scan (step 2). */
function DiscoverySummary({
  report,
  onContinue,
}: {
  report: RepositoryDiscoveryReportView;
  onContinue: () => void;
}) {
  const withSuggestion = report.repositories.filter(
    (entry) => entry.suggested_template !== undefined,
  ).length;
  const existing = report.repositories.filter(
    (entry) => (entry.existing ?? []).length > 0,
  ).length;
  return (
    <div className="card">
      <div className="card__header">
        <div>
          <h2 className="card__title">Discovery complete</h2>
          <p className="muted">
            The scan read project files only. Nothing was built, cloned, started, or written.
          </p>
        </div>
        <button type="button" className="button button--primary" onClick={onContinue}>
          Select repositories
        </button>
      </div>
      <dl className="detail-grid">
        <div>
          <dt>Workspace root</dt>
          <dd>{report.root}</dd>
        </div>
        <div>
          <dt>Repositories found</dt>
          <dd>{report.repositories.length}</dd>
        </div>
        <div>
          <dt>With a unique template suggestion</dt>
          <dd>{withSuggestion}</dd>
        </div>
        <div>
          <dt>Already configured</dt>
          <dd>{existing}</dd>
        </div>
        <div>
          <dt>Repository cap reached</dt>
          <dd>{report.truncated ? 'yes — the list is incomplete' : 'no'}</dd>
        </div>
      </dl>
      {report.truncated ? (
        <p className="config-validation config-validation--error" role="alert">
          The configured repository cap stopped the scan. Narrow the workspace or raise the
          cap in the discovery configuration before importing.
        </p>
      ) : null}
      {(report.warnings ?? []).length > 0 ? (
        <div className="config-validation" role="status">
          <ul>
            {(report.warnings ?? []).map((warning) => (
              <li key={warning}>{warning}</li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}

/** The result card after a committed import (step 10). */
function ImportResult({
  commit,
  onReset,
  onNavigate,
}: {
  commit: BulkImportCommitView;
  onReset: () => void;
  onNavigate?: (page: PageId, intent?: NavIntent) => void;
}) {
  return (
    <div className="card">
      <div className="card__header">
        <div>
          <h2 className="card__title">Import committed</h2>
          <p className="muted">
            Configuration revision {commit.revision}. The daemon validated the whole file and
            wrote it atomically; the TUI and MCP clients observe the same core.
          </p>
        </div>
        <span className="badge badge--positive">committed</span>
      </div>
      <dl className="detail-grid">
        <div>
          <dt>Created services</dt>
          <dd>{commit.created_services.join(', ') || '—'}</dd>
        </div>
        <div>
          <dt>Created libraries</dt>
          <dd>{commit.created_libraries.join(', ') || '—'}</dd>
        </div>
        <div>
          <dt>Replaced</dt>
          <dd>
            {commit.updated.map((entity) => `${entity.kind} ${entity.id}`).join(', ') || '—'}
          </dd>
        </div>
        <div>
          <dt>Skipped</dt>
          <dd>
            {commit.skipped.map((entity) => `${entity.kind} ${entity.id}`).join(', ') || '—'}
          </dd>
        </div>
        <div>
          <dt>Configuration totals</dt>
          <dd>
            {commit.counts.workspaces} workspaces · {commit.counts.services} services ·{' '}
            {commit.counts.libraries} libraries
          </dd>
        </div>
      </dl>
      <p className="muted">
        Nothing was built, started, or migrated. Use Bulk Operations to prepare the imported
        services.
      </p>
      <div className="detail-actions">
        {onNavigate ? (
          <button
            type="button"
            className="button button--primary"
            onClick={() => onNavigate('services')}
          >
            Open services
          </button>
        ) : null}
        <button type="button" className="button" onClick={onReset}>
          Start another import
        </button>
      </div>
    </div>
  );
}

/**
 * Template-based bulk service import (`tasks/032.md` §7–§15).
 *
 * The ten-step wizard keeps the safe order explicit: discover, review, select,
 * review detections, assign templates, apply overrides, validate, preview,
 * confirm, and only then commit. Discovery and planning are read-only daemon
 * queries; the commit sends the reviewed plan's revision, validates the whole
 * document atomically in the application core, and re-checks policy — this
 * screen never writes the configuration file itself (`tasks/032.md` §10, §15).
 */
export function BulkImportPage({
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

  const [step, setStep] = useState(1);
  const [report, setReport] = useState<RepositoryDiscoveryReportView | null>(null);
  const [rows, setRows] = useState<Record<string, ImportRowDraft>>({});
  const [bulk, setBulk] = useState<BulkOverridesDraft>(emptyBulkOverrides);
  const [portBase, setPortBase] = useState('8080');
  const [search, setSearch] = useState('');
  const [category, setCategory] = useState('all');

  const [scanning, setScanning] = useState(false);
  const [scanError, setScanError] = useState<DesktopError | null>(null);

  const [plan, setPlan] = useState<BulkImportPlanView | null>(null);
  const [plannedKey, setPlannedKey] = useState<string | null>(null);
  const [planning, setPlanning] = useState(false);
  const [planError, setPlanError] = useState<DesktopError | null>(null);

  const [confirmImport, setConfirmImport] = useState(false);
  const [committing, setCommitting] = useState(false);
  const [commit, setCommit] = useState<BulkImportCommitView | null>(null);
  const [failure, setFailure] = useState<{ title: string; error: DesktopError } | null>(null);

  const configuration = useAsync<ConfigurationSnapshotView>(
    () => desktopApi.getConfiguration(),
    [connected, activeWorkspace?.id],
    { enabled: connected },
  );
  const presets = useAsync<MigrationPresetSummaryView[]>(
    () => desktopApi.listMigrationPresets(),
    [connected],
    { enabled: connected },
  );

  const templates = configuration.data?.templates ?? [];
  const discovery = configuration.data?.discovery;

  const reset = useCallback(() => {
    setStep(1);
    setReport(null);
    setRows({});
    setBulk(emptyBulkOverrides());
    setPortBase('8080');
    setSearch('');
    setCategory('all');
    setScanError(null);
    setPlan(null);
    setPlannedKey(null);
    setPlanError(null);
    setConfirmImport(false);
    setCommit(null);
    setFailure(null);
  }, []);

  const workspaceRef = useRef(activeWorkspaceId);
  useEffect(() => {
    if (workspaceRef.current === activeWorkspaceId) {
      return;
    }
    workspaceRef.current = activeWorkspaceId;
    reset();
  }, [activeWorkspaceId, reset]);

  const drafts = useMemo(() => Object.values(rows), [rows]);
  const selectedEntries = useMemo(
    () =>
      (report?.repositories ?? []).filter(
        (entry) => rows[entry.repository.relative_path]?.selected === true,
      ),
    [report, rows],
  );
  const selectedDrafts = useMemo(
    () => drafts.filter((draft) => draft.selected),
    [drafts],
  );
  const problems = useMemo(() => draftProblems(drafts, portBase), [drafts, portBase]);
  const { portBase: portBaseValue, invalid: portBaseInvalid } = parsePortBase(portBase);

  const request = useMemo(
    () =>
      activeWorkspace
        ? buildImportRequest(activeWorkspace.id, drafts, bulk, portBaseValue)
        : null,
    [activeWorkspace, drafts, bulk, portBaseValue],
  );
  const currentKey = request ? requestKey(request) : '';
  const stale = plan !== null && plannedKey !== currentKey;
  const importable = plan !== null && plan.valid && !stale && plan.creates + plan.updates > 0;

  const scan = async () => {
    if (!activeWorkspace) {
      return;
    }
    setScanning(true);
    setScanError(null);
    setCommit(null);
    setPlan(null);
    setPlannedKey(null);
    try {
      const result = await desktopApi.discoverRepositories({
        workspace_id: activeWorkspace.id,
      });
      const initial: Record<string, ImportRowDraft> = {};
      for (const entry of result.repositories) {
        initial[entry.repository.relative_path] = newRowDraft(
          entry.repository.relative_path,
        );
      }
      setReport(result);
      setRows(initial);
      setStep(2);
    } catch (cause) {
      setScanError(toDesktopError(cause));
    } finally {
      setScanning(false);
    }
  };

  const updateRow = (relativePath: string, patch: Partial<ImportRowDraft>) => {
    setRows((current) => {
      const existing = current[relativePath];
      if (!existing) {
        return current;
      }
      return { ...current, [relativePath]: { ...existing, ...patch } };
    });
  };

  const runPlan = async () => {
    if (!request) {
      return;
    }
    setPlanning(true);
    setPlanError(null);
    try {
      const result = await desktopApi.planBulkImport(request);
      setPlan(result);
      setPlannedKey(currentKey);
    } catch (cause) {
      setPlan(null);
      setPlannedKey(null);
      setPlanError(toDesktopError(cause));
    } finally {
      setPlanning(false);
    }
  };

  const goNext = async () => {
    if (step === 7) {
      setStep(8);
      await runPlan();
      return;
    }
    setStep((current) => Math.min(10, current + 1));
  };

  const commitImport = async () => {
    if (!request || !plan || stale) {
      return;
    }
    setCommitting(true);
    try {
      const result = await desktopApi.commitBulkImport(
        request,
        plan.revision,
        plan.updates > 0,
      );
      setCommit(result);
      setConfirmImport(false);
      setStep(10);
    } catch (cause) {
      setConfirmImport(false);
      setFailure({
        title: 'Unable to import the selected repositories',
        error: toDesktopError(cause),
      });
    } finally {
      setCommitting(false);
    }
  };

  if (workspacesError) {
    return <ErrorState error={workspacesError} onRetry={reloadWorkspaces} />;
  }
  if (!connected) {
    return <LoadingState label="Waiting for the daemon connection…" />;
  }
  if (workspacesLoading && !activeWorkspace) {
    return <LoadingState label="Loading workspaces…" />;
  }
  if (!activeWorkspace) {
    return (
      <EmptyState
        title="No workspace selected"
        description="Configure a workspace in ~/.service-orchestrator/config.toml to discover repositories."
      />
    );
  }

  return (
    <div className="page">
      <BulkImportWizard step={step} />

      {commit ? (
        <ImportResult commit={commit} onReset={reset} onNavigate={onNavigate} />
      ) : null}

      {!commit && step === 1 ? (
        <div className="card">
          <div className="card__header">
            <div>
              <h2 className="card__title">Bulk service import</h2>
              <p className="muted">
                Workspace: {activeWorkspace.name} · {activeWorkspace.root}
              </p>
            </div>
            <button
              type="button"
              className="button button--primary"
              onClick={() => void scan()}
              disabled={scanning}
            >
              {scanning ? 'Scanning…' : 'Scan workspace'}
            </button>
          </div>
          <p className="muted">
            Discovery walks the workspace root within the configured bounds, skips build
            output and VCS directories, and inspects each repository read-only. A repository
            is never cloned, built, or executed during discovery.
          </p>
          <dl className="detail-grid">
            <div>
              <dt>Maximum depth</dt>
              <dd>{discovery?.max_depth ?? 'configured default (6)'}</dd>
            </div>
            <div>
              <dt>Repository cap</dt>
              <dd>{discovery?.max_repositories ?? 'configured default (200)'}</dd>
            </div>
            <div>
              <dt>Follow symlinks</dt>
              <dd>
                {(discovery?.follow_symlinks ?? true)
                  ? 'yes, inside the workspace root only'
                  : 'no'}
              </dd>
            </div>
            <div>
              <dt>Extra exclusions</dt>
              <dd>{(discovery?.exclude ?? []).join(', ') || 'none'}</dd>
            </div>
          </dl>
          {scanError ? <ErrorState error={scanError} onRetry={() => void scan()} /> : null}
        </div>
      ) : null}

      {!commit && step === 2 && report ? (
        <DiscoverySummary report={report} onContinue={() => setStep(3)} />
      ) : null}

      {!commit && step === 2 && !report ? (
        <EmptyState
          title="No discovery result"
          description="Scan the workspace to see its repositories."
        />
      ) : null}

      {!commit && step === 3 && report ? (
        <div className="card">
          <RepositorySelection
            repositories={report.repositories}
            rows={rows}
            search={search}
            category={category}
            onSearch={setSearch}
            onCategory={setCategory}
            onToggle={(path, selected) => updateRow(path, { selected })}
            onSelectAll={(selected) =>
              setRows((current) =>
                Object.fromEntries(
                  Object.entries(current).map(([path, row]) => [
                    path,
                    { ...row, selected },
                  ]),
                ),
              )
            }
            onSelectVisible={(selected, paths) => {
              setRows((current) => {
                const next = { ...current };
                for (const path of paths) {
                  if (next[path]) {
                    next[path] = { ...next[path], selected };
                  }
                }
                return next;
              });
            }}
          />
          {selectedDrafts.length === 0 ? (
            <p className="muted" role="note">
              Select at least one repository to continue. Nothing is imported automatically.
            </p>
          ) : null}
        </div>
      ) : null}

      {!commit && step === 4 ? (
        <div className="card">
          <DetectionReview repositories={selectedEntries} />
        </div>
      ) : null}

      {!commit && step === 5 ? (
        <div className="card">
          {configuration.loading && templates.length === 0 ? (
            <LoadingState label="Loading templates…" />
          ) : null}
          {configuration.error ? (
            <ErrorState error={configuration.error} onRetry={configuration.reload} />
          ) : null}
          {templates.length === 0 && !configuration.loading ? (
            <div className="config-validation" role="status">
              <p>
                No templates are configured, so only repositories whose detections identify a
                type can be imported. Create a template first.
              </p>
              {onNavigate ? (
                <button
                  type="button"
                  className="button button--small"
                  onClick={() => onNavigate('templates')}
                >
                  Open Templates
                </button>
              ) : null}
            </div>
          ) : null}
          <TemplateAssignment
            repositories={selectedEntries}
            rows={rows}
            templates={templates}
            onTemplate={(path, template) => updateRow(path, { template })}
          />
        </div>
      ) : null}

      {!commit && step === 6 ? (
        <div className="card">
          {presets.error ? (
            <ErrorState error={presets.error} onRetry={presets.reload} />
          ) : null}
          <BulkOverridesForm
            value={bulk}
            presets={presets.data ?? []}
            portBase={portBase}
            onChange={setBulk}
            onPortBase={setPortBase}
          />
          {problems.length > 0 ? (
            <div className="config-validation config-validation--error" role="alert">
              <ul>
                {problems.map((problem) => (
                  <li key={problem}>{problem}</li>
                ))}
              </ul>
            </div>
          ) : null}
        </div>
      ) : null}

      {!commit && step === 7 ? (
        <div className="card">
          <RepositoryOverridesTable
            rows={selectedDrafts}
            repositories={report?.repositories ?? []}
            presets={presets.data ?? []}
            onChange={updateRow}
          />
          {problems.length > 0 ? (
            <div className="config-validation config-validation--error" role="alert">
              <ul>
                {problems.map((problem) => (
                  <li key={problem}>{problem}</li>
                ))}
              </ul>
            </div>
          ) : null}
        </div>
      ) : null}

      {!commit && (step === 8 || step === 9) ? (
        <div className="card">
          {planning ? <LoadingState label="Validating the proposed configuration…" /> : null}
          {planError ? (
            <ErrorState
              error={planError}
              onRetry={() => void runPlan()}
            />
          ) : null}
          {stale && !planning ? (
            <p className="page__notice" role="alert">
              The selection or overrides changed after validation. Re-validate before
              importing; the previous plan is no longer current.
            </p>
          ) : null}
          {plan && !planning ? (
            <>
              <BulkImportReview plan={plan} mode={step === 9 ? 'preview' : 'validate'} />
              {!plan.valid ? (
                <p className="config-validation config-validation--error" role="alert">
                  Resolve the reported problems before importing. No partial import is
                  possible: the whole document must validate.
                </p>
              ) : null}
              {plan.valid && plan.creates + plan.updates === 0 ? (
                <p className="muted" role="status">
                  Every selected repository is skipped, so there is nothing to import.
                </p>
              ) : null}
            </>
          ) : null}
          {!plan && !planning && !planError ? (
            <div className="detail-actions">
              <button
                type="button"
                className="button button--primary"
                onClick={() => void runPlan()}
                disabled={problems.length > 0 || selectedDrafts.length === 0}
              >
                Validate import
              </button>
            </div>
          ) : null}
        </div>
      ) : null}

      {!commit ? (
        <div className="import-nav">
          <button
            type="button"
            className="button"
            onClick={() => setStep((current) => Math.max(1, current - 1))}
            disabled={step === 1 || planning || committing}
          >
            Back
          </button>
          <span className="muted" aria-live="polite">
            Step {step} of 10
          </span>
          {step === 1 ? (
            <span className="muted">Scan the workspace to begin.</span>
          ) : step === 7 ? (
            <button
              type="button"
              className="button button--primary"
              onClick={() => void goNext()}
              disabled={
                selectedDrafts.length === 0 || problems.length > 0 || planning
              }
            >
              {planning ? 'Validating…' : 'Validate import'}
            </button>
          ) : step === 8 ? (
            <button
              type="button"
              className="button button--primary"
              onClick={() => setStep(9)}
              disabled={!importable}
            >
              Preview changes
            </button>
          ) : step === 9 ? (
            <button
              type="button"
              className="button button--primary"
              onClick={() => setConfirmImport(true)}
              disabled={!importable || committing}
            >
              Import{' '}
              {plan
                ? `${plan.creates + plan.updates} ${
                    plan.creates + plan.updates === 1 ? 'entity' : 'entities'
                  }…`
                : '…'}
            </button>
          ) : (
            <button
              type="button"
              className="button button--primary"
              onClick={() => void goNext()}
              disabled={
                (step === 3 && selectedDrafts.length === 0) ||
                (portBaseInvalid && step >= 7)
              }
            >
              Next
            </button>
          )}
        </div>
      ) : null}

      {confirmImport && plan ? (
        <ConfirmationDialog
          title={
            plan.updates > 0
              ? 'Replace existing configuration?'
              : 'Import the selected repositories?'
          }
          tone={plan.updates > 0 ? 'danger' : 'default'}
          confirmLabel={plan.updates > 0 ? 'Replace and import' : 'Import'}
          busy={committing}
          message={
            <>
              <p>
                This creates {plan.creates} {plan.creates === 1 ? 'entity' : 'entities'} and
                replaces {plan.updates}. {plan.skips} will be skipped.
              </p>
              {plan.updates > 0 ? (
                <p>
                  Replacing overwrites hand-written configuration. The daemon independently
                  verifies an interactive same-user request; this confirmation is not
                  authorization.
                </p>
              ) : null}
              <p>
                The whole proposed file is validated and written atomically. Nothing is built,
                started, or migrated.
              </p>
            </>
          }
          onConfirm={() => void commitImport()}
          onCancel={() => setConfirmImport(false)}
        />
      ) : null}

      {failure ? (
        <ErrorDialog
          title={failure.title}
          error={failure.error}
          labels={selectedDrafts.map((row) => row.relativePath)}
          onClose={() => setFailure(null)}
        />
      ) : null}
    </div>
  );
}
