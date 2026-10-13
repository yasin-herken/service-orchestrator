import { vi } from 'vitest';

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
  TaskEvent,
  TaskView,
  VersionAlignmentView,
  WorkflowCancellationView,
  WorkspaceStatus,
  WorkspaceSummary,
} from '../types/models';
import type { PrepareOptions, TaskLogsQuery } from '../services/desktopApi';

type ConnectionHandler = (snapshot: ConnectionSnapshot) => void;
type TaskHandler = (event: TaskEvent) => void;

const connectionHandlers = new Set<ConnectionHandler>();
const taskHandlers = new Set<TaskHandler>();

function subscribeConnections(handler: ConnectionHandler): Promise<() => void> {
  connectionHandlers.add(handler);
  return Promise.resolve(() => connectionHandlers.delete(handler));
}

function subscribeTasks(handler: TaskHandler): Promise<() => void> {
  taskHandlers.add(handler);
  return Promise.resolve(() => taskHandlers.delete(handler));
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
  getConnectionState: vi.fn<() => Promise<ConnectionSnapshot>>(),
  onConnectionChanged: vi.fn<(handler: ConnectionHandler) => Promise<() => void>>(
    subscribeConnections,
  ),
  onTaskEvent: vi.fn<(handler: TaskHandler) => Promise<() => void>>(subscribeTasks),
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

/** Clears subscriptions and mock behaviour between tests. */
export function resetMockDesktopApi(): void {
  connectionHandlers.clear();
  taskHandlers.clear();
  vi.resetAllMocks();
  mockDesktopApi.onConnectionChanged.mockImplementation(subscribeConnections);
  mockDesktopApi.onTaskEvent.mockImplementation(subscribeTasks);
}
