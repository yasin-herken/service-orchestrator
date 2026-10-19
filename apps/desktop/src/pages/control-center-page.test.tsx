import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { flush, happyPathDefaults, renderApp } from '../test/harness';
import { emitConnection, mockDesktopApi, resetMockDesktopApi } from '../test/mocks';
import {
  makeDisconnected,
  makeEnvironmentPlan,
  makePartiallySucceededStatus,
  makeServices,
  makeTaskView,
  makeWorkspace,
} from '../test/fixtures';
import {
  CONTROL_CENTER_STORAGE_KEY,
  defaultControlCenterOptions,
} from '../components/control-center/model';

vi.mock('../services/desktopApi', async () => ({
  desktopApi: (await import('../test/mocks')).mockDesktopApi,
}));

beforeEach(() => {
  resetMockDesktopApi();
  happyPathDefaults();
  window.localStorage.clear();
});

/** Renders the app on its default screen and waits for the Control Center. */
async function openControlCenter() {
  const user = userEvent.setup();
  const view = await renderApp();
  await screen.findByRole('region', { name: 'Service selection' });
  return { user, view };
}

/** Opens the picker, checks the given services, and applies the selection. */
async function pickServices(
  user: ReturnType<typeof userEvent.setup>,
  serviceIds: readonly string[],
) {
  await user.click(screen.getByRole('button', { name: 'Select services…' }));
  const dialog = await screen.findByRole('dialog', { name: 'Select services' });
  for (const serviceId of serviceIds) {
    await user.click(
      within(dialog).getByRole('checkbox', { name: `Select ${serviceId}` }),
    );
  }
  await user.click(
    within(dialog).getByRole('button', { name: 'Apply selection' }),
  );
}

/** The chip list of applied services. */
function selectedChips(): HTMLElement {
  return screen.getByRole('list', { name: 'Selected service list' });
}

/** Saves the current options and prepares a non-terminal workflow. */
async function executeCurrent(
  user: ReturnType<typeof userEvent.setup>,
) {
  await user.click(screen.getByRole('button', { name: 'Prepare & Run' }));
  await waitFor(() => expect(mockDesktopApi.prepareEnvironment).toHaveBeenCalledTimes(1));
}

