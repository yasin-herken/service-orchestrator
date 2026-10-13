import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { flush, happyPathDefaults, renderApp } from '../test/harness';
import { emitConnection, mockDesktopApi, resetMockDesktopApi } from '../test/mocks';
import { makeDisconnected, makeWorkflow } from '../test/fixtures';

vi.mock('../services/desktopApi', async () => ({
  desktopApi: (await import('../test/mocks')).mockDesktopApi,
}));

beforeEach(() => {
  resetMockDesktopApi();
  happyPathDefaults();
});

/** Opens the Tasks screen and waits for the table. */
async function openTasks() {
  const user = userEvent.setup();
  await renderApp();
  await user.click(screen.getByRole('button', { name: 'Tasks' }));
  await screen.findByRole('table');
  return user;
}

describe('task listing', () => {
  it('shows the bounded newest-first history with distinct terminal states', async () => {
    await openTasks();

    const table = screen.getByRole('table');
    expect(within(table).getByText('migration_apply')).toBeInTheDocument();
    expect(within(table).getByText('build_service')).toBeInTheDocument();
    expect(within(table).getByText('start_service')).toBeInTheDocument();
    // A failed task is never shown as a generic "completed".
    expect(within(table).getAllByText('Failed').length).toBeGreaterThan(0);
    expect(within(table).getAllByText('Running').length).toBeGreaterThan(0);
  });

  it('requests the bounded listing rather than unbounded history', async () => {
    await openTasks();

    expect(mockDesktopApi.listTasks).toHaveBeenCalledWith(200);
  });
});

describe('task filtering', () => {
  it('filters by status', async () => {
    const user = await openTasks();

    await user.selectOptions(screen.getByLabelText('Filter by status'), 'failed');

    const table = screen.getByRole('table');
    expect(within(table).getByText('migration_apply')).toBeInTheDocument();
    expect(within(table).queryByText('start_service')).not.toBeInTheDocument();
    expect(screen.getByText(/^1 task/)).toBeInTheDocument();
  });

  it('filters by free text', async () => {
    const user = await openTasks();

    await user.type(screen.getByLabelText('Search tasks'), 'build_service');

    await waitFor(() => expect(screen.getByText(/^1 task/)).toBeInTheDocument());
    const table = screen.getByRole('table');
    expect(table).toHaveTextContent('build_service');
    expect(table).not.toHaveTextContent('migration_apply');
  });

  it('clears every filter at once', async () => {
    const user = await openTasks();
    await user.selectOptions(screen.getByLabelText('Filter by status'), 'failed');
    await waitFor(() => expect(screen.getByText(/^1 task/)).toBeInTheDocument());

    await user.click(screen.getByRole('button', { name: 'Clear filters' }));

    await waitFor(() => expect(screen.getByText(/^3 tasks/)).toBeInTheDocument());
  });

  it('scopes the workspace filter to the active workspace', async () => {
    await openTasks();

    const workspaceFilter = screen.getByLabelText('Filter by workspace');
    expect(workspaceFilter).toHaveValue('demo');
  });
});

