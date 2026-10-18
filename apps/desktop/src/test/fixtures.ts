import type {
  AffectedReferenceView,
  ArtifactView,
  ArtifactVerificationView,
  BranchView,
  BuildCapabilitiesView,
  BuildTargetSummary,
  BulkImportCommitView,
  BulkImportPlanView,
  BulkImportRowView,
  BulkSummaryView,
  ConfigChangeView,
  ConfigurationCommitView,
  ConfigurationCountsView,
  ConfigurationPreviewView,
  ConfigurationReloadView,
  ConfigurationSnapshotView,
  ConfigurationValidationView,
  ConnectionSnapshot,
  DaemonStatus,
  DiscoveredRepositoryView,
  DiscoveryScanInputView,
  EnvironmentPlanView,
  GitStatusView,
  GitSyncPreview,
  GitSyncPreviewEntry,
  GroupView,
  HealthDetailView,
  HealthOutcomeView,
  HealthRowView,
  LibrarySummary,
  LogEntryView,
  LogPageView,
  MigrationPresetSummaryView,
  MigrationRowView,
  MigrationStatusView,
  OperationOutcome,
  OperationPresetSummaryView,
  PrepareEnvironmentInput,
  ProfileView,
  ProjectSuggestionView,
  RawArtifactView,
  RawDiscoveryView,
  RawGroupView,
  RawLibraryView,
  RawMigrationPresetView,
  RawOperationPresetView,
  RawProfileView,
  RawServiceView,
  RawTemplateView,
  RepositoryDiscoveryReportView,
  RepositoryInspectionView,
  RepositoryProbeView,
  RepositorySummary,
  RunningServiceImpactView,
  ServiceDetail,
  ServiceSummary,
  TaskView,
  TechnologyDetectionView,
  ValidationIssueView,
  VersionAlignmentView,
  WorkflowStatusView,
  WorkflowSummaryView,
  WorkflowView,
  WorkspaceStatus,
  WorkspaceSummary,
} from '../types/models';

/** Builds a service row with sensible defaults, overridable per test. */
export function makeService(overrides: Partial<ServiceSummary> = {}): ServiceSummary {
  return {
    id: 'orders-api',
    display_name: 'orders-api',
    kind: 'backend',
    workspace_id: 'demo',
    process_state: 'running',
    health_state: 'healthy',
    git_state: 'clean',
    branch: 'main',
    port: 8081,
    ...overrides,
  };
}

/** The demo workspace, mirroring the fixture configuration. */
export function makeWorkspace(overrides: Partial<WorkspaceSummary> = {}): WorkspaceSummary {
  return {
    id: 'demo',
    name: 'Demo Platform',
    root: '/Users/dev/Projects/demo',
    service_count: 2,
    ...overrides,
  };
}

/** The demo workspace's two services (backend + frontend), as configured. */
export function makeServices(): ServiceSummary[] {
  return [
    makeService(),
    makeService({
      id: 'admin-ui',
      display_name: 'admin-ui',
      kind: 'frontend',
      process_state: 'not_running',
      health_state: 'unknown',
      port: 3000,
    }),
  ];
}

/** Builds a workspace-status response around the given services. */
export function makeWorkspaceStatus(services: ServiceSummary[]): WorkspaceStatus {
  const workspace = makeWorkspace();
  return {
    workspace_id: workspace.id,
    name: workspace.name,
    root: workspace.root,
    services,
  };
}

/** Builds a daemon status snapshot with sensible defaults. */
export function makeDaemonStatus(overrides: Partial<DaemonStatus> = {}): DaemonStatus {
  return {
    pid: 4242,
    socket_path: '/Users/dev/.service-orchestrator/run/daemon.sock',
    protocol_version: 1,
    clients: 1,
    active_tasks: 0,
    managed_services: 1,
    started_at_unix: 1_700_000_000,
    ...overrides,
  };
}

/** A connected snapshot. */
export function makeConnected(): ConnectionSnapshot {
  return { state: 'connected', message: null, since_unix: 1_700_000_000 };
}

/** A disconnected snapshot with a reason. */
export function makeDisconnected(message: string): ConnectionSnapshot {
  return { state: 'disconnected', message, since_unix: 1_700_000_010 };
}

/** Builds a normalized operation outcome with sensible defaults. */
export function makeOperationOutcome(
  overrides: Partial<OperationOutcome> = {},
): OperationOutcome {
  return {
    status: 'task',
    task_id: 'task-1',
    workflow_id: null,
    kind: 'start_service',
    ...overrides,
  };
}

/** Builds a detailed service view with sensible defaults. */
export function makeServiceDetail(overrides: Partial<ServiceDetail> = {}): ServiceDetail {
  return {
    id: 'orders-api',
    display_name: 'orders-api',
    kind: 'backend',
    workspace_id: 'demo',
    path: 'backend/orders-api',
    repository: {
      url: 'https://example.com/orders-api.git',
      local_path: 'backend/orders-api',
      default_branch: 'main',
      provider: 'github',
    },
    runtimes: [{ kind: 'java', version: '21' }],
    build_system: 'maven',
    capabilities: ['run', 'build'],
    process_state: 'running',
    health_state: 'healthy',
    git_state: 'clean',
    branch: 'main',
    managed: true,
    pid: 4321,
    ports: [8081],
    dependencies: [
      { target: 'admin-ui', target_kind: 'service', kind: 'runtime', required: true },
    ],
    health_checks: [{ kind: 'http', required: true }],
    health_outcomes: [
      {
        check_type: 'http',
        target: 'http://localhost:8081/actuator/health',
        required: true,
        state: 'healthy',
        duration_ms: 12,
        message: null,
        checked_at: 1_700_000_000,
      },
    ],
    last_checked_at: 1_700_000_000,
    last_failure: null,
    recent_errors: [],
    ...overrides,
  };
}

