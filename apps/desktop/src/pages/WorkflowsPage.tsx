import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { ConfirmationDialog } from '../components/ConfirmationDialog';
import { ErrorDialog } from '../components/ErrorDialog';
import { EmptyState, ErrorState, LoadingState } from '../components/StateViews';
import { WorkflowCancellationDialog } from '../components/workflow/WorkflowCancellationDialog';
import { WorkflowHistory } from '../components/workflow/WorkflowHistory';
import { WorkflowOptions, defaultWorkflowOptions, toPrepareInput, type WorkflowOptionsValue } from '../components/workflow/WorkflowOptions';
import { WorkflowPreview } from '../components/workflow/WorkflowPreview';
import { WorkflowProgress } from '../components/workflow/WorkflowProgress';
import { WorkflowResults } from '../components/workflow/WorkflowResults';
import { WorkflowWizard, type WorkflowMode } from '../components/workflow/WorkflowWizard';
import { GroupSelector } from '../components/workflow/GroupSelector';
import { ProfileSelector } from '../components/workflow/ProfileSelector';
import { ServiceSelection } from '../components/workflow/ServiceSelection';
import { useConnection } from '../context/ConnectionContext';
import { useTaskEvents } from '../context/TaskEventsContext';
import { useWorkspaces } from '../context/WorkspaceContext';
import { useAsync } from '../hooks/useAsync';
import { desktopApi } from '../services/desktopApi';
import { toDesktopError } from '../services/errors';
import type {
  DesktopError,
  EnvironmentPlanView,
  GroupView,
  ProfileView,
  ServiceSummary,
  TaskView,
  WorkflowStatusView,
} from '../types/models';
import { isTerminalWorkflowState } from '../types/models';
import type { NavIntent, PageId } from '../types/nav';

/** Which single source the selection comes from (exactly one, always). */
type SelectionSource = 'services' | 'profile' | 'group';

/** How often the running workflow is re-read while connected. */
const POLL_MS = 1500;

/**
 * The Workflows screen (`tasks/022.md` §5–§27).
 *
 * The flow is configure → plan → execute → results, and every step delegates
 * to the shared daemon: planning calls the read-only `plan_prepare_environment`,
 * execution calls `prepare_environment` and follows the daemon's real workflow
 * id, progress and results come from `get_workflow_status` plus the task
 * engine, and cancellation uses the existing cancellation API. This screen
 * never resolves dependencies, never schedules steps, and never treats its own
 * confirmation as authorization — the daemon re-validates everything
 * (`tasks/022.md` §12, §17, §32).
 */
