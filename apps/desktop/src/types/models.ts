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

// -- git ----------------------------------------------------------------------

/** How a bulk Git operation resolves (`sync`, `fetch`, or `checkout`). */
export type GitSyncMode = 'sync' | 'fetch' | 'checkout';

/**
 * A repository row: validated configuration plus the last observed state.
 *
 * The list is configuration-derived and performs no Git operation, so the Git
 * screen opens cheaply with 100+ repositories; real working-tree state is
 * observed on demand (`tasks/020.md` §4, §35).
 */
export interface RepositorySummary {
  /** A stable, workspace-aware identifier (`workspace:service`). */
  id: string;
  service_id: string;
  workspace_id: string;
  name: string;
  /** The service directory relative to the workspace root, when configured. */
  path: string | null;
  repository: RepositoryView | null;
  /** The last observed Git state, or `unknown` before inspection. */
  git_state: GitState;
  branch: string | null;
}

/** A structured `{ code, message }` error embedded in a per-item result. */
export interface CommandErrorView {
  code: string;
  message: string;
}

/** One repository's on-demand inspection result, success or failure. */
export interface RepositoryInspectionView {
  repository_id: string;
  service_id: string;
  status: GitStatusView | null;
  error: CommandErrorView | null;
}

/** A full working-tree inspection of one repository. */
export interface GitStatusView {
  service_id: string;
  /** The checkout lifecycle (`available`, `not_cloned`, …). */
  lifecycle: string;
  branch: string | null;
  commit: string | null;
  detached_head: boolean;
  unborn: boolean;
  upstream: string | null;
  ahead: number | null;
  behind: number | null;
  staged_changes: number;
  unstaged_changes: number;
  untracked_files: number;
  conflicts: number;
  clean: boolean;
  has_local_changes: boolean;
  diverged: boolean;
}

/** One branch in a repository listing. */
export interface BranchView {
  name: string;
  /** `local` or `remote`. */
  kind: string;
  current: boolean;
  upstream: string | null;
  commit: string | null;
}

/** One repository's entry in a bulk Git preview. */
export interface GitSyncPreviewEntry {
  repository_id: string;
  service_id: string;
  name: string;
  current_branch: string | null;
  target_branch_exists: boolean;
  target_branch_kind: string | null;
  lifecycle: string;
  git_state: GitState;
  clean: boolean;
  ahead: number | null;
  behind: number | null;
  diverged: boolean;
  /** The operations the daemon will run for this repository. */
  expected_operations: string[];
  warnings: string[];
  blocked: boolean;
}

/** A read-only preview of a bulk Git operation over a selection. */
export interface GitSyncPreview {
  workspace_id: string;
  target_branch: string;
  mode: GitSyncMode;
  entries: GitSyncPreviewEntry[];
  total: number;
  ready: number;
  dirty: number;
  missing_branch: number;
  diverged: number;
  unavailable: number;
}

// -- build --------------------------------------------------------------------

/** One build operation in a provider's capability list. */
export interface BuildOperationView {
  /** `clean`, `build`, `test`, `install_dependencies`, `publish_local_artifact`. */
  operation: string;
  supported: boolean;
  /** Whether the application core exposes the operation for a service. */
  exposed: boolean;
  reason: string | null;
}

/** The build operations a build system's provider supports. */
export interface BuildCapabilitiesView {
  build_system: string;
  operations: BuildOperationView[];
  empty: boolean;
}

/** One service row on the Builds screen: configuration plus capabilities. */
export interface BuildTargetSummary {
  service_id: string;
  display_name: string;
  workspace_id: string;
  kind: string;
  /** The configured build system (`maven`, `npm`), when known. */
  build_system: string | null;
  runtimes: RuntimeView[];
  capability: BuildCapabilitiesView | null;
  capability_error: string | null;
}

// -- library ------------------------------------------------------------------