/** Builds a repository row with sensible defaults. */
export function makeRepository(
  overrides: Partial<RepositorySummary> = {},
): RepositorySummary {
  return {
    id: 'demo:orders-api',
    service_id: 'orders-api',
    workspace_id: 'demo',
    name: 'orders-api',
    path: 'backend/orders-api',
    repository: {
      url: 'https://example.com/orders-api.git',
      local_path: 'backend/orders-api',
      default_branch: 'main',
      provider: 'github',
    },
    git_state: 'clean',
    branch: 'main',
    ...overrides,
  };
}

/** The demo workspace's two repositories, as configured. */
export function makeRepositories(): RepositorySummary[] {
  return [
    makeRepository(),
    makeRepository({
      id: 'demo:admin-ui',
      service_id: 'admin-ui',
      name: 'admin-ui',
      path: 'frontend/admin-ui',
      repository: {
        url: 'https://example.com/admin-ui.git',
        local_path: 'frontend/admin-ui',
        default_branch: 'main',
        provider: 'github',
      },
    }),
  ];
}

/** Builds a working-tree inspection with sensible defaults. */
export function makeGitStatus(overrides: Partial<GitStatusView> = {}): GitStatusView {
  return {
    service_id: 'orders-api',
    lifecycle: 'available',
    branch: 'main',
    commit: '0123456789abcdef',
    detached_head: false,
    unborn: false,
    upstream: 'origin/main',
    ahead: 0,
    behind: 0,
    staged_changes: 0,
    unstaged_changes: 0,
    untracked_files: 0,
    conflicts: 0,
    clean: true,
    has_local_changes: false,
    diverged: false,
    ...overrides,
  };
}

/** Builds a repository branch listing with a local and a remote ref. */
export function makeBranches(): BranchView[] {
  return [
    { name: 'main', kind: 'local', current: true, upstream: 'origin/main', commit: 'abc' },
    { name: 'origin/main', kind: 'remote', current: false, upstream: null, commit: 'abc' },
  ];
}

/** Builds one bulk-sync preview entry. */
export function makePreviewEntry(
  overrides: Partial<GitSyncPreviewEntry> = {},
): GitSyncPreviewEntry {
  return {
    repository_id: 'demo:orders-api',
    service_id: 'orders-api',
    name: 'orders-api',
    current_branch: 'main',
    target_branch_exists: true,
    target_branch_kind: 'local',
    lifecycle: 'available',
    git_state: 'clean',
    clean: true,
    ahead: 0,
    behind: 0,
    diverged: false,
    expected_operations: ['fetch', 'checkout', 'pull'],
    warnings: [],
    blocked: false,
    ...overrides,
  };
}

/** Builds a bulk Git preview with sensible defaults. */
export function makeGitSyncPreview(
  overrides: Partial<GitSyncPreview> = {},
): GitSyncPreview {
  const entries = overrides.entries ?? [
    makePreviewEntry(),
    makePreviewEntry({
      repository_id: 'demo:admin-ui',
      service_id: 'admin-ui',
      name: 'admin-ui',
      current_branch: 'feature/x',
      target_branch_kind: 'remote',
      git_state: 'modified',
      clean: false,
      warnings: ['working tree has uncommitted local changes'],
    }),
  ];
  return {
    workspace_id: 'demo',
    target_branch: 'main',
    mode: 'sync',
    entries,
    total: entries.length,
    ready: entries.filter((entry) => !entry.blocked && entry.warnings.length === 0).length,
    dirty: entries.filter((entry) => !entry.clean).length,
    missing_branch: entries.filter((entry) => !entry.target_branch_exists).length,
    diverged: entries.filter((entry) => entry.diverged).length,
    unavailable: entries.filter((entry) => entry.lifecycle !== 'available').length,
    ...overrides,
  };
}

/** Builds one repository inspection result. */
export function makeInspection(
  overrides: Partial<RepositoryInspectionView> = {},
): RepositoryInspectionView {
  return {
    repository_id: 'demo:orders-api',
    service_id: 'orders-api',
    status: makeGitStatus(),
    error: null,
    ...overrides,
  };
}

/** Builds the maven build operations the real provider declares. */
export function makeMavenCapabilities(): BuildCapabilitiesView {
  return {
    build_system: 'maven',
    empty: false,
    operations: [
      {
        operation: 'clean',
        supported: true,
        exposed: false,
        reason: 'supported by the provider but not exposed by the application core for services',
      },
      { operation: 'build', supported: true, exposed: true, reason: null },
      {
        operation: 'test',
        supported: true,
        exposed: false,
        reason: 'supported by the provider but not exposed by the application core for services',
      },
      {
        operation: 'install_dependencies',
        supported: true,
        exposed: false,
        reason: 'supported by the provider but not exposed by the application core for services',
      },
      {
        operation: 'publish_local_artifact',
        supported: true,
        exposed: false,
        reason: 'supported by the provider but not exposed by the application core for services',
      },
    ],
  };
}

/** Builds the npm build operations the real provider declares. */
export function makeNpmCapabilities(): BuildCapabilitiesView {
  return {
    build_system: 'npm',
    empty: false,
    operations: [
      { operation: 'clean', supported: false, exposed: false, reason: 'the configured build provider does not support this operation' },
      { operation: 'build', supported: true, exposed: true, reason: null },
      { operation: 'test', supported: false, exposed: false, reason: 'the configured build provider does not support this operation' },
      { operation: 'install_dependencies', supported: true, exposed: false, reason: 'supported by the provider but not exposed by the application core for services' },
      { operation: 'publish_local_artifact', supported: false, exposed: false, reason: 'the configured build provider does not support this operation' },
    ],
  };
}

/** Builds a build-target row with sensible defaults. */
export function makeBuildTarget(
  overrides: Partial<BuildTargetSummary> = {},
): BuildTargetSummary {
  return {
    service_id: 'orders-api',
    display_name: 'orders-api',
    workspace_id: 'demo',
    kind: 'backend',
    build_system: 'maven',
    runtimes: [{ kind: 'java', version: '21' }],
    capability: makeMavenCapabilities(),
    capability_error: null,
    ...overrides,
  };
}

