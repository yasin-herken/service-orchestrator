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
  /** The owning workspace, resolved from configuration when it could be. */
  workspace_id: string | null;
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

/** One step of a workflow, in display form. */
export interface WorkflowStepView {
  id: string;
  description: string | null;
  /** The kind of task the step performs (`migration_apply`, …). */
  task_kind: string;
  /** The ids of the steps that must complete before this one. */
  depends_on: string[];
}

/**
 * A workflow's declared structure, in display form.
 *
 * Relationships come from real step ids, task kinds, and dependency edges —
 * never from parsing task names (`tasks/021.md` §16).
 */
export interface WorkflowView {
  id: string;
  kind: string;
  steps: WorkflowStepView[];
}

// -- migrations ----------------------------------------------------------------

/**
 * A service's migration setup in display form.
 *
 * The snapshot is configuration-derived and performs no database operation:
 * `pending` and `locked` are explicit unknowns (`null`) until a real provider
 * inspection task answers (`tasks/021.md` §4, §6).
 */
export interface MigrationStatusView {
  service_id: string;
  workspace_id: string;
  /** Whether migrations are enabled for the service. */
  enabled: boolean;
  /** The configured migration provider (`liquibase`, …). */
  provider: string;
  /** How the provider is invoked (`cli`, `maven`, …). */
  execution_mode: string;
  /** The declared environment of the target database (`local`, `staging`, …). */
  environment: string;
  /** A display-safe identity of the target database (never credentials). */
  database_identity: string;
  /** Whether applying to the target requires explicit authorization. */
  requires_authorization: boolean;
  timeout_seconds: number | null;
  /** Pending changesets, when known — always `null` from the snapshot. */
  pending: number | null;
  /** Whether the target is locked, when known — always `null` from the snapshot. */
  locked: boolean | null;
}

/** One row of the Migrations table: configuration plus the latest task. */
export interface MigrationRowView {
  status: MigrationStatusView;
  latest_task: TaskView | null;
}

// -- health --------------------------------------------------------------------

/** One row of the Health table: runtime state plus last observed health. */
export interface HealthRowView {
  service_id: string;
  display_name: string;
  workspace_id: string;
  /** The last observed process state — deliberately distinct from health. */
  process_state: ProcessState;
  health_state: HealthState;
  /** When the service was last checked, seconds since the Unix epoch. */
  checked_at: number | null;
  last_healthy_at: number | null;
  last_unhealthy_at: number | null;
  consecutive_failures: number;
  process_check: HealthOutcomeView | null;
  tcp_check: HealthOutcomeView | null;
  http_check: HealthOutcomeView | null;
}

/** The detailed health view of one service. */
export interface HealthDetailView {
  row: HealthRowView;
  /** The latest outcome of every configured check, with duration and reason. */
  checks: HealthOutcomeView[];
  managed: boolean;
  pid: number | null;
  ports: number[];
  last_failure: DiagnosticFailureView | null;
  /** The most recent error-level log lines, oldest first. */
  recent_errors: string[];
}

// -- realtime log streaming ----------------------------------------------------

/**
 * A realtime log-stream event pushed to the adapter's `daemon://log` event.
 *
 * `kind` discriminates: `entry` carries a log line, `lagged` reports entries
 * the daemon dropped for this subscriber, and `closed` ends the stream.
 */
