import type {
  ArtifactView,
  ArtifactVerificationView,
  BranchView,
  BuildCapabilitiesView,
  BuildTargetSummary,
  BulkSummaryView,
  ConnectionSnapshot,
  DaemonStatus,
  GitStatusView,
  GitSyncPreview,
  GitSyncPreviewEntry,
  LibrarySummary,
  LogPageView,
  OperationOutcome,
  RepositoryInspectionView,
  RepositorySummary,
  ServiceDetail,
  ServiceSummary,
  TaskView,
  VersionAlignmentView,
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
      runtimes: [{ kind: 'node', version: '20.19.4' }],
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
