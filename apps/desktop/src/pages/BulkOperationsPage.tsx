import { useEffect, useMemo, useState } from 'react';

import { PresetEditor } from '../components/bulk-operations/PresetEditor';
import { OperationOptions } from '../components/bulk-operations/OperationOptions';
import { ServicePicker } from '../components/bulk-operations/ServicePicker';
import {
  defaultOperationOptions,
  optionsFromRequest,
  selectionFromRequest,
  toOperationPreset,
  toOperationRequest,
  type OperationOptionsValue,
  type OperationSelectionValue,
} from '../components/bulk-operations/model';
import { ConfirmationDialog } from '../components/ConfirmationDialog';
import { ErrorDialog } from '../components/ErrorDialog';
import { EmptyState, ErrorState, LoadingState } from '../components/StateViews';
import { GroupSelector } from '../components/workflow/GroupSelector';
import { ProfileSelector } from '../components/workflow/ProfileSelector';
import { WorkflowCancellationDialog } from '../components/workflow/WorkflowCancellationDialog';
import { WorkflowPreview } from '../components/workflow/WorkflowPreview';
import { WorkflowProgress } from '../components/workflow/WorkflowProgress';
import { WorkflowResults } from '../components/workflow/WorkflowResults';
import { WorkflowWizard, type WorkflowMode } from '../components/workflow/WorkflowWizard';
import { SaveConfirmationDialog } from '../components/configuration/SaveConfirmationDialog';
import { useConnection } from '../context/ConnectionContext';
import { useTaskRevision } from '../context/TaskEventsContext';
import { useWorkspaces } from '../context/WorkspaceContext';
import { useAsync } from '../hooks/useAsync';
import { useConfigurationReview } from '../hooks/useConfigurationReview';
import { useWorkflowRun } from '../hooks/useWorkflowRun';
import { desktopApi } from '../services/desktopApi';
import { toDesktopError } from '../services/errors';
import type {
  ConfigurationSnapshotView,
  DesktopError,
  EnvironmentPlanView,
  GroupView,
  MigrationPresetSummaryView,
  OperationPresetSummaryView,
  PrepareEnvironmentInput,
  ProfileView,
  RawOperationPresetView,
  ServiceSummary,
} from '../types/models';
import type { NavIntent, PageId } from '../types/nav';

/** Which single source the selection comes from (exactly one, always). */
type SelectionSource = 'services' | 'profile' | 'group';

/** The editor state for a saved preset. */
type PresetEditorState =
  | { mode: 'create' }
  | { mode: 'rename'; preset: RawOperationPresetView };

/**
 * The single-screen Bulk Operations dashboard (`tasks/032.md` §22–§38).
 *
 * Selection, Git, libraries, migrations, build, runtime, health, preview,
 * progress, and results live on one screen. Everything delegates to the shared
 * application core: planning calls the read-only `plan_prepare_environment`,
 * execution calls `prepare_environment` and follows the daemon's real workflow
 * id. Saved presets persist through the existing configuration manager, store
 * only the selection and flags, and never execute or authorize anything on
 * load.
 */
