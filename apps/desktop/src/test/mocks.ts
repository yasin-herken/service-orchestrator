import { vi } from 'vitest';

import type {
  ArtifactVerificationView,
  BranchView,
  BuildCapabilitiesView,
  BuildTargetSummary,
  BulkImportCommitView,
  BulkImportInputView,
  BulkImportPlanView,
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
  DiscoveryScanInputView,
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
  MigrationPresetSummaryView,
  MigrationRowView,
  MigrationStatusView,
  OperationOutcome,
  OperationPresetSummaryView,
  PrepareEnvironmentInput,
  ProfileView,
  ProjectSuggestionView,
  RepositoryDiscoveryReportView,
  RepositoryInspectionView,
  RepositoryProbeView,
  RepositorySummary,
  ServiceDetail,
  ServiceSummary,
  TaskEvent,
  TaskView,
  VersionAlignmentView,
  WorkflowCancellationView,
  WorkflowStatusView,
  WorkflowSummaryView,
  WorkflowView,
  WorkspaceStatus,
  WorkspaceSummary,
} from '../types/models';
import type { LogsQuery, PrepareOptions, TaskLogsQuery } from '../services/desktopApi';

type ConnectionHandler = (snapshot: ConnectionSnapshot) => void;
type TaskHandler = (event: TaskEvent) => void;
type LogHandler = (event: LogStreamEvent) => void;
type ConfigurationHandler = (event: ConfigurationChangedEvent) => void;

const connectionHandlers = new Set<ConnectionHandler>();
const taskHandlers = new Set<TaskHandler>();
const logHandlers = new Set<LogHandler>();
const configurationHandlers = new Set<ConfigurationHandler>();

function subscribeConnections(handler: ConnectionHandler): Promise<() => void> {
  connectionHandlers.add(handler);
  return Promise.resolve(() => connectionHandlers.delete(handler));
}

function subscribeTasks(handler: TaskHandler): Promise<() => void> {
  taskHandlers.add(handler);
  return Promise.resolve(() => taskHandlers.delete(handler));
}

function subscribeLogs(handler: LogHandler): Promise<() => void> {
  logHandlers.add(handler);
  return Promise.resolve(() => logHandlers.delete(handler));
}

function subscribeConfiguration(handler: ConfigurationHandler): Promise<() => void> {
  configurationHandlers.add(handler);
  return Promise.resolve(() => configurationHandlers.delete(handler));
}

/**
 * A fully mocked desktop API for frontend unit tests.
 *
 * Test files install it with:
 *
 * ```ts
 * vi.mock('../services/desktopApi', async () => ({
 *   desktopApi: (await import('../test/mocks')).mockDesktopApi,
 * }));
 * ```
 */
