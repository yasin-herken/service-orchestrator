import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { flush, happyPathDefaults, renderApp } from '../test/harness';
import { emitConnection, mockDesktopApi, resetMockDesktopApi } from '../test/mocks';
import { makeDisconnected, makeLogPage, makeTaskView } from '../test/fixtures';

vi.mock('../services/desktopApi', async () => ({
  desktopApi: (await import('../test/mocks')).mockDesktopApi,
}));

beforeEach(() => {
  resetMockDesktopApi();
  happyPathDefaults();
});

/** Opens the Migrations screen and waits for the table. */
async function openMigrations() {
  const user = userEvent.setup();
  await renderApp();
  await user.click(screen.getByRole('button', { name: 'Migrations' }));
  await screen.findByRole('table');
  return user;
}

/** Opens the detail panel for the first (local) migration target. */
async function openDetail(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getAllByRole('row')[1]);
  return screen.findByRole('region', { name: 'Migration detail' });
}

describe('migration listing', () => {
  it('lists every migration-enabled service with its configuration', async () => {
    await openMigrations();

    const table = screen.getByRole('table');
    expect(within(table).getByText('orders-api')).toBeInTheDocument();
    expect(within(table).getByText('billing-api')).toBeInTheDocument();
    expect(within(table).getAllByText('liquibase')).toHaveLength(2);
    expect(within(table).getByText('jdbc:postgresql://localhost:5432/orders')).toBeInTheDocument();
  });

  it('shows “Unknown” for an unknown pending count instead of inventing one', async () => {
    await openMigrations();

    expect(screen.getAllByText('Unknown').length).toBeGreaterThan(0);
  });

  it('marks a non-local target as requiring authorization', async () => {
    await openMigrations();

    const rows = screen.getAllByRole('row');
    const staging = rows.find((row) => row.textContent?.includes('billing-api'));
    expect(staging).toBeTruthy();
    expect(staging).toHaveTextContent(/authorization required/i);
  });

  it('shows an empty state when the workspace has no migration targets', async () => {
    mockDesktopApi.listMigrations.mockResolvedValue([]);
    const user = userEvent.setup();
    await renderApp();
    await user.click(screen.getByRole('button', { name: 'Migrations' }));

    expect(await screen.findByText('No migration-enabled services')).toBeInTheDocument();
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
  });
});

describe('migration status', () => {
  it('opens the configuration snapshot for one service', async () => {
    const user = await openMigrations();
    const panel = await openDetail(user);

    expect(mockDesktopApi.getMigrationStatus).toHaveBeenCalledWith('orders-api');
    await within(panel).findByText('jdbc:postgresql://localhost:5432/orders');
    expect(panel).toHaveTextContent('liquibase');
    expect(panel).toHaveTextContent(/maven/i);
  });

  it('shows a stale-guarded error when the snapshot cannot be read', async () => {
    mockDesktopApi.getMigrationStatus.mockRejectedValueOnce({
      code: 'WORKSPACE_NOT_FOUND',
      message: 'the workspace is not loaded',
    });
    const user = await openMigrations();
    const panel = await openDetail(user);

    await within(panel).findByText('the workspace is not loaded');
  });
});

describe('SQL preview', () => {
  it('waits for the preview task and shows its SQL', async () => {
    mockDesktopApi.getTask.mockResolvedValue(
      makeTaskView({ id: 'task-1', kind: 'migration_preview', status: 'succeeded' }),
    );
    mockDesktopApi.getTaskLogs.mockResolvedValue(
      makeLogPage({
        entries: [
          {
            timestamp: 1_700_000_010,
            level: 'info',
            message: 'ALTER TABLE orders ADD COLUMN note text;',
            source: 'migration',
            stream: null,
            service_id: 'orders-api',
            task_id: 'task-1',
            process_id: null,
            sequence: 1,
          },
        ],
      }),
    );
    const user = await openMigrations();
    const panel = await openDetail(user);

    await user.click(within(panel).getByRole('button', { name: 'SQL preview…' }));

    const dialog = await screen.findByRole('dialog');
    expect(mockDesktopApi.previewMigration).toHaveBeenCalledWith('orders-api');
    await within(dialog).findByText(/ALTER TABLE orders ADD COLUMN note text;/);
  });

  it('reports a preview that never produced SQL instead of applying anything', async () => {
    mockDesktopApi.getTask.mockResolvedValue(
      makeTaskView({ id: 'task-1', kind: 'migration_preview', status: 'failed' }),
    );
    const user = await openMigrations();
    const panel = await openDetail(user);

    await user.click(within(panel).getByRole('button', { name: 'SQL preview…' }));

    const dialog = await screen.findByRole('dialog');
    await waitFor(() =>
      expect(within(dialog).getByRole('alert')).toHaveTextContent(/PREVIEW_FAILED|did not finish/i),
    );
    expect(mockDesktopApi.applyMigration).not.toHaveBeenCalled();
  });
});