export function BulkOperationsPage({
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
  const terminalRevision = useTaskRevision();

  const [mode, setMode] = useState<WorkflowMode>('configure');
  const [source, setSource] = useState<SelectionSource>('services');
  const [selectedIds, setSelectedIds] = useState<ReadonlySet<string>>(new Set());
  const [profileId, setProfileId] = useState<string | null>(null);
  const [groupId, setGroupId] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const [kind, setKind] = useState('all');
  const [options, setOptions] = useState<OperationOptionsValue>(defaultOperationOptions());

  const [presetId, setPresetId] = useState('');
  const [loadedPresetId, setLoadedPresetId] = useState<string | null>(null);
  const [presetRevision, setPresetRevision] = useState(0);
  const [presetEditor, setPresetEditor] = useState<PresetEditorState | null>(null);
  const [confirmDeletePreset, setConfirmDeletePreset] = useState(false);
  const [confirmDefault, setConfirmDefault] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const [plan, setPlan] = useState<EnvironmentPlanView | null>(null);
  const [planning, setPlanning] = useState(false);
  const [planError, setPlanError] = useState<DesktopError | null>(null);
  const [plannedAt, setPlannedAt] = useState<number | null>(null);

  const [confirmExecute, setConfirmExecute] = useState(false);
  const [confirmCancel, setConfirmCancel] = useState(false);
  const [failure, setFailure] = useState<{ title: string; error: DesktopError } | null>(null);

  const run = useWorkflowRun();

  const services = useAsync<ServiceSummary[]>(
    () =>
      activeWorkspace
        ? desktopApi.listServices(activeWorkspace.id)
        : Promise.resolve([]),
    [activeWorkspace?.id, terminalRevision],
    { enabled: connected && activeWorkspace !== null },
  );
  const profiles = useAsync<ProfileView[]>(
    () =>
      activeWorkspace
        ? desktopApi.listProfiles(activeWorkspace.id)
        : Promise.resolve([]),
    [activeWorkspace?.id],
    { enabled: connected && activeWorkspace !== null },
  );
  const groups = useAsync<GroupView[]>(
    () =>
      activeWorkspace ? desktopApi.listGroups(activeWorkspace.id) : Promise.resolve([]),
    [activeWorkspace?.id],
    { enabled: connected && activeWorkspace !== null },
  );
  const migrationPresets = useAsync<MigrationPresetSummaryView[]>(
    () => desktopApi.listMigrationPresets(),
    [connected, presetRevision],
    { enabled: connected },
  );
  const operationPresets = useAsync<OperationPresetSummaryView[]>(
    () =>
      activeWorkspace
        ? desktopApi.listOperationPresets(activeWorkspace.id)
        : Promise.resolve([]),
    [activeWorkspace?.id, presetRevision],
    { enabled: connected && activeWorkspace !== null },
  );
  const configuration = useAsync<ConfigurationSnapshotView>(
    () => desktopApi.getConfiguration(),
    [connected, presetRevision],
    { enabled: connected },
  );

  const review = useConfigurationReview({
    onCommitted: (commit) => {
      setNotice(`Saved configuration revision ${commit.revision}.`);
      setPresetRevision((revision) => revision + 1);
    },
    onRefresh: () => setPresetRevision((revision) => revision + 1),
  });

  const presets = operationPresets.data ?? [];
  const loadedPreset = presets.find((preset) => preset.id === loadedPresetId);
  const rawPreset = (configuration.data?.operation_presets ?? []).find(
    (preset) => preset.id === presetId,
  );
  const defaultPreset = presets.find((preset) => preset.default);
  const destructivePreset = (migrationPresets.data ?? []).find(
    (preset) => preset.id === options.migrationPreset && preset.destructive,
  );

  const selection = useMemo<OperationSelectionValue>(() => {
    if (source === 'profile') {
      return { serviceIds: [], profileId, groupId: null };
    }
    if (source === 'group') {
      return { serviceIds: [], profileId: null, groupId };
    }
    return { serviceIds: [...selectedIds].sort(), profileId: null, groupId: null };
  }, [source, selectedIds, profileId, groupId]);

  const hasSelection =
    selection.serviceIds.length > 0 ||
    selection.profileId !== null ||
    selection.groupId !== null;

  const request = useMemo<PrepareEnvironmentInput | null>(
    () =>
      activeWorkspace
        ? toOperationRequest(activeWorkspace.id, selection, options)
        : null,
    [activeWorkspace, selection, options],
  );

  // A workspace change scopes everything: no selection, preset, plan, or
  // workflow state bleeds into another workspace.
  useEffect(() => {
    setMode('configure');
    setSource('services');
    setSelectedIds(new Set());
    setProfileId(null);
    setGroupId(null);
    setOptions(defaultOperationOptions());
    setPresetId('');
    setLoadedPresetId(null);
    setPlan(null);
    setPlanError(null);
    setPlannedAt(null);
    setNotice(null);
    run.reset();
    // `run.reset` is stable (a useCallback with no dependencies).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeWorkspaceId]);

  // The daemon declares the workflow terminal; the screen only reflects it.
  useEffect(() => {
    if (mode === 'running' && run.terminal) {
      setMode('results');
    }
  }, [mode, run.terminal]);

  const applyRequest = (next: PrepareEnvironmentInput) => {
    const nextSelection = selectionFromRequest(next);
    setSource(
      nextSelection.profileId
        ? 'profile'
        : nextSelection.groupId
          ? 'group'
          : 'services',
    );
    setSelectedIds(new Set(nextSelection.serviceIds));
    setProfileId(nextSelection.profileId);
    setGroupId(nextSelection.groupId);
    setOptions(optionsFromRequest(next));
    setSearch('');
    setKind('all');
  };

  const loadPreset = async (id: string) => {
    setPresetId(id);
    if (id === '') {
      setLoadedPresetId(null);
      return;
    }
    if (!activeWorkspace) {
      return;
    }
    setNotice(null);
    try {
      const next = await desktopApi.getOperationPresetRequest(activeWorkspace.id, id);
      applyRequest(next);
      setLoadedPresetId(id);
      setPlan(null);
      setPlannedAt(null);
      const preset = presets.find((candidate) => candidate.id === id);
      setNotice(
        `Loaded “${preset?.name ?? id}”. Loading a preset executes nothing and carries no authorization.`,
      );
      setMode('configure');
    } catch (cause) {
      setFailure({ title: 'Unable to load the preset', error: toDesktopError(cause) });
    }
  };

  const clearPreset = () => {
    setPresetId('');
    setLoadedPresetId(null);
    setSource('services');
    setSelectedIds(new Set());
    setProfileId(null);
    setGroupId(null);
    setOptions(defaultOperationOptions());
    setSearch('');
    setKind('all');
    setPlan(null);
    setPlannedAt(null);
    setNotice(null);
  };

  const selectionSummary = useMemo(() => {
    if (selection.profileId) {
      return `profile ${selection.profileId}`;
    }
    if (selection.groupId) {
      return `group ${selection.groupId}`;
    }
    return `${selection.serviceIds.length} selected service${
      selection.serviceIds.length === 1 ? '' : 's'
    }`;
  }, [selection]);

  const savePreset = (values: {
    id: string;
    name: string;
    description: string;
    isDefault: boolean;
  }) => {
    if (!activeWorkspace || !configuration.data) {
      return;
    }
    const entity = toOperationPreset(
      activeWorkspace.id,
      selection,
      options,
      values.id,
      values.name,
      values.description,
      values.isDefault,
    );
    setPresetEditor(null);
    void review.review({
      title: 'Create operation preset',
      message: `Adds preset “${entity.id}”. It stores only the selection and operation flags; it never stores credentials or authorization and never executes on load.`,
      destructive: false,
      expectedRevision: configuration.data.revision,
      change: { kind: 'operation_preset', change: { action: 'create', entity } },
    });
  };

  const renamePreset = (values: {
    id: string;
    name: string;
    description: string;
  }) => {
    if (!configuration.data || !rawPreset) {
      return;
    }
    setPresetEditor(null);
    void review.review({
      title: 'Rename operation preset',
      message: `Updates the display name of preset “${rawPreset.id}”.`,
      destructive: false,
      expectedRevision: configuration.data.revision,
      change: {
        kind: 'operation_preset',
        change: {
          action: 'update',
          id: rawPreset.id,
          entity: {
            ...rawPreset,
            name: values.name,
            description: values.description !== '' ? values.description : undefined,
          },
        },
      },
    });
  };

  const deletePreset = () => {
    if (!configuration.data || !rawPreset) {
      return;
    }
    setConfirmDeletePreset(false);
    void review.review({
      title: 'Remove operation preset',
      message: `Removes preset “${rawPreset.id}” from the configuration. This does not affect services, running processes, or history.`,
      destructive: true,
      expectedRevision: configuration.data.revision,
      change: {
        kind: 'operation_preset',
        change: { action: 'remove', id: rawPreset.id },
      },
    });
  };

  const makeDefault = async () => {
    if (!configuration.data || !rawPreset) {
      return;
    }
    setConfirmDefault(false);
    setBusy(true);
    try {
      let revision = configuration.data.revision;
      const others = configuration.data.operation_presets.filter(
        (preset) => preset.default === true && preset.id !== rawPreset.id,
      );
      for (const other of others) {
        const commit = await desktopApi.commitConfiguration(
          {
            kind: 'operation_preset',
            change: { action: 'update', id: other.id, entity: { ...other, default: false } },
          },
          revision,
          false,
        );
        revision = commit.revision;
      }
      const commit = await desktopApi.commitConfiguration(
        {
          kind: 'operation_preset',
          change: { action: 'update', id: rawPreset.id, entity: { ...rawPreset, default: true } },
        },
        revision,
        false,
      );
      setNotice(`“${rawPreset.id}” is now the default at revision ${commit.revision}.`);
      setPresetRevision((current) => current + 1);
    } catch (cause) {
      setFailure({ title: 'Unable to set the default preset', error: toDesktopError(cause) });
    } finally {
      setBusy(false);
    }
  };

  const runPlan = async () => {
    if (!request) {
      return;
    }
    setMode('plan');
    setPlanning(true);
    setPlanError(null);
    try {
      const result = await desktopApi.planPrepareEnvironment(request);
      setPlan(result);
      setPlannedAt(Math.floor(Date.now() / 1000));
    } catch (cause) {
      setPlanError(toDesktopError(cause));
    } finally {
      setPlanning(false);
    }
  };

  const execute = async () => {
    setConfirmExecute(false);
    if (!request) {
      return;
    }
    setBusy(true);
    setFailure(null);
    try {
      const outcome = await desktopApi.prepareEnvironment(request);
      if (!outcome.workflow_id) {
        setFailure({
          title: 'The daemon scheduled no workflow',
          error: {
            code: 'NO_WORKFLOW',
            message:
              'The daemon accepted the request but scheduled no workflow. Nothing is shown as running.',
          },
        });
        return;
      }
      run.begin(outcome.workflow_id);
      setMode('running');
    } catch (cause) {
      setFailure({ title: 'Unable to start the operation', error: toDesktopError(cause) });
    } finally {
      setBusy(false);
    }
  };

  const cancelWorkflow = async () => {
    setConfirmCancel(false);
    setBusy(true);
    await run.cancel();
    setBusy(false);
  };

  const authorizationHint =
    failure && failure.error.code === 'PermissionDenied'
      ? 'The daemon refused this protected operation because it could not verify an authorized interactive request. A frontend confirmation is never authorization: the daemon checks the client kind and the peer user on every request.'
      : undefined;

  if (workspacesError) {
    return <ErrorState error={workspacesError} onRetry={reloadWorkspaces} />;
  }
  if (workspacesLoading && !activeWorkspace) {
    return <LoadingState label="Loading workspaces…" />;
  }
  if (!activeWorkspace) {
    return (
      <EmptyState
        title="No workspace selected"
        description="Configure a workspace in ~/.service-orchestrator/config.toml to run bulk operations."
      />
    );
  }

  const stalePlan = plannedAt !== null ? Math.floor(Date.now() / 1000) - plannedAt : null;

  return (
    <div className="page">
      <WorkflowWizard mode={mode} />

      {notice ? (
        <p className="page__notice" role="status">
          {notice}
        </p>
      ) : null}

      {mode === 'configure' ? (
        <>
          <div className="card">
            <div className="card__header">
              <div>
                <h2 className="card__title">Bulk operations</h2>
                <p className="muted">
                  Workspace: {activeWorkspace.name}. One screen for the daily preparation of
                  selected services.
                </p>
              </div>
            </div>
            <div className="bulk-bar bulk-bar--presets">
              <label className="bulk-bar__select">
                <span className="bulk-bar__count">Saved preset</span>
                <select
                  className="input input--select"
                  value={presetId}
                  onChange={(event) => void loadPreset(event.target.value)}
                  aria-label="Saved operation preset"
                >
                  <option value="">No preset</option>
                  {presets.map((preset) => (
                    <option key={preset.id} value={preset.id}>
                      {preset.name}
                      {preset.default ? ' (default)' : ''}
                    </option>
                  ))}
                </select>
              </label>
              {loadedPreset ? (
                <span className="badge badge--info">loaded · not executed</span>
              ) : null}
              <button
                type="button"
                className="button button--small"
                disabled={!hasSelection || configuration.data === null}
                onClick={() => setPresetEditor({ mode: 'create' })}
              >
                Save as preset…
              </button>
              <button
                type="button"
                className="button button--small"
                disabled={rawPreset === undefined}
                onClick={() =>
                  rawPreset ? setPresetEditor({ mode: 'rename', preset: rawPreset }) : undefined
                }
              >
                Rename…
              </button>
              <button
                type="button"
                className="button button--small"
                disabled={rawPreset === undefined}
                onClick={() => setConfirmDeletePreset(true)}
              >
                Delete…
              </button>
              <button
                type="button"
                className="button button--small"
                disabled={rawPreset === undefined || rawPreset.default === true || busy}
                onClick={() => setConfirmDefault(true)}
              >
                Set default
              </button>
              <button type="button" className="button button--small" onClick={clearPreset}>
                Clear
              </button>
            </div>
            <p className="muted">
              Presets store only the selection and operation flags. They never store
              credentials or authorization, and loading one never executes it
              {defaultPreset ? ` · current default: ${defaultPreset.name}` : ''}.
            </p>
            {operationPresets.error ? (
              <ErrorState error={operationPresets.error} onRetry={operationPresets.reload} />
            ) : null}
            {configuration.error ? (
              <ErrorState error={configuration.error} onRetry={configuration.reload} />
            ) : null}
          </div>

          <div className="bulk-ops-grid">
            <div className="card">
              <div className="bulk-bar" role="radiogroup" aria-label="Selection source">
                <span className="bulk-bar__count">Selection source</span>
                <div className="bulk-bar__select">
                  {(['services', 'profile', 'group'] as const).map((value) => (
                    <label key={value} className="selection-list__source">
                      <input
                        type="radio"
                        name="bulk-operation-source"
                        checked={source === value}
                        disabled={!connected}
                        onChange={() => {
                          setSource(value);
                          setSelectedIds(new Set());
                          setProfileId(null);
                          setGroupId(null);
                          setPlan(null);
                        }}
                        aria-label={`Select by ${value}`}
                      />
                      {value === 'services'
                        ? 'Services'
                        : value === 'profile'
                          ? 'Profile'
                          : 'Group'}
                    </label>
                  ))}
                </div>
              </div>

              {source === 'services' ? (
                services.loading ? (
                  <LoadingState label="Loading services…" />
                ) : services.error ? (
                  <ErrorState error={services.error} onRetry={services.reload} />
                ) : (
                  <ServicePicker
                    services={services.data ?? []}
                    selectedIds={selectedIds}
                    search={search}
                    kind={kind}
                    disabled={!connected}
                    onSearch={setSearch}
                    onKind={setKind}
                    onToggle={(serviceId, selected) =>
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
                    onSelectAll={() =>
                      setSelectedIds(
                        new Set((services.data ?? []).map((service) => service.id)),
                      )
                    }
                    onSelectNone={() => setSelectedIds(new Set())}
                    onSelectVisible={(ids) =>
                      setSelectedIds((current) => {
                        const next = new Set(current);
                        for (const id of ids) {
                          next.add(id);
                        }
                        return next;
                      })
                    }
                  />
                )
              ) : null}

              {source === 'profile' ? (
                profiles.loading ? (
                  <LoadingState label="Loading profiles…" />
                ) : profiles.error ? (
                  <ErrorState error={profiles.error} onRetry={profiles.reload} />
                ) : (
                  <ProfileSelector
                    profiles={profiles.data ?? []}
                    selectedId={profileId}
                    disabled={!connected}
                    onSelect={setProfileId}
                  />
                )
              ) : null}

              {source === 'group' ? (
                groups.loading ? (
                  <LoadingState label="Loading groups…" />
                ) : groups.error ? (
                  <ErrorState error={groups.error} onRetry={groups.reload} />
                ) : (
                  <GroupSelector
                    groups={groups.data ?? []}
                    selectedId={groupId}
                    disabled={!connected}
                    onSelect={setGroupId}
                  />
                )
              ) : null}

              {!hasSelection ? (
                <p className="muted" role="note">
                  Select services, a profile, or a group. Only the selection and explicitly
                  resolved dependencies are affected.
                </p>
              ) : null}
            </div>

            <div className="card">
              {migrationPresets.error ? (
                <ErrorState
                  error={migrationPresets.error}
                  onRetry={migrationPresets.reload}
                />
              ) : null}
              <OperationOptions
                value={options}
                presets={migrationPresets.data ?? []}
                destructivePreset={destructivePreset}
                disabled={!connected}
                onChange={(next) => {
                  setOptions(next);
                  setPlan(null);
                }}
                onOpenLibraries={
                  onNavigate ? () => onNavigate('libraries') : undefined
                }
              />
            </div>
          </div>

          <div className="card">
            <div className="detail-actions">
              <button
                type="button"
                className="button button--primary"
                disabled={!connected || !hasSelection || planning}
                onClick={() => void runPlan()}
              >
                {planning ? 'Planning…' : 'Preview plan…'}
              </button>
              <span className="muted">
                The daemon resolves dependencies, phases, and authorization requirements.
                Nothing runs until you confirm execution.
              </span>
            </div>
          </div>
        </>
      ) : null}

      {mode === 'plan' ? (
        <div className="card">
          {planning ? <LoadingState label="Asking the daemon for a plan…" /> : null}
          {planError ? (
            <ErrorState
              error={planError}
              onRetry={() => {
                setPlanError(null);
                void runPlan();
              }}
            />
          ) : null}
          {!planning && !planError && plan ? (
            <>
              <WorkflowPreview plan={plan} workspaceName={activeWorkspace.name} />
              {destructivePreset || plan.requires_destructive_authorization ? (
                <div className="config-validation config-validation--error" role="alert">
                  <p>
                    <span className="badge badge--negative">Destructive</span> This plan
                    contains a database reset. It may target only declared environments in the
                    preset’s allowed list, and the daemon requires explicit interactive
                    authorization per target.
                  </p>
                  {destructivePreset ? (
                    <p className="muted">
                      Preset “{destructivePreset.name}” · contexts:{' '}
                      {destructivePreset.contexts.join(', ') || 'service defaults'}
                    </p>
                  ) : null}
                </div>
              ) : null}
              <p className="muted">
                Plan generated by the daemon
                {stalePlan !== null ? ` ${stalePlan}s ago` : ''}. It is read-only and may
                become stale: execution re-validates every precondition and authorization.
              </p>
              <div className="detail-actions">
                <button
                  type="button"
                  className="button"
                  disabled={busy}
                  onClick={() => setMode('configure')}
                >
                  Back
                </button>
                <button
                  type="button"
                  className="button"
                  disabled={busy}
                  onClick={() => void runPlan()}
                >
                  Refresh plan
                </button>
                <button
                  type="button"
                  className={`button ${
                    plan.requires_destructive_authorization ? 'button--danger' : 'button--primary'
                  }`}
                  disabled={!connected || busy}
                  onClick={() => setConfirmExecute(true)}
                >
                  Execute operation…
                </button>
              </div>
            </>
          ) : null}
          {!planning && !planError && !plan ? (
            <EmptyState
              title="No plan yet"
              description="Go back and request a read-only plan before executing anything."
              action={
                <button type="button" className="button" onClick={() => setMode('configure')}>
                  Back to configuration
                </button>
              }
            />
          ) : null}
        </div>
      ) : null}

      {mode === 'running' ? (
        <div className="card">
          {run.error ? <ErrorState error={run.error} /> : null}
          {!run.status && !run.error ? (
            <LoadingState label="Waiting for the daemon to report the workflow…" />
          ) : null}
          {run.status ? (
            <WorkflowProgress
              status={run.status}
              tasks={run.tasks}
              startedAt={run.observedAt}
            />
          ) : null}
          <div className="detail-actions">
            <button
              type="button"
              className="button"
              disabled={!connected || busy}
              onClick={() => onNavigate?.('tasks')}
            >
              Open tasks
            </button>
            <button
              type="button"
              className="button button--danger"
              disabled={!connected || busy || !run.status}
              onClick={() => setConfirmCancel(true)}
            >
              Cancel workflow…
            </button>
          </div>
          <p className="dialog__note muted">
            Workflow <strong>{run.workflowId}</strong> runs in the shared daemon; the same id
            is visible from the TUI and MCP.
          </p>
        </div>
      ) : null}

      {mode === 'results' ? (
        <div className="card">
          {run.error ? <ErrorState error={run.error} /> : null}
          {!run.status && !run.error ? (
            <LoadingState label="Loading workflow results…" />
          ) : null}
          {run.status ? (
            <WorkflowResults
              status={run.status}
              tasks={run.tasks}
              onPlanAgain={() => setMode(plan ? 'plan' : 'configure')}
              onOpenTask={(taskId) => onNavigate?.('tasks', { taskId })}
              onViewLogs={(serviceId) => onNavigate?.('logs', { serviceId })}
            />
          ) : null}
          <div className="detail-actions">
            <button
              type="button"
              className="button"
              onClick={() => {
                setMode('configure');
                setPlan(null);
                setPlannedAt(null);
              }}
            >
              Back to bulk operations
            </button>
          </div>
        </div>
      ) : null}

      {presetEditor ? (
        <PresetEditor
          mode={presetEditor.mode}
          initial={presetEditor.mode === 'rename' ? presetEditor.preset : null}
          selectionSummary={selectionSummary}
          defaultExists={defaultPreset !== undefined && defaultPreset.id !== presetId}
          onSave={(values) => {
            if (presetEditor.mode === 'create') {
              savePreset(values);
            } else {
              renamePreset(values);
            }
          }}
          onClose={() => setPresetEditor(null)}
        />
      ) : null}

      {confirmDeletePreset ? (
        <ConfirmationDialog
          title="Remove this preset?"
          message={
            loadedPreset
              ? `Removes preset “${loadedPreset.id}”. Services, running processes, and history are unaffected.`
              : 'Removes the selected preset.'
          }
          confirmLabel="Remove preset"
          tone="danger"
          onConfirm={deletePreset}
          onCancel={() => setConfirmDeletePreset(false)}
        />
      ) : null}

      {confirmDefault ? (
        <ConfirmationDialog
          title="Set the default preset?"
          message={
            <>
              <p>
                Makes “{rawPreset?.id}” the workspace default. If another preset is currently
                the default, it is cleared first; at most one preset may be the default.
              </p>
              <p>This writes configuration only. Nothing is executed.</p>
            </>
          }
          confirmLabel="Set default"
          busy={busy}
          onConfirm={() => void makeDefault()}
          onCancel={() => setConfirmDefault(false)}
        />
      ) : null}

      {confirmExecute && request && plan ? (
        <ConfirmationDialog
          title={
            plan.requires_destructive_authorization
              ? 'Execute a destructive operation?'
              : 'Execute this operation?'
          }
          tone={plan.requires_destructive_authorization ? 'danger' : 'default'}
          confirmLabel="Execute operation"
          busy={busy}
          message={
            <>
              <p>
                {plan.step_count} planned step{plan.step_count === 1 ? '' : 's'} across{' '}
                {plan.phases.length} phase{plan.phases.length === 1 ? '' : 's'} in workspace{' '}
                <strong>{activeWorkspace.name}</strong>.
              </p>
              {plan.requires_destructive_authorization ? (
                <p>
                  This plan contains a database reset that drops objects. The daemon verifies
                  each target’s declared environment and requires explicit interactive
                  same-user authorization. No retry or rollback is attempted on failure.
                </p>
              ) : null}
            </>
          }
          onConfirm={() => void execute()}
          onCancel={() => setConfirmExecute(false)}
        >
          <dl className="dialog__facts">
            <div>
              <dt>Selection</dt>
              <dd>{selectionSummary}</dd>
            </div>
            <div>
              <dt>Git</dt>
              <dd>{options.sync ? 'fetch and fast-forward' : 'skipped'}</dd>
            </div>
            <div>
              <dt>Libraries</dt>
              <dd>{options.buildLibraries ? 'build required' : 'not built'}</dd>
            </div>
            <div>
              <dt>Migrations</dt>
              <dd>
                {options.runLiquibase
                  ? options.migrationPreset || 'service presets'
                  : 'not run'}
              </dd>
            </div>
            <div>
              <dt>Build</dt>
              <dd>
                {options.build
                  ? `build · ${options.buildOperation || 'service default'}`
                  : 'no build'}
              </dd>
            </div>
            <div>
              <dt>Runtime</dt>
              <dd>{options.runtime}</dd>
            </div>
            <div>
              <dt>Health</dt>
              <dd>{options.waitForHealth ? 'wait for readiness' : 'do not wait'}</dd>
            </div>
          </dl>
          <p className="dialog__note">
            This confirmation is intent, not authorization: the daemon independently verifies
            the client and the requesting user, and its decision is final.
          </p>
        </ConfirmationDialog>
      ) : null}

      {confirmCancel && run.status ? (
        <WorkflowCancellationDialog
          status={run.status}
          busy={busy}
          onConfirm={() => void cancelWorkflow()}
          onCancel={() => setConfirmCancel(false)}
        />
      ) : null}

      {review.pending ? (
        <SaveConfirmationDialog
          title={review.pending.title}
          message={review.pending.message}
          destructive={review.pending.destructive}
          preview={review.preview}
          loading={review.loading}
          error={review.error}
          conflict={review.conflict}
          saving={review.saving}
          saveError={review.saveError}
          onConfirm={() => void review.confirm()}
          onCancel={review.cancel}
          onRefresh={review.refresh}
        />
      ) : null}

      {failure ? (
        <ErrorDialog
          title={failure.title}
          error={failure.error}
          labels={selection.serviceIds}
          hint={authorizationHint}
          onClose={() => setFailure(null)}
        />
      ) : null}
    </div>
  );
}
