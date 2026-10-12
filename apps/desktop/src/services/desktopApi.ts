import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

import type {
  ConnectionSnapshot,
  DaemonStatus,
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
 * IPC operation to the shared daemon (`tasks/018.md` §7, §12). React never
 * talks to the operating system directly.
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
  getConnectionState: () => call<ConnectionSnapshot>('get_connection_state'),
  onConnectionChanged: (handler) =>
    subscribe<ConnectionSnapshot>(DAEMON_CONNECTION_EVENT, handler),
  onTaskEvent: (handler) => subscribe<TaskEvent>(TASK_EVENT, handler),
};