export const mockDesktopApi = {
  getDaemonStatus: vi.fn<() => Promise<DaemonStatus>>(),
  listWorkspaces: vi.fn<() => Promise<WorkspaceSummary[]>>(),
  listServices: vi.fn<(workspaceId: string) => Promise<ServiceSummary[]>>(),
  getWorkspaceStatus: vi.fn<(workspaceId: string) => Promise<WorkspaceStatus>>(),
  getServiceStatus: vi.fn<(workspaceId: string, serviceId: string) => Promise<ServiceSummary>>(),
  getServiceDetail: vi.fn<(workspaceId: string, serviceId: string) => Promise<ServiceDetail>>(),
  startService: vi.fn<(serviceId: string) => Promise<OperationOutcome>>(),
  stopService: vi.fn<(serviceId: string) => Promise<OperationOutcome>>(),
  restartService: vi.fn<(serviceId: string) => Promise<OperationOutcome>>(),
  startServices: vi.fn<(workspaceId: string, serviceIds: string[]) => Promise<OperationOutcome>>(),
  stopServices: vi.fn<(workspaceId: string, serviceIds: string[]) => Promise<OperationOutcome>>(),
  restartServices: vi.fn<
    (workspaceId: string, serviceIds: string[]) => Promise<OperationOutcome>
  >(),
  checkServiceHealth: vi.fn<(serviceId: string) => Promise<OperationOutcome>>(),
  listRepositories: vi.fn<(workspaceId: string) => Promise<RepositorySummary[]>>(),
  getGitStatus: vi.fn<(serviceId: string) => Promise<GitStatusView>>(),
  inspectRepositories: vi.fn<
    (workspaceId: string, serviceIds: string[]) => Promise<RepositoryInspectionView[]>
  >(),
  listBranches: vi.fn<(serviceId: string) => Promise<BranchView[]>>(),
  previewGitSync: vi.fn<
    (
      workspaceId: string,
      serviceIds: string[],
      targetBranch: string,
      mode: GitSyncMode,
    ) => Promise<GitSyncPreview>
  >(),
  fetchRepository: vi.fn<(serviceId: string) => Promise<OperationOutcome>>(),
  pullRepository: vi.fn<(serviceId: string) => Promise<OperationOutcome>>(),
  checkoutRepository: vi.fn<
    (serviceId: string, branch: string) => Promise<OperationOutcome>
  >(),
  syncRepository: vi.fn<(serviceId: string) => Promise<OperationOutcome>>(),
  fetchRepositories: vi.fn<
    (workspaceId: string, serviceIds: string[]) => Promise<OperationOutcome>
  >(),
  syncRepositories: vi.fn<
    (workspaceId: string, serviceIds: string[]) => Promise<OperationOutcome>
  >(),
  checkoutRepositories: vi.fn<
    (workspaceId: string, serviceIds: string[], branch: string) => Promise<OperationOutcome>
  >(),
  listBuildTargets: vi.fn<(workspaceId: string) => Promise<BuildTargetSummary[]>>(),
  getBuildCapabilities: vi.fn<(serviceId: string) => Promise<BuildCapabilitiesView>>(),
  buildService: vi.fn<(serviceId: string) => Promise<OperationOutcome>>(),
  buildServices: vi.fn<
    (workspaceId: string, serviceIds: string[]) => Promise<OperationOutcome>
  >(),
  listLibraries: vi.fn<(workspaceId: string) => Promise<LibrarySummary[]>>(),
  getLibraryDependents: vi.fn<(libraryId: string) => Promise<LibraryDependentView[]>>(),
  verifyLibraryArtifact: vi.fn<(libraryId: string) => Promise<ArtifactVerificationView>>(),
  inspectVersionAlignment: vi.fn<(libraryId: string) => Promise<VersionAlignmentView>>(),
  buildLibrary: vi.fn<(libraryId: string) => Promise<OperationOutcome>>(),
  installLibrary: vi.fn<(libraryId: string) => Promise<OperationOutcome>>(),
  syncLibrary: vi.fn<(libraryId: string) => Promise<OperationOutcome>>(),
  prepareLibrary: vi.fn<
    (libraryId: string, options: PrepareOptions) => Promise<OperationOutcome>
  >(),
  prepareLibraries: vi.fn<
    (
      workspaceId: string,
      libraryIds: string[],
      options: PrepareOptions,
    ) => Promise<OperationOutcome>
  >(),
  getTask: vi.fn<(taskId: string) => Promise<TaskView>>(),
  summarizeWorkflow: vi.fn<(workflowId: string) => Promise<BulkSummaryView>>(),
  getTaskLogs: vi.fn<(taskId: string, query?: TaskLogsQuery) => Promise<LogPageView>>(),
  cancelTask: vi.fn<(taskId: string) => Promise<TaskView>>(),
  cancelWorkflow: vi.fn<(workflowId: string) => Promise<WorkflowCancellationView>>(),
  listTasks: vi.fn<(limit?: number) => Promise<TaskView[]>>(),
  getWorkflow: vi.fn<(workflowId: string) => Promise<WorkflowView>>(),
  listProfiles: vi.fn<(workspaceId: string) => Promise<ProfileView[]>>(),
  getProfile: vi.fn<(workspaceId: string, profileId: string) => Promise<ProfileView>>(),
  listGroups: vi.fn<(workspaceId: string) => Promise<GroupView[]>>(),
  getGroup: vi.fn<(workspaceId: string, groupId: string) => Promise<GroupView>>(),
  planPrepareEnvironment: vi.fn<
    (input: PrepareEnvironmentInput) => Promise<EnvironmentPlanView>
  >(),
  prepareEnvironment: vi.fn<(input: PrepareEnvironmentInput) => Promise<OperationOutcome>>(),
  getWorkflowStatus: vi.fn<(workflowId: string) => Promise<WorkflowStatusView>>(),
  listWorkflows: vi.fn<
    (workspaceId: string, limit?: number) => Promise<WorkflowSummaryView[]>
  >(),
  listMigrations: vi.fn<(workspaceId: string) => Promise<MigrationRowView[]>>(),
  getMigrationStatus: vi.fn<(serviceId: string) => Promise<MigrationStatusView>>(),
  validateMigration: vi.fn<(serviceId: string) => Promise<OperationOutcome>>(),
  inspectMigrationStatus: vi.fn<(serviceId: string) => Promise<OperationOutcome>>(),
  inspectMigrationPending: vi.fn<(serviceId: string) => Promise<OperationOutcome>>(),
  previewMigration: vi.fn<(serviceId: string) => Promise<OperationOutcome>>(),
  applyMigration: vi.fn<(serviceId: string, confirmed: boolean) => Promise<OperationOutcome>>(),
  applySelectedMigrations: vi.fn<
    (workspaceId: string, serviceIds: string[], confirmed: boolean) => Promise<OperationOutcome>
  >(),
  listHealth: vi.fn<(workspaceId: string) => Promise<HealthRowView[]>>(),
  getHealth: vi.fn<(workspaceId: string, serviceId: string) => Promise<HealthDetailView>>(),
  checkServiceReadiness: vi.fn<(serviceId: string) => Promise<OperationOutcome>>(),
  getLogs: vi.fn<(query?: LogsQuery) => Promise<LogPageView>>(),
  subscribeServiceLogs: vi.fn<(subscriptionId: string, serviceId: string) => Promise<void>>(),
  unsubscribeServiceLogs: vi.fn<(subscriptionId: string) => Promise<void>>(),
  getConfiguration: vi.fn<() => Promise<ConfigurationSnapshotView>>(),
  validateConfiguration: vi.fn<() => Promise<ConfigurationValidationView>>(),
  previewConfiguration: vi.fn<
    (change: ConfigChangeView, expectedRevision: string) => Promise<ConfigurationPreviewView>
  >(),
  commitConfiguration: vi.fn<
    (
      change: ConfigChangeView,
      expectedRevision: string,
      confirmed: boolean,
    ) => Promise<ConfigurationCommitView>
  >(),
  reloadConfiguration: vi.fn<() => Promise<ConfigurationReloadView>>(),
  inspectRepository: vi.fn<(path: string) => Promise<RepositoryProbeView>>(),
  detectProjectConfiguration: vi.fn<(path: string) => Promise<ProjectSuggestionView>>(),
  discoverRepositories: vi.fn<
    (input: DiscoveryScanInputView) => Promise<RepositoryDiscoveryReportView>
  >(),
  planBulkImport: vi.fn<(input: BulkImportInputView) => Promise<BulkImportPlanView>>(),
  commitBulkImport: vi.fn<
    (
      input: BulkImportInputView,
      expectedRevision: string,
      confirmed: boolean,
    ) => Promise<BulkImportCommitView>
  >(),
  listMigrationPresets: vi.fn<() => Promise<MigrationPresetSummaryView[]>>(),
  listOperationPresets: vi.fn<
    (workspaceId?: string) => Promise<OperationPresetSummaryView[]>
  >(),
  getOperationPresetRequest: vi.fn<
    (workspaceId: string, presetId: string) => Promise<PrepareEnvironmentInput>
  >(),
  getConnectionState: vi.fn<() => Promise<ConnectionSnapshot>>(),
  onConnectionChanged: vi.fn<(handler: ConnectionHandler) => Promise<() => void>>(
    subscribeConnections,
  ),
  onTaskEvent: vi.fn<(handler: TaskHandler) => Promise<() => void>>(subscribeTasks),
  onLogEvent: vi.fn<(handler: LogHandler) => Promise<() => void>>(subscribeLogs),
  onConfigurationChanged: vi.fn<
    (handler: ConfigurationHandler) => Promise<() => void>
  >(subscribeConfiguration),
};