describe('control center selection', () => {
  it('is the default screen and asks for an explicit selection', async () => {
    await openControlCenter();

    expect(
      screen.getByRole('heading', { name: 'Workspace Control Center', level: 1 }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('heading', { name: 'One screen · one selection · one execution' }),
    ).toBeInTheDocument();
    expect(screen.getByText('Nothing is selected. Use the service picker or press ⌘K / Ctrl+K.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Prepare & Run' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Preview plan' })).toBeDisabled();
  });

  it('filters by search and kind while preserving checked services', async () => {
    const { user } = await openControlCenter();

    await user.click(screen.getByRole('button', { name: 'Select services…' }));
    const dialog = screen.getByRole('dialog', { name: 'Select services' });
    expect(within(dialog).getByText('2 visible · 0 selected')).toBeInTheDocument();

    await user.type(within(dialog).getByLabelText('Search services'), 'admin');
    expect(within(dialog).getByText('1 visible · 0 selected')).toBeInTheDocument();
    await user.click(
      within(dialog).getByRole('checkbox', { name: 'Select admin-ui' }),
    );

    // Clearing the filter keeps the checked service checked.
    await user.clear(within(dialog).getByLabelText('Search services'));
    expect(
      within(dialog).getByRole('checkbox', { name: 'Select admin-ui' }),
    ).toBeChecked();

    await user.selectOptions(
      within(dialog).getByLabelText('Filter services by type'),
      'backend',
    );
    expect(within(dialog).getByText('1 visible · 1 selected')).toBeInTheDocument();
    expect(
      within(dialog).getByRole('checkbox', { name: 'Select orders-api' }),
    ).not.toBeChecked();
    expect(
      within(dialog).queryByRole('checkbox', { name: 'Select admin-ui' }),
    ).not.toBeInTheDocument();
  });

  it('applies a multi-selection of services', async () => {
    const { user } = await openControlCenter();

    await pickServices(user, ['orders-api', 'admin-ui']);

    expect(within(selectedChips()).getByText('orders-api')).toBeInTheDocument();
    expect(within(selectedChips()).getByText('admin-ui')).toBeInTheDocument();
    expect(screen.getByText('2 selected · dependencies are resolved by the plan')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Prepare & Run' })).toBeEnabled();
  });

  it('selects a profile’s resolved services', async () => {
    const { user } = await openControlCenter();

    await user.click(screen.getByRole('button', { name: 'Select services…' }));
    const dialog = screen.getByRole('dialog', { name: 'Select services' });
    await user.selectOptions(
      within(dialog).getByLabelText('Select by profile'),
      'full-stack',
    );
    await user.click(
      within(dialog).getByRole('button', { name: 'Apply selection' }),
    );

    expect(within(selectedChips()).getByText('orders-api')).toBeInTheDocument();
    expect(within(selectedChips()).getByText('admin-ui')).toBeInTheDocument();
  });

  it('opens the picker with ⌘K / Ctrl+K', async () => {
    await openControlCenter();

    fireEvent.keyDown(window, { key: 'k', metaKey: true });

    expect(
      await screen.findByRole('dialog', { name: 'Select services' }),
    ).toBeInTheDocument();
  });

  it('persists the selection without storing authorization or a workflow', async () => {
    const { user } = await openControlCenter();
    await pickServices(user, ['orders-api']);
    await user.click(screen.getByLabelText('Build services'));

    await waitFor(() => {
      const raw = window.localStorage.getItem(CONTROL_CENTER_STORAGE_KEY);
      expect(raw).not.toBeNull();
      expect(raw).not.toContain('authorization');
      expect(raw).not.toContain('workflow');
      const saved = JSON.parse(raw as string);
      expect(saved.serviceIds).toEqual(['orders-api']);
      expect(saved.options.build).toBe(true);
    });
  });

  it('rehydrates saved state without planning or executing anything', async () => {
    window.localStorage.setItem(
      CONTROL_CENTER_STORAGE_KEY,
      JSON.stringify({
        version: 1,
        workspaceId: 'demo',
        serviceIds: ['admin-ui'],
        options: { ...defaultControlCenterOptions(), build: true },
      }),
    );

    await renderApp();
    await screen.findByRole('region', { name: 'Service selection' });
    await waitFor(() =>
      expect(within(selectedChips()).getByText('admin-ui')).toBeInTheDocument(),
    );
    expect(screen.getByLabelText('Build services')).toBeChecked();
    expect(mockDesktopApi.planPrepareEnvironment).not.toHaveBeenCalled();
    expect(mockDesktopApi.prepareEnvironment).not.toHaveBeenCalled();
  });

  it('restores the last workspace without planning or executing', async () => {
    mockDesktopApi.listWorkspaces.mockResolvedValue([
      makeWorkspace(),
      makeWorkspace({
        id: 'ops',
        name: 'Operations',
        root: '/Users/dev/Projects/ops',
      }),
    ]);
    mockDesktopApi.listServices.mockImplementation(() =>
      Promise.resolve(makeServices()),
    );
    window.localStorage.setItem(
      CONTROL_CENTER_STORAGE_KEY,
      JSON.stringify({
        version: 1,
        workspaceId: 'ops',
        serviceIds: ['orders-api'],
        options: { ...defaultControlCenterOptions(), build: true },
      }),
    );

    await renderApp();
    await screen.findByRole('region', { name: 'Service selection' });

    await waitFor(() =>
      expect(screen.getByLabelText('Active workspace')).toHaveValue('ops'),
    );
    await waitFor(() =>
      expect(mockDesktopApi.listServices).toHaveBeenCalledWith('ops'),
    );
    await waitFor(() =>
      expect(within(selectedChips()).getByText('orders-api')).toBeInTheDocument(),
    );
    expect(mockDesktopApi.planPrepareEnvironment).not.toHaveBeenCalled();
    expect(mockDesktopApi.prepareEnvironment).not.toHaveBeenCalled();
  });
});

describe('control center operations', () => {
  it('shows only the controls relevant to the current operation', async () => {
    const { user } = await openControlCenter();

    // Git is on by default, so its sub-controls and branch override are shown.
    expect(screen.getByLabelText('Target branch')).toBeInTheDocument();
    expect(screen.getByLabelText('Git fetch')).toBeInTheDocument();
    // Build, migration, and health overrides stay hidden until enabled.
    expect(screen.queryByLabelText('Build operation')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Liquibase preset')).not.toBeInTheDocument();

    await user.click(screen.getByLabelText('Git sync'));
    expect(screen.queryByLabelText('Target branch')).not.toBeInTheDocument();
    expect(screen.queryByLabelText('Git fetch')).not.toBeInTheDocument();

    await user.click(screen.getByLabelText('Build services'));
    expect(screen.getByLabelText('Build operation')).toBeInTheDocument();

    await user.click(screen.getByLabelText('Run Liquibase migrations'));
    expect(screen.getByLabelText('Liquibase preset')).toBeInTheDocument();
  });

  it('maps branch, preset, and build overrides onto one previewed request', async () => {
    const { user } = await openControlCenter();
    await pickServices(user, ['orders-api']);

    await user.type(screen.getByLabelText('Target branch'), 'develop');
    await user.click(screen.getByLabelText('Run Liquibase migrations'));
    await user.selectOptions(screen.getByLabelText('Liquibase preset'), 'update');
    await user.click(screen.getByLabelText('Build services'));
    await user.selectOptions(screen.getByLabelText('Build operation'), 'build');

    await user.click(screen.getByRole('button', { name: 'Preview plan' }));
    await screen.findByRole('region', { name: 'Workflow plan' });

    expect(mockDesktopApi.planPrepareEnvironment).toHaveBeenCalledWith(
      expect.objectContaining({
        workspace_id: 'demo',
        services: ['orders-api'],
        branch: 'develop',
        sync: true,
        git_fetch: true,
        git_checkout: true,
        git_pull: true,
        run_liquibase: true,
        migration_preset: 'update',
        build: true,
        build_operation: 'build',
        start: false,
        restart: false,
        stop: false,
      }),
    );
    // Planning is read-only.
    expect(mockDesktopApi.prepareEnvironment).not.toHaveBeenCalled();
  });

  it('applies advanced overrides to the next plan only', async () => {
    const { user } = await openControlCenter();
    await pickServices(user, ['orders-api']);

    await user.click(screen.getByRole('button', { name: 'Advanced settings…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Advanced settings' });
    // Configuration-owned values are shown read-only.
    expect(within(dialog).getByText('backend/orders-api')).toBeInTheDocument();
    expect(within(dialog).getByText('./mvnw clean install')).toBeInTheDocument();

    await user.click(
      within(dialog).getByLabelText('Include required dependencies'),
    );
    await user.click(within(dialog).getByRole('button', { name: 'Done' }));

    await user.click(screen.getByRole('button', { name: 'Preview plan' }));
    await screen.findByRole('region', { name: 'Workflow plan' });

    expect(mockDesktopApi.planPrepareEnvironment).toHaveBeenCalledWith(
      expect.objectContaining({ include_dependencies: false }),
    );
  });

  it('runs quick actions through the same coordinated request', async () => {
    const { user } = await openControlCenter();
    await pickServices(user, ['orders-api']);

    await user.click(screen.getByRole('button', { name: 'Build Only' }));

    await waitFor(() => expect(mockDesktopApi.prepareEnvironment).toHaveBeenCalledTimes(1));
    expect(mockDesktopApi.prepareEnvironment).toHaveBeenCalledWith(
      expect.objectContaining({
        workspace_id: 'demo',
        services: ['orders-api'],
        sync: false,
        git_fetch: false,
        git_checkout: false,
        git_pull: false,
        build: true,
        run_liquibase: false,
        start: false,
        stop: false,
        restart: false,
      }),
    );
    // The quick action is reflected in the inline controls.
    expect(screen.getByLabelText('Build services')).toBeChecked();
    expect(screen.getByLabelText('Git sync')).not.toBeChecked();
  });
});

describe('control center planning and execution', () => {
  it('previews the daemon plan with dependencies, phases, and warnings', async () => {
    const { user } = await openControlCenter();
    await pickServices(user, ['orders-api']);

    await user.click(screen.getByRole('button', { name: 'Preview plan' }));
    const plan = await screen.findByRole('region', { name: 'Workflow plan' });

    expect(within(plan).getByText('git-sync')).toBeInTheDocument();
    expect(within(plan).getByText('service-build')).toBeInTheDocument();
    expect(
      within(plan).getByText(
        "working tree of 'admin-ui' has uncommitted local changes",
      ),
    ).toBeInTheDocument();
    expect(plan).toHaveTextContent('shared-kit');
    expect(plan).toHaveTextContent('No protected steps identified');
  });

  it('marks a plan stale after the configuration changes', async () => {
    const { user } = await openControlCenter();
    await pickServices(user, ['orders-api']);
    await user.click(screen.getByRole('button', { name: 'Preview plan' }));
    await screen.findByRole('region', { name: 'Workflow plan' });

    await user.click(screen.getByLabelText('Build services'));

    expect(
      screen.getByText(
        /The request changed after this plan was generated. Preview it again/i,
      ),
    ).toBeInTheDocument();
  });

  it('submits exactly one prepare_environment workflow from Prepare & Run', async () => {
    const { user } = await openControlCenter();
    await pickServices(user, ['orders-api']);

    await executeCurrent(user);

    expect(mockDesktopApi.planPrepareEnvironment).toHaveBeenCalledTimes(1);
    expect(mockDesktopApi.prepareEnvironment).toHaveBeenCalledWith(
      expect.objectContaining({ workspace_id: 'demo', services: ['orders-api'] }),
    );
    const progress = await screen.findByRole('region', { name: 'Workflow progress' });
    expect(progress).toHaveTextContent('wf-env-1');
    expect(progress).toHaveTextContent('git-sync');
    expect(progress).toHaveTextContent('1/1 succeeded');
  });

  it('requires explicit authorization before a destructive migration reset', async () => {
    mockDesktopApi.planPrepareEnvironment.mockResolvedValue(
      makeEnvironmentPlan({ requires_destructive_authorization: true }),
    );
    const { user } = await openControlCenter();
    await pickServices(user, ['orders-api']);

    await user.click(screen.getByLabelText('Run Liquibase migrations'));
    await user.selectOptions(screen.getByLabelText('Liquibase preset'), 'reset_local');

    // The inline warning states the protected nature before any request.
    expect(screen.getByText(/drops database objects/i)).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Prepare & Run' }));
    const dialog = await screen.findByRole('dialog', {
      name: 'Execute a destructive operation?',
    });
    expect(mockDesktopApi.prepareEnvironment).not.toHaveBeenCalled();

    await user.click(
      within(dialog).getByRole('button', { name: 'Execute operation' }),
    );

    await waitFor(() =>
      expect(mockDesktopApi.prepareEnvironment).toHaveBeenCalledWith(
        expect.objectContaining({
          migration_preset: 'reset_local',
          run_liquibase: true,
        }),
      ),
    );
  });

  it('reports a structured build failure with actionable guidance and no retry', async () => {
    mockDesktopApi.prepareEnvironment.mockRejectedValueOnce({
      code: 'MAVEN_BUILD_FAILED',
      message: 'mvn clean install exited with code 1',
    });
    const { user } = await openControlCenter();
    await pickServices(user, ['orders-api']);

    await executeCurrent(user);

    const dialog = await screen.findByRole('dialog', {
      name: 'Unable to start the environment workflow',
    });
    expect(dialog).toHaveTextContent('mvn clean install exited with code 1');
    expect(dialog).toHaveTextContent('MAVEN_BUILD_FAILED');
    expect(dialog).toHaveTextContent(/Open the failed task logs/i);
    expect(mockDesktopApi.prepareEnvironment).toHaveBeenCalledTimes(1);
  });
});

describe('control center progress and results', () => {
  it('follows daemon task state and opens task logs in place', async () => {
    mockDesktopApi.listTasks.mockResolvedValue([
      makeTaskView({
        id: 'task-build-orders',
        kind: 'build_service',
        status: 'running',
        service_id: 'orders-api',
        workflow_id: 'wf-env-1',
        progress: { current: 1, total: 3, message: 'compiling', fraction: 0.33 },
      }),
    ]);
    const { user } = await openControlCenter();
    await pickServices(user, ['orders-api']);

    await executeCurrent(user);

    const progress = await screen.findByRole('region', { name: 'Workflow progress' });
    expect(await within(progress).findByText('compiling')).toBeInTheDocument();

    await user.click(
      within(progress).getByRole('button', {
        name: 'Open logs for task-build-orders',
      }),
    );

    expect(
      await screen.findByRole('dialog', {
        name: 'Task logs — task-build-orders',
      }),
    ).toBeInTheDocument();
    expect(mockDesktopApi.getTaskLogs).toHaveBeenCalledWith(
      'task-build-orders',
      expect.anything(),
    );
  });

  it('shows the daemon’s terminal results inline without leaving the screen', async () => {
    mockDesktopApi.getWorkflowStatus.mockResolvedValue(makePartiallySucceededStatus());
    const { user } = await openControlCenter();
    await pickServices(user, ['orders-api', 'admin-ui']);

    await executeCurrent(user);

    const results = await screen.findByRole('region', { name: 'Workflow results' });
    expect(results).toHaveTextContent('Partially succeeded');
    expect(results).toHaveTextContent('BUILD_FAILED');
    expect(results).toHaveTextContent('npm run build exited with code 1');
    expect(
      await screen.findByRole('region', { name: 'Final runtime and health' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('heading', { name: 'Workspace Control Center', level: 1 }),
    ).toBeInTheDocument();
  });

  it('cancels only after confirmation and through the daemon workflow id', async () => {
    const { user } = await openControlCenter();
    await pickServices(user, ['orders-api']);
    await executeCurrent(user);
    await screen.findByRole('region', { name: 'Workflow progress' });

    await user.click(screen.getByRole('button', { name: 'Cancel workflow…' }));
    const dialog = await screen.findByRole('dialog', {
      name: 'Cancel workflow wf-env-1',
    });
    expect(dialog).toHaveTextContent(/does not roll back/i);

    await user.click(
      within(dialog).getByRole('button', { name: 'Cancel workflow' }),
    );

    await waitFor(() =>
      expect(mockDesktopApi.cancelWorkflow).toHaveBeenCalledWith('wf-env-1'),
    );
    expect(mockDesktopApi.cancelWorkflow).toHaveBeenCalledTimes(1);
  });
});

describe('control center reconnection', () => {
  it('disables mutating actions while disconnected and preserves the selection', async () => {
    const { user } = await openControlCenter();
    await pickServices(user, ['orders-api']);

    act(() => emitConnection(makeDisconnected('socket gone')));
    await flush();

    expect(screen.getByText(/Mutating actions are disabled/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Prepare & Run' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Build Only' })).toBeDisabled();
    expect(within(selectedChips()).getByText('orders-api')).toBeInTheDocument();

    act(() => emitConnection({ state: 'connected', message: null, since_unix: 1 }));
    await flush();

    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Prepare & Run' })).toBeEnabled(),
    );
  });

  it('keeps observing the daemon workflow without replaying operations', async () => {
    const { user } = await openControlCenter();
    await pickServices(user, ['orders-api']);
    await executeCurrent(user);
    await screen.findByRole('region', { name: 'Workflow progress' });

    act(() => emitConnection(makeDisconnected('socket gone')));
    await flush();

    expect(
      screen.getByRole('heading', { name: 'Workflow wf-env-1' }),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Cancel workflow…' })).toBeDisabled();
    expect(mockDesktopApi.prepareEnvironment).toHaveBeenCalledTimes(1);

    act(() => emitConnection({ state: 'connected', message: null, since_unix: 1 }));
    await flush();

    // Reconnecting resumes observation; it never resubmits the workflow.
    expect(mockDesktopApi.prepareEnvironment).toHaveBeenCalledTimes(1);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Cancel workflow…' })).toBeEnabled(),
    );
  });
});

describe('control center empty states', () => {
  it('waits for the daemon connection instead of claiming no workspace', async () => {
    mockDesktopApi.getConnectionState.mockResolvedValue({
      state: 'connecting',
      message: null,
      since_unix: 1,
    });

    await renderApp();

    expect(
      screen.getByText('Waiting for the daemon connection…'),
    ).toBeInTheDocument();
    expect(screen.queryByText('No workspace configured')).not.toBeInTheDocument();
  });

  it('shows services once the workspace is available', async () => {
    const { user } = await openControlCenter();
    await user.click(screen.getByRole('button', { name: 'Select services…' }));
    const dialog = screen.getByRole('dialog', { name: 'Select services' });
    expect(within(dialog).getByText('orders-api')).toBeInTheDocument();
    expect(within(dialog).getByText('admin-ui')).toBeInTheDocument();
    expect(mockDesktopApi.listServices).toHaveBeenCalledWith('demo');
  });
});
