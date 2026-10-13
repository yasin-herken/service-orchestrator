import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { flush, happyPathDefaults, renderApp } from '../test/harness';
import { emitConnection, mockDesktopApi, resetMockDesktopApi } from '../test/mocks';
import {
  makeDisconnected,
  makePartiallySucceededStatus,
  makeWorkflowStatus,
} from '../test/fixtures';

vi.mock('../services/desktopApi', async () => ({
  desktopApi: (await import('../test/mocks')).mockDesktopApi,
}));

beforeEach(() => {
  resetMockDesktopApi();
  happyPathDefaults();
});

/** Opens the Workflows screen and waits for the configuration card. */
async function openWorkflows() {
  const user = userEvent.setup();
  await renderApp();
  await user.click(screen.getByRole('button', { name: 'Workflows' }));
  await screen.findByText('Prepare a development environment');
  return user;
}

/** Selects services, previews the plan, and returns to the plan view. */
async function planWorkflow(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByLabelText('Select orders-api'));
  await user.click(screen.getByRole('button', { name: 'Preview plan…' }));
  await screen.findByRole('region', { name: 'Workflow plan' });
  return user;
}

describe('service selection', () => {
  it('selects services explicitly and never the whole workspace implicitly', async () => {
    const user = await openWorkflows();

    expect(screen.getByLabelText('Select orders-api')).not.toBeChecked();
    expect(screen.getByLabelText('Select admin-ui')).not.toBeChecked();
    // The plan button stays disabled until something is explicitly selected.
    expect(screen.getByRole('button', { name: 'Preview plan…' })).toBeDisabled();

    await user.click(screen.getByLabelText('Select orders-api'));
    expect(screen.getByLabelText('Select orders-api')).toBeChecked();
    expect(screen.getByLabelText('Select admin-ui')).not.toBeChecked();
    expect(screen.getByRole('button', { name: 'Preview plan…' })).toBeEnabled();
  });

  it('supports selecting every visible service on request', async () => {
    const user = await openWorkflows();

    await user.click(screen.getByRole('button', { name: 'Select all visible' }));

    expect(screen.getByLabelText('Select orders-api')).toBeChecked();
    expect(screen.getByLabelText('Select admin-ui')).toBeChecked();
    expect(screen.getByText('2 selected')).toBeInTheDocument();
  });
});

describe('profile selection', () => {
  it('lists profiles with their services, groups, and resolved services', async () => {
    const user = await openWorkflows();

    await user.click(screen.getByLabelText('Select by profile'));

    expect(await screen.findByLabelText('Select profile full-stack')).toBeInTheDocument();
    expect(mockDesktopApi.listProfiles).toHaveBeenCalledWith('demo');
    await user.click(screen.getByLabelText('Select profile full-stack'));

    expect(screen.getByText('orders-api')).toBeInTheDocument();
    expect(screen.getByText('core')).toBeInTheDocument();
    expect(screen.getByText('Full stack')).toBeInTheDocument();
  });

  it('plans with the profile id instead of an implicit service list', async () => {
    const user = await openWorkflows();
    await user.click(screen.getByLabelText('Select by profile'));
    await user.click(await screen.findByLabelText('Select profile full-stack'));

    await user.click(screen.getByRole('button', { name: 'Preview plan…' }));
    await screen.findByRole('region', { name: 'Workflow plan' });

    expect(mockDesktopApi.planPrepareEnvironment).toHaveBeenCalledWith(
      expect.objectContaining({ workspace_id: 'demo', profile: 'full-stack' }),
    );
  });
});