describe('task details', () => {
  it('opens one task with its structured failure', async () => {
    const user = await openTasks();

    const row = screen.getAllByRole('row').find((r) => r.textContent?.includes('migration_apply'));
    await user.click(row!);

    const panel = await screen.findByRole('region', { name: 'Task detail' });
    expect(panel).toHaveTextContent('MIGRATION_APPLY_FAILED');
    expect(panel).toHaveTextContent('changeset 3 was rejected by the database');
  });

  it('shows the workflow siblings from the same event-sourced engine view', async () => {
    const user = await openTasks();

    const row = screen.getAllByRole('row').find((r) => r.textContent?.includes('start_service'));
    await user.click(row!);

    const panel = await screen.findByRole('region', { name: 'Task detail' });
    // task-1 belongs to wf-1, so the daemon's workflow view is requested.
    expect(mockDesktopApi.getWorkflow).toHaveBeenCalledWith('wf-1');
    await within(panel).findByText(/apply migrations/);
  });

  it('surfaces a structured workflow-load error instead of a silent gap', async () => {
    mockDesktopApi.getWorkflow.mockRejectedValueOnce({
      code: 'WORKFLOW_NOT_FOUND',
      message: 'the workflow is unknown to the daemon',
    });
    const user = await openTasks();

    const row = screen.getAllByRole('row').find((r) => r.textContent?.includes('start_service'));
    await user.click(row!);

    const panel = await screen.findByRole('region', { name: 'Task detail' });
    expect(
      await within(panel).findByText('the workflow is unknown to the daemon'),
    ).toBeInTheDocument();
    expect(within(panel).getByText('WORKFLOW_NOT_FOUND')).toBeInTheDocument();
  });

  it('shows the declared workflow steps and dependencies', async () => {
    mockDesktopApi.getWorkflow.mockResolvedValue(
      makeWorkflow({
        id: 'wf-1',
        steps: [
          { id: 's1', description: 'sync repository', task_kind: 'git_sync', depends_on: [] },
          {
            id: 's2',
            description: 'apply migrations',
            task_kind: 'migration_apply',
            depends_on: ['s1'],
          },
        ],
      }),
    );
    const user = await openTasks();

    const row = screen.getAllByRole('row').find((r) => r.textContent?.includes('start_service'));
    await user.click(row!);

    const panel = await screen.findByRole('region', { name: 'Task detail' });
    expect(panel).toHaveTextContent('sync repository');
    expect(panel).toHaveTextContent('apply migrations');
  });
});

describe('task cancellation', () => {
  it('requests daemon-side cancellation and reports the returned state', async () => {
    const user = await openTasks();

    const row = screen.getAllByRole('row').find((r) => r.textContent?.includes('start_service'));
    await user.click(row!);
    const panel = await screen.findByRole('region', { name: 'Task detail' });

    await user.click(within(panel).getByRole('button', { name: 'Cancel task' }));

    await waitFor(() => expect(mockDesktopApi.cancelTask).toHaveBeenCalledWith('task-1'));
    // The returned task state is the authority; nothing claims a rollback.
    await waitFor(() =>
      expect(within(panel).getAllByText(/cancelled/i).length).toBeGreaterThan(0),
    );
  });

  it('surfaces a daemon refusal to cancel', async () => {
    mockDesktopApi.cancelTask.mockRejectedValueOnce({
      code: 'PermissionDenied',
      message: 'cancellation is not allowed for this client',
    });
    const user = await openTasks();

    const row = screen.getAllByRole('row').find((r) => r.textContent?.includes('start_service'));
    await user.click(row!);
    const panel = await screen.findByRole('region', { name: 'Task detail' });
    await user.click(within(panel).getByRole('button', { name: 'Cancel task' }));

    await within(panel).findByText('cancellation is not allowed for this client');
  });
});

describe('daemon disconnection', () => {
  it('disables cancellation while disconnected but keeps loaded history', async () => {
    const user = await openTasks();

    const row = screen.getAllByRole('row').find((r) => r.textContent?.includes('start_service'));
    await user.click(row!);
    const panel = await screen.findByRole('region', { name: 'Task detail' });

    act(() => emitConnection(makeDisconnected('the daemon is not running')));
    await flush();

    expect(screen.getAllByText(/Disconnected from the daemon/i).length).toBeGreaterThan(0);
    expect(within(panel).getByRole('button', { name: 'Cancel task' })).toBeDisabled();
    // History stays visible; nothing is replayed.
    expect(screen.getByRole('table')).toHaveTextContent('migration_apply');
  });
});

describe('workspace switching', () => {
  it('re-queries the task listing for the newly selected workspace', async () => {
    const user = await openTasks();

    await user.selectOptions(screen.getByLabelText('Active workspace'), 'demo');

    await flush();
    expect(mockDesktopApi.listTasks).toHaveBeenCalledWith(200);
  });
});

describe('cross-screen navigation', () => {
  it('opens the logs screen scoped to the selected task', async () => {
    const user = await openTasks();

    const row = screen.getAllByRole('row').find((r) => r.textContent?.includes('migration_apply'));
    await user.click(row!);
    const panel = await screen.findByRole('region', { name: 'Task detail' });

    await user.click(within(panel).getByRole('button', { name: /view.*logs/i }));

    const scope = await screen.findByLabelText('Log scope kind');
    expect(scope).toHaveValue('task');
    expect(screen.getByLabelText('Log task id')).toHaveValue('task-3');
  });
});
