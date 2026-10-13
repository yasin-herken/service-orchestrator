import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { flush, happyPathDefaults, renderApp } from '../test/harness';
import { emitConnection, emitTaskEvent, mockDesktopApi, resetMockDesktopApi } from '../test/mocks';
import {
  makeBranches,
  makeDisconnected,
  makeRepositories,
  makeRepository,
  makeWorkspace,
} from '../test/fixtures';

vi.mock('../services/desktopApi', async () => ({
  desktopApi: (await import('../test/mocks')).mockDesktopApi,
}));

beforeEach(() => {
  resetMockDesktopApi();
  happyPathDefaults();
});

/** Opens the Git screen. */
async function openGit() {
  const user = userEvent.setup();
  await renderApp();
  await user.click(screen.getByRole('button', { name: 'Git' }));
  await screen.findByRole('table');
  return user;
}

/** The data rows of the repository table (header excluded). */
function dataRows(): HTMLElement[] {
  return screen.getAllByRole('row').slice(1);
}

/** The checkbox that selects one repository row. */
function rowCheckbox(name: string): HTMLInputElement {
  return screen.getByRole('checkbox', { name: `Select ${name}` }) as HTMLInputElement;
}

describe('git repository list', () => {
  it('renders repository rows with path, branch, and state', async () => {
    await openGit();

    const rows = dataRows();
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent('admin-ui');
    expect(rows[0]).toHaveTextContent('frontend/admin-ui');
    expect(rows[0]).toHaveTextContent('main');
    within(rows[0]).getByText('clean');
  });

  it('filters repositories by search text and Git state', async () => {
    mockDesktopApi.listRepositories.mockResolvedValue([
      ...makeRepositories(),
      makeRepositoryModified(),
    ]);
    const user = await openGit();

    await user.type(screen.getByRole('searchbox', { name: 'Search repositories' }), 'orders');
    await waitFor(() => expect(dataRows()).toHaveLength(1));
    expect(dataRows()[0]).toHaveTextContent('orders-api');

    await user.clear(screen.getByRole('searchbox', { name: 'Search repositories' }));
    await user.selectOptions(screen.getByLabelText('Filter by Git state'), 'modified');
    await waitFor(() => expect(dataRows()).toHaveLength(1));
    expect(dataRows()[0]).toHaveTextContent('legacy-api');
  });
});

describe('git selection', () => {
  it('selects, selects all visible, and clears repositories', async () => {
    const user = await openGit();

    await user.click(rowCheckbox('orders-api'));
    expect(rowCheckbox('orders-api')).toBeChecked();
    expect(
      screen.getByRole('toolbar', { name: 'Repository selection actions' }),
    ).toHaveTextContent('1 selected');

    await user.click(screen.getByRole('checkbox', { name: 'Select all visible repositories' }));
    expect(rowCheckbox('admin-ui')).toBeChecked();

    await user.click(screen.getByRole('button', { name: 'Clear' }));
    expect(rowCheckbox('orders-api')).not.toBeChecked();
    expect(rowCheckbox('admin-ui')).not.toBeChecked();
  });
});

describe('git inspection', () => {
  it('inspects a repository on demand and shows commit and ahead/behind', async () => {
    const user = await openGit();

    await user.click(dataRows()[1]);
    const panel = await screen.findByRole('region', { name: 'Repository detail' });
    await waitFor(() =>
      expect(mockDesktopApi.inspectRepositories).toHaveBeenCalledWith('demo', ['orders-api']),
    );
    expect(panel).toHaveTextContent('0123456789abcdef');
    expect(panel).toHaveTextContent('origin/main');
    await within(panel).findByText('0 / 0');
  });

  it('loads branches and checks out a non-current branch', async () => {
    mockDesktopApi.listBranches.mockResolvedValue([
      ...makeBranches(),
      { name: 'develop', kind: 'local', current: false, upstream: null, commit: 'def' },
    ]);
    const user = await openGit();

    await user.click(dataRows()[1]);
    const panel = await screen.findByRole('region', { name: 'Repository detail' });
    await within(panel).findByRole('option', { name: 'develop' });

    await user.selectOptions(within(panel).getByLabelText('Checkout branch'), 'develop');
    await user.click(within(panel).getByRole('button', { name: 'Checkout' }));

    await waitFor(() =>
      expect(mockDesktopApi.checkoutRepository).toHaveBeenCalledWith('orders-api', 'develop'),
    );
  });
});

