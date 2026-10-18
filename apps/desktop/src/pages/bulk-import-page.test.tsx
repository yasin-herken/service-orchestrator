import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { happyPathDefaults, renderApp } from '../test/harness';
import { mockDesktopApi, resetMockDesktopApi } from '../test/mocks';
import {
  makeBulkImportPlan,
  makeBulkImportRow,
  makeDiscoveredRepository,
  makeDiscoveryReport,
} from '../test/fixtures';

vi.mock('../services/desktopApi', async () => ({
  desktopApi: (await import('../test/mocks')).mockDesktopApi,
}));

beforeEach(() => {
  resetMockDesktopApi();
  happyPathDefaults();
});

/** Opens the Bulk Import screen. */
async function openBulkImport() {
  const user = userEvent.setup();
  await renderApp();
  await user.click(screen.getByRole('button', { name: 'Bulk Import' }));
  await screen.findByText('Bulk service import');
  return user;
}

/** Scans the workspace and continues to the selection step. */
async function scanWorkspace(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole('button', { name: 'Scan workspace' }));
  await screen.findByText('Discovery complete');
  await user.click(screen.getByRole('button', { name: 'Select repositories' }));
  await screen.findByRole('table', { name: 'Discovered repositories' });
}

/** Clicks the wizard's Next button. */
async function next(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole('button', { name: 'Next' }));
}

