import { beforeEach, describe, expect, it } from 'vitest';

import {
  CONTROL_CENTER_STORAGE_KEY,
  QUICK_ACTIONS,
  actionableHint,
  buildOperationLabel,
  configuredDefaultBranches,
  currentPhase,
  defaultControlCenterOptions,
  exposedBuildOperations,
  filterServices,
  hasGitOperation,
  hasOperation,
  healthChecksConflict,
  loadControlCenterState,
  normalizeOptions,
  optionsFromRequest,
  quickActionOptions,
  requestKey,
  saveControlCenterState,
  serviceKinds,
  serviceRuntimeLabel,
  toControlCenterRequest,
  type ControlCenterOptions,
} from './model';
import {
  makeBuildTargets,
  makeRawService,
  makeServices,
  makeWorkflowStatus,
} from '../../test/fixtures';

beforeEach(() => {
  window.localStorage.clear();
});

function options(
  overrides: Partial<ControlCenterOptions> = {},
): ControlCenterOptions {
  return { ...defaultControlCenterOptions(), ...overrides };
}

describe('control center defaults', () => {
  it('synchronizes only, leaving every other phase opt-in', () => {
    const value = defaultControlCenterOptions();
    expect(value.sync).toBe(true);
    expect(value.gitFetch).toBe(true);
    expect(value.gitCheckout).toBe(true);
    expect(value.gitPull).toBe(true);
    expect(value.includeDependencies).toBe(true);
    expect(value.buildLibraries).toBe(false);
    expect(value.runLiquibase).toBe(false);
    expect(value.build).toBe(false);
    expect(value.runtime).toBe('none');
    expect(value.waitForHealth).toBe(true);
    expect(value.healthChecks).toBe(false);
    expect(hasGitOperation(value)).toBe(true);
    expect(hasOperation(value)).toBe(true);
  });

  it('treats an all-off Git toggle combination as no Git work', () => {
    const value = options({
      gitFetch: false,
      gitCheckout: false,
      gitPull: false,
    });
    expect(hasGitOperation(value)).toBe(false);
    expect(hasOperation(value)).toBe(false);
  });
});

describe('request mapping', () => {
  it('forwards the sorted selection and the active Git steps', () => {
    const request = toControlCenterRequest(
      'demo',
      ['orders-api', 'admin-ui'],
      defaultControlCenterOptions(),
    );
    expect(request).toEqual({
      workspace_id: 'demo',
      services: ['admin-ui', 'orders-api'],
      sync: true,
      git_fetch: true,
      git_checkout: true,
      git_pull: true,
      include_dependencies: true,
      build_libraries: false,
      run_liquibase: false,
      build: false,
      start: false,
      restart: false,
      stop: false,
      wait_for_health: false,
      health_checks: false,
    });
  });

  it('never forwards an override that cannot take effect', () => {
    const request = toControlCenterRequest(
      'demo',
      ['orders-api'],
      options({
        sync: false,
        branch: 'release/1.0',
        build: false,
        buildOperation: 'clean',
        runLiquibase: false,
        migrationPreset: 'reset_local',
      }),
    );
    expect(request).not.toHaveProperty('branch');
    expect(request).not.toHaveProperty('build_operation');
    expect(request).not.toHaveProperty('migration_preset');
    expect(request.git_fetch).toBe(false);
    expect(request.git_checkout).toBe(false);
    expect(request.git_pull).toBe(false);
  });

  it('maps runtime, migration, and build overrides onto real fields', () => {
    const request = toControlCenterRequest(
      'demo',
      ['orders-api'],
      options({
        branch: ' develop ',
        build: true,
        buildOperation: 'build',
        runLiquibase: true,
        migrationPreset: 'update',
        runtime: 'restart',
        waitForHealth: true,
        healthChecks: true,
      }),
    );
    expect(request).toMatchObject({
      branch: 'develop',
      build: true,
      build_operation: 'build',
      run_liquibase: true,
      migration_preset: 'update',
      start: false,
      restart: true,
      stop: false,
      wait_for_health: true,
      health_checks: true,
    });
  });

  it('drops standalone health checks when stopping services', () => {
    const value = options({ runtime: 'stop', healthChecks: true });
    expect(healthChecksConflict(value)).toBe(true);
    expect(
      toControlCenterRequest('demo', ['orders-api'], value).health_checks,
    ).toBe(false);
  });

  it('round-trips options through a request', () => {
    const value = options({
      branch: 'main',
      buildLibraries: true,
      runLiquibase: true,
      migrationPreset: 'update',
      build: true,
      buildOperation: 'test',
      runtime: 'start',
      waitForHealth: true,
    });
    expect(optionsFromRequest(toControlCenterRequest('demo', ['a'], value))).toEqual(
      value,
    );
  });

  it('produces a stable request key and an empty key for no request', () => {
    const request = toControlCenterRequest(
      'demo',
      ['orders-api'],
      defaultControlCenterOptions(),
    );
    expect(requestKey({ ...request })).toBe(requestKey(request));
    expect(
      requestKey(
        toControlCenterRequest(
          'demo',
          ['admin-ui'],
          defaultControlCenterOptions(),
        ),
      ),
    ).not.toBe(requestKey(request));
    expect(requestKey(null)).toBe('');
  });
});