describe('group selection', () => {
  it('lists groups with their members and plans with the group id', async () => {
    const user = await openWorkflows();
    await user.click(screen.getByLabelText('Select by group'));
    await user.click(await screen.findByLabelText('Select group core'));

    expect(screen.getByText('admin-ui')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Preview plan…' }));
    await screen.findByRole('region', { name: 'Workflow plan' });

    expect(mockDesktopApi.planPrepareEnvironment).toHaveBeenCalledWith(
      expect.objectContaining({ workspace_id: 'demo', group: 'core' }),
    );
  });
});

describe('workflow options', () => {
  it('keeps safe defaults and maps every change onto the request', async () => {
    const user = await openWorkflows();

    // Safe defaults: sync on, migrations/build/start off.
    expect(screen.getByLabelText('Synchronize repositories (fetch)')).toBeChecked();
    expect(screen.getByLabelText('Run migrations')).not.toBeChecked();
    expect(screen.getByLabelText('Build services')).not.toBeChecked();
    expect(screen.getByLabelText('Start services')).not.toBeChecked();

    await user.click(screen.getByLabelText('Select orders-api'));
    await user.click(screen.getByLabelText('Run migrations'));
    await user.click(screen.getByLabelText('Start services'));
    await user.type(screen.getByLabelText('Target branch'), 'develop');
    await user.click(screen.getByRole('button', { name: 'Preview plan…' }));
    await screen.findByRole('region', { name: 'Workflow plan' });

    expect(mockDesktopApi.planPrepareEnvironment).toHaveBeenCalledWith(
      expect.objectContaining({
        services: ['orders-api'],
        run_liquibase: true,
        start: true,
        branch: 'develop',
        sync: true,
        build: false,
      }),
    );
  });
});

describe('workflow planning', () => {
  it('renders the backend-generated plan with dependencies and warnings', async () => {
    const user = await openWorkflows();
    await planWorkflow(user);

    // Real plan values, not recomputed in React.
    expect(screen.getAllByText('admin-ui').length).toBeGreaterThan(0);
    expect(
      screen.getByText('required dependency (added automatically)'),
    ).toBeInTheDocument();
    expect(screen.getByText('shared-kit')).toBeInTheDocument();
    expect(
      screen.getByText(
        "working tree of 'admin-ui' has uncommitted local changes",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText(/Read-only plan generated by the daemon/)).toBeInTheDocument();
    expect(screen.getByText(/Revalidates every precondition|re-validates/)).toBeInTheDocument();
  });

  it('shows a structured planning error instead of executing anything', async () => {
    mockDesktopApi.planPrepareEnvironment.mockRejectedValueOnce({
      code: 'INVALID_INPUT',
      message: 'select at least one service, or a profile or group',
    });
    const user = await openWorkflows();
    await user.click(screen.getByLabelText('Select orders-api'));
    await user.click(screen.getByRole('button', { name: 'Preview plan…' }));

    const state = await screen.findByRole('alert');
    expect(state).toHaveTextContent('select at least one service');
    expect(mockDesktopApi.prepareEnvironment).not.toHaveBeenCalled();
  });
});

describe('workflow execution', () => {
  it('asks for confirmation, then follows the daemon workflow id', async () => {
    const user = await openWorkflows();
    await planWorkflow(user);

    await user.click(screen.getByRole('button', { name: 'Execute workflow…' }));
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent(/intent, not authorization/i);

    await user.click(within(dialog).getByRole('button', { name: 'Execute workflow' }));

    await waitFor(() =>
      expect(mockDesktopApi.prepareEnvironment).toHaveBeenCalledWith(
        expect.objectContaining({ workspace_id: 'demo', services: ['orders-api'] }),
      ),
    );
    // The running screen follows the daemon's real workflow id.
    expect(await screen.findByText(/Workflow wf-env-1/)).toBeInTheDocument();
  });

  it('does not execute when the confirmation is dismissed', async () => {
    const user = await openWorkflows();
    await planWorkflow(user);

    await user.click(screen.getByRole('button', { name: 'Execute workflow…' }));
    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));

    expect(mockDesktopApi.prepareEnvironment).not.toHaveBeenCalled();
  });

  it('surfaces a daemon authorization refusal without retrying', async () => {
    mockDesktopApi.prepareEnvironment.mockRejectedValueOnce({
      code: 'PermissionDenied',
      message: 'the daemon could not verify an authorized desktop request',
    });
    const user = await openWorkflows();
    await planWorkflow(user);
    await user.click(screen.getByRole('button', { name: 'Execute workflow…' }));
    const confirm = await screen.findByRole('dialog');
    await user.click(within(confirm).getByRole('button', { name: 'Execute workflow' }));

    const error = await screen.findByRole('dialog');
    expect(error).toHaveTextContent('the daemon could not verify an authorized desktop request');
    expect(error).toHaveTextContent(/never authorization/i);
    expect(mockDesktopApi.prepareEnvironment).toHaveBeenCalledTimes(1);
  });
});