/** The demo workspace's two build targets (Maven backend + npm frontend). */
export function makeBuildTargets(): BuildTargetSummary[] {
  return [
    makeBuildTarget(),
    makeBuildTarget({
      service_id: 'admin-ui',
      display_name: 'admin-ui',
      kind: 'frontend',
      build_system: 'npm',
      runtimes: [{ kind: 'node', version: '20.x' }],
      capability: makeNpmCapabilities(),
    }),
  ];
}

/** Builds an artifact coordinate with sensible defaults. */
export function makeArtifact(overrides: Partial<ArtifactView> = {}): ArtifactView {
  return {
    group_id: 'com.demo',
    artifact_id: 'shared-kit',
    version: '1.8.0-SNAPSHOT',
    packaging: 'jar',
    classifier: null,
    file_name: 'shared-kit-1.8.0-SNAPSHOT.jar',
    coordinate: 'com.demo:shared-kit:1.8.0-SNAPSHOT',
    ...overrides,
  };
}

/** Builds a library row with sensible defaults. */
export function makeLibrary(overrides: Partial<LibrarySummary> = {}): LibrarySummary {
  return {
    id: 'shared-kit',
    display_name: 'shared-kit',
    workspace_id: 'demo',
    build_system: 'maven',
    repository_url: 'https://example.com/shared-kit.git',
    artifact: makeArtifact(),
    artifacts: [makeArtifact()],
    branch: 'main',
    commit: '0123456789abcdef',
    git_state: 'clean',
    repository_error: null,
    configured_version: '1.8.0-SNAPSHOT',
    dependents: [
      { service_id: 'orders-api', workspace_id: 'demo', required: true, kind: 'build' },
    ],
    ...overrides,
  };
}

/** The demo workspace's single library. */
export function makeLibraries(): LibrarySummary[] {
  return [makeLibrary()];
}

/** A mixed set of tasks covering several statuses, newest first. */
export function makeTasks(): TaskView[] {
  return [
    makeTaskView({
      id: 'task-3',
      kind: 'migration_apply',
      status: 'failed',
      created_at: 1_700_000_200,
      started_at: 1_700_000_201,
      finished_at: 1_700_000_210,
      duration_ms: 9_000,
      progress: { current: 1, total: 1, message: null, fraction: 1 },
      failure: {
        code: 'MIGRATION_APPLY_FAILED',
        message: 'changeset 3 was rejected by the database',
        detail: 'exit 1',
      },
    }),
    makeTaskView({
      id: 'task-2',
      kind: 'build_service',
      status: 'succeeded',
      created_at: 1_700_000_150,
      started_at: 1_700_000_151,
      finished_at: 1_700_000_160,
      duration_ms: 9_000,
      progress: { current: 3, total: 3, message: null, fraction: 1 },
    }),
    makeTaskView({
      id: 'task-1',
      kind: 'start_service',
      status: 'running',
      service_id: 'orders-api',
      created_at: 1_700_000_100,
      started_at: 1_700_000_101,
      workflow_id: 'wf-1',
    }),
  ];
}

/** Builds an artifact-verification result. */
export function makeVerification(
  overrides: Partial<ArtifactVerificationView> = {},
): ArtifactVerificationView {
  return {
    library_id: 'shared-kit',
    workspace_id: 'demo',
    artifact: makeArtifact(),
    present: true,
    path: '/Users/dev/.m2/repository/com/demo/shared-kit/1.8.0-SNAPSHOT/shared-kit-1.8.0-SNAPSHOT.jar',
    message: "artifact 'com.demo:shared-kit:1.8.0-SNAPSHOT' is present",
    ...overrides,
  };
}

/** Builds a version-alignment view (aligned by default). */
export function makeAlignment(
  overrides: Partial<VersionAlignmentView> = {},
): VersionAlignmentView {
  const consumers = overrides.consumers ?? [
    {
      service_id: 'orders-api',
      workspace_id: 'demo',
      path: '/Users/dev/Projects/demo/backend/orders-api',
      current_version: '1.8.0-SNAPSHOT',
      status: 'aligned' as const,
      detail: null,
    },
  ];
  return {
    library_id: 'shared-kit',
    workspace_id: 'demo',
    target: makeArtifact(),
    consumers,
    aligned_count: consumers.filter((consumer) => consumer.status === 'aligned').length,
    total: consumers.length,
    fully_aligned: consumers.every((consumer) => consumer.status === 'aligned'),
    ...overrides,
  };
}

/** Builds a task view with sensible defaults (running). */
export function makeTaskView(overrides: Partial<TaskView> = {}): TaskView {
  return {
    id: 'task-1',
    kind: 'build_service',
    status: 'running',
    service_id: 'orders-api',
    library_id: null,
    workflow_id: null,
    workspace_id: 'demo',
    created_at: 1_700_000_000,
    started_at: 1_700_000_001,
    finished_at: null,
    duration_ms: 1_500,
    progress: { current: 1, total: 3, message: 'running build for orders-api', fraction: 0.33 },
    failure: null,
    dependencies: [],
    ...overrides,
  };
}

/** Builds a workflow summary. */
export function makeBulkSummary(overrides: Partial<BulkSummaryView> = {}): BulkSummaryView {
  return {
    total: 2,
    succeeded: 2,
    failed: 0,
    cancelled: 0,
    skipped: 0,
    pending: 0,
    complete: true,
    ...overrides,
  };
}

