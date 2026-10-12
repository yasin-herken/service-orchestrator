import type {
  ConnectionSnapshot,
  DaemonStatus,
  OperationOutcome,
  ServiceDetail,
  ServiceSummary,
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