describe('bulk import wizard', () => {
  it('walks the ten steps, validates, previews, and commits only after confirmation', async () => {
    const user = await openBulkImport();

    expect(screen.getByText('Step 1 of 10')).toBeInTheDocument();
    expect(screen.getByText(/Discovery walks the workspace root/)).toBeInTheDocument();

    await scanWorkspace(user);
    expect(screen.getByText('2 visible · 2 selected')).toBeInTheDocument();

    // Every discovered repository is selected for a bulk import by default.
    expect(screen.getByLabelText('Include backend/orders-api')).toBeChecked();
    expect(screen.getByLabelText('Include frontend/admin-ui')).toBeChecked();

    await next(user); // step 4
    expect(await screen.findByText('Detected technologies')).toBeInTheDocument();
    expect(screen.getByText('Suggested template: spring-boot-backend')).toBeInTheDocument();

    await next(user); // step 5
    expect(screen.getByText('Assign templates')).toBeInTheDocument();
    // Assign one template explicitly; the other keeps automatic matching.
    await user.selectOptions(
      screen.getByLabelText('Template for backend/orders-api'),
      'spring-boot-backend',
    );

    await next(user); // step 6
    expect(screen.getByText('Shared overrides')).toBeInTheDocument();
    expect(screen.getByLabelText('Port base')).toHaveValue('8080');

    await next(user); // step 7
    expect(screen.getByText('Per-repository exceptions')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Validate import' }));

    expect(await screen.findByText('Validated import plan')).toBeInTheDocument();
    expect(mockDesktopApi.planBulkImport).toHaveBeenCalledTimes(1);
    const planned = mockDesktopApi.planBulkImport.mock.calls[0][0];
    expect(planned.workspace_id).toBe('demo');
    expect(planned.repositories).toHaveLength(2);
    expect(planned.repositories).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          relative_path: 'backend/orders-api',
          template: 'spring-boot-backend',
          existing: 'skip',
        }),
        expect.objectContaining({ relative_path: 'frontend/admin-ui' }),
      ]),
    );
    // Planning is read-only.
    expect(mockDesktopApi.commitBulkImport).not.toHaveBeenCalled();
    expect(mockDesktopApi.prepareEnvironment).not.toHaveBeenCalled();

    await user.click(screen.getByRole('button', { name: 'Preview changes' }));
    expect(
      await screen.findByText('Preview of the configuration change'),
    ).toBeInTheDocument();
    expect(screen.getByText('Resolved configurations')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: /Import 2 entities/ }));

    expect(
      await screen.findByText('Import the selected repositories?'),
    ).toBeInTheDocument();
    expect(mockDesktopApi.commitBulkImport).not.toHaveBeenCalled();

    await user.click(screen.getByRole('button', { name: 'Import' }));

    await waitFor(() =>
      expect(mockDesktopApi.commitBulkImport).toHaveBeenCalledWith(
        expect.objectContaining({
          workspace_id: 'demo',
          bulk: {},
          port_base: 8080,
        }),
        'rev-1',
        false,
      ),
    );

    expect(await screen.findByText('Import committed')).toBeInTheDocument();
    const commit = mockDesktopApi.commitBulkImport.mock.calls[0][0];
    expect(commit.repositories).toHaveLength(2);

    await user.click(screen.getByRole('button', { name: 'Open services' }));
    expect(
      await screen.findByRole('heading', { name: 'Services', level: 1 }),
    ).toBeInTheDocument();
  });

  it('refuses to continue with an empty selection', async () => {
    const user = await openBulkImport();
    await scanWorkspace(user);

    await user.click(screen.getByRole('button', { name: 'Select none' }));

    expect(screen.getByText('2 visible · 0 selected')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled();
    expect(screen.getByText(/Select at least one repository/)).toBeInTheDocument();
  });

  it('reports an invalid port locally and blocks validation', async () => {
    const user = await openBulkImport();
    await scanWorkspace(user);

    await next(user); // 4
    await next(user); // 5
    await next(user); // 6
    await next(user); // 7

    const ports = screen.getByLabelText('Ports for backend/orders-api');
    await user.type(ports, 'eighty');

    expect(screen.getByRole('alert')).toHaveTextContent(/invalid port/);
    expect(screen.getByRole('button', { name: 'Validate import' })).toBeDisabled();
    expect(mockDesktopApi.planBulkImport).not.toHaveBeenCalled();
  });

  it('shows the discovery roots and bounds instead of scanning implicitly', async () => {
    const user = await openBulkImport();

    expect(mockDesktopApi.discoverRepositories).not.toHaveBeenCalled();

    const bounds = screen.getByText('Maximum depth').closest('div');
    expect(bounds).not.toBeNull();
    expect(within(bounds as HTMLElement).getByText('6')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Scan workspace' }));
    await screen.findByText('Discovery complete');

    expect(mockDesktopApi.discoverRepositories).toHaveBeenCalledWith({
      workspace_id: 'demo',
    });
  });

  it('forwards destructive intent only after the user chooses to replace', async () => {
    mockDesktopApi.discoverRepositories.mockResolvedValue(
      makeDiscoveryReport({
        repositories: [
          makeDiscoveredRepository({
            existing: [{ kind: 'service', id: 'orders-api' }],
          }),
        ],
      }),
    );
    mockDesktopApi.planBulkImport.mockResolvedValue(
      makeBulkImportPlan({
        rows: [
          makeBulkImportRow({
            action: { action: 'update', existing: 'orders-api', kind: 'service' },
            template_source: 'explicit',
          }),
        ],
        creates: 0,
        updates: 1,
      }),
    );

    const user = await openBulkImport();
    await user.click(screen.getByRole('button', { name: 'Scan workspace' }));
    await screen.findByText('Discovery complete');
    await user.click(screen.getByRole('button', { name: 'Select repositories' }));

    await next(user); // 4
    await next(user); // 5
    await next(user); // 6
    await next(user); // 7

    const policy = screen.getByLabelText('Existing entity policy for backend/orders-api');
    // The default never replaces hand-written configuration.
    expect(policy).toHaveValue('skip');
    await user.selectOptions(policy, 'update');

    await user.click(screen.getByRole('button', { name: 'Validate import' }));
    await screen.findByText('Validated import plan');
    await user.click(screen.getByRole('button', { name: 'Preview changes' }));

    await user.click(screen.getByRole('button', { name: /Import 1 entity/ }));
    expect(
      await screen.findByText('Replace existing configuration?'),
    ).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Replace and import' }));

    await waitFor(() =>
      expect(mockDesktopApi.commitBulkImport).toHaveBeenCalledWith(
        expect.objectContaining({
          repositories: [expect.objectContaining({ existing: 'update' })],
        }),
        'rev-1',
        true,
      ),
    );
  });
});