/** Builds a bounded page of logs. */
export function makeLogPage(overrides: Partial<LogPageView> = {}): LogPageView {
  return {
    entries: [
      {
        timestamp: 1_700_000_010,
        level: 'info',
        message: 'Compiling 3 source files',
        source: 'build',
        stream: 'stdout',
        service_id: 'orders-api',
        task_id: 'task-1',
        process_id: null,
        sequence: 1,
      },
      {
        timestamp: 1_700_000_011,
        level: 'error',
        message: 'BUILD FAILURE: missing dependency',
        source: 'build',
        stream: 'stderr',
        service_id: 'orders-api',
        task_id: 'task-1',
        process_id: null,
        sequence: 2,
      },
    ],
    next_cursor: null,
    truncated: false,
    ...overrides,
  };
}

/** Builds one log entry with sensible defaults. */
export function makeLogEntry(
  sequence: number,
  overrides: Partial<LogEntryView> = {},
): LogEntryView {
  return {
    timestamp: 1_700_000_000 + sequence,
    level: 'info',
    message: `line ${sequence}`,
    source: 'process',
    stream: 'stdout',
    service_id: 'orders-api',
    task_id: null,
    process_id: 'proc-1',
    sequence,
    ...overrides,
  };
}

/** Builds `count` consecutive log entries, oldest first. */
export function makeLogEntries(count: number): LogEntryView[] {
  return Array.from({ length: count }, (_, index) => makeLogEntry(index + 1));
}

// -- migrations ----------------------------------------------------------------

/** Builds a configuration-derived migration snapshot (local target). */
export function makeMigrationStatus(
  overrides: Partial<MigrationStatusView> = {},
): MigrationStatusView {
  return {
    service_id: 'orders-api',
    workspace_id: 'demo',
    enabled: true,
    provider: 'liquibase',
    execution_mode: 'maven',
    environment: 'local',
    database_identity: 'jdbc:postgresql://localhost:5432/orders',
    requires_authorization: false,
    timeout_seconds: 300,
    pending: null,
    locked: null,
    ...overrides,
  };
}

/** One migration row: a local service plus a staging service. */
export function makeMigrationRows(): MigrationRowView[] {
  return [
    {
      status: makeMigrationStatus(),
      latest_task: makeTaskView({
        id: 'task-mig-1',
        kind: 'migration_apply',
        status: 'succeeded',
        created_at: 1_700_000_100,
        started_at: 1_700_000_101,
        finished_at: 1_700_000_105,
        duration_ms: 4_000,
        progress: { current: 1, total: 1, message: null, fraction: 1 },
      }),
    },
    {
      status: makeMigrationStatus({
        service_id: 'billing-api',
        environment: 'staging',
        execution_mode: 'cli',
        database_identity: 'jdbc:postgresql://staging-db:5432/billing',
        requires_authorization: true,
      }),
      latest_task: null,
    },
  ];
}

// -- health --------------------------------------------------------------------

/** Builds one observed health-check outcome. */
export function makeHealthOutcome(
  overrides: Partial<HealthOutcomeView> = {},
): HealthOutcomeView {
  return {
    check_type: 'http',
    target: 'http://localhost:8081/actuator/health',
    required: true,
    state: 'healthy',
    duration_ms: 12,
    message: null,
    checked_at: 1_700_000_000,
    ...overrides,
  };
}

/** Builds one health row with sensible defaults. */
export function makeHealthRow(overrides: Partial<HealthRowView> = {}): HealthRowView {
  return {
    service_id: 'orders-api',
    display_name: 'orders-api',
    workspace_id: 'demo',
    process_state: 'running',
    health_state: 'healthy',
    checked_at: 1_700_000_000,
    last_healthy_at: 1_700_000_000,
    last_unhealthy_at: 1_699_999_000,
    consecutive_failures: 0,
    process_check: makeHealthOutcome({ check_type: 'process', target: null }),
    tcp_check: makeHealthOutcome({ check_type: 'tcp', target: 'localhost:8081' }),
    http_check: makeHealthOutcome(),
    ...overrides,
  };
}

/** The demo workspace's two health rows. */
export function makeHealthRows(): HealthRowView[] {
  return [
    makeHealthRow(),
    makeHealthRow({
      service_id: 'admin-ui',
      display_name: 'admin-ui',
      process_state: 'not_running',
      health_state: 'unknown',
      checked_at: null,
      last_healthy_at: null,
      last_unhealthy_at: null,
      consecutive_failures: 0,
      process_check: null,
      tcp_check: null,
      http_check: null,
    }),
  ];
}

/** Builds a detailed health view for one service. */
export function makeHealthDetail(
  overrides: Partial<HealthDetailView> = {},
): HealthDetailView {
  return {
    row: makeHealthRow(),
    checks: [
      makeHealthOutcome({ check_type: 'process', target: null }),
      makeHealthOutcome({ check_type: 'tcp', target: 'localhost:8081' }),
      makeHealthOutcome(),
    ],
    managed: true,
    pid: 4321,
    ports: [8081],
    last_failure: null,
    recent_errors: [],
    ...overrides,
  };
}

// -- workflows -----------------------------------------------------------------

/** Builds a workflow with real step ids and dependency edges. */
export function makeWorkflow(overrides: Partial<WorkflowView> = {}): WorkflowView {
  return {
    id: 'wf-1',
    kind: 'prepare_environment',
    steps: [
      { id: 'step-1', description: 'sync repository', task_kind: 'git_sync', depends_on: [] },
      {
        id: 'step-2',
        description: 'apply migrations',
        task_kind: 'migration_apply',
        depends_on: ['step-1'],
      },
      {
        id: 'step-3',
        description: 'build service',
        task_kind: 'build_service',
        depends_on: ['step-2'],
      },
    ],
    ...overrides,
  };
}

// -- profiles, groups, and environment workflows -------------------------------

/** Builds a profile expanded for the demo workspace. */
export function makeProfile(overrides: Partial<ProfileView> = {}): ProfileView {
  return {
    id: 'full-stack',
    display_name: 'Full stack',
    description: 'Backend plus frontend for local development',
    workspace_id: 'demo',
    services: ['orders-api'],
    groups: ['core'],
    resolved_services: ['orders-api', 'admin-ui'],
    ...overrides,
  };
}