describe('bulk git synchronization', () => {
  it('previews a bulk sync with dirty warnings before executing', async () => {
    const user = await openGit();

    await user.click(screen.getByRole('checkbox', { name: 'Select all visible repositories' }));
    await user.click(screen.getByRole('button', { name: 'Sync selected…' }));

    const dialog = screen.getByRole('dialog', { name: 'Synchronize 2 repositories' });
    // Nothing executes until the user explicitly asks for a preview and runs it.
    expect(mockDesktopApi.syncRepositories).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole('button', { name: 'Preview' }));
    await waitFor(() =>
      expect(mockDesktopApi.previewGitSync).toHaveBeenCalledWith(
        'demo',
        ['admin-ui', 'orders-api'],
        'main',
        'sync',
      ),
    );

    await within(dialog).findByText(/working tree has uncommitted local changes/);
    expect(dialog).toHaveTextContent('1 dirty');
    expect(dialog).toHaveTextContent(/never discarded/i);

    await user.click(within(dialog).getByRole('button', { name: 'Execute' }));
    await waitFor(() =>
      expect(mockDesktopApi.syncRepositories).toHaveBeenCalledWith('demo', [
        'admin-ui',
        'orders-api',
      ]),
    );
  });

  it('reports real per-repository workflow progress from task events', async () => {
    const user = await openGit();

    await user.click(screen.getByRole('checkbox', { name: 'Select all visible repositories' }));
    await user.click(screen.getByRole('button', { name: 'Sync selected…' }));
    const dialog = screen.getByRole('dialog', { name: 'Synchronize 2 repositories' });
    await user.click(within(dialog).getByRole('button', { name: 'Preview' }));
    await within(dialog).findByText(/ready/);
    await user.click(within(dialog).getByRole('button', { name: 'Execute' }));

    const progress = await screen.findByRole('dialog', { name: /Sync — 2 repositories/ });
    expect(progress).toHaveTextContent('Waiting for the daemon');

    act(() => {
      emitTaskEvent({
        TaskCreated: {
          task: {
            id: 'g1',
            kind: 'git_sync',
            status: 'succeeded',
            service: 'orders-api',
            workflow: 'wf-sync',
            progress: { current: 1, total: 1, message: null },
          },
        },
      });
      emitTaskEvent({
        TaskCreated: {
          task: {
            id: 'g2',
            kind: 'git_sync',
            status: 'failed',
            service: 'admin-ui',
            workflow: 'wf-sync',
            progress: { current: 1, total: 1, message: null },
            error: { code: 'GIT_LOCAL_CHANGES', message: 'working tree has local changes' },
          },
        },
      });
    });
    await flush();

    const updated = screen.getByRole('dialog', { name: /Sync — 2 repositories/ });
    expect(updated).toHaveTextContent('Succeeded');
    expect(updated).toHaveTextContent('Failed');
    expect(updated).toHaveTextContent('working tree has local changes');
  });

  it('surfaces a structured error when the daemon rejects the sync', async () => {
    mockDesktopApi.syncRepositories.mockRejectedValueOnce({
      code: 'GIT_AUTH_FAILED',
      message: 'authentication failed for the remote',
    });
    const user = await openGit();

    await user.click(screen.getByRole('checkbox', { name: 'Select all visible repositories' }));
    await user.click(screen.getByRole('button', { name: 'Sync selected…' }));
    const dialog = screen.getByRole('dialog', { name: 'Synchronize 2 repositories' });
    // Skip the preview: the dialog still allows executing the selected mode.
    await user.click(within(dialog).getByRole('button', { name: 'Preview' }));
    await within(dialog).findByText(/ready/);
    await user.click(within(dialog).getByRole('button', { name: 'Execute' }));

    const error = await screen.findByRole('dialog', { name: /Unable to sync repositories/ });
    expect(error).toHaveTextContent('authentication failed for the remote');
    expect(error).toHaveTextContent('GIT_AUTH_FAILED');
  });
});

describe('git cancellation and connection', () => {
  it('cancels an active bulk workflow through the task API', async () => {
    const user = await openGit();

    await user.click(screen.getByRole('checkbox', { name: 'Select all visible repositories' }));
    await user.click(screen.getByRole('button', { name: 'Sync selected…' }));
    const dialog = screen.getByRole('dialog', { name: 'Synchronize 2 repositories' });
    await user.click(within(dialog).getByRole('button', { name: 'Preview' }));
    await within(dialog).findByText(/ready/);
    await user.click(within(dialog).getByRole('button', { name: 'Execute' }));

    const progress = await screen.findByRole('dialog', { name: /Sync — 2 repositories/ });
    await user.click(within(progress).getByRole('button', { name: 'Cancel workflow' }));

    await waitFor(() =>
      expect(mockDesktopApi.cancelWorkflow).toHaveBeenCalledWith('wf-sync'),
    );
  });

  it('disables git actions while disconnected and keeps loaded state', async () => {
    const user = await openGit();
    await user.click(screen.getByRole('checkbox', { name: 'Select all visible repositories' }));

    act(() => emitConnection(makeDisconnected('the daemon is not running')));
    await flush();

    expect(await screen.findByRole('status')).toHaveTextContent('Disconnected from the daemon');
    expect(screen.getByRole('button', { name: 'Sync selected…' })).toBeDisabled();
    expect(rowCheckbox('orders-api')).toBeInTheDocument();
  });

  it('clears the selection when switching workspaces', async () => {
    mockDesktopApi.listWorkspaces.mockResolvedValue([
      makeWorkspace(),
      makeWorkspace({ id: 'second', name: 'Second Platform', service_count: 1 }),
    ]);
    mockDesktopApi.listRepositories.mockImplementation(async (workspaceId) =>
      workspaceId === 'demo'
        ? Promise.resolve(makeRepositories())
        : Promise.resolve([
            makeRepositoryModified({
              id: 'second:solo-api',
              service_id: 'solo-api',
              workspace_id: 'second',
              name: 'solo-api',
            }),
          ]),
    );

    const user = await openGit();
    await user.click(rowCheckbox('orders-api'));

    await user.selectOptions(screen.getByLabelText('Active workspace'), 'second');
    await screen.findByText('solo-api');

    expect(
      screen.getByRole('toolbar', { name: 'Repository selection actions' }),
    ).toHaveTextContent('0 selected');
  });
});

/** A third, modified repository used by filter tests. */
function makeRepositoryModified(overrides = {}) {
  return makeRepository({
    id: 'demo:legacy-api',
    service_id: 'legacy-api',
    workspace_id: 'demo',
    name: 'legacy-api',
    path: 'backend/legacy-api',
    repository: {
      url: 'https://example.com/legacy-api.git',
      local_path: 'backend/legacy-api',
      default_branch: 'main',
      provider: 'github',
    },
    git_state: 'modified',
    branch: 'feature/x',
    ...overrides,
  });
}
