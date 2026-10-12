/**
 * Typed models for the desktop GUI.
 *
 * These mirror the Rust serialization shapes exactly (snake_case field names,
 * snake_case enum variants) so the frontend never hand-translates domain data.
 * The Rust source of truth lives in the desktop adapter's `dto` module, which
 * maps from the application core's types.
 */

/** A diagnostic snapshot of the shared daemon (client-crate `DaemonStatus`). */
export interface DaemonStatus {
  /** Operating-system process id of the daemon. */
  pid: number;
  /** Unix domain socket the daemon listens on. */
  socket_path: string;
  /** IPC protocol version the daemon speaks. */
  protocol_version: number;
  /** How many clients are currently connected. */
  clients: number;
  /** How many tasks are active. */
  active_tasks: number;
  /** How many services are managed as running processes. */
  managed_services: number;
  /** Daemon start time, seconds since the Unix epoch. */
  started_at_unix: number;
}

/** A workspace as shown in the workspace selector. */
export interface WorkspaceSummary {
  id: string;
  name: string;
  /** Absolute filesystem root of the workspace. */
  root: string;
  /** Number of configured services. */
  service_count: number;
}

/** Process lifecycle state (domain `ProcessState`). */
export type ProcessState =
  | 'not_running'
  | 'starting'
  | 'running'
  | 'stopping'
  | 'stopped'
  | 'failed'
  | 'unknown';

/** Health state (application `HealthState`). */
export type HealthState =
  | 'unknown'
  | 'checking'
  | 'healthy'
  | 'unhealthy'
  | 'degraded'
  | 'unreachable';

/** Git working-copy state (domain `GitState`). */
export type GitState = 'clean' | 'modified' | 'ahead' | 'behind' | 'diverged' | 'unknown';

/** Well-known service kinds; the domain keeps the type open. */
export const SERVICE_KINDS = ['backend', 'frontend', 'library'] as const;

/** A service row: configuration plus its last observed runtime state. */
export interface ServiceSummary {
  id: string;
  display_name: string;
  /** `backend`, `frontend`, `library`, or a project-specific category. */
  kind: string;
  workspace_id: string;
  process_state: ProcessState;
  health_state: HealthState;
  git_state: GitState;
  /** Branch currently checked out, when known. */
  branch: string | null;
  /** First declared port, when the service declares one. */
  port: number | null;
}

/** A workspace with the status of each of its services. */
export interface WorkspaceStatus {
  workspace_id: string;
  name: string;
  root: string;
  services: ServiceSummary[];
}

/** Daemon connection state as reported by the adapter. */
export type ConnectionState =
  | 'connecting'
  | 'connected'
  | 'disconnected'
  | 'reconnecting'
  | 'error';

/** The adapter's connection status, pushed to the frontend on every change. */
export interface ConnectionSnapshot {
  state: ConnectionState;
  /** Human-readable detail while not connected; `null` when connected. */
  message: string | null;
  /** When this state was entered, seconds since the Unix epoch. */
  since_unix: number;
}

/** A structured error surfaced by a Tauri command. */
export interface DesktopError {
  /** Stable machine code, e.g. `DAEMON_UNAVAILABLE` or `INVALID_INPUT`. */
  code: string;
  /** Human-readable, user-safe message. */
  message: string;
}

/**
 * A task-engine event forwarded from the daemon.
 *
 * `TaskEvent` is externally tagged on the wire, so each payload is keyed by
 * its variant name (`TaskCreated`, `TaskStarted`, …). `Lagged` is synthesized
 * by the adapter when the local subscription fell behind.
 */
export type TaskEvent = Record<string, unknown>;

/** Event name carrying a `ConnectionSnapshot` from the Rust adapter. */
export const DAEMON_CONNECTION_EVENT = 'daemon://connection';

/** Event name carrying a forwarded `TaskEvent` from the Rust adapter. */
export const TASK_EVENT = 'daemon://task';
