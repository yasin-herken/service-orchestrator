import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import type {
  ArtifactVerificationView,
  BranchView,
  BuildCapabilitiesView,
  BuildTargetSummary,
  BulkSummaryView,
  ConfigChangeView,
  ConfigurationChangedEvent,
  ConfigurationCommitView,
  ConfigurationPreviewView,
  ConfigurationReloadView,
  ConfigurationSnapshotView,
  ConfigurationValidationView,
  ConnectionSnapshot,
  DaemonStatus,
  EnvironmentPlanView,
  GitStatusView,
  GitSyncMode,
  GitSyncPreview,
  GroupView,
  HealthDetailView,
  HealthRowView,
  LibraryDependentView,
  LibrarySummary,
  LogPageView,
  LogStreamEvent,
  MigrationRowView,
  MigrationStatusView,
  OperationOutcome,
  PrepareEnvironmentInput,
  ProfileView,
  ProjectSuggestionView,
  RepositoryInspectionView,
  RepositoryProbeView,
  RepositorySummary,
  ServiceDetail,
  ServiceSummary,
  TaskView,
  VersionAlignmentView,
  WorkflowCancellationView,
  WorkflowStatusView,
  WorkflowSummaryView,
  WorkspaceStatus,
  WorkspaceSummary,
  WorkflowView,
} from '../types/models';
import {
  CONFIGURATION_EVENT,
  DAEMON_CONNECTION_EVENT,
  LOG_EVENT,
  TASK_EVENT,
  type TaskEvent,
} from '../types/models';
import { toDesktopError } from './errors';

/** The options for the prepare-library workflows. */
export interface PrepareOptions {
  /** Whether to synchronize the repository first. */
  sync: boolean;
  /** An optional branch to check out after synchronizing. */
  branch: string | null;
  build: boolean;
  install: boolean;
  verify: boolean;
}

/** A bounded, filterable request for a task's logs. */
export interface TaskLogsQuery {
  /** Return entries after this sequence cursor (follow mode). */
  after?: number;
  /** The maximum number of entries to return. */
  limit?: number;
  /** Only return entries at or above this severity. */
  minimumLevel?: string;
  /** Only return entries whose message contains this text. */
  text?: string;
}

/**
 * A bounded, filterable request for the unified Logs screen.
 *
 * The scope is hierarchical: `taskId` selects that task's logs, otherwise
 * `serviceId` selects that service's logs, otherwise the workspace-wide
 * application log is queried (`tasks/021.md` §23).
 */
export interface LogsQuery {
  taskId?: string;
  serviceId?: string;
  workspaceId?: string;
  /** Only return entries from this source (`process`, `task`, `build`, …). */
  source?: string;
  /** Only return entries at or above this severity. */
  minimumLevel?: string;
  /** Only return entries whose message contains this text. */
  text?: string;
  /** Return entries after this sequence cursor (follow mode). */
  after?: number;
  /** The maximum number of entries to return (the daemon clamps it). */
  limit?: number;
}

/**
 * The typed desktop API consumed by React.
 *
 * Every method maps one-to-one onto a Tauri command that proxies an existing
 * IPC operation to the shared daemon (`tasks/018.md` §7, §12; `tasks/019.md`
 * §3, §10). React never talks to the operating system directly, and the mutating
 * methods never start, stop, or restart anything themselves: they schedule the
 * same application use cases the TUI and MCP use and return the daemon's
 * normalized outcome.
 */
