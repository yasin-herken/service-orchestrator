import { useEffect, useMemo, useRef, useState } from 'react';

import { AdvancedSettingsDialog } from '../components/control-center/AdvancedSettingsDialog';
import {
  ServiceLogDialog,
  TaskLogDialog,
  type ControlCenterLogTarget,
} from '../components/control-center/LogDialogs';
import { LibraryToolsDialog } from '../components/control-center/LibraryToolsDialog';
import { OperationControls } from '../components/control-center/OperationControls';
import { ServiceSelectionDialog } from '../components/control-center/ServiceSelectionDialog';
import {
  QUICK_ACTIONS,
  actionableHint,
  configuredDefaultBranches,
  currentPhase,
  defaultControlCenterOptions,
  exposedBuildOperations,
  hasOperation,
  loadControlCenterState,
  quickActionOptions,
  requestKey,
  saveControlCenterState,
  toControlCenterRequest,
  type ControlCenterOptions,
  type QuickAction,
  type StoredControlCenterState,
} from '../components/control-center/model';
import { ConfirmationDialog } from '../components/ConfirmationDialog';
import { ErrorDialog } from '../components/ErrorDialog';
import { EmptyState, ErrorState, LoadingState } from '../components/StateViews';
import { HealthBadge, ProcessBadge } from '../components/StatusBadge';
import { WorkflowCancellationDialog } from '../components/workflow/WorkflowCancellationDialog';
import { WorkflowPreview } from '../components/workflow/WorkflowPreview';
import { WorkflowProgress } from '../components/workflow/WorkflowProgress';
import { WorkflowResults } from '../components/workflow/WorkflowResults';
import { useConnection } from '../context/ConnectionContext';
import { useWorkspaces } from '../context/WorkspaceContext';
import { useAsync } from '../hooks/useAsync';
import { useWorkflowRun } from '../hooks/useWorkflowRun';
import { desktopApi } from '../services/desktopApi';
import { toDesktopError } from '../services/errors';
import type {
  BuildTargetSummary,
  ConfigurationSnapshotView,
  DesktopError,
  EnvironmentPlanView,
  GroupView,
  MigrationPresetSummaryView,
  PrepareEnvironmentInput,
  ProfileView,
  ServiceSummary,
} from '../types/models';
import type { PageId } from '../types/nav';
import { isTerminalTaskStatus } from '../utils/taskStatus';

/** A service listing tagged with the workspace it belongs to. */
interface ServicesPayload {
  workspaceId: string;
  services: ServiceSummary[];
}

/** A build-target listing tagged with the workspace it belongs to. */
interface BuildTargetsPayload {
  workspaceId: string;
  targets: BuildTargetSummary[];
}

/**
 * The Workspace Control Center (`tasks/033.md`).
 *
 * One screen for the everyday developer workflow: choose workspace services,
 * configure the existing environment operations inline, inspect the daemon's
 * read-only plan, submit one coordinated `prepare_environment` workflow, and
 * watch progress, results, tasks, and logs without changing tabs.
 *
 * The daemon stays authoritative. React calls only the typed desktop API; the
 * Tauri adapter forwards the same request to the shared daemon and the
 * application core composes, validates, authorizes, and executes it.
 */
