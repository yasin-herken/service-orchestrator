import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import type {
  ConnectionSnapshot,
  DaemonStatus,
  OperationOutcome,
  ServiceDetail,
  ServiceSummary,
  WorkspaceStatus,
  WorkspaceSummary,
} from '../types/models';
import { DAEMON_CONNECTION_EVENT, TASK_EVENT, type TaskEvent } from '../types/models';
import { toDesktopError } from './errors';

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
  getConnectionState: () => call<ConnectionSnapshot>('get_connection_state'),
  onConnectionChanged: (handler) =>
    subscribe<ConnectionSnapshot>(DAEMON_CONNECTION_EVENT, handler),
  onTaskEvent: (handler) => subscribe<TaskEvent>(TASK_EVENT, handler),
};