/** Builds the demo workspace's profiles. */
export function makeProfiles(): ProfileView[] {
  return [makeProfile()];
}

/** Builds a service group expanded for the demo workspace. */
export function makeGroup(overrides: Partial<GroupView> = {}): GroupView {
  return {
    id: 'core',
    display_name: 'Core services',
    description: null,
    workspace_id: 'demo',
    services: ['admin-ui'],
    ...overrides,
  };
}

/** Builds the demo workspace's groups. */
export function makeGroups(): GroupView[] {
  return [makeGroup()];
}

/** Builds a read-only environment plan with sensible defaults. */
export function makeEnvironmentPlan(
  overrides: Partial<EnvironmentPlanView> = {},
): EnvironmentPlanView {
  return {
    workspace_id: 'demo',
    target_branch: 'main',
    selected_services: ['orders-api'],
    resolved_services: ['orders-api', 'admin-ui'],
    included_dependencies: ['admin-ui'],
    required_libraries: ['shared-kit'],
    phases: [
      {
        kind: 'git-sync',
        steps: [
          {
            phase: 'git-sync',
            task: 'git_sync',
            service: 'orders-api',
            library: null,
            description: 'sync orders-api',
          },
        ],
      },
      {
        kind: 'service-build',
        steps: [
          {
            phase: 'service-build',
            task: 'build_service',
            service: 'orders-api',
            library: null,
            description: 'build orders-api',
          },
        ],
      },
    ],
    step_count: 2,
    requires_destructive_authorization: false,
    warnings: ["working tree of 'admin-ui' has uncommitted local changes"],
    ...overrides,
  };
}

/** Builds an aggregated workflow status (running by default). */
export function makeWorkflowStatus(
  overrides: Partial<WorkflowStatusView> = {},
): WorkflowStatusView {
  return {
    workflow_id: 'wf-env-1',
    kind: 'prepare_environment',
    state: 'running',
    total: 3,
    succeeded: 1,
    failed: 0,
    cancelled: 0,
    skipped: 0,
    pending: 2,
    started_at: 1_700_000_000,
    finished_at: null,
    phases: [
      { phase: 'git-sync', total: 1, succeeded: 1, failed: 0, pending: 0 },
      { phase: 'service-build', total: 2, succeeded: 0, failed: 0, pending: 2 },
    ],
    failures: [],
    ...overrides,
  };
}

/** Builds a partially succeeded workflow status with one structured failure. */
export function makePartiallySucceededStatus(): WorkflowStatusView {
  return makeWorkflowStatus({
    state: 'partially_succeeded',
    succeeded: 2,
    failed: 1,
    pending: 0,
    finished_at: 1_700_000_060,
    phases: [
      { phase: 'git-sync', total: 1, succeeded: 1, failed: 0, pending: 0 },
      { phase: 'service-build', total: 2, succeeded: 1, failed: 1, pending: 0 },
    ],
    failures: [
      {
        task_id: 'task-build-admin',
        kind: 'build_service',
        service: 'admin-ui',
        library: null,
        code: 'BUILD_FAILED',
        message: 'npm run build exited with code 1',
      },
    ],
  });
}

/** Builds one bounded workflow-history row. */
export function makeWorkflowSummary(
  overrides: Partial<WorkflowSummaryView> = {},
): WorkflowSummaryView {
  return {
    workflow_id: 'wf-env-1',
    kind: 'prepare_environment',
    state: 'succeeded',
    workspace_id: 'demo',
    total: 3,
    succeeded: 3,
    failed: 0,
    cancelled: 0,
    skipped: 0,
    pending: 0,
    started_at: 1_700_000_000,
    finished_at: 1_700_000_045,
    ...overrides,
  };
}

/** The demo workspace's recent workflow history, newest first. */
export function makeWorkflowHistory(): WorkflowSummaryView[] {
  return [
    makeWorkflowSummary(),
    makeWorkflowSummary({
      workflow_id: 'wf-env-0',
      state: 'partially_succeeded',
      total: 3,
      succeeded: 2,
      failed: 1,
      started_at: 1_699_999_000,
      finished_at: 1_699_999_040,
    }),
  ];
}

// -- configuration management --------------------------------------------------

/** Builds a raw service mirroring the demo fixture. */
export function makeRawService(
  overrides: Partial<RawServiceView> = {},
): RawServiceView {
  return {
    id: 'orders-api',
    name: 'orders-api',
    type: 'backend',
    workspace: 'demo',
    path: 'backend/orders-api',
    default_branch: 'main',
    repository: {
      url: 'https://example.com/orders-api.git',
      default_branch: 'main',
      provider: 'github',
    },
    runtime: { type: 'java', version: '21' },
    commands: { build: './mvnw clean install', start: './mvnw spring-boot:run' },
    build_system: 'maven',
    health_checks: [{ type: 'http', url: 'http://localhost:8081/actuator/health' }],
    ports: [{ port: 8081 }],
    dependencies: { services: ['admin-ui'] },
    environment: { LOGGING_LEVEL: 'INFO' },
    ...overrides,
  };
}

/** Builds a raw library mirroring the demo fixture. */
export function makeRawLibrary(
  overrides: Partial<RawLibraryView> = {},
): RawLibraryView {
  return {
    id: 'shared-kit',
    name: 'shared-kit',
    workspace: 'demo',
    path: 'libraries/shared-kit',
    default_branch: 'main',
    type: 'maven',
    repository: { url: 'https://example.com/shared-kit.git' },
    artifact: makeRawArtifact(),
    commands: { build: './mvnw clean install' },
    build_system: 'maven',
    ...overrides,
  };
}

/** Builds a raw artifact coordinate. */
export function makeRawArtifact(
  overrides: Partial<RawArtifactView> = {},
): RawArtifactView {
  return {
    group_id: 'com.demo',
    artifact_id: 'shared-kit',
    version: '1.8.0-SNAPSHOT',
    packaging: 'jar',
    ...overrides,
  };
}