describe('quick actions', () => {
  it('offers every documented action', () => {
    expect(QUICK_ACTIONS.map((action) => action.label)).toEqual([
      'Git Sync Only',
      'Build Only',
      'Start Only',
      'Stop Only',
      'Restart Only',
      'Health Check Only',
      'Full Prepare Environment',
    ]);
  });

  it('builds each action from existing request fields', () => {
    const current = options({ branch: 'release/2', includeDependencies: false });

    expect(
      toControlCenterRequest('demo', ['a'], quickActionOptions('git-sync', current)),
    ).toMatchObject({
      sync: true,
      build: false,
      include_dependencies: false,
      branch: 'release/2',
    });

    expect(
      toControlCenterRequest('demo', ['a'], quickActionOptions('build', current)),
    ).toMatchObject({ sync: false, git_fetch: false, build: true });

    expect(
      toControlCenterRequest('demo', ['a'], quickActionOptions('start', current)),
    ).toMatchObject({ sync: false, start: true, wait_for_health: true });

    expect(
      toControlCenterRequest('demo', ['a'], quickActionOptions('stop', current)),
    ).toMatchObject({ sync: false, stop: true, wait_for_health: false });

    expect(
      toControlCenterRequest('demo', ['a'], quickActionOptions('restart', current)),
    ).toMatchObject({ sync: false, restart: true, wait_for_health: true });

    expect(
      toControlCenterRequest('demo', ['a'], quickActionOptions('health', current)),
    ).toMatchObject({ sync: false, restart: false, health_checks: true });

    expect(
      toControlCenterRequest('demo', ['a'], quickActionOptions('full', current)),
    ).toMatchObject({
      sync: true,
      build_libraries: true,
      build: true,
      start: true,
      wait_for_health: true,
    });
  });
});

describe('persistence', () => {
  it('saves and loads the selection and options only', () => {
    saveControlCenterState({
      version: 1,
      workspaceId: 'demo',
      serviceIds: ['orders-api'],
      options: options({ build: true }),
    });

    const stored = window.localStorage.getItem(CONTROL_CENTER_STORAGE_KEY);
    expect(stored).not.toContain('authorization');
    expect(stored).not.toContain('workflow');

    const loaded = loadControlCenterState();
    expect(loaded?.workspaceId).toBe('demo');
    expect(loaded?.serviceIds).toEqual(['orders-api']);
    expect(loaded?.options.build).toBe(true);
  });

  it('ignores malformed or unsupported stored state', () => {
    window.localStorage.setItem(CONTROL_CENTER_STORAGE_KEY, 'not json');
    expect(loadControlCenterState()).toBeNull();

    window.localStorage.setItem(
      CONTROL_CENTER_STORAGE_KEY,
      JSON.stringify({ version: 2, workspaceId: 'demo', serviceIds: [] }),
    );
    expect(loadControlCenterState()).toBeNull();
  });

  it('normalizes persisted options field by field', () => {
    const normalized = normalizeOptions({
      sync: 'yes',
      runtime: 'destroy',
      gitPull: false,
      branch: 42,
    });
    expect(normalized.sync).toBe(true);
    expect(normalized.runtime).toBe('none');
    expect(normalized.gitPull).toBe(false);
    expect(normalized.branch).toBe('');
  });
});