export interface OrchestratorDesktopApi {
  /** Daemon diagnostic status (starts/connects the daemon on first use). */
  getDaemonStatus(): Promise<DaemonStatus>;
  /** All configured workspaces. */
  listWorkspaces(): Promise<WorkspaceSummary[]>;
  /** Service rows for one workspace. */
  listServices(workspaceId: string): Promise<ServiceSummary[]>;
  /** A workspace with the status of each of its services. */
  getWorkspaceStatus(workspaceId: string): Promise<WorkspaceStatus>;
  /** One service's status, validated against its workspace. */
  getServiceStatus(workspaceId: string, serviceId: string): Promise<ServiceSummary>;
  /** One service's configuration, observed state, and diagnostics. */
  getServiceDetail(workspaceId: string, serviceId: string): Promise<ServiceDetail>;
  /** Schedules a start for one service. */
  startService(serviceId: string): Promise<OperationOutcome>;
  /** Schedules a stop for one service. */
  stopService(serviceId: string): Promise<OperationOutcome>;
  /** Schedules the daemon-coordinated restart of one service. */
  restartService(serviceId: string): Promise<OperationOutcome>;
  /** Schedules a start workflow for the selected services of a workspace. */
  startServices(workspaceId: string, serviceIds: string[]): Promise<OperationOutcome>;
  /** Schedules a stop workflow for the selected services of a workspace. */
  stopServices(workspaceId: string, serviceIds: string[]): Promise<OperationOutcome>;
  /** Schedules a restart workflow for the selected services of a workspace. */
  restartServices(workspaceId: string, serviceIds: string[]): Promise<OperationOutcome>;
  /** Runs every configured health check for one service. */
  checkServiceHealth(serviceId: string): Promise<OperationOutcome>;
  /** Configured repositories for one workspace (no Git operation). */
  listRepositories(workspaceId: string): Promise<RepositorySummary[]>;
  /** One repository's working-tree status, inspected on demand. */
  getGitStatus(serviceId: string): Promise<GitStatusView>;
  /** Several repositories' statuses, each reported independently. */
  inspectRepositories(
    workspaceId: string,
    serviceIds: string[],
  ): Promise<RepositoryInspectionView[]>;
  /** A repository's local and remote-tracking branches. */
  listBranches(serviceId: string): Promise<BranchView[]>;
  /** A read-only preview of a bulk Git operation over a selection. */
  previewGitSync(
    workspaceId: string,
    serviceIds: string[],
    targetBranch: string,
    mode: GitSyncMode,
  ): Promise<GitSyncPreview>;
  /** Fetches one repository. */
  fetchRepository(serviceId: string): Promise<OperationOutcome>;
  /** Fast-forward pulls one repository. */
  pullRepository(serviceId: string): Promise<OperationOutcome>;
  /** Checks out a branch in one repository. */
  checkoutRepository(serviceId: string, branch: string): Promise<OperationOutcome>;
  /** Synchronizes one repository (fetch, checkout, fast-forward). */
  syncRepository(serviceId: string): Promise<OperationOutcome>;
  /** Fetches the selected repositories as one workflow. */
  fetchRepositories(workspaceId: string, serviceIds: string[]): Promise<OperationOutcome>;
  /** Synchronizes the selected repositories as one workflow. */
  syncRepositories(workspaceId: string, serviceIds: string[]): Promise<OperationOutcome>;
  /** Checks out a branch in the selected repositories as one workflow. */
  checkoutRepositories(
    workspaceId: string,
    serviceIds: string[],
    branch: string,
  ): Promise<OperationOutcome>;
  /** Build targets with their provider capabilities. */
  listBuildTargets(workspaceId: string): Promise<BuildTargetSummary[]>;
  /** The build operations one service's build system supports. */
  getBuildCapabilities(serviceId: string): Promise<BuildCapabilitiesView>;
  /** Builds one service. */
  buildService(serviceId: string): Promise<OperationOutcome>;
  /** Builds the selected services as one workflow. */
  buildServices(workspaceId: string, serviceIds: string[]): Promise<OperationOutcome>;
  /** A workspace's libraries with observed state and dependents. */
  listLibraries(workspaceId: string): Promise<LibrarySummary[]>;
  /** The services that depend on one library. */
  getLibraryDependents(libraryId: string): Promise<LibraryDependentView[]>;
  /** Verifies a library's artifact in the local repository. */
  verifyLibraryArtifact(libraryId: string): Promise<ArtifactVerificationView>;
  /** Inspects a library's consumer dependency version alignment. */
  inspectVersionAlignment(libraryId: string): Promise<VersionAlignmentView>;
  /** Builds one library. */
  buildLibrary(libraryId: string): Promise<OperationOutcome>;
  /** Installs one library's artifact into the local repository. */
  installLibrary(libraryId: string): Promise<OperationOutcome>;
  /** Synchronizes one library's repository. */
  syncLibrary(libraryId: string): Promise<OperationOutcome>;
  /** Runs the prepare-library workflow for one library. */
  prepareLibrary(
    libraryId: string,
    options: PrepareOptions,
  ): Promise<OperationOutcome>;
  /** Prepares the selected libraries as one dependency-ordered workflow. */
  prepareLibraries(
    workspaceId: string,
    libraryIds: string[],
    options: PrepareOptions,
  ): Promise<OperationOutcome>;
  /** One task by id. */
  getTask(taskId: string): Promise<TaskView>;
  /** Aggregates a workflow's tasks into a bulk summary. */
  summarizeWorkflow(workflowId: string): Promise<BulkSummaryView>;
  /** One bounded page of a task's structured logs. */
  getTaskLogs(taskId: string, query?: TaskLogsQuery): Promise<LogPageView>;
  /** Requests cancellation of one task and returns its latest state. */
  cancelTask(taskId: string): Promise<TaskView>;
  /** Requests cancellation of every active task in a workflow. */
  cancelWorkflow(workflowId: string): Promise<WorkflowCancellationView>;
  /** Active tasks plus a bounded slice of recent history, newest first. */
  listTasks(limit?: number): Promise<TaskView[]>;
  /** One workflow's declared steps and dependency edges. */
  getWorkflow(workflowId: string): Promise<WorkflowView>;
  /** The workspace's profiles, expanded with their resolved services. */
  listProfiles(workspaceId: string): Promise<ProfileView[]>;
  /** One profile, validated against its workspace. */
  getProfile(workspaceId: string, profileId: string): Promise<ProfileView>;
  /** The workspace's service groups with their members. */
  listGroups(workspaceId: string): Promise<GroupView[]>;
  /** One service group, validated against its workspace. */
  getGroup(workspaceId: string, groupId: string): Promise<GroupView>;
  /**
   * A read-only environment plan for a selection.
   *
   * Planning executes nothing; the daemon computes the phases, steps, and
   * dependency closure from configuration (`tasks/022.md` §12).
   */
  planPrepareEnvironment(input: PrepareEnvironmentInput): Promise<EnvironmentPlanView>;
  /**
   * Schedules the prepare-environment workflow for a selection.
   *
   * The daemon re-validates the request and its policy at execution time; the
   * client-side flags are never treated as authorization.
   */
  prepareEnvironment(input: PrepareEnvironmentInput): Promise<OperationOutcome>;
  /** A live snapshot of one workflow's phases, steps, and failures. */
  getWorkflowStatus(workflowId: string): Promise<WorkflowStatusView>;
  /** Bounded workspace-scoped workflow history, newest first. */
  listWorkflows(workspaceId: string, limit?: number): Promise<WorkflowSummaryView[]>;
  /** A workspace's migration-enabled services with their latest task. */
  listMigrations(workspaceId: string): Promise<MigrationRowView[]>;
  /** One service's configuration-derived migration snapshot. */
  getMigrationStatus(serviceId: string): Promise<MigrationStatusView>;
  /** Schedules a migration-configuration validation task. */
  validateMigration(serviceId: string): Promise<OperationOutcome>;
  /** Schedules a migration-status inspection task. */
  inspectMigrationStatus(serviceId: string): Promise<OperationOutcome>;
  /** Schedules a pending-changeset inspection task. */
  inspectMigrationPending(serviceId: string): Promise<OperationOutcome>;
  /** Schedules a read-only SQL preview task. */
  previewMigration(serviceId: string): Promise<OperationOutcome>;
  /**
   * Applies one service's migrations.
   *
   * `confirmed` is the user's explicit confirmation; the daemon independently
   * verifies the client kind and peer identity, and its policy decision is
   * final (`tasks/021.md` §8).
   */
  applyMigration(serviceId: string, confirmed: boolean): Promise<OperationOutcome>;
  /** Applies the selected services' migrations as one coordinated workflow. */
  applySelectedMigrations(
    workspaceId: string,
    serviceIds: string[],
    confirmed: boolean,
  ): Promise<OperationOutcome>;
  /** A workspace's services with observed runtime and health state. */
  listHealth(workspaceId: string): Promise<HealthRowView[]>;
  /** One service's detailed health view, validated against its workspace. */
  getHealth(workspaceId: string, serviceId: string): Promise<HealthDetailView>;
  /** Runs only the required readiness checks for one service. */
  checkServiceReadiness(serviceId: string): Promise<OperationOutcome>;
  /** One bounded, cursor-paginated unified log query. */
  getLogs(query?: LogsQuery): Promise<LogPageView>;
  /** Attaches a realtime subscription to a service's log stream. */
  subscribeServiceLogs(subscriptionId: string, serviceId: string): Promise<void>;
  /** Detaches a realtime subscription; unknown ids are ignored. */
  unsubscribeServiceLogs(subscriptionId: string): Promise<void>;
  /**
   * Reads the active configuration as a raw, editable snapshot.
   *
   * The daemon is the only configuration reader-writer; the GUI parses no TOML
   * and never writes a file (`tasks/023.md` §38, §41).
   */
  getConfiguration(): Promise<ConfigurationSnapshotView>;
  /** Validates the active configuration file without changing it. */
  validateConfiguration(): Promise<ConfigurationValidationView>;
  /**
   * Previews a proposed change without committing it.
   *
   * The daemon computes the diff, validation, affected references, and
   * running-service impact; `expectedRevision` is the snapshot revision the
   * editor was based on, and a stale value is reported as `CONFIG_CONFLICT`.
   */
  previewConfiguration(
    change: ConfigChangeView,
    expectedRevision: string,
  ): Promise<ConfigurationPreviewView>;
  /**
   * Validates and persists a configuration change.
   *
   * `confirmed` expresses the user's intent for a removal; the daemon
   * independently verifies the client kind and peer identity and its policy
   * decision is final — a confirmation boolean is never trusted authorization
   * (`tasks/023.md` §30, §46).
   */
  commitConfiguration(
    change: ConfigChangeView,
    expectedRevision: string,
    confirmed: boolean,
  ): Promise<ConfigurationCommitView>;
  /** Re-reads and validates the configuration file from disk. */
  reloadConfiguration(): Promise<ConfigurationReloadView>;
  /**
   * Inspects a local repository directory, read-only.
   *
   * The daemon validates the path (absolute, no traversal) and returns
   * structured observations; no build, script, or Git command runs
   * (`tasks/023.md` §17, §20).
   */
  inspectRepository(path: string): Promise<RepositoryProbeView>;
  /** Detects a suggested service configuration from a repository. */
  detectProjectConfiguration(path: string): Promise<ProjectSuggestionView>;
  /** The adapter's current daemon connection state. */
  getConnectionState(): Promise<ConnectionSnapshot>;
  /** Subscribes to daemon connection changes; resolves to an unsubscriber. */
  onConnectionChanged(handler: (snapshot: ConnectionSnapshot) => void): Promise<UnlistenFn>;
  /** Subscribes to forwarded task-engine events; resolves to an unsubscriber. */
  onTaskEvent(handler: (event: TaskEvent) => void): Promise<UnlistenFn>;
  /** Subscribes to realtime log-stream events; resolves to an unsubscriber. */
  onLogEvent(handler: (event: LogStreamEvent) => void): Promise<UnlistenFn>;
  /** Subscribes to active-configuration changes; resolves to an unsubscriber. */
  onConfigurationChanged(
    handler: (event: ConfigurationChangedEvent) => void,
  ): Promise<UnlistenFn>;
}