describe('workflow progress', () => {
  it('shows phase progress and per-service task state from the daemon', async () => {
    mockDesktopApi.getWorkflowStatus.mockResolvedValue(makeWorkflowStatus());
    mockDesktopApi.listTasks.mockResolvedValue([
      {
        id: 'task-git-orders',
        kind: 'git_sync',
        status: 'succeeded',
        service_id: 'orders-api',
        library_id: null,
        workflow_id: 'wf-env-1',
        workspace_id: 'demo',
        created_at: 1_700_000_000,
        started_at: 1_700_000_001,
        finished_at: 1_700_000_010,
        duration_ms: 9_000,
        progress: { current: 1, total: 1, message: null, fraction: 1 },
        failure: null,
        dependencies: [],
      },
      {
        id: 'task-build-orders',
        kind: 'build_service',
        status: 'running',
        service_id: 'orders-api',
        library_id: null,
        workflow_id: 'wf-env-1',
        workspace_id: 'demo',
        created_at: 1_700_000_020,
        started_at: 1_700_000_021,
        finished_at: null,
        duration_ms: null,
        progress: { current: 1, total: 3, message: 'compiling', fraction: 0.33 },
        failure: null,
        dependencies: [],
      },
    ]);
    const user = await openWorkflows();
    await planWorkflow(user);
    await user.click(screen.getByRole('button', { name: 'Execute workflow…' }));
    const confirm = await screen.findByRole('dialog');
    await user.click(within(confirm).getByRole('button', { name: 'Execute workflow' }));

    const progress = await screen.findByRole('region', { name: 'Workflow progress' });
    await within(progress).findByText('git-sync');
    expect(progress).toHaveTextContent('1/1 succeeded');
    expect(progress).toHaveTextContent('compiling');
    expect(progress).toHaveTextContent('Running');
  });

  it('moves to results when the daemon reports a terminal state', async () => {
    mockDesktopApi.getWorkflowStatus.mockResolvedValue(makePartiallySucceededStatus());
    const user = await openWorkflows();
    await planWorkflow(user);
    await user.click(screen.getByRole('button', { name: 'Execute workflow…' }));
    const confirm = await screen.findByRole('dialog');
    await user.click(within(confirm).getByRole('button', { name: 'Execute workflow' }));

    const results = await screen.findByRole('region', { name: 'Workflow results' });
    expect(results).toHaveTextContent('Partially succeeded');
    expect(results).toHaveTextContent('BUILD_FAILED');
    expect(results).toHaveTextContent('npm run build exited with code 1');
  });
});

describe('workflow cancellation', () => {
  it('confirms with consequences, then cancels through the daemon', async () => {
    mockDesktopApi.getWorkflowStatus.mockResolvedValue(makeWorkflowStatus());
    const user = await openWorkflows();
    await planWorkflow(user);
    await user.click(screen.getByRole('button', { name: 'Execute workflow…' }));
    const confirmExecute = await screen.findByRole('dialog');
    await user.click(within(confirmExecute).getByRole('button', { name: 'Execute workflow' }));

    await screen.findByRole('region', { name: 'Workflow progress' });
    await user.click(screen.getByRole('button', { name: 'Cancel workflow…' }));

    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent(/does not roll back/i);
    expect(dialog).toHaveTextContent('wf-env-1');
    await user.click(within(dialog).getByRole('button', { name: 'Cancel workflow' }));

    await waitFor(() =>
      expect(mockDesktopApi.cancelWorkflow).toHaveBeenCalledWith('wf-env-1'),
    );
  });

  it('keeps the workflow running when the cancellation is dismissed', async () => {
    mockDesktopApi.getWorkflowStatus.mockResolvedValue(makeWorkflowStatus());
    const user = await openWorkflows();
    await planWorkflow(user);
    await user.click(screen.getByRole('button', { name: 'Execute workflow…' }));
    const confirmExecute = await screen.findByRole('dialog');
    await user.click(within(confirmExecute).getByRole('button', { name: 'Execute workflow' }));

    await screen.findByRole('region', { name: 'Workflow progress' });
    await user.click(screen.getByRole('button', { name: 'Cancel workflow…' }));
    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));

    expect(mockDesktopApi.cancelWorkflow).not.toHaveBeenCalled();
  });
});