export function WorkflowsPage({
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
  const { terminalRevision } = useTaskEvents();

  const [mode, setMode] = useState<WorkflowMode>('configure');
  const [source, setSource] = useState<SelectionSource>('services');
  const [selectedIds, setSelectedIds] = useState<ReadonlySet<string>>(new Set());
  const [profileId, setProfileId] = useState<string | null>(null);
  const [groupId, setGroupId] = useState<string | null>(null);
  const [options, setOptions] = useState<WorkflowOptionsValue>(defaultWorkflowOptions());

  const [plan, setPlan] = useState<EnvironmentPlanView | null>(null);
  const [planning, setPlanning] = useState(false);
  const [planError, setPlanError] = useState<DesktopError | null>(null);
  const [plannedAt, setPlannedAt] = useState<number | null>(null);

  const [workflowId, setWorkflowId] = useState<string | null>(null);
  const [status, setStatus] = useState<WorkflowStatusView | null>(null);
  const [tasks, setTasks] = useState<TaskView[]>([]);
  const [statusError, setStatusError] = useState<DesktopError | null>(null);
  const [observedAt, setObservedAt] = useState(0);

  const [confirmExecute, setConfirmExecute] = useState(false);
  const [confirmCancel, setConfirmCancel] = useState(false);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<{ title: string; error: DesktopError } | null>(null);

  const workspaceIdRef = useRef(activeWorkspaceId);

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
  const profiles = useAsync<ProfileView[]>(
    () => {
      if (!activeWorkspace) {
        return Promise.resolve([]);
      }
      return desktopApi.listProfiles(activeWorkspace.id);
    },
    [activeWorkspace?.id],
    { enabled: connected && activeWorkspace !== null },
  );
  const groups = useAsync<GroupView[]>(
    () => {
      if (!activeWorkspace) {
        return Promise.resolve([]);
      }
      return desktopApi.listGroups(activeWorkspace.id);
    },
    [activeWorkspace?.id],
    { enabled: connected && activeWorkspace !== null },
  );
  const history = useAsync(
    () => {
      if (!activeWorkspace) {
        return Promise.resolve([]);
      }
      return desktopApi.listWorkflows(activeWorkspace.id);
    },
    [activeWorkspace?.id, terminalRevision],
    { enabled: connected && activeWorkspace !== null },
  );

  // Workspace switching scopes everything: no selection, plan, or workflow
  // state bleeds into another workspace (`tasks/022.md` §34).
  useEffect(() => {
    if (workspaceIdRef.current === activeWorkspaceId) {
      return;
    }
    workspaceIdRef.current = activeWorkspaceId;
    setMode('configure');
    setSource('services');
    setSelectedIds(new Set());
    setProfileId(null);
    setGroupId(null);
    setOptions(defaultWorkflowOptions());
    setPlan(null);
    setPlanError(null);
    setPlannedAt(null);
    setWorkflowId(null);
    setStatus(null);
    setTasks([]);
    setStatusError(null);
    setConfirmExecute(false);
    setConfirmCancel(false);
  }, [activeWorkspaceId]);

  // Cross-screen entry points preselect their source; they all land on this
  // same planning API (`tasks/022.md` §6).
  useEffect(() => {
    if (!intent) {
      return;
    }
    if (intent.workflowServiceIds && intent.workflowServiceIds.length > 0) {
      setSource('services');
      setSelectedIds(new Set(intent.workflowServiceIds));
      setProfileId(null);
      setGroupId(null);
      setMode('configure');
    } else if (intent.workflowProfileId) {
      setSource('profile');
      setProfileId(intent.workflowProfileId);
      setSelectedIds(new Set());
      setGroupId(null);
      setMode('configure');
    } else if (intent.workflowGroupId) {
      setSource('group');
      setGroupId(intent.workflowGroupId);
      setSelectedIds(new Set());
      setProfileId(null);
      setMode('configure');
    }
  }, [intent]);

  const selection = useMemo(() => {
    if (source === 'profile') {
      return { serviceIds: [] as string[], profileId, groupId: null };
    }
    if (source === 'group') {
      return { serviceIds: [] as string[], profileId: null, groupId };
    }
    return { serviceIds: [...selectedIds].sort(), profileId: null, groupId: null };
  }, [source, selectedIds, profileId, groupId]);

  const hasSelection =
    selection.serviceIds.length > 0 ||
    selection.profileId !== null ||
    selection.groupId !== null;

  const request = useMemo(
    () =>
      activeWorkspace
        ? toPrepareInput(activeWorkspace.id, selection, options)
        : null,
    [activeWorkspace, selection, options],
  );

  // Poll the running workflow, and refresh once whenever a task settles or the
  // connection returns. Only the daemon decides when the workflow is terminal.
  useEffect(() => {
    if (!workflowId || !connected) {
      return undefined;
    }
    if (mode !== 'running' && mode !== 'results') {
      return undefined;
    }
    let cancelled = false;
    const load = async () => {
      try {
        const view = await desktopApi.getWorkflowStatus(workflowId);
        if (cancelled) {
          return;
        }
        setStatus(view);
        setStatusError(null);
        if (view.state === 'queued' || view.state === 'running') {
          setObservedAt((current) => current || Math.floor(Date.now() / 1000));
        }
        const page = await desktopApi.listTasks(200);
        if (cancelled) {
          return;
        }
        setTasks(page.filter((task) => task.workflow_id === workflowId));
        if (mode === 'running' && isTerminalWorkflowState(view.state)) {
          setMode('results');
          history.reload();
        }
      } catch (cause) {
        if (!cancelled) {
          setStatusError(toDesktopError(cause));
        }
      }
    };
    void load();
    const timer = mode === 'running' ? setInterval(() => void load(), POLL_MS) : undefined;
    return () => {
      cancelled = true;
      if (timer !== undefined) {
        clearInterval(timer);
      }
    };
    // `history.reload` is stable (see useAsync).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, workflowId, connected, terminalRevision]);

  const runPlan = useCallback(async () => {
    if (!request) {
      return;
    }
    // The plan view renders the loading, error, and result states of this
    // single request, so it is entered up front.
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
  }, [request]);

  const runExecute = useCallback(async () => {
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
              'The daemon accepted the request but scheduled no workflow to follow. Nothing is shown as running.',
          },
        });
        return;
      }
      setWorkflowId(outcome.workflow_id);
      setStatus(null);
      setTasks([]);
      setStatusError(null);
      setObservedAt(Math.floor(Date.now() / 1000));
      setMode('running');
      history.reload();
    } catch (cause) {
      setFailure({ title: 'Unable to start the workflow', error: toDesktopError(cause) });
    } finally {
      setBusy(false);
    }
  }, [request, history]);

  const runCancel = useCallback(async () => {
    if (!workflowId) {
      return;
    }
    setConfirmCancel(false);
    setBusy(true);
    try {
      await desktopApi.cancelWorkflow(workflowId);
    } catch (cause) {
      setFailure({ title: 'Unable to cancel the workflow', error: toDesktopError(cause) });
    } finally {
      setBusy(false);
    }
  }, [workflowId]);

  const inspectWorkflow = useCallback(async (id: string) => {
    try {
      const view = await desktopApi.getWorkflowStatus(id);
      setWorkflowId(id);
      setStatus(view);
      setStatusError(null);
      setMode(isTerminalWorkflowState(view.state) ? 'results' : 'running');
      setObservedAt(Math.floor(Date.now() / 1000));
    } catch (cause) {
      setFailure({ title: 'Unable to read the workflow', error: toDesktopError(cause) });
    }
  }, []);

  const authorizationHint =
    failure && failure.error.code === 'PermissionDenied'
      ? 'The daemon refused this protected operation because it could not verify an authorized interactive request. A frontend confirmation is never authorization: the daemon checks the client kind and the peer user on every request, including while a plan is being executed.'
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
        description="Configure a workspace in ~/.service-orchestrator/config.toml to plan environment workflows."
      />
    );
  }

  const stalePlan =
    plannedAt !== null ? Math.floor(Date.now() / 1000) - plannedAt : null;

  return (
    <div className="page">
      <WorkflowWizard mode={mode} />

      {!connected ? (
        <div className="page__notice" role="status">
          Disconnected from the daemon. Planning and execution are disabled until the
          connection returns; the current workflow id is kept and its authoritative
          state is reloaded on reconnect. Nothing is replayed automatically.
        </div>
      ) : null}

      {mode === 'configure' ? (
        <>
          <div className="card">
            <div className="card__header">
              <h2 className="card__title">Prepare a development environment</h2>
              <span className="muted">Workspace: {activeWorkspace.name}</span>
            </div>
            <div className="bulk-bar" role="radiogroup" aria-label="Selection source">
              <span className="bulk-bar__count">Selection source</span>
              <div className="bulk-bar__select">
                {(['services', 'profile', 'group'] as const).map((value) => (
                  <label key={value} className="selection-list__source">
                    <input
                      type="radio"
                      name="workflow-source"
                      checked={source === value}
                      disabled={!connected}
                      onChange={() => {
                        setSource(value);
                        setSelectedIds(new Set());
                        setProfileId(null);
                        setGroupId(null);
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
                <ServiceSelection
                  services={services.data ?? []}
                  selectedIds={selectedIds}
                  disabled={!connected}
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
                  onToggleAll={(selected) =>
                    setSelectedIds(
                      selected
                        ? new Set((services.data ?? []).map((row) => row.id))
                        : new Set(),
                    )
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

            <WorkflowOptions
              value={options}
              disabled={!connected}
              onChange={setOptions}
            />

            <div className="detail-actions">
              <button
                type="button"
                className="button button--primary"
                disabled={!connected || !hasSelection || planning}
                onClick={() => void runPlan()}
              >
                {planning ? 'Planning…' : 'Preview plan…'}
              </button>
              {!hasSelection ? (
                <span className="muted" role="note">
                  Select services, a profile, or a group — never the whole workspace
                  implicitly.
                </span>
              ) : null}
            </div>
          </div>

          <WorkflowHistory
            rows={history.data ?? []}
            loading={history.loading}
            error={history.error}
            onRetry={history.reload}
            onSelect={(id) => void inspectWorkflow(id)}
          />
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
              <p className="muted" role="note">
                Plan generated by the daemon{stalePlan !== null ? ` ${stalePlan}s ago` : ''}.
                It is read-only and may become stale: the daemon re-validates every
                precondition and its current authorization decisions again at
                execution time.
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
                  className="button button--primary"
                  disabled={!connected || busy}
                  onClick={() => setConfirmExecute(true)}
                >
                  Execute workflow…
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
          {statusError ? <ErrorState error={statusError} /> : null}
          {!status && !statusError ? (
            <LoadingState label="Waiting for the daemon to report the workflow…" />
          ) : null}
          {status ? (
            <WorkflowProgress status={status} tasks={tasks} startedAt={observedAt} />
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
              disabled={!connected || busy || !status}
              onClick={() => setConfirmCancel(true)}
            >
              Cancel workflow…
            </button>
          </div>
          <p className="dialog__note muted">
            Workflow <strong>{workflowId}</strong> runs in the shared daemon; the same id
            is visible from the TUI and MCP.
          </p>
        </div>
      ) : null}

      {mode === 'results' ? (
        <div className="card">
          {statusError ? <ErrorState error={statusError} /> : null}
          {!status && !statusError ? (
            <LoadingState label="Loading workflow results…" />
          ) : null}
          {status ? (
            <WorkflowResults
              status={status}
              tasks={tasks}
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
                history.reload();
              }}
            >
              Back to workflows
            </button>
          </div>
        </div>
      ) : null}

      {confirmExecute && request && plan ? (
        <ConfirmationDialog
          title="Execute this workflow?"
          message={
            <>
              {plan.step_count} planned step
              {plan.step_count === 1 ? '' : 's'} across {plan.phases.length} phase
              {plan.phases.length === 1 ? '' : 's'} in workspace{' '}
              <strong>{activeWorkspace.name}</strong>. The daemon re-validates this
              request and its own authorization before scheduling anything.
            </>
          }
          confirmLabel="Execute workflow"
          busy={busy}
          onConfirm={() => void runExecute()}
          onCancel={() => setConfirmExecute(false)}
        >
          <dl className="dialog__facts">
            <div>
              <dt>Selection</dt>
              <dd>
                {selection.profileId
                  ? `profile ${selection.profileId}`
                  : selection.groupId
                    ? `group ${selection.groupId}`
                    : `${selection.serviceIds.length} service(s)`}
              </dd>
            </div>
            <div>
              <dt>Git sync</dt>
              <dd>{options.sync ? 'fetch' : 'skipped'}</dd>
            </div>
            <div>
              <dt>Migrations</dt>
              <dd>{options.run_liquibase ? 'will run' : 'not run'}</dd>
            </div>
            <div>
              <dt>Build / start</dt>
              <dd>
                {options.build ? 'build' : 'no build'}
                {' · '}
                {options.start ? 'start' : 'no start'}
              </dd>
            </div>
          </dl>
          <p className="dialog__note">
            This confirmation is intent, not authorization: the daemon independently
            verifies the client and the requesting user, and its decision is final. A
            stale plan is revalidated at execution time.
          </p>
        </ConfirmationDialog>
      ) : null}

      {confirmCancel && status ? (
        <WorkflowCancellationDialog
          status={status}
          busy={busy}
          onConfirm={() => void runCancel()}
          onCancel={() => setConfirmCancel(false)}
        />
      ) : null}

      {failure ? (
        <ErrorDialog
          title={failure.title}
          error={failure.error}
          labels={workflowId ? [workflowId] : []}
          hint={authorizationHint ?? undefined}
          onClose={() => setFailure(null)}
        />
      ) : null}
    </div>
  );
}
