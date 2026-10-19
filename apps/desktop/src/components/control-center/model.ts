import type {
  BuildTargetSummary,
  DesktopError,
  PrepareEnvironmentInput,
  RawServiceView,
  ServiceSummary,
  WorkflowStatusView,
} from '../../types/models';

/** How the runtime phase behaves. Start, restart, and stop are exclusive. */
export type ControlCenterRuntimeAction = 'none' | 'start' | 'restart' | 'stop';

/** The editable Control Center options; all map to real request fields. */
export interface ControlCenterOptions {
  /** Optional branch; sent only when Git synchronization checks out. */
  branch: string;
  sync: boolean;
  gitFetch: boolean;
  gitCheckout: boolean;
  gitPull: boolean;
  includeDependencies: boolean;
  buildLibraries: boolean;
  runLiquibase: boolean;
  migrationPreset: string;
  build: boolean;
  buildOperation: string;
  runtime: ControlCenterRuntimeAction;
  waitForHealth: boolean;
  healthChecks: boolean;
}

/** The saved, versioned Control Center state (never a secret or an authorization). */
export interface StoredControlCenterState {
  version: 1;
  workspaceId: string;
  serviceIds: string[];
  options: ControlCenterOptions;
}

/** The localStorage key. Bumping the version intentionally drops old state. */
export const CONTROL_CENTER_STORAGE_KEY = 'service-orchestrator.control-center.v1';

/**
 * Safe defaults: synchronize with the configured default branch, include
 * required dependencies, and change nothing else. Migrations, builds, service
 * startup, and standalone checks are explicit opt-ins.
 */
export function defaultControlCenterOptions(): ControlCenterOptions {
  return {
    branch: '',
    sync: true,
    gitFetch: true,
    gitCheckout: true,
    gitPull: true,
    includeDependencies: true,
    buildLibraries: false,
    runLiquibase: false,
    migrationPreset: '',
    build: false,
    buildOperation: '',
    runtime: 'none',
    waitForHealth: true,
    healthChecks: false,
  };
}

/** The quick actions offered on the main screen. */
export type QuickAction =
  | 'git-sync'
  | 'build'
  | 'start'
  | 'stop'
  | 'restart'
  | 'health'
  | 'full';

/** One quick action definition. */
export interface QuickActionDefinition {
  id: QuickAction;
  label: string;
  description: string;
}

/** Every quick action, in display order. */
export const QUICK_ACTIONS: readonly QuickActionDefinition[] = [
  {
    id: 'git-sync',
    label: 'Git Sync Only',
    description: 'Fetch, check out the configured branch, and fast-forward.',
  },
  {
    id: 'build',
    label: 'Build Only',
    description: 'Build the selected services with their configured operation.',
  },
  {
    id: 'start',
    label: 'Start Only',
    description: 'Start the selected services and wait for configured health.',
  },
  {
    id: 'stop',
    label: 'Stop Only',
    description: 'Stop the selected services in reverse dependency order.',
  },
  {
    id: 'restart',
    label: 'Restart Only',
    description: 'Stop, then start the selected services without rebuilding.',
  },
  {
    id: 'health',
    label: 'Health Check Only',
    description: 'Run configured health checks without starting anything.',
  },
  {
    id: 'full',
    label: 'Full Prepare Environment',
    description: 'Sync, build libraries and services, start, and wait for health.',
  },
] as const;

const NO_GIT = {
  sync: false,
  gitFetch: false,
  gitCheckout: false,
  gitPull: false,
} as const;

/**
 * Returns the options a quick action sets.
 *
 * A quick action is a named composition of existing request fields, not a new
 * execution path. Branch and build-operation overrides are carried only into
 * actions that use them.
 */
