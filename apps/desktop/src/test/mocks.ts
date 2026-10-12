import { vi } from 'vitest';

import type {
  ConnectionSnapshot,
  DaemonStatus,
  OperationOutcome,
  ServiceDetail,
  ServiceSummary,
  TaskEvent,
  WorkspaceStatus,
  WorkspaceSummary,
} from '../types/models';

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