/** Builds a raw service group. */
export function makeRawGroup(overrides: Partial<RawGroupView> = {}): RawGroupView {
  return {
    id: 'core',
    name: 'Core services',
    workspace: 'demo',
    services: ['admin-ui'],
    ...overrides,
  };
}

/** Builds a raw developer profile. */
export function makeRawProfile(
  overrides: Partial<RawProfileView> = {},
): RawProfileView {
  return {
    id: 'full-stack',
    name: 'Full stack',
    description: 'Backend plus frontend for local development',
    workspace: 'demo',
    services: ['orders-api'],
    groups: ['core'],
    ...overrides,
  };
}

/** Builds a raw Spring Boot backend template. */
export function makeRawTemplate(
  overrides: Partial<RawTemplateView> = {},
): RawTemplateView {
  return {
    id: 'spring-boot-backend',
    name: 'Spring Boot Backend',
    description: 'Maven Spring Boot service with actuator health checks.',
    category: 'spring_boot_backend',
    type: 'backend',
    default_branch: 'main',
    build_system: 'maven',
    build_operation: 'build',
    runtime: { type: 'java', version: '21' },
    commands: {
      build: './mvnw clean install',
      start: './mvnw spring-boot:run',
    },
    liquibase: { enabled: true, execution_mode: 'maven', preset: 'status' },
    health_checks: [
      { type: 'http', url: 'http://localhost:${port}/actuator/health' },
    ],
    environment: { SPRING_PROFILES_ACTIVE: 'local' },
    ...overrides,
  };
}

/** Builds a raw named migration preset. */
export function makeRawMigrationPreset(
  overrides: Partial<RawMigrationPresetView> = {},
): RawMigrationPresetView {
  return {
    id: 'update',
    name: 'Update',
    description: 'Apply pending changesets.',
    provider: 'liquibase',
    execution_mode: 'maven',
    goals: ['liquibase:update'],
    contexts: ['sql', 'data', 'onlyDev'],
    destructive: false,
    read_only: false,
    ...overrides,
  };
}

/** Builds a raw destructive reset preset scoped to local targets only. */
export function makeRawResetPreset(
  overrides: Partial<RawMigrationPresetView> = {},
): RawMigrationPresetView {
  return makeRawMigrationPreset({
    id: 'reset-local',
    name: 'Reset local database',
    goals: ['liquibase:dropAll', 'liquibase:update'],
    destructive: true,
    allowed_environments: ['local'],
    ...overrides,
  });
}

/** Builds a raw saved bulk-operation preset. */
export function makeRawOperationPreset(
  overrides: Partial<RawOperationPresetView> = {},
): RawOperationPresetView {
  return {
    id: 'quick-local',
    name: 'Quick local',
    description: 'Sync, build, and restart the core services.',
    workspace: 'demo',
    default: true,
    services: ['orders-api'],
    git: { enabled: true, branch: 'main' },
    build: { enabled: true, operation: 'build' },
    runtime: { action: 'restart' },
    health: { enabled: true },
    ...overrides,
  };
}

/** Builds the global discovery settings. */
export function makeRawDiscovery(
  overrides: Partial<RawDiscoveryView> = {},
): RawDiscoveryView {
  return {
    max_depth: 6,
    exclude: ['vendor'],
    follow_symlinks: true,
    max_repositories: 200,
    ...overrides,
  };
}

/**
 * Builds a raw configuration snapshot matching the demo fixture.
 *
 * It is deliberately a *snapshot*: tests that commit a change can resolve the
 * mock against a new snapshot to model the daemon's single source of truth.
 */
export function makeConfigurationSnapshot(
  overrides: Partial<ConfigurationSnapshotView> = {},
): ConfigurationSnapshotView {
  return {
    revision: 'rev-1',
    config_version: 1,
    workspaces: [
      { id: 'demo', name: 'Demo Platform', root: '~/Projects/demo' },
    ],
    services: [makeRawService()],
    libraries: [makeRawLibrary()],
    profiles: [makeRawProfile()],
    groups: [makeRawGroup()],
    templates: [makeRawTemplate()],
    migration_presets: [makeRawMigrationPreset(), makeRawResetPreset()],
    operation_presets: [makeRawOperationPreset()],
    execution: { max_parallel_tasks: 4 },
    discovery: makeRawDiscovery(),
    ...overrides,
  };
}

/** Builds a compact entity-count summary. */
export function makeConfigurationCounts(
  overrides: Partial<ConfigurationCountsView> = {},
): ConfigurationCountsView {
  return {
    workspaces: 1,
    services: 1,
    libraries: 1,
    profiles: 1,
    groups: 1,
    ...overrides,
  };
}

/** Builds one actionable validation issue. */
export function makeValidationIssue(
  overrides: Partial<ValidationIssueView> = {},
): ValidationIssueView {
  return {
    file: '/Users/dev/.service-orchestrator/config.toml',
    location: 'services[0].workspace',
    problem: "unknown workspace 'ghost'",
    expected: 'an existing workspace id',
    suggested_fix: "declare a workspace with id 'ghost'",
    ...overrides,
  };
}

/** The outcome of validating the active configuration file. */
export function makeConfigurationValidation(
  overrides: Partial<ConfigurationValidationView> = {},
): ConfigurationValidationView {
  return {
    revision: 'rev-1',
    valid: true,
    issues: [],
    ...overrides,
  };
}

/** Builds a running-service impact entry. */
export function makeRunningImpact(
  overrides: Partial<RunningServiceImpactView> = {},
): RunningServiceImpactView {
  return {
    service_id: 'orders-api',
    running: true,
    change: 'updated',
    action_required:
      'The running process still uses the previous configuration; restart it to apply the change.',
    ...overrides,
  };
}