/** Invokes a Tauri command, normalizing rejections into `DesktopError`. */
async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw toDesktopError(error);
  }
}

/** Subscribes to one adapter event, normalizing listener failures. */
async function subscribe<P>(
  event: string,
  handler: (payload: P) => void,
): Promise<UnlistenFn> {
  try {
    return await listen<P>(event, (e) => handler(e.payload));
  } catch (error) {
    throw toDesktopError(error);
  }
}

/** The real, Tauri-backed desktop API. */
export const desktopApi: OrchestratorDesktopApi = {
  getDaemonStatus: () => call<DaemonStatus>('get_daemon_status'),
  listWorkspaces: () => call<WorkspaceSummary[]>('list_workspaces'),
  listServices: (workspaceId) =>
    call<ServiceSummary[]>('list_services', { workspaceId }),
  getWorkspaceStatus: (workspaceId) =>
    call<WorkspaceStatus>('get_workspace_status', { workspaceId }),
  getServiceStatus: (workspaceId, serviceId) =>
    call<ServiceSummary>('get_service_status', { workspaceId, serviceId }),
  getServiceDetail: (workspaceId, serviceId) =>
    call<ServiceDetail>('get_service_detail', { workspaceId, serviceId }),
  startService: (serviceId) => call<OperationOutcome>('start_service', { serviceId }),
  stopService: (serviceId) => call<OperationOutcome>('stop_service', { serviceId }),
  restartService: (serviceId) =>
    call<OperationOutcome>('restart_service', { serviceId }),
  startServices: (workspaceId, serviceIds) =>
    call<OperationOutcome>('start_services', { workspaceId, serviceIds }),
  stopServices: (workspaceId, serviceIds) =>
    call<OperationOutcome>('stop_services', { workspaceId, serviceIds }),
  restartServices: (workspaceId, serviceIds) =>
    call<OperationOutcome>('restart_services', { workspaceId, serviceIds }),
  checkServiceHealth: (serviceId) =>
    call<OperationOutcome>('check_service_health', { serviceId }),
  listRepositories: (workspaceId) =>
    call<RepositorySummary[]>('list_repositories', { workspaceId }),
  getGitStatus: (serviceId) => call<GitStatusView>('get_git_status', { serviceId }),
  inspectRepositories: (workspaceId, serviceIds) =>
    call<RepositoryInspectionView[]>('inspect_repositories', { workspaceId, serviceIds }),
  listBranches: (serviceId) => call<BranchView[]>('list_branches', { serviceId }),
  previewGitSync: (workspaceId, serviceIds, targetBranch, mode) =>
    call<GitSyncPreview>('preview_git_sync', {
      workspaceId,
      serviceIds,
      targetBranch,
      mode,
    }),
  fetchRepository: (serviceId) => call<OperationOutcome>('fetch_repository', { serviceId }),
  pullRepository: (serviceId) => call<OperationOutcome>('pull_repository', { serviceId }),
  checkoutRepository: (serviceId, branch) =>
    call<OperationOutcome>('checkout_repository', { serviceId, branch }),
  syncRepository: (serviceId) => call<OperationOutcome>('sync_repository', { serviceId }),
  fetchRepositories: (workspaceId, serviceIds) =>
    call<OperationOutcome>('fetch_repositories', { workspaceId, serviceIds }),
  syncRepositories: (workspaceId, serviceIds) =>
    call<OperationOutcome>('sync_repositories', { workspaceId, serviceIds }),
  checkoutRepositories: (workspaceId, serviceIds, branch) =>
    call<OperationOutcome>('checkout_repositories', { workspaceId, serviceIds, branch }),
  listBuildTargets: (workspaceId) =>
    call<BuildTargetSummary[]>('list_build_targets', { workspaceId }),
  getBuildCapabilities: (serviceId) =>
    call<BuildCapabilitiesView>('get_build_capabilities', { serviceId }),
  buildService: (serviceId) => call<OperationOutcome>('build_service', { serviceId }),
  buildServices: (workspaceId, serviceIds) =>
    call<OperationOutcome>('build_services', { workspaceId, serviceIds }),
  listLibraries: (workspaceId) =>
    call<LibrarySummary[]>('list_libraries', { workspaceId }),
  getLibraryDependents: (libraryId) =>
    call<LibraryDependentView[]>('get_library_dependents', { libraryId }),
  verifyLibraryArtifact: (libraryId) =>
    call<ArtifactVerificationView>('verify_library_artifact', { libraryId }),
  inspectVersionAlignment: (libraryId) =>
    call<VersionAlignmentView>('inspect_version_alignment', { libraryId }),
  buildLibrary: (libraryId) => call<OperationOutcome>('build_library', { libraryId }),
  installLibrary: (libraryId) => call<OperationOutcome>('install_library', { libraryId }),
  syncLibrary: (libraryId) => call<OperationOutcome>('sync_library', { libraryId }),
  prepareLibrary: (libraryId, options) =>
    call<OperationOutcome>('prepare_library', { libraryId, ...options }),
  prepareLibraries: (workspaceId, libraryIds, options) =>
    call<OperationOutcome>('prepare_libraries', { workspaceId, libraryIds, ...options }),
  getTask: (taskId) => call<TaskView>('get_task', { taskId }),
  summarizeWorkflow: (workflowId) =>
    call<BulkSummaryView>('summarize_workflow', { workflowId }),
  getTaskLogs: (taskId, query = {}) =>
    call<LogPageView>('get_task_logs', {
      taskId,
      after: query.after,
      limit: query.limit,
      minimumLevel: query.minimumLevel,
      text: query.text,
    }),
  cancelTask: (taskId) => call<TaskView>('cancel_task', { taskId }),
  cancelWorkflow: (workflowId) =>
    call<WorkflowCancellationView>('cancel_workflow', { workflowId }),
  listTasks: (limit) => call<TaskView[]>('list_tasks', { limit }),
  getWorkflow: (workflowId) => call<WorkflowView>('get_workflow', { workflowId }),
  listProfiles: (workspaceId) =>
    call<ProfileView[]>('list_profiles', { workspaceId }),
  getProfile: (workspaceId, profileId) =>
    call<ProfileView>('get_profile', { workspaceId, profileId }),
  listGroups: (workspaceId) => call<GroupView[]>('list_groups', { workspaceId }),
  getGroup: (workspaceId, groupId) =>
    call<GroupView>('get_group', { workspaceId, groupId }),
  planPrepareEnvironment: (input) =>
    call<EnvironmentPlanView>('plan_prepare_environment', { input }),
  prepareEnvironment: (input) =>
    call<OperationOutcome>('prepare_environment', { input }),
  getWorkflowStatus: (workflowId) =>
    call<WorkflowStatusView>('get_workflow_status', { workflowId }),
  listWorkflows: (workspaceId, limit) =>
    call<WorkflowSummaryView[]>('list_workflows', { workspaceId, limit }),
  listMigrations: (workspaceId) =>
    call<MigrationRowView[]>('list_migrations', { workspaceId }),
  getMigrationStatus: (serviceId) =>
    call<MigrationStatusView>('get_migration_status', { serviceId }),
  validateMigration: (serviceId) =>
    call<OperationOutcome>('validate_migration', { serviceId }),
  inspectMigrationStatus: (serviceId) =>
    call<OperationOutcome>('inspect_migration_status', { serviceId }),
  inspectMigrationPending: (serviceId) =>
    call<OperationOutcome>('inspect_migration_pending', { serviceId }),
  previewMigration: (serviceId) =>
    call<OperationOutcome>('preview_migration', { serviceId }),
  applyMigration: (serviceId, confirmed) =>
    call<OperationOutcome>('apply_migration', { serviceId, confirmed }),
  applySelectedMigrations: (workspaceId, serviceIds, confirmed) =>
    call<OperationOutcome>('apply_selected_migrations', {
      workspaceId,
      serviceIds,
      confirmed,
    }),
  listHealth: (workspaceId) => call<HealthRowView[]>('list_health', { workspaceId }),
  getHealth: (workspaceId, serviceId) =>
    call<HealthDetailView>('get_health', { workspaceId, serviceId }),
  checkServiceReadiness: (serviceId) =>
    call<OperationOutcome>('check_service_readiness', { serviceId }),
  getLogs: (query = {}) =>
    call<LogPageView>('get_logs', {
      taskId: query.taskId,
      serviceId: query.serviceId,
      workspaceId: query.workspaceId,
      source: query.source,
      minimumLevel: query.minimumLevel,
      text: query.text,
      after: query.after,
      limit: query.limit,
    }),
  subscribeServiceLogs: (subscriptionId, serviceId) =>
    call<void>('subscribe_service_logs', { subscriptionId, serviceId }),
  unsubscribeServiceLogs: (subscriptionId) =>
    call<void>('unsubscribe_service_logs', { subscriptionId }),
  getConfiguration: () => call<ConfigurationSnapshotView>('get_configuration'),
  validateConfiguration: () =>
    call<ConfigurationValidationView>('validate_configuration'),
  previewConfiguration: (change, expectedRevision) =>
    call<ConfigurationPreviewView>('preview_configuration', {
      change,
      expectedRevision,
    }),
  commitConfiguration: (change, expectedRevision, confirmed) =>
    call<ConfigurationCommitView>('commit_configuration', {
      change,
      expectedRevision,
      confirmed,
    }),
  reloadConfiguration: () =>
    call<ConfigurationReloadView>('reload_configuration'),
  inspectRepository: (path) =>
    call<RepositoryProbeView>('inspect_repository_path', { path }),
  detectProjectConfiguration: (path) =>
    call<ProjectSuggestionView>('detect_project_configuration', { path }),
  getConnectionState: () => call<ConnectionSnapshot>('get_connection_state'),
  onConnectionChanged: (handler) =>
    subscribe<ConnectionSnapshot>(DAEMON_CONNECTION_EVENT, handler),
  onTaskEvent: (handler) => subscribe<TaskEvent>(TASK_EVENT, handler),
  onLogEvent: (handler) => subscribe<LogStreamEvent>(LOG_EVENT, handler),
  onConfigurationChanged: (handler) =>
    subscribe<ConfigurationChangedEvent>(CONFIGURATION_EVENT, handler),
};