export function ControlCenterPage({
  onNavigate,
}: {
  onNavigate?: (page: PageId) => void;
}) {
  const {
    workspaces,
    activeWorkspace,
    activeWorkspaceId,
    selectWorkspace,
    loading: workspacesLoading,
    error: workspacesError,
    reload: reloadWorkspaces,
  } = useWorkspaces();
  const { snapshot, reconnecting, reconnect } = useConnection();
  const connected = snapshot.state === 'connected';
  const run = useWorkflowRun();

  const [options, setOptions] = useState<ControlCenterOptions>(
    defaultControlCenterOptions,
  );
  const [selectedIds, setSelectedIds] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [hydratedWorkspace, setHydratedWorkspace] = useState<string | null>(null);
  const [workspaceRestored, setWorkspaceRestored] = useState(false);

  const [selectionOpen, setSelectionOpen] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [libraryOpen, setLibraryOpen] = useState(false);
  const [logTarget, setLogTarget] = useState<ControlCenterLogTarget | null>(null);

  const [plan, setPlan] = useState<EnvironmentPlanView | null>(null);
  const [plannedKey, setPlannedKey] = useState('');
  const [planning, setPlanning] = useState(false);
  const [planError, setPlanError] = useState<DesktopError | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [pendingRequest, setPendingRequest] =
    useState<PrepareEnvironmentInput | null>(null);
  const [confirmExecute, setConfirmExecute] = useState(false);
  const [confirmCancel, setConfirmCancel] = useState(false);
  const [failure, setFailure] = useState<{
    title: string;
    error: DesktopError;
  } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [resultRevision, setResultRevision] = useState(0);

  const savedState = useRef<StoredControlCenterState | null | undefined>(
    undefined,
  );
  if (savedState.current === undefined) {
    savedState.current = loadControlCenterState();
  }
  const hydratedRef = useRef<string | null>(null);
  const settledWorkflow = useRef<string | null>(null);

  const services = useAsync<ServicesPayload>(
    () => {
      const workspaceId = activeWorkspaceId;
      if (!workspaceId) {
        return Promise.resolve({ workspaceId: '', services: [] });
      }
      return desktopApi
        .listServices(workspaceId)
        .then((list) => ({ workspaceId, services: list }));
    },
    [activeWorkspaceId, resultRevision],
    { enabled: connected && workspaceRestored && activeWorkspace !== null },
  );
  const buildTargets = useAsync<BuildTargetsPayload>(
    () => {
      const workspaceId = activeWorkspaceId;
      if (!workspaceId) {
        return Promise.resolve({ workspaceId: '', targets: [] });
      }
      return desktopApi
        .listBuildTargets(workspaceId)
        .then((targets) => ({ workspaceId, targets }));
    },
    [activeWorkspaceId],
    { enabled: connected && workspaceRestored && activeWorkspace !== null },
  );
  const profiles = useAsync<ProfileView[]>(
    () =>
      activeWorkspaceId
        ? desktopApi.listProfiles(activeWorkspaceId)
        : Promise.resolve([]),
    [activeWorkspaceId],
    { enabled: connected && workspaceRestored && activeWorkspace !== null },
  );
  const groups = useAsync<GroupView[]>(
    () =>
      activeWorkspaceId
        ? desktopApi.listGroups(activeWorkspaceId)
        : Promise.resolve([]),
    [activeWorkspaceId],
    { enabled: connected && workspaceRestored && activeWorkspace !== null },
  );
  const migrationPresets = useAsync<MigrationPresetSummaryView[]>(
    () => desktopApi.listMigrationPresets(),
    [connected],
    { enabled: connected },
  );
  const configuration = useAsync<ConfigurationSnapshotView>(
    () => desktopApi.getConfiguration(),
    [connected],
    { enabled: connected },
  );

  const servicesReady =
    services.data !== null &&
    services.data.workspaceId === activeWorkspaceId;
  const serviceList =
    services.data !== null && services.data.workspaceId === activeWorkspaceId
      ? services.data.services
      : [];
  const targetList =
    buildTargets.data !== null &&
    buildTargets.data.workspaceId === activeWorkspaceId
      ? buildTargets.data.targets
      : [];
  const presets = migrationPresets.data ?? [];
  // The configuration snapshot spans every workspace, but service ids are only
  // workspace-scoped. Never let another workspace's raw service configure this
  // screen.
  const rawServices = useMemo(
    () =>
      (configuration.data?.services ?? []).filter(
        (service) => service.workspace === activeWorkspaceId,
      ),
    [configuration.data, activeWorkspaceId],
  );

  const selectedServiceList = useMemo(
    () => serviceList.filter((service) => selectedIds.has(service.id)),
    [serviceList, selectedIds],
  );
  const targetById = useMemo(
    () => new Map(targetList.map((target) => [target.service_id, target])),
    [targetList],
  );
  const selectedTargets = useMemo(
    () =>
      selectedServiceList
        .map((service) => targetById.get(service.id))
        .filter((target): target is BuildTargetSummary => target !== undefined),
    [selectedServiceList, targetById],
  );
  const buildOperations = useMemo(
    () => exposedBuildOperations(selectedTargets),
    [selectedTargets],
  );
  const defaultBranches = useMemo(
    () =>
      activeWorkspaceId
        ? configuredDefaultBranches(activeWorkspaceId, [...selectedIds], rawServices)
        : [],
    [activeWorkspaceId, selectedIds, rawServices],
  );
  // A preset is only in effect when migrations are enabled; otherwise its
  // destructive warning must not leak into an unrelated plan.
  const destructivePreset = options.runLiquibase
    ? presets.find(
        (preset) =>
          preset.id === options.migrationPreset && preset.destructive,
      )
    : undefined;

  const request = useMemo<PrepareEnvironmentInput | null>(() => {
    if (
      !servicesReady ||
      !activeWorkspaceId ||
      selectedIds.size === 0
    ) {
      return null;
    }
    return toControlCenterRequest(activeWorkspaceId, [...selectedIds], options);
  }, [servicesReady, activeWorkspaceId, selectedIds, options]);
  const requestSignature = requestKey(request);
  const planFresh = plan !== null && plannedKey === requestSignature;
  const hasSelection = selectedIds.size > 0;
  const operationSelected = hasOperation(options);
  const busy = planning || submitting || cancelling;
  const canSubmit =
    connected && hasSelection && operationSelected && request !== null && !busy;
  // The request can change while an asynchronous plan is in flight. Submission
  // compares against this live ref so an old plan can never submit a request
  // the developer can no longer see.
  const currentRequestRef = useRef<PrepareEnvironmentInput | null>(request);
  currentRequestRef.current = request;

  // Restore the last workspace once the daemon has returned the workspace
  // list. This changes local navigation only and never reads a service or
  // submits a request (`tasks/033.md` §18).
  useEffect(() => {
    // Wait for the provider to settle on an active workspace first; otherwise a
    // restore in the same commit can be overwritten by its first-workspace
    // default. Workspace-scoped reads stay disabled until this decision is
    // made, so no request is made for the workspace being replaced.
    if (workspaceRestored || activeWorkspaceId === null || workspaces.length === 0) {
      return;
    }
    const saved = savedState.current;
    if (
      saved &&
      saved.workspaceId !== activeWorkspaceId &&
      workspaces.some((workspace) => workspace.id === saved.workspaceId)
    ) {
      selectWorkspace(saved.workspaceId);
      return;
    }
    setWorkspaceRestored(true);
  }, [workspaceRestored, workspaces, activeWorkspaceId, selectWorkspace]);

  // Loading a saved selection is inert: it applies local state only and never
  // plans, executes, or authorizes anything (`tasks/033.md` §18).
  useEffect(() => {
    if (!activeWorkspaceId || services.data === null) {
      return;
    }
    if (services.data.workspaceId !== activeWorkspaceId) {
      return;
    }
    if (hydratedRef.current === activeWorkspaceId) {
      return;
    }
    hydratedRef.current = activeWorkspaceId;
    const saved = savedState.current;
    if (saved && saved.workspaceId === activeWorkspaceId) {
      const available = new Set(
        services.data.services.map((service) => service.id),
      );
      setSelectedIds(
        new Set(saved.serviceIds.filter((serviceId) => available.has(serviceId))),
      );
      setOptions(saved.options);
    } else {
      setSelectedIds(new Set());
      setOptions(defaultControlCenterOptions());
    }
    setPlan(null);
    setPlannedKey('');
    setPlanError(null);
    setNotice(null);
    setConfirmExecute(false);
    setPendingRequest(null);
    setSelectionOpen(false);
    setAdvancedOpen(false);
    run.reset();
    setHydratedWorkspace(activeWorkspaceId);
    // `run.reset` is stable; the workspace and its loaded services are the inputs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeWorkspaceId, services.data]);

  // Persist only the selection and options. No plan, workflow id, credential,
  // or authorization is ever stored, and saving is best-effort.
  useEffect(() => {
    if (!activeWorkspaceId || hydratedWorkspace !== activeWorkspaceId) {
      return;
    }
    const next: StoredControlCenterState = {
      version: 1,
      workspaceId: activeWorkspaceId,
      serviceIds: [...selectedIds].sort(),
      options,
    };
    // Keep the in-memory snapshot aligned with what was just persisted. This
    // prevents a later workspace switch from rehydrating an older app-launch
    // snapshot instead of resetting to the last saved state.
    savedState.current = next;
    saveControlCenterState(next);
  }, [activeWorkspaceId, hydratedWorkspace, selectedIds, options]);

  // A confirmation belongs to a live daemon session. Dropping the connection
  // closes it so no protected action can be confirmed against stale intent.
  useEffect(() => {
    if (connected) {
      return;
    }
    setConfirmExecute(false);
    setPendingRequest(null);
    setConfirmCancel(false);
  }, [connected]);

  // When the daemon declares the workflow terminal, refresh the selected
  // services once so final runtime and health states are authoritative.
  useEffect(() => {
    if (
      run.workflowId !== null &&
      run.terminal &&
      settledWorkflow.current !== run.workflowId
    ) {
      settledWorkflow.current = run.workflowId;
      setResultRevision((current) => current + 1);
    }
  }, [run.workflowId, run.terminal]);

  const applySelection = (ids: ReadonlySet<string>) => {
    setSelectedIds(new Set(ids));
    setSelectionOpen(false);
  };

  const changeOptions = (next: ControlCenterOptions) => {
    setOptions(next);
  };

  const planRequest = async (
    input: PrepareEnvironmentInput,
  ): Promise<EnvironmentPlanView | null> => {
    setPlanning(true);
    setPlanError(null);
    setFailure(null);
    try {
      const result = await desktopApi.planPrepareEnvironment(input);
      setPlan(result);
      setPlannedKey(requestKey(input));
      return result;
    } catch (cause) {
      // A failed planning call must not leave an older plan looking current.
      setPlan(null);
      setPlannedKey('');
      setPlanError(toDesktopError(cause));
      return null;
    } finally {
      setPlanning(false);
    }
  };

  const submitRequest = async (input: PrepareEnvironmentInput) => {
    setSubmitting(true);
    setFailure(null);
    setNotice(null);
    try {
      const outcome = await desktopApi.prepareEnvironment(input);
      if (!outcome.workflow_id) {
        setNotice(
          `The daemon scheduled no workflow (${outcome.kind}: ${outcome.status}).`,
        );
        return;
      }
      run.begin(outcome.workflow_id);
      setNotice(
        `Workflow ${outcome.workflow_id} submitted to the shared daemon. Selection and options are preserved.`,
      );
    } catch (cause) {
      setFailure({
        title: 'Unable to start the environment workflow',
        error: toDesktopError(cause),
      });
    } finally {
      setSubmitting(false);
      setConfirmExecute(false);
      setPendingRequest(null);
    }
  };

  const prepareAndRun = async (
    input: PrepareEnvironmentInput,
    nextOptions: ControlCenterOptions,
  ) => {
    const planned = await planRequest(input);
    if (!planned) {
      return;
    }
    if (requestKey(currentRequestRef.current) !== requestKey(input)) {
      setNotice(
        'The request changed while it was being planned. Review the current request and run again.',
      );
      return;
    }
    const requiresAuthorization =
      planned.requires_destructive_authorization ||
      (nextOptions.runLiquibase &&
        presets.some(
          (preset) => preset.id === nextOptions.migrationPreset && preset.destructive,
        ));
    if (requiresAuthorization) {
      setPendingRequest(input);
      setConfirmExecute(true);
      return;
    }
    await submitRequest(input);
  };

  const runCurrent = () => {
    if (!request) {
      return;
    }
    void prepareAndRun(request, options);
  };

  const runQuickAction = (action: QuickAction) => {
    if (!activeWorkspaceId || selectedIds.size === 0) {
      return;
    }
    const next = quickActionOptions(action, options);
    setOptions(next);
    const input = toControlCenterRequest(
      activeWorkspaceId,
      [...selectedIds],
      next,
    );
    // `setOptions` has not necessarily rendered yet; reflect the quick action
    // immediately so the in-flight consistency check sees the submitted input.
    currentRequestRef.current = input;
    void prepareAndRun(input, next);
  };

  const cancelWorkflow = async () => {
    setConfirmCancel(false);
    setCancelling(true);
    await run.cancel();
    setCancelling(false);
  };

  const openTaskLogs = (taskId: string) => {
    const task = run.tasks.find((candidate) => candidate.id === taskId);
    setLogTarget({
      kind: 'task',
      taskId,
      live: task !== undefined && !isTerminalTaskStatus(task.status),
    });
  };

  // Keyboard shortcuts stay on the one screen: select services and run the
  // current configuration. Escape is owned by the open dialog.
  const submitRef = useRef(canSubmit);
  submitRef.current = canSubmit;
  const runRef = useRef(runCurrent);
  runRef.current = runCurrent;
  const dialogOpen =
    selectionOpen ||
    advancedOpen ||
    libraryOpen ||
    logTarget !== null ||
    confirmExecute ||
    confirmCancel ||
    failure !== null;
  const dialogOpenRef = useRef(dialogOpen);
  dialogOpenRef.current = dialogOpen;
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || dialogOpenRef.current) {
        return;
      }
      const key = event.key.toLowerCase();
      if (key === 'k') {
        event.preventDefault();
        setSelectionOpen(true);
      } else if (key === 'enter' && submitRef.current) {
        event.preventDefault();
        runRef.current();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);

  if (workspacesError) {
    return <ErrorState error={workspacesError} onRetry={reloadWorkspaces} />;
  }
  if (workspacesLoading && !activeWorkspace) {
    return <LoadingState label="Loading workspaces…" />;
  }
  if (!activeWorkspace) {
    if (snapshot.state === 'connecting' || snapshot.state === 'reconnecting') {
      return <LoadingState label="Waiting for the daemon connection…" />;
    }
    if (!connected) {
      return (
        <div className="page control-center">
          <section className="card" aria-label="Daemon connection">
            <div className={`connection-banner connection-banner--${snapshot.state}`}>
              <span className="connection-banner__text">
                {snapshot.message ??
                  'The daemon is not connected. Mutating actions are disabled; nothing is replayed automatically.'}
              </span>
              <button
                type="button"
                className="button button--small"
                disabled={reconnecting}
                onClick={() => void reconnect()}
              >
                {reconnecting ? 'Reconnecting…' : 'Reconnect'}
              </button>
            </div>
            <p className="muted">
              Workspace state cannot be read until the shared daemon is
              reachable. The last selection and options are preserved locally.
            </p>
          </section>
        </div>
      );
    }
    return (
      <EmptyState
        title="No workspace configured"
        description="Add a workspace to ~/.service-orchestrator/config.toml (or set SERVICE_ORCHESTRATOR_CONFIG) and reconnect."
      />
    );
  }

  const phase = currentPhase(run.status);
  const planRequiresAuthorization =
    (plan?.requires_destructive_authorization ?? false) ||
    destructivePreset !== undefined;

  return (
    <div className="page control-center">
      <section className="card" aria-label="Control center overview">
        <div className="card__header">
          <div>
            <h2 className="card__title">One screen · one selection · one execution</h2>
            <p className="muted">
              Workspace <strong>{activeWorkspace.name}</strong> · {activeWorkspace.root}
            </p>
          </div>
          <span className="badge badge--info">
            {services.loading ? 'refreshing' : `${serviceList.length} services`}
          </span>
        </div>

        {!connected ? (
          <div
            className={`connection-banner connection-banner--${snapshot.state}`}
            role="status"
          >
            <span className="connection-banner__text">
              {snapshot.state === 'reconnecting'
                ? 'The daemon connection was lost — reconnecting…'
                : 'The daemon is not connected. Mutating actions are disabled; your selection is preserved and nothing is replayed automatically.'}
            </span>
            <button
              type="button"
              className="button button--small"
              disabled={reconnecting}
              onClick={() => void reconnect()}
            >
              {reconnecting ? 'Reconnecting…' : 'Reconnect'}
            </button>
          </div>
        ) : null}

        <div className="control-center__quick-actions" role="group" aria-label="Quick actions">
          {QUICK_ACTIONS.map((action) => (
            <button
              key={action.id}
              type="button"
              className="button button--small"
              title={action.description}
              disabled={!connected || !servicesReady || !hasSelection || busy}
              onClick={() => runQuickAction(action.id)}
            >
              {action.label}
            </button>
          ))}
        </div>
        <p className="muted">
          Quick actions submit the same coordinated{' '}
          <code>prepare_environment</code> workflow through the daemon; they do
          not use a separate execution path.
        </p>
      </section>

      <div className="control-center__grid">
        <section className="card" aria-label="Service selection">
          <div className="card__header">
            <div>
              <h2 className="card__title">Selected services</h2>
              <p className="muted">
                {hasSelection
                  ? `${selectedIds.size} selected · dependencies are resolved by the plan`
                  : 'Choose services, a profile, or a group to begin.'}
              </p>
            </div>
            <button
              type="button"
              className="button button--small button--primary"
              disabled={busy}
              onClick={() => setSelectionOpen(true)}
            >
              Select services…
            </button>
          </div>

          {services.error ? (
            <ErrorState error={services.error} onRetry={services.reload} />
          ) : null}
          {!servicesReady && services.loading ? (
            <LoadingState label="Loading services…" />
          ) : null}

          {selectedServiceList.length > 0 ? (
            <ul className="control-center__chips" aria-label="Selected service list">
              {selectedServiceList.map((service) => (
                <li key={service.id} className="control-center__chip">
                  <span>{service.display_name}</span>
                  <span className="muted">{service.kind}</span>
                  <ProcessBadge state={service.process_state} />
                  <HealthBadge state={service.health_state} />
                </li>
              ))}
            </ul>
          ) : (
            <p className="muted" role="note">
              Nothing is selected. Use the service picker or press ⌘K / Ctrl+K.
            </p>
          )}

          {buildTargets.error ? (
            <p className="muted" role="status">
              Build capabilities are unavailable: {buildTargets.error.message}
            </p>
          ) : null}
        </section>

        <section className="card" aria-label="Operation configuration">
          {migrationPresets.error ? (
            <ErrorState
              error={migrationPresets.error}
              onRetry={migrationPresets.reload}
            />
          ) : null}
          <OperationControls
            value={options}
            presets={presets}
            destructivePreset={destructivePreset}
            buildOperations={buildOperations}
            defaultBranches={defaultBranches}
            disabled={busy}
            onChange={changeOptions}
            onOpenAdvanced={() => setAdvancedOpen(true)}
            onOpenLibraries={() => setLibraryOpen(true)}
          />
        </section>
      </div>

      <section className="card" aria-label="Plan and execute">
        <div className="card__header">
          <div>
            <h2 className="card__title">Plan &amp; run</h2>
            <p className="muted">
              The daemon resolves dependencies, phases, and authorization. Planning
              changes nothing.
            </p>
          </div>
          <div className="detail-actions control-center__run-actions">
            <button
              type="button"
              className="button"
              disabled={
                !connected ||
                !servicesReady ||
                !hasSelection ||
                !operationSelected ||
                busy
              }
              onClick={() => {
                if (request) {
                  void planRequest(request);
                }
              }}
            >
              {planning ? 'Planning…' : 'Preview plan'}
            </button>
            <button
              type="button"
              className={`button ${
                planRequiresAuthorization ? 'button--danger' : 'button--primary'
              }`}
              disabled={!canSubmit}
              onClick={runCurrent}
            >
              {submitting
                ? 'Submitting…'
                : planning
                  ? 'Planning…'
                  : 'Prepare & Run'}
            </button>
          </div>
        </div>

        {!hasSelection ? (
          <p className="muted" role="note">
            Select at least one service before planning or running.
          </p>
        ) : !operationSelected ? (
          <p className="muted" role="note">
            Enable at least one operation before planning or running.
          </p>
        ) : null}

        {planError ? (
          <ErrorState
            error={planError}
            onRetry={() => {
              if (request) {
                void planRequest(request);
              }
            }}
          />
        ) : null}

        {plan ? (
          <div className="control-center__plan">
            {!planFresh ? (
              <div className="config-conflict" role="status">
                The request changed after this plan was generated. Preview it again
                to inspect the current request; execution always re-plans and
                re-validates.
              </div>
            ) : null}
            {planRequiresAuthorization ? (
              <div className="config-validation config-validation--error" role="alert">
                <p>
                  <span className="badge badge--negative">Destructive</span> This
                  plan contains a protected database reset. The daemon verifies each
                  declared target and requires explicit interactive authorization.
                  Nothing is retried or rolled back automatically.
                </p>
                {destructivePreset ? (
                  <p className="muted">
                    Preset “{destructivePreset.name}” · contexts:{' '}
                    {destructivePreset.contexts.join(', ') ||
                      'service defaults'}
                  </p>
                ) : null}
              </div>
            ) : null}
            <WorkflowPreview plan={plan} workspaceName={activeWorkspace.name} />
            {plan.warnings.length > 0 ? (
              <p className="muted">
                Warnings are the daemon&apos;s own observations; they do not block
                execution unless the daemon reports a failure.
              </p>
            ) : null}
          </div>
        ) : null}
      </section>

      {run.workflowId ? (
        <section className="card">
          <div className="card__header">
            <div>
              <h2 className="card__title">Progress</h2>
              <p className="muted">
                Workflow <strong>{run.workflowId}</strong>
                {phase ? ` · current phase ${phase}` : ''}
              </p>
            </div>
            <button
              type="button"
              className="button button--danger button--small"
              disabled={!connected || busy || !run.status || run.terminal}
              onClick={() => setConfirmCancel(true)}
            >
              Cancel workflow…
            </button>
          </div>
          {run.error ? <ErrorState error={run.error} /> : null}
          {!run.status && !run.error ? (
            <LoadingState label="Waiting for the daemon to report the workflow…" />
          ) : null}
          {run.status ? (
            <WorkflowProgress
              status={run.status}
              tasks={run.tasks}
              startedAt={run.observedAt}
              onOpenTask={openTaskLogs}
            />
          ) : null}
          <p className="dialog__note muted">
            Progress comes from the daemon&apos;s task engine. The GUI never declares
            a terminal state, and reconnecting resumes observation without replaying
            any operation.
          </p>
        </section>
      ) : null}

      {run.terminal && run.status ? (
        <>
          <section className="card">
            <WorkflowResults
              status={run.status}
              tasks={run.tasks}
              onPlanAgain={() => {
                run.reset();
                setPlan(null);
                setPlannedKey('');
                setNotice(
                  'Configuration retained. Preview the plan or Prepare & Run when ready.',
                );
              }}
              onOpenTask={openTaskLogs}
              onViewLogs={(serviceId) =>
                setLogTarget({ kind: 'service', serviceId })
              }
            />
          </section>

          <section className="card" aria-label="Final runtime and health">
            <div className="card__header">
              <h2 className="card__title">Final runtime and health</h2>
              {services.loading ? (
                <span className="muted">Refreshing…</span>
              ) : null}
            </div>
            {selectedServiceList.length === 0 ? (
              <p className="muted">
                The selection is no longer available in this workspace.
              </p>
            ) : (
              <ul className="control-center__final-states">
                {selectedServiceList.map((service) => (
                  <li key={service.id}>
                    <span className="control-center__final-name">
                      {service.display_name}
                    </span>
                    <ProcessBadge state={service.process_state} />
                    <HealthBadge state={service.health_state} />
                    <span className="muted">
                      {service.branch ?? 'no branch'}
                      {service.port !== null ? ` · port ${service.port}` : ''}
                    </span>
                  </li>
                ))}
              </ul>
            )}
            <p className="muted">
              Read from the daemon after the workflow settled; the Control Center
              does not guess a final state.
            </p>
          </section>
        </>
      ) : null}

      {notice ? (
        <p className="page__notice" role="status">
          {notice}
        </p>
      ) : null}

      {selectionOpen ? (
        <ServiceSelectionDialog
          services={serviceList}
          buildTargets={targetList}
          profiles={profiles.data ?? []}
          groups={groups.data ?? []}
          selectedIds={selectedIds}
          loading={services.loading && !servicesReady}
          onApply={applySelection}
          onClose={() => setSelectionOpen(false)}
        />
      ) : null}

      {advancedOpen ? (
        <AdvancedSettingsDialog
          value={options}
          presets={presets}
          buildOperations={buildOperations}
          selectedServices={selectedServiceList}
          rawServices={rawServices}
          configurationError={configuration.error?.message ?? null}
          onReloadConfiguration={configuration.reload}
          onChange={changeOptions}
          onOpenConfiguration={() => {
            setAdvancedOpen(false);
            onNavigate?.('configuration');
          }}
          onClose={() => setAdvancedOpen(false)}
        />
      ) : null}

      {libraryOpen ? (
        <LibraryToolsDialog
          workspaceId={activeWorkspace.id}
          selectedIds={selectedIds}
          connected={connected}
          onClose={() => setLibraryOpen(false)}
        />
      ) : null}

      {logTarget?.kind === 'task' ? (
        <TaskLogDialog
          target={logTarget}
          onClose={() => setLogTarget(null)}
        />
      ) : null}
      {logTarget?.kind === 'service' ? (
        <ServiceLogDialog
          target={logTarget}
          workspaceId={activeWorkspace.id}
          connected={connected}
          onClose={() => setLogTarget(null)}
        />
      ) : null}

      {confirmExecute && pendingRequest ? (
        <ConfirmationDialog
          title="Execute a destructive operation?"
          tone="danger"
          confirmLabel="Execute operation"
          busy={submitting}
          message={
            <>
              <p>
                This plan contains a protected operation. The daemon verifies the
                target and requires explicit interactive same-user authorization.
              </p>
              <p>
                The request is the exact plan you reviewed. No migration is retried
                automatically and no Git reset or clean runs on your behalf.
              </p>
            </>
          }
          onConfirm={() => void submitRequest(pendingRequest)}
          onCancel={() => {
            setConfirmExecute(false);
            setPendingRequest(null);
          }}
        >
          <dl className="dialog__facts">
            <div>
              <dt>Workspace</dt>
              <dd>{activeWorkspace.name}</dd>
            </div>
            <div>
              <dt>Selection</dt>
              <dd>
                {selectedIds.size} service{selectedIds.size === 1 ? '' : 's'}
              </dd>
            </div>
            <div>
              <dt>Migration preset</dt>
              <dd>{options.migrationPreset || 'each service’s preset'}</dd>
            </div>
            <div>
              <dt>Planned tasks</dt>
              <dd>{plan?.step_count ?? '—'}</dd>
            </div>
          </dl>
          <p className="dialog__note">
            This confirmation is intent, not authorization: the daemon independently
            verifies the client and the requesting user, and its decision is final.
          </p>
        </ConfirmationDialog>
      ) : null}

      {confirmCancel && run.status ? (
        <WorkflowCancellationDialog
          status={run.status}
          busy={cancelling}
          onConfirm={() => void cancelWorkflow()}
          onCancel={() => setConfirmCancel(false)}
        />
      ) : null}

      {failure ? (
        <ErrorDialog
          title={failure.title}
          error={failure.error}
          labels={[...selectedIds]}
          hint={actionableHint(failure.error) ?? undefined}
          onClose={() => setFailure(null)}
        />
      ) : null}
    </div>
  );
}