/** A Maven-style artifact coordinate in display form. */
export interface ArtifactView {
  group_id: string;
  artifact_id: string;
  version: string;
  packaging: string;
  classifier: string | null;
  file_name: string;
  /** The canonical `group:artifact:version` coordinate. */
  coordinate: string;
}

/** A service that depends on a library. */
export interface LibraryDependentView {
  service_id: string;
  workspace_id: string;
  required: boolean;
  /** Whether the dependency is needed for `build` or `runtime`. */
  kind: string;
}

/** One library row on the Libraries screen. */
export interface LibrarySummary {
  id: string;
  display_name: string;
  workspace_id: string;
  build_system: string | null;
  repository_url: string | null;
  artifact: ArtifactView | null;
  artifacts: ArtifactView[];
  branch: string | null;
  commit: string | null;
  git_state: GitState;
  repository_error: string | null;
  configured_version: string | null;
  dependents: LibraryDependentView[];
}

/** The result of verifying a library's artifact in the local repository. */
export interface ArtifactVerificationView {
  library_id: string;
  workspace_id: string;
  artifact: ArtifactView;
  present: boolean;
  path: string | null;
  message: string;
}

/** The alignment of one consumer with a library's target version. */
export type AlignmentStatus = 'aligned' | 'mismatch' | 'unknown' | 'unresolved';

/** One consumer's alignment with a library's target version. */
export interface ConsumerAlignmentView {
  service_id: string;
  workspace_id: string;
  path: string | null;
  /** The version the consumer resolves to, when known. */
  current_version: string | null;
  status: AlignmentStatus;
  detail: string | null;
}

/** A structured version-alignment view for a library and its consumers. */
export interface VersionAlignmentView {
  library_id: string;
  workspace_id: string;
  target: ArtifactView;
  consumers: ConsumerAlignmentView[];
  aligned_count: number;
  total: number;
  fully_aligned: boolean;
}

// -- tasks, workflows, and logs ----------------------------------------------

/** Task lifecycle status (`queued`, `running`, `succeeded`, …). */
export type TaskStatus =
  | 'queued'
  | 'running'
  | 'succeeded'
  | 'failed'
  | 'cancelled'
  | 'skipped'
  | 'blocked';

/** How far along a task is. */
export interface TaskProgressView {
  current: number;
  total: number | null;
  message: string | null;
  /** The completion fraction in `0.0..=1.0`, when the total is known. */
  fraction: number | null;
}

/** A structured task failure. */
export interface TaskFailureView {
  code: string;
  message: string;
  detail: string | null;
}

/** A task-engine task in display form (read through the shared adapter). */
export interface TaskView {
  id: string;
  /** The task kind label (`build_service`, `git_sync`, …). */
  kind: string;
  status: string;
  service_id: string | null;
  library_id: string | null;
  workflow_id: string | null;
  created_at: number;
  started_at: number | null;
  finished_at: number | null;
  duration_ms: number | null;
  progress: TaskProgressView;
  failure: TaskFailureView | null;
  dependencies: string[];
}

/** An aggregate of a workflow's per-task outcomes. */
export interface BulkSummaryView {
  total: number;
  succeeded: number;
  failed: number;
  cancelled: number;
  skipped: number;
  pending: number;
  complete: boolean;
}

/** The result of requesting cancellation of a workflow's tasks. */
export interface WorkflowCancellationView {
  cancelled_tasks: number;
  already_complete: boolean;
}

/** A single structured log entry. */
export interface LogEntryView {
  timestamp: number;
  /** `trace`, `debug`, `info`, `warn`, or `error`. */
  level: string;
  message: string;
  /** `process`, `task`, `build`, `library`, … */
  source: string;
  /** `stdout` / `stderr`, for process entries. */
  stream: string | null;
  service_id: string | null;
  task_id: string | null;
  process_id: string | null;
  sequence: number;
}

/** A bounded page of log entries with its continuation cursor. */
export interface LogPageView {
  entries: LogEntryView[];
  next_cursor: number | null;
  truncated: boolean;
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