describe('workflow history', () => {
  it('lists bounded daemon-side history with final statuses', async () => {
    await openWorkflows();

    const history = await screen.findByRole('region', { name: 'Recent workflows' });
    expect(within(history).getByText('wf-env-1')).toBeInTheDocument();
    expect(within(history).getByText('wf-env-0')).toBeInTheDocument();
    expect(within(history).getByText('Partially succeeded')).toBeInTheDocument();
    expect(mockDesktopApi.listWorkflows).toHaveBeenCalledWith('demo');
  });

  it('inspects a historical workflow by its daemon id', async () => {
    mockDesktopApi.getWorkflowStatus.mockResolvedValue(
      makeWorkflowStatus({ workflow_id: 'wf-env-1', state: 'succeeded' }),
    );
    const user = await openWorkflows();
    const history = await screen.findByRole('region', { name: 'Recent workflows' });

    await user.click(within(history).getAllByRole('button', { name: 'Inspect' })[0]);

    await waitFor(() =>
      expect(mockDesktopApi.getWorkflowStatus).toHaveBeenCalledWith('wf-env-1'),
    );
    expect(
      await screen.findByRole('region', { name: 'Workflow results' }),
    ).toBeInTheDocument();
  });
});

describe('workspace switching', () => {
  it('re-queries profiles and groups for the newly selected workspace', async () => {
    mockDesktopApi.listWorkspaces.mockResolvedValue([
      { id: 'demo', name: 'Demo', root: '/p/demo', service_count: 2 },
      { id: 'second', name: 'Second', root: '/p/second', service_count: 1 },
    ]);
    const user = await openWorkflows();
    await user.click(screen.getByLabelText('Select by profile'));

    await user.selectOptions(screen.getByLabelText('Active workspace'), 'second');

    await waitFor(() => expect(mockDesktopApi.listProfiles).toHaveBeenCalledWith('second'));
    // The old selection does not leak into the new workspace.
    expect(screen.queryByLabelText('Select profile full-stack')).not.toBeInTheDocument();
  });
});

describe('daemon disconnection', () => {
  it('disables planning and execution while keeping the workflow id', async () => {
    mockDesktopApi.getWorkflowStatus.mockResolvedValue(makeWorkflowStatus());
    const user = await openWorkflows();
    await planWorkflow(user);

    act(() => emitConnection(makeDisconnected('the daemon is not running')));
    await flush();

    expect(screen.getByText(/Disconnected from the daemon/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Execute workflow…' })).toBeDisabled();
    expect(mockDesktopApi.prepareEnvironment).not.toHaveBeenCalled();
  });

  it('disables cancellation for a running workflow when disconnected', async () => {
    mockDesktopApi.getWorkflowStatus.mockResolvedValue(makeWorkflowStatus());
    const user = await openWorkflows();
    await planWorkflow(user);
    await user.click(screen.getByRole('button', { name: 'Execute workflow…' }));
    const confirmExecute = await screen.findByRole('dialog');
    await user.click(within(confirmExecute).getByRole('button', { name: 'Execute workflow' }));
    await screen.findByRole('region', { name: 'Workflow progress' });

    act(() => emitConnection(makeDisconnected('the daemon is not running')));
    await flush();

    expect(screen.getByText(/Disconnected from the daemon/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Cancel workflow…' })).toBeDisabled();
    // The workflow identity survives the disconnect.
    expect(screen.getByText(/Workflow wf-env-1/)).toBeInTheDocument();
  });
});