export function quickActionOptions(
  action: QuickAction,
  current: ControlCenterOptions = defaultControlCenterOptions(),
): ControlCenterOptions {
  const defaults = defaultControlCenterOptions();
  switch (action) {
    case 'git-sync':
      return {
        ...defaults,
        branch: current.branch,
        includeDependencies: current.includeDependencies,
      };
    case 'build':
      return {
        ...defaults,
        ...NO_GIT,
        includeDependencies: current.includeDependencies,
        build: true,
        buildOperation: current.buildOperation,
      };
    case 'start':
      return {
        ...defaults,
        ...NO_GIT,
        includeDependencies: current.includeDependencies,
        runtime: 'start',
        waitForHealth: true,
      };
    case 'stop':
      return {
        ...defaults,
        ...NO_GIT,
        includeDependencies: current.includeDependencies,
        runtime: 'stop',
        waitForHealth: false,
      };
    case 'restart':
      return {
        ...defaults,
        ...NO_GIT,
        includeDependencies: current.includeDependencies,
        runtime: 'restart',
        waitForHealth: true,
      };
    case 'health':
      return {
        ...defaults,
        ...NO_GIT,
        includeDependencies: current.includeDependencies,
        runtime: 'none',
        waitForHealth: false,
        healthChecks: true,
      };
    case 'full':
      return {
        ...defaults,
        branch: current.branch,
        includeDependencies: current.includeDependencies,
        buildLibraries: true,
        build: true,
        buildOperation: current.buildOperation,
        runtime: 'start',
        waitForHealth: true,
      };
  }
}

/** Whether any Git step would be scheduled. */
export function hasGitOperation(options: ControlCenterOptions): boolean {
  return (
    options.sync && (options.gitFetch || options.gitCheckout || options.gitPull)
  );
}

/** Whether the options request at least one real phase. */
export function hasOperation(options: ControlCenterOptions): boolean {
  return (
    hasGitOperation(options) ||
    options.buildLibraries ||
    options.runLiquibase ||
    options.build ||
    options.runtime !== 'none' ||
    options.healthChecks
  );
}

/**
 * Whether health checks conflict with the current runtime action.
 *
 * Stopping and checking health in one workflow would report an ambiguous
 * state, so the core rejects it; the UI prevents it before submission.
 */
export function healthChecksConflict(options: ControlCenterOptions): boolean {
  return options.healthChecks && options.runtime === 'stop';
}

/**
 * Builds the daemon request from the current selection and options.
 *
 * Only fields with meaning are sent: a branch without checkout, a preset
 * without migration, or a build operation without a build is never forwarded
 * as if it were active. The daemon re-validates everything at execution time.
 */
export function toControlCenterRequest(
  workspaceId: string,
  serviceIds: readonly string[],
  options: ControlCenterOptions,
): PrepareEnvironmentInput {
  const branch = options.branch.trim();
  const start = options.runtime === 'start';
  const restart = options.runtime === 'restart';
  const stop = options.runtime === 'stop';
  return {
    workspace_id: workspaceId,
    services: [...serviceIds].sort(),
    ...(branch !== '' && options.sync && options.gitCheckout ? { branch } : {}),
    sync: options.sync,
    git_fetch: options.sync && options.gitFetch,
    git_checkout: options.sync && options.gitCheckout,
    git_pull: options.sync && options.gitPull,
    include_dependencies: options.includeDependencies,
    build_libraries: options.buildLibraries,
    run_liquibase: options.runLiquibase,
    ...(options.runLiquibase && options.migrationPreset !== ''
      ? { migration_preset: options.migrationPreset }
      : {}),
    build: options.build,
    ...(options.build && options.buildOperation !== ''
      ? { build_operation: options.buildOperation }
      : {}),
    start,
    restart,
    stop,
    wait_for_health: options.waitForHealth && (start || restart),
    health_checks: options.healthChecks && !stop,
  };
}

/** A stable key for the current request, used to decide whether a plan is stale. */
export function requestKey(request: PrepareEnvironmentInput | null): string {
  return request ? JSON.stringify(request) : '';
}

/** Maps a wire request back onto options (used when loading a saved selection). */
export function optionsFromRequest(
  request: PrepareEnvironmentInput,
): ControlCenterOptions {
  return {
    branch: request.branch ?? '',
    sync: request.sync ?? true,
    gitFetch: request.git_fetch ?? true,
    gitCheckout: request.git_checkout ?? true,
    gitPull: request.git_pull ?? true,
    includeDependencies: request.include_dependencies ?? true,
    buildLibraries: request.build_libraries ?? false,
    runLiquibase: request.run_liquibase ?? false,
    migrationPreset: request.migration_preset ?? '',
    build: request.build ?? false,
    buildOperation: request.build_operation ?? '',
    runtime: request.restart
      ? 'restart'
      : request.start
        ? 'start'
        : request.stop
          ? 'stop'
          : 'none',
    waitForHealth: request.wait_for_health ?? true,
    healthChecks: request.health_checks ?? false,
  };
}

