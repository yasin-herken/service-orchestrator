import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import type {
  ArtifactVerificationView,
  BranchView,
  BuildCapabilitiesView,
  BuildTargetSummary,
  BulkSummaryView,
  ConnectionSnapshot,
  DaemonStatus,
  GitStatusView,
  GitSyncMode,
  GitSyncPreview,
  LibraryDependentView,
  LibrarySummary,
  LogPageView,
  OperationOutcome,
  RepositoryInspectionView,
  RepositorySummary,
  ServiceDetail,
  ServiceSummary,
  TaskView,
  VersionAlignmentView,
  WorkflowCancellationView,
  WorkspaceStatus,
  WorkspaceSummary,
} from '../types/models';
import { DAEMON_CONNECTION_EVENT, TASK_EVENT, type TaskEvent } from '../types/models';
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
  /** The adapter's current daemon connection state. */
  getConnectionState(): Promise<ConnectionSnapshot>;
  /** Subscribes to daemon connection changes; resolves to an unsubscriber. */
  onConnectionChanged(handler: (snapshot: ConnectionSnapshot) => void): Promise<UnlistenFn>;
  /** Subscribes to forwarded task-engine events; resolves to an unsubscriber. */
  onTaskEvent(handler: (event: TaskEvent) => void): Promise<UnlistenFn>;
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
  getConnectionState: () => call<ConnectionSnapshot>('get_connection_state'),
  onConnectionChanged: (handler) =>
    subscribe<ConnectionSnapshot>(DAEMON_CONNECTION_EVENT, handler),
  onTaskEvent: (handler) => subscribe<TaskEvent>(TASK_EVENT, handler),
};