describe('selection helpers', () => {
  it('filters by search and kind without touching the selection', () => {
    const services = makeServices();
    expect(filterServices(services, 'admin', 'all').map((s) => s.id)).toEqual([
      'admin-ui',
    ]);
    expect(filterServices(services, '', 'backend').map((s) => s.id)).toEqual([
      'orders-api',
    ]);
    expect(filterServices(services, 'missing', 'all')).toEqual([]);
    expect(serviceKinds(services)).toEqual(['backend', 'frontend']);
  });

  it('uses the exposed provider operations only', () => {
    expect(exposedBuildOperations(makeBuildTargets())).toEqual(['build']);
    expect(
      exposedBuildOperations([
        makeBuildTargets()[0],
        makeBuildTargets()[1],
        { ...makeBuildTargets()[0], service_id: 'other', capability: null },
      ]),
    ).toEqual(['build']);
  });

  it('labels runtimes from the daemon capability rows', () => {
    const targets = makeBuildTargets();
    expect(serviceRuntimeLabel(targets[0])).toBe('java 21');
    expect(serviceRuntimeLabel(targets[1])).toBe('node 20.x');
    expect(serviceRuntimeLabel(undefined)).toBe('');
  });

  it('reads configured default branches for the selected workspace only', () => {
    const raw = [
      makeRawService(),
      makeRawService({
        id: 'admin-ui',
        default_branch: 'develop',
        repository: undefined,
      }),
      makeRawService({ id: 'no-branch', default_branch: undefined, repository: undefined }),
      makeRawService({
        id: 'orders-api',
        workspace: 'other',
        default_branch: 'release',
      }),
    ];
    expect(
      configuredDefaultBranches('demo', ['orders-api', 'admin-ui'], raw),
    ).toEqual(['main', 'develop']);
    expect(configuredDefaultBranches('demo', [], raw)).toEqual([]);
    expect(configuredDefaultBranches('demo', ['no-branch'], raw)).toEqual([]);
    expect(configuredDefaultBranches('other', ['orders-api'], raw)).toEqual([
      'release',
    ]);
  });

  it('humanizes build operations', () => {
    expect(buildOperationLabel('publish_local_artifact')).toBe(
      'Publish local artifact',
    );
    expect(buildOperationLabel('custom')).toBe('custom');
  });
});

describe('presentation helpers', () => {
  it('reports the first unfinished phase from daemon counters', () => {
    expect(currentPhase(null)).toBeNull();
    expect(currentPhase(makeWorkflowStatus())).toBe('service-build');
    expect(
      currentPhase(
        makeWorkflowStatus({
          phases: [
            { phase: 'git-sync', total: 1, succeeded: 0, failed: 0, pending: 1 },
            { phase: 'service-build', total: 2, succeeded: 0, failed: 0, pending: 2 },
          ],
        }),
      ),
    ).toBe('git-sync');
    expect(
      currentPhase(
        makeWorkflowStatus({
          phases: [{ phase: 'git-sync', total: 1, succeeded: 1, failed: 0, pending: 0 }],
        }),
      ),
    ).toBe('git-sync');
  });

  it('explains stable failure codes without changing behavior', () => {
    expect(
      actionableHint({ code: 'GIT_WORKING_TREE_DIRTY', message: 'dirty' }),
    ).toMatch(/never runs a destructive Git reset/i);
    expect(
      actionableHint({ code: 'PermissionDenied', message: 'denied' }),
    ).toMatch(/not authorization/i);
    expect(
      actionableHint({ code: 'RUNTIME_VERSION_MISMATCH', message: 'node 18' }),
    ).toMatch(/pinned version/i);
    expect(
      actionableHint({
        code: 'MIGRATION_DATABASE_TARGET_NOT_AUTHORIZED',
        message: 'target',
      }),
    ).toMatch(/database environment/i);
    expect(
      actionableHint({ code: 'BUILD_PROCESS_FAILED', message: 'mvn not found' }),
    ).toMatch(/runtime toolchain/i);
    expect(
      actionableHint({ code: 'HEALTH_CONFIGURATION_INVALID', message: 'bad url' }),
    ).toMatch(/health check is invalid/i);
    expect(actionableHint({ code: 'SOMETHING_ELSE', message: 'x' })).toBeNull();
  });
});