function readBoolean(value: unknown, fallback: boolean): boolean {
  return typeof value === 'boolean' ? value : fallback;
}

function readString(value: unknown, fallback: string): string {
  return typeof value === 'string' ? value : fallback;
}

function readRuntime(value: unknown): ControlCenterRuntimeAction {
  return value === 'start' || value === 'restart' || value === 'stop' ? value : 'none';
}

/** Validates a persisted options object field by field, falling back safely. */
export function normalizeOptions(value: unknown): ControlCenterOptions {
  const defaults = defaultControlCenterOptions();
  if (typeof value !== 'object' || value === null) {
    return defaults;
  }
  const source = value as Record<string, unknown>;
  return {
    branch: readString(source.branch, defaults.branch),
    sync: readBoolean(source.sync, defaults.sync),
    gitFetch: readBoolean(source.gitFetch, defaults.gitFetch),
    gitCheckout: readBoolean(source.gitCheckout, defaults.gitCheckout),
    gitPull: readBoolean(source.gitPull, defaults.gitPull),
    includeDependencies: readBoolean(
      source.includeDependencies,
      defaults.includeDependencies,
    ),
    buildLibraries: readBoolean(source.buildLibraries, defaults.buildLibraries),
    runLiquibase: readBoolean(source.runLiquibase, defaults.runLiquibase),
    migrationPreset: readString(source.migrationPreset, defaults.migrationPreset),
    build: readBoolean(source.build, defaults.build),
    buildOperation: readString(source.buildOperation, defaults.buildOperation),
    runtime: readRuntime(source.runtime),
    waitForHealth: readBoolean(source.waitForHealth, defaults.waitForHealth),
    healthChecks: readBoolean(source.healthChecks, defaults.healthChecks),
  };
}

/**
 * Reads the saved selection and options.
 *
 * Loading is inert: it returns data for the caller to apply to local state and
 * never calls the daemon. Unknown versions, malformed JSON, and storage
 * failures all yield `null`.
 */
export function loadControlCenterState(): StoredControlCenterState | null {
  if (typeof window === 'undefined' || window.localStorage === undefined) {
    return null;
  }
  try {
    const raw = window.localStorage.getItem(CONTROL_CENTER_STORAGE_KEY);
    if (!raw) {
      return null;
    }
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== 'object' || parsed === null) {
      return null;
    }
    const source = parsed as Record<string, unknown>;
    if (source.version !== 1 || typeof source.workspaceId !== 'string') {
      return null;
    }
    const serviceIds = Array.isArray(source.serviceIds)
      ? source.serviceIds.filter((id): id is string => typeof id === 'string')
      : [];
    return {
      version: 1,
      workspaceId: source.workspaceId,
      serviceIds,
      options: normalizeOptions(source.options),
    };
  } catch {
    return null;
  }
}

/**
 * Persists the selection and options only.
 *
 * No credentials, authorization, plan, or workflow id are stored, and saving is
 * best-effort: a full or unavailable localStorage never breaks the screen.
 */
export function saveControlCenterState(state: StoredControlCenterState): void {
  if (typeof window === 'undefined' || window.localStorage === undefined) {
    return;
  }
  try {
    window.localStorage.setItem(CONTROL_CENTER_STORAGE_KEY, JSON.stringify(state));
  } catch {
    // Storage is an optimization, never a source of truth.
  }
}

/** A human-readable label for a build operation. */
export function buildOperationLabel(operation: string): string {
  switch (operation) {
    case 'clean':
      return 'Clean';
    case 'build':
      return 'Build';
    case 'test':
      return 'Test';
    case 'install_dependencies':
      return 'Install dependencies';
    case 'publish_local_artifact':
      return 'Publish local artifact';
    default:
      return operation;
  }
}

/**
 * Returns actionable guidance for a structured daemon failure.
 *
 * The mapping is presentation only: it explains what a stable error code means
 * and what the developer can do, without changing or retrying anything.
 */