/** Delivers a connection snapshot to every mounted subscriber. */
export function emitConnection(snapshot: ConnectionSnapshot): void {
  for (const handler of [...connectionHandlers]) {
    handler(snapshot);
  }
}

/** Delivers a task event to every mounted subscriber. */
export function emitTaskEvent(event: TaskEvent): void {
  for (const handler of [...taskHandlers]) {
    handler(event);
  }
}

/** Delivers a realtime log-stream event to every mounted subscriber. */
export function emitLogEvent(event: LogStreamEvent): void {
  for (const handler of [...logHandlers]) {
    handler(event);
  }
}

/** Delivers a configuration-change event to every mounted subscriber. */
export function emitConfigurationChanged(event: ConfigurationChangedEvent): void {
  for (const handler of [...configurationHandlers]) {
    handler(event);
  }
}

/** Clears subscriptions and mock behaviour between tests. */
export function resetMockDesktopApi(): void {
  connectionHandlers.clear();
  taskHandlers.clear();
  logHandlers.clear();
  configurationHandlers.clear();
  vi.resetAllMocks();
  mockDesktopApi.onConnectionChanged.mockImplementation(subscribeConnections);
  mockDesktopApi.onTaskEvent.mockImplementation(subscribeTasks);
  mockDesktopApi.onLogEvent.mockImplementation(subscribeLogs);
  mockDesktopApi.onConfigurationChanged.mockImplementation(subscribeConfiguration);
  mockDesktopApi.subscribeServiceLogs.mockResolvedValue(undefined);
  mockDesktopApi.unsubscribeServiceLogs.mockResolvedValue(undefined);
}