describe('migration execution', () => {
  it('asks for explicit confirmation before applying, passing it as intent', async () => {
    const user = await openMigrations();
    const panel = await openDetail(user);

    await user.click(within(panel).getByRole('button', { name: 'Apply migration…' }));

    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent(/can modify the database schema/i);
    await user.click(within(dialog).getByRole('button', { name: 'Apply migration' }));

    await waitFor(() =>
      expect(mockDesktopApi.applyMigration).toHaveBeenCalledWith('orders-api', true),
    );
  });

  it('does not apply when the user cancels the confirmation', async () => {
    const user = await openMigrations();
    const panel = await openDetail(user);

    await user.click(within(panel).getByRole('button', { name: 'Apply migration…' }));
    const dialog = await screen.findByRole('dialog');
    await user.click(within(dialog).getByRole('button', { name: 'Cancel' }));

    expect(mockDesktopApi.applyMigration).not.toHaveBeenCalled();
  });

  it('runs bulk apply as one coordinated workflow', async () => {
    const user = await openMigrations();

    await user.click(screen.getByLabelText('Select orders-api'));
    await user.click(screen.getByLabelText('Select billing-api'));
    await user.click(screen.getByRole('button', { name: 'Apply selected…' }));

    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent(/one coordinated workflow/i);
    await user.click(within(dialog).getByRole('button', { name: 'Apply selected' }));

    await waitFor(() =>
      expect(mockDesktopApi.applySelectedMigrations).toHaveBeenCalledWith(
        'demo',
        ['billing-api', 'orders-api'],
        true,
      ),
    );
  });

  it('shows real task progress after applying', async () => {
    const user = await openMigrations();
    const panel = await openDetail(user);

    await user.click(within(panel).getByRole('button', { name: 'Apply migration…' }));
    const confirm = await screen.findByRole('dialog');
    await user.click(within(confirm).getByRole('button', { name: 'Apply migration' }));

    const progress = await screen.findByRole('dialog');
    expect(progress).toHaveTextContent(/migration/i);
    // The outcome shown is the daemon's real task id, not a fabricated one.
    expect(mockDesktopApi.applyMigration).toHaveBeenCalledWith('orders-api', true);
  });
});

describe('authorization failures', () => {
  it('explains a daemon refusal instead of retrying', async () => {
    mockDesktopApi.applyMigration.mockRejectedValueOnce({
      code: 'PermissionDenied',
      message: 'the daemon could not verify an authorized desktop request',
    });
    const user = await openMigrations();
    const panel = await openDetail(user);

    await user.click(within(panel).getByRole('button', { name: 'Apply migration…' }));
    const confirm = await screen.findByRole('dialog');
    await user.click(within(confirm).getByRole('button', { name: 'Apply migration' }));

    const error = await screen.findByRole('dialog');
    expect(error).toHaveTextContent('the daemon could not verify an authorized desktop request');
    expect(error).toHaveTextContent(/never treated as authorization/i);
    expect(mockDesktopApi.applyMigration).toHaveBeenCalledTimes(1);
  });
});

describe('daemon disconnection', () => {
  it('disables migration actions while disconnected', async () => {
    const user = await openMigrations();
    const panel = await openDetail(user);

    act(() => emitConnection(makeDisconnected('the daemon is not running')));
    await flush();

    expect(screen.getByText(/Disconnected from the daemon/i)).toBeInTheDocument();
    expect(
      within(panel).getByRole('button', { name: 'Apply migration…' }),
    ).toBeDisabled();
    expect(mockDesktopApi.applyMigration).not.toHaveBeenCalled();
  });
});

describe('workspace switching', () => {
  it('re-queries migration targets for the newly selected workspace', async () => {
    mockDesktopApi.listWorkspaces.mockResolvedValue([
      { id: 'demo', name: 'Demo', root: '/p/demo', service_count: 2 },
      { id: 'second', name: 'Second', root: '/p/second', service_count: 1 },
    ]);
    const user = await openMigrations();

    await user.selectOptions(screen.getByLabelText('Active workspace'), 'second');

    await waitFor(() => expect(mockDesktopApi.listMigrations).toHaveBeenCalledWith('second'));
  });
});