export interface LogStreamEvent {
  subscription_id: string;
  service_id: string;
  kind: 'entry' | 'lagged' | 'closed';
  entry: LogEntryView | null;
  /** How many entries the daemon dropped, for `lagged` events. */
  dropped: number | null;
  message: string | null;
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

/** Event name carrying a `LogStreamEvent` from the Rust adapter. */
export const LOG_EVENT = 'daemon://log';

/** Event name carrying a `ConfigurationChangedEvent` from the Rust adapter. */
export const CONFIGURATION_EVENT = 'daemon://configuration';

// -- configuration management --------------------------------------------------

/**
 * The raw configuration schema, mirrored from the config crate's `raw` module.
 *
 * These are the *unvalidated* on-disk shapes: the daemon validates and persists
 * them, and the GUI only ever edits a value the daemon returns. `Option` and
 * empty collections are omitted on the wire (`skip_serializing_if`), so every
 * optional field is `undefined` when absent.
 */

/** A literal or explicitly-secret environment value (`raw::RawEnvValue`). */
export interface RawEnvVariableView {
  value: string;
  secret: boolean;
}

/** An environment value: a bare string or a structured variable. */
export type RawEnvValueView = string | RawEnvVariableView;

/** A source repository (`raw::RawRepository`). */
export interface RawRepositoryView {
  url: string;
  default_branch?: string;
  provider?: string;
}

/** A runtime requirement (`raw::RawRuntime`). */
export interface RawRuntimeView {
  /** `java`, `node`, `maven`, `npm`, or `other:<name>`. */
  type: string;
  version?: string;
}

/** An artifact coordinate (`raw::RawArtifact`). */
export interface RawArtifactView {
  group_id: string;
  artifact_id: string;
  version: string;
  packaging?: string;
  classifier?: string;
}

/** A structured command with an explicit program and arguments. */
export interface RawStructuredCommandView {
  program: string;
  args?: string[];
  working_dir?: string;
  environment?: Record<string, RawEnvValueView>;
}

/** A command in short-string or structured form (`raw::RawCommand`). */
export type RawCommandView = string | RawStructuredCommandView;

/** The configured commands of a service or library (`raw::RawCommands`). */
export interface RawCommandsView {
  install?: RawCommandView;
  build?: RawCommandView;
  start?: RawCommandView;
  stop?: RawCommandView;
  restart?: RawCommandView;
  test?: RawCommandView;
  liquibase?: RawCommandView;
  custom?: Record<string, RawCommandView>;
}

/** A per-operation Liquibase command table (`raw::RawMigrationCommands`). */
export interface RawMigrationCommandsView {
  validate?: RawCommandView;
  status?: RawCommandView;
  pending?: RawCommandView;
  preview?: RawCommandView;
  update?: RawCommandView;
}

/** A declared migration target database (`raw::RawMigrationDatabase`). */
export interface RawMigrationDatabaseView {
  environment?: string;
  database_type?: string;
  name?: string;
  host?: string;
  schema?: string;
  contexts?: string[];
  labels?: string[];
}

/** Optional Liquibase configuration (`raw::RawLiquibase`). */
export interface RawLiquibaseView {
  enabled?: boolean;
  command?: RawCommandView;
  provider?: string;
  execution_mode?: string;
  commands?: RawMigrationCommandsView;
  timeout_seconds?: number;
  environment?: Record<string, RawEnvValueView>;
  database?: RawMigrationDatabaseView;
}

/** A configured health check (`raw::RawHealthCheck`). */
export interface RawHealthCheckView {
  /** `process`, `port`, or `http`. */
  type: string;
  url?: string;
  expected_status?: number;
  port?: number;
  timeout_seconds?: number;
  interval_seconds?: number;
  required?: boolean;
}

/** A declared port (`raw::RawPort`). */
export interface RawPortView {
  port: number;
  protocol?: string;
}

/** A detailed dependency reference (`raw::RawDetailedDependency`). */
export interface RawDetailedDependencyView {
  id: string;
  kind?: string;
  required?: boolean;
}

/** A dependency reference in short-string or detailed form. */
export type RawDependencyRefView = string | RawDetailedDependencyView;

/** The dependency tables of a service or library (`raw::RawDependencies`). */
export interface RawDependenciesView {
  services?: RawDependencyRefView[];
  libraries?: RawDependencyRefView[];
}

/** A raw workspace (`raw::RawWorkspace`). */
export interface RawWorkspaceView {
  id: string;
  name?: string;
  root: string;
  environment?: Record<string, RawEnvValueView>;
}

/** A raw service (`raw::RawService`). */
export interface RawServiceView {
  id: string;
  name?: string;
  /** `backend`, `frontend`, `library`, or `other:<name>`. */
  type: string;
  workspace: string;
  path?: string;
  default_branch?: string;
  repository?: RawRepositoryView;
  runtime?: RawRuntimeView;
  runtimes?: RawRuntimeView[];
  commands?: RawCommandsView;
  build_system?: string;
  liquibase?: RawLiquibaseView;
  health_checks?: RawHealthCheckView[];
  ports?: RawPortView[];
  dependencies?: RawDependenciesView;
  environment?: Record<string, RawEnvValueView>;
  capabilities?: string[];
}

/** A raw library (`raw::RawLibrary`). */
export interface RawLibraryView {
  id: string;
  name?: string;
  workspace: string;
  path?: string;
  default_branch?: string;
  type?: string;
  repository?: RawRepositoryView;
  artifact?: RawArtifactView;
  artifacts?: RawArtifactView[];
  runtime?: RawRuntimeView;
  runtimes?: RawRuntimeView[];
  commands?: RawCommandsView;
  build_system?: string;
  dependencies?: RawDependenciesView;
  environment?: Record<string, RawEnvValueView>;
}

/** A raw service group (`raw::RawGroup`). */
export interface RawGroupView {
  id: string;
  name?: string;
  description?: string;
  workspace?: string;
  services?: string[];
}

/** A raw developer profile (`raw::RawProfile`). */
export interface RawProfileView {
  id: string;
  name?: string;
  description?: string;
  workspace?: string;
  services?: string[];
  groups?: string[];
}

/** Global task execution settings (`raw::RawExecution`). */
export interface RawExecutionView {
  max_parallel_tasks?: number;
  default_timeout_seconds?: number;
}

/** A read-only, editable snapshot of the raw configuration. */
export interface ConfigurationSnapshotView {
  /** The revision the snapshot was read at. */
  revision: string;
  config_version: number;
  workspaces: RawWorkspaceView[];
  services: RawServiceView[];
  libraries: RawLibraryView[];
  profiles: RawProfileView[];
  groups: RawGroupView[];
  execution: RawExecutionView;
}

/** The kind of configuration entity a change targets (`EntityKind`). */
export type EntityKindView = 'workspace' | 'service' | 'library' | 'profile' | 'group';

/** A short identity for an entity in a diff (`EntitySummary`). */
export interface EntitySummaryView {
  kind: EntityKindView;
  id: string;
  name: string | null;
}

/** One changed field of an updated entity (`FieldChange`). */
export interface FieldChangeView {
  field: string;
  before: string | null;
  after: string | null;
}

/** An entity that changed in place (`UpdatedEntity`). */
export interface UpdatedEntityView {
  kind: EntityKindView;
  id: string;
  name: string | null;
  fields: FieldChangeView[];
}

/** A structured, human-readable configuration diff (`ConfigurationDiff`). */
export interface ConfigurationDiffView {
  added: EntitySummaryView[];
  updated: UpdatedEntityView[];
  removed: EntitySummaryView[];
}

/** A reference from another entity that points at a change target. */
export interface AffectedReferenceView {
  source_kind: EntityKindView;
  source_id: string;
  relation: string;
  target: string;
}

/** The impact of a change on a long-running process (`RunningServiceImpact`). */
export interface RunningServiceImpactView {
  service_id: string;
  running: boolean;
  /** `added`, `updated`, or `removed`. */
  change: string;
  action_required: string;
}

/** One actionable configuration problem (`ValidationError`). */
export interface ValidationIssueView {
  file: string | null;
  location: string;
  problem: string;
  expected: string | null;
  suggested_fix: string | null;
}

/** A create/update/remove change to a single entity (`EntityChange<T>`). */
export interface CreateEntityChangeView<T> {
  action: 'create';
  entity: T;
}
export interface UpdateEntityChangeView<T> {
  action: 'update';
  id: string;
  entity: T;
}
export interface RemoveEntityChangeView {
  action: 'remove';
  id: string;
}
export type EntityChangeView<T> =
  | CreateEntityChangeView<T>
  | UpdateEntityChangeView<T>
  | RemoveEntityChangeView;

/**
 * A single configuration change (`ConfigChange`).
 *
 * The wire form is adjacently tagged: `{ kind, change }`, and the change itself
 * is tagged by `action` (`tasks/023.md` §41). The GUI builds exactly this shape
 * and the daemon decodes it — there is no second schema.
 */
export type ConfigChangeView =
  | { kind: 'workspace'; change: EntityChangeView<RawWorkspaceView> }
  | { kind: 'service'; change: EntityChangeView<RawServiceView> }
  | { kind: 'library'; change: EntityChangeView<RawLibraryView> }
  | { kind: 'profile'; change: EntityChangeView<RawProfileView> }
  | { kind: 'group'; change: EntityChangeView<RawGroupView> };

/** The result of previewing a change without committing it. */
export interface ConfigurationPreviewView {
  revision: string;
  proposed_revision: string;
  valid: boolean;
  diff: ConfigurationDiffView;
  issues: ValidationIssueView[];
  affected_references: AffectedReferenceView[];
  running_impact: RunningServiceImpactView[];
}

/** The outcome of validating the active configuration file. */
export interface ConfigurationValidationView {
  revision: string;
  valid: boolean;
  issues: ValidationIssueView[];
}

/** A compact count of the declared entities (`ConfigurationCounts`). */
export interface ConfigurationCountsView {
  workspaces: number;
  services: number;
  libraries: number;
  profiles: number;
  groups: number;
}

/** The outcome of committing a configuration change. */
export interface ConfigurationCommitView {
  revision: string;
  description: string;
  counts: ConfigurationCountsView;
}

/** The outcome of reloading the configuration from disk. */
export interface ConfigurationReloadView {
  revision: string;
  counts: ConfigurationCountsView;
}

/** Whether an execution provider for a build system is installed. */
export type ProviderAvailabilityView =
  | 'available'
  | 'provider_not_installed'
  | 'not_applicable';

/** A build system detected in a repository (`DetectedBuildSystem`). */
export interface DetectedBuildSystemView {
  system: string;
  label: string;
  availability: ProviderAvailabilityView;
  evidence: string[];
  note?: string;
}

/** A runtime requirement suggested by a repository (`DetectedRuntime`). */
export interface DetectedRuntimeView {
  kind: string;
  label: string;
  required_version?: string;
  evidence: string[];
}

/** A non-build integration detected in a repository (`DetectedIntegration`). */
export interface DetectedIntegrationView {
  kind: string;
  label: string;
  evidence: string[];
}

/** A bounded, read-only summary of a Maven project (`MavenMetadata`). */
export interface MavenMetadataView {
  group_id?: string;
  artifact_id?: string;
  version?: string;
  packaging?: string;
  java_version?: string;
  modules?: string[];
  wrapper_available: boolean;
  spring_boot: boolean;
}

/** A bounded, read-only summary of a Node project (`NodeMetadata`). */
export interface NodeMetadataView {
  name?: string;
  version?: string;
  required_node_version?: string;
  package_manager?: string;
  scripts?: string[];
  react: boolean;
}

/** A read-only inspection of a local repository directory. */
export interface RepositoryProbeView {
  path: string;
  exists: boolean;
  is_directory: boolean;
  is_git_repository: boolean;
  branch?: string;
  remote?: string;
  project_files?: string[];
  build_systems?: DetectedBuildSystemView[];
  runtimes?: DetectedRuntimeView[];
  integrations?: DetectedIntegrationView[];
  maven?: MavenMetadataView;
  node?: NodeMetadataView;
  notes?: string[];
}

/** A suggested command (never a shell line) (`SuggestedCommand`). */
export interface SuggestedCommandView {
  program: string;
  args: string[];
}

/** A starting-point service configuration from a repository (`ProjectConfigurationSuggestion`). */
export interface ProjectSuggestionView {
  path: string;
  service_type?: string;
  build_system?: string;
  runtimes?: DetectedRuntimeView[];
  build_command?: SuggestedCommandView;
  start_command?: SuggestedCommandView;
  integrations?: DetectedIntegrationView[];
  warnings?: string[];
  inspection: RepositoryProbeView;
}

/** An active-configuration change pushed to the adapter's `daemon://configuration`. */
export interface ConfigurationChangedEvent {
  /** The revision of the configuration after the change. */
  revision: string;
  /** A short, human-readable description of what changed. */
  description: string;
}

// -- profiles, groups, and environment workflows -------------------------------

/**
 * A configured developer profile expanded for its workspace (`ProfileView`).
 *
 * `resolved_services` is configuration-derived (explicit services plus the
 * members of referenced groups); the full dependency closure comes from the
 * read-only plan, never from the frontend (`tasks/022.md` §8, §12).
 */
export interface ProfileView {
  id: string;
  display_name: string;
  description: string | null;
  workspace_id: string;
  /** Services referenced directly by the profile. */
  services: string[];
  /** Groups referenced by the profile. */
  groups: string[];
  /** Explicit services plus referenced group members, deduplicated. */
  resolved_services: string[];
}

/** A configured service group expanded for its workspace (`GroupView`). */
export interface GroupView {
  id: string;
  display_name: string;
  description: string | null;
  workspace_id: string;
  /** Member services, in configured order. */
  services: string[];
}

/** One planned step of an environment preparation (`PlanStepView`). */
export interface PlanStepView {
  /** The phase the step belongs to (`git-sync`, `migration`, …). */
  phase: string;
  /** The task kind the step performs (`git_sync`, `liquibase`, …). */
  task: string;
  /** The service the step acts on, when any. */
  service: string | null;
  /** The library the step acts on, when any. */
  library: string | null;
  /** A human-readable description. */
  description: string;
}

/** A planned phase and its steps (`PlanPhaseView`). */
export interface PlanPhaseView {
  /** The phase label, in canonical execution order. */
  kind: string;
  /** The planned steps of the phase, in planned order. */
  steps: PlanStepView[];
}

/**
 * The daemon's read-only plan for preparing a development environment
 * (`EnvironmentPlanView`).
 *
 * Every field is copied from the application core: the GUI renders this
 * backend-generated plan and never reconstructs dependency or execution logic
 * (`tasks/022.md` §12, §13).
 */
export interface EnvironmentPlanView {
  workspace_id: string;
  /** The branch that would be checked out, if any. */
  target_branch: string | null;
  /** Services the caller explicitly selected. */
  selected_services: string[];
  /** Every service that would participate, including dependencies. */
  resolved_services: string[];
  /** Services added automatically as required dependencies. */
  included_dependencies: string[];
  /** Libraries that would be prepared, in dependency order. */
  required_libraries: string[];
  /** Planned phases, in execution order. */
  phases: PlanPhaseView[];
  /** Total number of steps across every phase. */
  step_count: number;
  /** Whether any step requires destructive authorization. */
  requires_destructive_authorization: boolean;
  /** Non-fatal observations reported by the planner. */
  warnings: string[];
}

/** Per-phase progress within a running workflow (`WorkflowPhaseProgressView`). */
export interface WorkflowPhaseProgressView {
  /** The phase label. */
  phase: string;
  total: number;
  succeeded: number;
  failed: number;
  /** Tasks still queued, blocked, or running. */
  pending: number;
}

/** A structured failure within a workflow (`WorkflowTaskFailureView`). */
export interface WorkflowTaskFailureView {
  task_id: string;
  /** The task kind label. */
  kind: string;
  service: string | null;
  library: string | null;
  /** A stable, machine-readable error code. */
  code: string;
  message: string;
}

/**
 * Aggregate workflow states reported by the daemon.
 *
 * The state is derived by the core from the workflow's tasks; the GUI never
 * computes one itself (`tasks/022.md` §19, §25).
 */
export type WorkflowRunState =
  | 'queued'
  | 'running'
  | 'succeeded'
  | 'partially_succeeded'
  | 'failed'
  | 'cancelled';

/** Human-readable labels for `WorkflowRunState`. */
export const WORKFLOW_RUN_STATE_LABELS: Record<WorkflowRunState, string> = {
  queued: 'Queued',
  running: 'Running',
  succeeded: 'Succeeded',
  partially_succeeded: 'Partially succeeded',
  failed: 'Failed',
  cancelled: 'Cancelled',
};

/** States a workflow can no longer leave. */
const WORKFLOW_TERMINAL_STATES: ReadonlySet<string> = new Set([
  'succeeded',
  'partially_succeeded',
  'failed',
  'cancelled',
]);

/** Whether a workflow state is terminal. */
export function isTerminalWorkflowState(state: string): boolean {
  return WORKFLOW_TERMINAL_STATES.has(state);
}

/**
 * An aggregated, structured status for one workflow (`WorkflowStatusView`).
 *
 * Counts, phase progress, and structured failures come straight from the
 * application core; the GUI never fabricates percentages or outcomes.
 */
export interface WorkflowStatusView {
  workflow_id: string;
  /** The workflow kind label (`prepare_environment`, …). */
  kind: string;
  /** The aggregate state (`queued`, `running`, `partially_succeeded`, …). */
  state: string;
  total: number;
  succeeded: number;
  failed: number;
  cancelled: number;
  skipped: number;
  /** Tasks still queued, blocked, or running. */
  pending: number;
  /** When the first task started, seconds since the epoch. */
  started_at: number | null;
  /** When the last task finished, seconds since the epoch. */
  finished_at: number | null;
  /** Progress per phase, in canonical phase order. */
  phases: WorkflowPhaseProgressView[];
  /** Every failed task, with its structured error. */
  failures: WorkflowTaskFailureView[];
}

/**
 * One workflow on the bounded, workspace-scoped history list
 * (`WorkflowSummaryView`).
 *
 * History is derived from the task engine's retained tasks: it is bounded and
 * disappears with daemon restarts — nothing is persisted or invented.
 */
export interface WorkflowSummaryView {
  workflow_id: string;
  kind: string;
  state: string;
  workspace_id: string;
  total: number;
  succeeded: number;
  failed: number;
  cancelled: number;
  skipped: number;
  pending: number;
  started_at: number | null;
  finished_at: number | null;
}

/**
 * Input for planning and executing an environment workflow
 * (`PrepareEnvironmentInput`).
 *
 * Exactly one of `services`, `profile`, or `group` must be set; the daemon
 * rejects anything else. Omitted flags fall back to the adapter's safe
 * defaults (sync and dependency inclusion on; migrations, library builds,
 * service builds, and startup off), and no confirmation flag exists on this
 * path — the daemon re-validates policy at execution time
 * (`tasks/022.md` §11, §32).
 */
export interface PrepareEnvironmentInput {
  workspace_id: string;
  /** Explicitly selected service identifiers. */
  services?: string[];
  /** Profile identifier (mutually exclusive with `services`/`group`). */
  profile?: string;
  /** Group identifier (mutually exclusive with `services`/`profile`). */
  group?: string;
  /** An optional branch to check out after synchronizing. */
  branch?: string;
  /** Synchronize repositories first. Defaults to `true`. */
  sync?: boolean;
  /** Include the required dependency closure. Defaults to `true`. */
  include_dependencies?: boolean;
  /** Build the required libraries. Defaults to `false`. */
  build_libraries?: boolean;
  /** Run Liquibase migrations. Defaults to `false`. */
  run_liquibase?: boolean;
  /** Build the selected services. Defaults to `false`. */
  build?: boolean;
  /** Start the selected services. Defaults to `false`. */
  start?: boolean;
  /** Wait for health after starting. Defaults to `true`. */
  wait_for_health?: boolean;
}