export function actionableHint(error: DesktopError): string | null {
  const code = error.code;
  if (code === 'GIT_WORKING_TREE_DIRTY') {
    return 'Commit, stash, or discard the local changes yourself. The Control Center never runs a destructive Git reset or clean automatically.';
  }
  if (code === 'GIT_BRANCH_NOT_FOUND') {
    return 'Check the target branch spelling, or fetch first so the remote branch exists locally.';
  }
  if (code === 'GIT_DETACHED_HEAD') {
    return 'The repository is in detached-HEAD state. Check out a branch yourself; the workflow only fast-forwards the current branch.';
  }
  if (code === 'GIT_NOT_CLONED' || code === 'GIT_INVALID_REPOSITORY') {
    return 'The configured path is not a valid checkout. Clone the repository or correct the service path.';
  }
  if (code === 'GIT_DIVERGED_BRANCH') {
    return 'The local and remote branches diverged. Reconcile them manually; the workflow only fast-forwards.';
  }
  if (code === 'GIT_UPSTREAM_NOT_CONFIGURED') {
    return 'Configure an upstream branch for the checkout before requesting a fast-forward pull.';
  }
  if (code === 'GIT_UNAVAILABLE') {
    return 'Install Git and make sure it is on the application PATH, then reconnect.';
  }
  if (
    code === 'BUILD_PROVIDER_UNAVAILABLE' ||
    code === 'BUILD_COMMAND_NOT_FOUND' ||
    code === 'BUILD_PROCESS_FAILED' ||
    code === 'BUILD_UNSUPPORTED_SYSTEM' ||
    code === 'BUILD_UNSUPPORTED_OPERATION' ||
    code === 'BUILD_INVALID_TARGET' ||
    code === 'BUILD_MISSING_PARAMETER' ||
    code === 'BUILD_INVALID_PARAMETER' ||
    code === 'BUILD_INTERNAL_ERROR'
  ) {
    return 'The configured build provider or toolchain cannot run this request. Check the runtime toolchain (Maven or Node/npm), the build system, and the service configuration.';
  }
  if (code === 'RUNTIME_NOT_FOUND') {
    return 'The configured runtime is not installed or is not on the application PATH. Install the pinned Java or Node version, then reconnect.';
  }
  if (code === 'RUNTIME_VERSION_MISMATCH') {
    return 'The installed Java or Node version differs from the configured pin. Install the pinned version or update the service configuration.';
  }
  if (
    code === 'MAVEN_BUILD_FAILED' ||
    code === 'NPM_BUILD_FAILED' ||
    code === 'BUILD_FAILED' ||
    code === 'BUILD_COMMAND_FAILED'
  ) {
    return 'Open the failed task logs for the compiler or test output. The workflow does not retry automatically.';
  }
  if (code === 'BUILD_TIMEOUT') {
    return 'The build exceeded its configured timeout. Inspect the task logs, then adjust the timeout or the build itself. Nothing is retried automatically.';
  }
  if (code === 'LIBRARY_BUILD_FAILED' || code === 'ARTIFACT_INSTALLATION_FAILED') {
    return 'Open the library task logs. Required libraries are built in dependency order before services.';
  }
  if (code === 'ARTIFACT_NOT_FOUND') {
    return 'The expected artifact is missing from the local Maven repository. Build and install the library, then verify it again.';
  }
  if (code === 'HEALTH_CHECK_TIMEOUT' || code === 'HEALTH_NOT_READY') {
    return 'The service is running but did not become ready in time. Inspect its logs and configured health check, or increase the timeout in configuration.';
  }
  if (code === 'HEALTH_UNREACHABLE' || code === 'HEALTH_CHECK_CONNECTION_REFUSED') {
    return 'The health endpoint is unreachable. Confirm the service is listening on the configured port and URL.';
  }
  if (
    code === 'HEALTH_CONFIGURATION_INVALID' ||
    code === 'HEALTH_CHECK_UNSUPPORTED' ||
    code === 'HEALTH_CHECK_INVALID_RESPONSE'
  ) {
    return 'The configured health check is invalid or returned an unexpected response. Review its type, URL, port, and expected status in configuration.';
  }
  if (code === 'PermissionDenied') {
    return 'The daemon refused a protected operation. Authorization is evaluated by the daemon for this interactive user; a frontend confirmation is not authorization.';
  }
  if (
    code === 'MIGRATION_DATABASE_TARGET_UNKNOWN' ||
    code === 'MIGRATION_DATABASE_TARGET_NOT_AUTHORIZED' ||
    code === 'MIGRATION_INVALID_TARGET'
  ) {
    return 'The daemon could not verify the declared database environment for every target. Declare a local environment and review the migration preset before retrying.';
  }
  if (code === 'PORT_CONFLICT' || code === 'PORT_IN_USE' || code === 'RUNTIME_PORT_ALREADY_IN_USE') {
    return 'Another process owns the configured port. Stop it or change the service port in configuration.';
  }
  if (code === 'DAEMON_UNAVAILABLE' || code === 'CONNECTION_LOST') {
    return 'Reconnect to the shared daemon. The current selection is preserved, and no operation is replayed automatically.';
  }
  if (code === 'InvalidOperation') {
    return 'Review the selected services and operations. The daemon rejected a capability or sequencing rule before scheduling anything.';
  }
  return null;
}