/** Builds a reference that points at a change target. */
export function makeAffectedReference(
  overrides: Partial<AffectedReferenceView> = {},
): AffectedReferenceView {
  return {
    source_kind: 'group',
    source_id: 'core',
    relation: 'group',
    target: 'admin-ui',
    ...overrides,
  };
}

/**
 * Builds a preview that adds one service.
 *
 * `valid` defaults to `true` and every collection is populated so a test can
 * assert the GUI renders the diff, issues, references, and impact without
 * configuring each one.
 */
export function makeConfigurationPreview(
  overrides: Partial<ConfigurationPreviewView> = {},
): ConfigurationPreviewView {
  return {
    revision: 'rev-1',
    proposed_revision: 'rev-2',
    valid: true,
    diff: {
      added: [{ kind: 'service', id: 'billing-api', name: 'billing-api' }],
      updated: [],
      removed: [],
    },
    issues: [],
    affected_references: [],
    running_impact: [],
    ...overrides,
  };
}

/** Builds a preview whose proposal fails validation. */
export function makeInvalidConfigurationPreview(): ConfigurationPreviewView {
  return makeConfigurationPreview({
    valid: false,
    proposed_revision: 'rev-1',
    diff: {
      added: [{ kind: 'service', id: 'billing-api', name: 'billing-api' }],
      updated: [],
      removed: [],
    },
    issues: [makeValidationIssue()],
  });
}

/** Builds the outcome of committing a change. */
export function makeConfigurationCommit(
  overrides: Partial<ConfigurationCommitView> = {},
): ConfigurationCommitView {
  return {
    revision: 'rev-2',
    description: "create service 'billing-api'",
    counts: makeConfigurationCounts({ services: 2 }),
    ...overrides,
  };
}

/** Builds the outcome of reloading the configuration. */
export function makeConfigurationReload(
  overrides: Partial<ConfigurationReloadView> = {},
): ConfigurationReloadView {
  return {
    revision: 'rev-2',
    counts: makeConfigurationCounts(),
    ...overrides,
  };
}

/** Builds a read-only repository probe with Maven, npm, and Gradle detected. */
export function makeRepositoryProbe(
  overrides: Partial<RepositoryProbeView> = {},
): RepositoryProbeView {
  return {
    path: '/Users/dev/Projects/demo/backend/orders-api',
    exists: true,
    is_directory: true,
    is_git_repository: true,
    branch: 'main',
    remote: 'https://example.com/orders-api.git',
    project_files: ['pom.xml', 'mvnw', 'package.json', 'build.gradle.kts'],
    build_systems: [
      {
        system: 'maven',
        label: 'Maven',
        availability: 'available',
        evidence: ['pom.xml'],
      },
      {
        system: 'npm',
        label: 'npm',
        availability: 'available',
        evidence: ['package.json'],
      },
      {
        system: 'gradle',
        label: 'Gradle',
        availability: 'provider_not_installed',
        evidence: ['build.gradle.kts'],
        note: 'Detected: Gradle — execution provider not installed.',
      },
    ],
    runtimes: [
      { kind: 'java', label: 'Java', required_version: '21', evidence: ['pom.xml'] },
      {
        kind: 'node',
        label: 'Node.js',
        required_version: '20.x',
        evidence: ['package.json'],
      },
    ],
    integrations: [{ kind: 'spring_boot', label: 'Spring Boot', evidence: ['pom.xml'] }],
    maven: {
      group_id: 'com.demo',
      artifact_id: 'orders-api',
      version: '1.0.0',
      packaging: 'jar',
      java_version: '21',
      wrapper_available: true,
      spring_boot: true,
    },
    node: {
      name: 'orders-api-ui',
      version: '1.0.0',
      required_node_version: '20.x',
      package_manager: 'npm',
      scripts: ['build', 'start'],
      react: false,
    },
    ...overrides,
  };
}

/** Builds a suggested service configuration from a Maven repository. */
export function makeProjectSuggestion(
  overrides: Partial<ProjectSuggestionView> = {},
): ProjectSuggestionView {
  return {
    path: '/Users/dev/Projects/demo/backend/orders-api',
    service_type: 'backend',
    build_system: 'maven',
    runtimes: [{ kind: 'java', label: 'Java', required_version: '21', evidence: ['pom.xml'] }],
    build_command: { program: './mvnw', args: ['clean', 'install'] },
    start_command: { program: './mvnw', args: ['spring-boot:run'] },
    integrations: [{ kind: 'spring_boot', label: 'Spring Boot', evidence: ['pom.xml'] }],
    warnings: [
      'Gradle was detected but its execution provider is not installed; Maven is suggested instead.',
    ],
    inspection: makeRepositoryProbe(),
    ...overrides,
  };
}

/** Builds a service-creation change for the demo workspace. */
export function makeServiceCreateChange(): ConfigChangeView {
  return {
    kind: 'service',
    change: {
      action: 'create',
      entity: makeRawService({ id: 'billing-api', name: 'billing-api' }),
    },
  };
}

// -- repository discovery and bulk import -------------------------------------

/** Builds one detected technology. */
export function makeDetection(
  overrides: Partial<TechnologyDetectionView> = {},
): TechnologyDetectionView {
  return {
    technology: 'maven',
    label: 'Maven',
    confidence: 'high',
    evidence: ['pom.xml'],
    ...overrides,
  };
}

/** Builds a discovered repository view with sensible defaults. */
export function makeDiscoveredRepository(
  overrides: Partial<DiscoveredRepositoryView> = {},
): DiscoveredRepositoryView {
  return {
    repository: {
      name: 'orders-api',
      relative_path: 'backend/orders-api',
      inspection: makeRepositoryProbe(),
      detections: [makeDetection()],
    },
    suggested_category: 'spring_boot_backend',
    suggested_template: 'spring-boot-backend',
    candidate_templates: ['spring-boot-backend'],
    ...overrides,
  };
}

