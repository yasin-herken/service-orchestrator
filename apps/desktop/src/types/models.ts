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

/** How a mutating operation resolved (adapter `OperationStatus`). */
export type OperationStatus = 'task' | 'workflow' | 'already_in_desired_state';

/**
 * The normalized outcome of a mutating operation.
 *
 * A `task` schedules one task; a `workflow` schedules a bulk workflow; and
 * `already_in_desired_state` means the daemon scheduled nothing because the
 * target was already there. The frontend follows the real ids to observe
 * task-engine progress and never marks an operation successful on its own.
 */
export interface OperationOutcome {
  status: OperationStatus;
  /** The scheduled task id, when a single task was scheduled. */
  task_id: string | null;
  /** The scheduled workflow id, when a bulk workflow was scheduled. */
  workflow_id: string | null;
  /** The task or workflow kind label (`start_service`, `start_services`, …). */
  kind: string;
}

/** A runtime requirement in display form (`java 21`, `node 20.19.4`). */
export interface RuntimeView {
  /** `java`, `node`, `maven`, `npm`, or a project-specific label. */
  kind: string;
  /** The pinned version, when the service pins one. */
  version: string | null;
}

/** The source repository of a service (never containing credentials). */
export interface RepositoryView {
  /** The clone URL. */
  url: string;
  /** The checkout path relative to the workspace root. */
  local_path: string;
  /** The branch checked out when no other branch is requested. */
  default_branch: string;
  /** The hosting provider label. */
  provider: string;
}

/** A declared dependency of a service. */
export interface DependencyView {
  /** The dependency target's identifier. */
  target: string;
  /** Whether the target is a `service` or a `library`. */
  target_kind: string;
  /** Whether the dependency is needed for `build` or `runtime`. */
  kind: string;
  /** Whether the dependency is mandatory. */
  required: boolean;
}

/** A configured health check of a service. */
export interface HealthCheckView {
  /** A display label such as `process`, `port 8081`, or `http <url>`. */
  kind: string;
  /** Whether the check is required for the service to be considered healthy. */
  required: boolean;
}

/** The latest observed outcome of one configured health check. */
export interface HealthOutcomeView {
  /** The probe type (`process`, `tcp`, `http`, or a provider label). */
  check_type: string;
  /** What was probed, if anything (`localhost:8081`, an HTTP URL). */
  target: string | null;
  /** Whether the check is required. */
  required: boolean;
  /** The observed check state. */
  state: HealthState;
  /** How long the probe took, in milliseconds. */
  duration_ms: number;
  /** A short, redacted explanation, present for a failure. */
  message: string | null;
  /** When the check ran, seconds since the Unix epoch. */
  checked_at: number;
}

/** A structured, observed failure recorded in a diagnostic. */
export interface DiagnosticFailureView {
  /** Where the failure was observed (`health`, `log`, or `runtime`). */
  source: string;
  /** A stable, machine-readable code. */
  code: string;
  /** A human-readable summary. */
  message: string;
  /** When the failure was observed, seconds since the Unix epoch. */
  at: number | null;
}

/**
 * A detailed view of one service: configuration plus observed facts.
 *
 * Anything the adapter cannot observe yet — for example the current commit,
 * which needs a Git operation deferred to a later task — is simply absent.
 */
export interface ServiceDetail {
  id: string;
  display_name: string;
  kind: string;
  workspace_id: string;
  /** The service directory relative to the workspace root, when configured. */
  path: string | null;
  repository: RepositoryView | null;
  runtimes: RuntimeView[];
  build_system: string | null;
  capabilities: string[];
  process_state: ProcessState;
  health_state: HealthState;
  git_state: GitState;
  branch: string | null;
  /** Whether the service is currently tracked as a managed process. */
  managed: boolean;
  /** The operating-system process id, while the process is alive. */
  pid: number | null;
  ports: number[];
  dependencies: DependencyView[];
  health_checks: HealthCheckView[];
  health_outcomes: HealthOutcomeView[];
  /** When the service was last checked, seconds since the Unix epoch. */
  last_checked_at: number | null;
  last_failure: DiagnosticFailureView | null;
  /** The most recent error-level log lines for the service. */
  recent_errors: string[];
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