/**
 * Filters services for the selection modal.
 *
 * Search matches the identifier or display name; the kind filter is exact.
 * Filtering never changes the selection, so checked services stay checked while
 * the user narrows the list (`tasks/033.md` §4).
 */
export function filterServices(
  services: readonly ServiceSummary[],
  search: string,
  kind: string,
): ServiceSummary[] {
  const needle = search.trim().toLowerCase();
  return services.filter((service) => {
    if (
      needle !== '' &&
      !service.id.toLowerCase().includes(needle) &&
      !service.display_name.toLowerCase().includes(needle)
    ) {
      return false;
    }
    return kind === 'all' || service.kind === kind;
  });
}

/** The distinct service kinds present in a workspace, sorted for display. */
export function serviceKinds(services: readonly ServiceSummary[]): string[] {
  return Array.from(new Set(services.map((service) => service.kind))).sort();
}

/** A compact runtime label for one build target (`java 21 · maven 21`). */
export function serviceRuntimeLabel(
  target: BuildTargetSummary | undefined,
): string {
  if (!target || target.runtimes.length === 0) {
    return '';
  }
  return target.runtimes
    .map((runtime) =>
      runtime.version ? `${runtime.kind} ${runtime.version}` : runtime.kind,
    )
    .join(' · ');
}

/**
 * The build operations every selected service's provider exposes.
 *
 * The union comes from the daemon's capability rows; the UI never hard-codes a
 * build system or assumes Maven (`tasks/033.md` §9). An operation absent here
 * is not offered, and the daemon still rejects an unsupported override.
 */
export function exposedBuildOperations(
  targets: readonly BuildTargetSummary[],
): string[] {
  const operations: string[] = [];
  for (const target of targets) {
    for (const operation of target.capability?.operations ?? []) {
      if (operation.exposed && !operations.includes(operation.operation)) {
        operations.push(operation.operation);
      }
    }
  }
  return operations;
}

/** The configured default branch of one raw service, when it declares one. */
function rawDefaultBranch(service: RawServiceView): string | null {
  return service.default_branch ?? service.repository?.default_branch ?? null;
}

/**
 * The distinct default branches of the selected services, from configuration.
 *
 * The configuration snapshot spans every workspace while service ids are only
 * workspace-scoped, so the workspace is part of the lookup. An empty result
 * means either no service is selected yet or none declares a default branch.
 * The request omits `branch` then, so each repository uses its own configured
 * default rather than an invented value.
 */
export function configuredDefaultBranches(
  workspaceId: string,
  serviceIds: readonly string[],
  rawServices: readonly RawServiceView[],
): string[] {
  const selected = new Set(serviceIds);
  const branches: string[] = [];
  for (const service of rawServices) {
    if (service.workspace !== workspaceId || !selected.has(service.id)) {
      continue;
    }
    const branch = rawDefaultBranch(service);
    if (branch !== null && !branches.includes(branch)) {
      branches.push(branch);
    }
  }
  return branches;
}

/**
 * The phase a running workflow is currently working on.
 *
 * This is presentation over the daemon's own phase counters: the first phase
 * with unfinished work, otherwise the last phase. No state is inferred beyond
 * what the status already reports.
 */
export function currentPhase(status: WorkflowStatusView | null): string | null {
  if (!status || status.phases.length === 0) {
    return null;
  }
  const active = status.phases.find(
    (phase) => phase.pending > 0 && phase.failed === 0,
  );
  return active?.phase ?? status.phases[status.phases.length - 1].phase;
}