/** Builds a bounded discovery report with two repositories. */
export function makeDiscoveryReport(
  overrides: Partial<RepositoryDiscoveryReportView> = {},
): RepositoryDiscoveryReportView {
  return {
    workspace_id: 'demo',
    root: '/Users/dev/Projects/demo',
    repositories: [
      makeDiscoveredRepository(),
      makeDiscoveredRepository({
        repository: {
          name: 'admin-ui',
          relative_path: 'frontend/admin-ui',
          inspection: makeRepositoryProbe({
            path: '/Users/dev/Projects/demo/frontend/admin-ui',
            integrations: [{ kind: 'react', label: 'React', evidence: ['package.json'] }],
          }),
          detections: [
            makeDetection({ technology: 'npm', label: 'npm', evidence: ['package.json'] }),
            makeDetection({
              technology: 'react',
              label: 'React',
              evidence: ['package.json'],
            }),
          ],
        },
        suggested_category: 'react_frontend',
        suggested_template: 'react-frontend',
        candidate_templates: ['react-frontend'],
      }),
    ],
    truncated: false,
    warnings: [],
    ...overrides,
  };
}

/** A discovery-scan request for the demo workspace. */
export function makeDiscoveryInput(
  overrides: Partial<DiscoveryScanInputView> = {},
): DiscoveryScanInputView {
  return {
    workspace_id: 'demo',
    follow_symlinks: true,
    ...overrides,
  };
}

/** Builds one bulk-import plan row. */
export function makeBulkImportRow(
  overrides: Partial<BulkImportRowView> = {},
): BulkImportRowView {
  return {
    relative_path: 'backend/orders-api',
    absolute_path: '/Users/dev/Projects/demo/backend/orders-api',
    template: 'spring-boot-backend',
    template_source: 'matched',
    entity_kind: 'service',
    entity_id: 'orders-api',
    action: { action: 'create' },
    detections: [makeDetection()],
    ports: [8081],
    proposal: {
      entity: 'service',
      service: makeRawService({ id: 'orders-api', name: undefined }),
    },
    warnings: [],
    errors: [],
    ...overrides,
  };
}

/** Builds a valid bulk-import plan that creates two services. */
export function makeBulkImportPlan(
  overrides: Partial<BulkImportPlanView> = {},
): BulkImportPlanView {
  return {
    workspace_id: 'demo',
    revision: 'rev-1',
    rows: [
      makeBulkImportRow(),
      makeBulkImportRow({
        relative_path: 'frontend/admin-ui',
        absolute_path: '/Users/dev/Projects/demo/frontend/admin-ui',
        template: 'react-frontend',
        entity_id: 'admin-ui',
        ports: [3000],
        proposal: {
          entity: 'service',
          service: makeRawService({
            id: 'admin-ui',
            name: undefined,
            type: 'frontend',
            path: 'frontend/admin-ui',
            build_system: 'npm',
            runtime: { type: 'node', version: '20.x' },
            ports: [{ port: 3000 }],
          }),
        },
      }),
    ],
    valid: true,
    creates: 2,
    updates: 0,
    skips: 0,
    rejects: 0,
    preview: makeConfigurationPreview({
      diff: {
        added: [
          { kind: 'service', id: 'orders-api', name: 'orders-api' },
          { kind: 'service', id: 'admin-ui', name: 'admin-ui' },
        ],
        updated: [],
        removed: [],
      },
    }),
    warnings: [],
    ...overrides,
  };
}

/** Builds the result of committing a bulk import. */
export function makeBulkImportCommit(
  overrides: Partial<BulkImportCommitView> = {},
): BulkImportCommitView {
  return {
    workspace_id: 'demo',
    revision: 'rev-2',
    created_services: ['orders-api', 'admin-ui'],
    created_libraries: [],
    updated: [],
    skipped: [],
    counts: makeConfigurationCounts({ services: 3 }),
    ...overrides,
  };
}

// -- migration and operation presets ------------------------------------------

/** Builds a migration-preset summary (the read-only built-in status preset). */
export function makeMigrationPresetSummary(
  overrides: Partial<MigrationPresetSummaryView> = {},
): MigrationPresetSummaryView {
  return {
    id: 'status',
    name: 'Status',
    description: 'Inspect pending changesets without changing the database.',
    contexts: [],
    read_only: true,
    destructive: false,
    allowed_environments: [],
    builtin: true,
    ...overrides,
  };
}

/** The built-in migration presets as the daemon lists them. */
export function makeMigrationPresetSummaries(): MigrationPresetSummaryView[] {
  return [
    makeMigrationPresetSummary(),
    makeMigrationPresetSummary({
      id: 'update',
      name: 'Update',
      description: 'Apply pending changesets.',
      contexts: ['sql', 'data', 'onlyDev'],
      read_only: false,
      destructive: false,
    }),
    makeMigrationPresetSummary({
      id: 'reset_local',
      name: 'Reset local database',
      description: 'Drop all objects and reapply every changeset.',
      contexts: ['sql', 'data', 'onlyDev'],
      read_only: false,
      destructive: true,
      allowed_environments: ['local'],
    }),
  ];
}

/** Builds a saved operation-preset summary. */
export function makeOperationPresetSummary(
  overrides: Partial<OperationPresetSummaryView> = {},
): OperationPresetSummaryView {
  return {
    id: 'quick-local',
    name: 'Quick local',
    description: 'Sync, build, and restart the core services.',
    workspace_id: 'demo',
    default: true,
    request: makeOperationPresetRequest(),
    ...overrides,
  };
}

/** The request a saved operation preset composes. */
export function makeOperationPresetRequest(
  overrides: Partial<PrepareEnvironmentInput> = {},
): PrepareEnvironmentInput {
  return {
    workspace_id: 'demo',
    services: ['orders-api', 'admin-ui'],
    branch: 'main',
    sync: true,
    include_dependencies: true,
    build_operation: 'build',
    build: true,
    start: false,
    restart: true,
    stop: false,
    wait_for_health: true,
    ...overrides,
  };
}
