import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { happyPathDefaults, renderApp } from '../test/harness';
import { mockDesktopApi, resetMockDesktopApi } from '../test/mocks';
import { makeArtifact, makeLibrary } from '../test/fixtures';

vi.mock('../services/desktopApi', async () => ({
  desktopApi: (await import('../test/mocks')).mockDesktopApi,
}));

beforeEach(() => {
  resetMockDesktopApi();
  happyPathDefaults();
});

/** Opens the Libraries screen. */
async function openLibraries() {
  const user = userEvent.setup();
  await renderApp();
  await user.click(screen.getByRole('button', { name: 'Libraries' }));
  await screen.findByRole('table');
  return user;
}

/** Opens the detail panel for the single library. */
async function openDetail(user: ReturnType<typeof userEvent.setup>) {
  const row = screen.getAllByRole('row')[1];
  await user.click(row);
  return screen.findByRole('region', { name: 'Library detail' });
}

describe('library list', () => {
  it('renders libraries with artifact coordinate, version, and dependents', async () => {
    await openLibraries();

    const row = screen.getAllByRole('row')[1];
    expect(row).toHaveTextContent('shared-kit');
    expect(row).toHaveTextContent('com.demo:shared-kit:1.8.0-SNAPSHOT');
    expect(row).toHaveTextContent('1.8.0-SNAPSHOT');
  });
});

describe('library artifact details', () => {
  it('shows artifacts, local availability, and dependents', async () => {
    const user = await openLibraries();
    const panel = await openDetail(user);

    const artifacts = within(panel).getByRole('region', { name: 'Artifacts' });
    expect(artifacts).toHaveTextContent('com.demo:shared-kit:1.8.0-SNAPSHOT');
    expect(artifacts).toHaveTextContent('jar');

    const availability = within(panel).getByRole('region', {
      name: 'Local artifact availability',
    });
    await within(availability).findByText('Present');
    expect(availability).toHaveTextContent('is present');

    const dependents = within(panel).getByRole('region', { name: 'Dependents' });
    expect(dependents).toHaveTextContent('orders-api');
    expect(dependents).toHaveTextContent('required');
  });

  it('surfaces an artifact verification failure', async () => {
    mockDesktopApi.verifyLibraryArtifact.mockRejectedValueOnce({
      code: 'ARTIFACT_STORE_UNAVAILABLE',
      message: 'the local Maven repository could not be read',
    });
    const user = await openLibraries();
    const panel = await openDetail(user);

    await within(panel).findByText('the local Maven repository could not be read');
    expect(panel).toHaveTextContent('ARTIFACT_STORE_UNAVAILABLE');
  });
});

describe('version alignment', () => {
  it('distinguishes aligned, mismatch, and unresolved consumers', async () => {
    mockDesktopApi.inspectVersionAlignment.mockResolvedValue({
      library_id: 'shared-kit',
      workspace_id: 'demo',
      target: makeArtifact(),
      consumers: [
        {
          service_id: 'orders-api',
          workspace_id: 'demo',
          path: '/p/orders-api',
          current_version: '1.8.0-SNAPSHOT',
          status: 'aligned',
          detail: null,
        },
        {
          service_id: 'user-service',
          workspace_id: 'demo',
          path: '/p/user-service',
          current_version: '1.7.0-SNAPSHOT',
          status: 'mismatch',
          detail: 'resolves to 1.7.0-SNAPSHOT',
        },
        {
          service_id: 'payment-service',
          workspace_id: 'demo',
          path: null,
          current_version: null,
          status: 'unresolved',
          detail: 'dependency not found',
        },
      ],
      aligned_count: 1,
      total: 3,
      fully_aligned: false,
    });

    const user = await openLibraries();
    const panel = await openDetail(user);
    const alignmentBlock = within(panel).getByRole('region', { name: 'Alignment inspection' });

    await user.click(within(alignmentBlock).getByRole('button', { name: 'Inspect' }));

    const alignment = await within(alignmentBlock).findByRole('region', {
      name: 'Version alignment',
    });
    expect(alignment).toHaveTextContent('Aligned');
    expect(alignment).toHaveTextContent('Mismatch');
    expect(alignment).toHaveTextContent('Unresolved');
    expect(alignment).toHaveTextContent('user-service');
    expect(alignment).toHaveTextContent('1.7.0-SNAPSHOT');
    expect(alignment).toHaveTextContent(/never enforced|does not rewrite/i);
  });
});

describe('library operations', () => {
  it('builds a library through the single build task', async () => {
    const user = await openLibraries();
    const panel = await openDetail(user);

    await user.click(within(panel).getByRole('button', { name: 'Build' }));
    await waitFor(() =>
      expect(mockDesktopApi.buildLibrary).toHaveBeenCalledWith('shared-kit'),
    );
  });

  it('prepares a library with optional git sync and reports the options', async () => {
    const user = await openLibraries();
    const panel = await openDetail(user);

    await user.click(within(panel).getByRole('button', { name: 'Prepare…' }));
    const dialog = screen.getByRole('dialog', { name: 'Prepare shared-kit' });

    await user.click(
      within(dialog).getByRole('checkbox', { name: /Synchronize the repository first/i }),
    );
    await user.click(within(dialog).getByRole('button', { name: 'Prepare' }));

    await waitFor(() =>
      expect(mockDesktopApi.prepareLibrary).toHaveBeenCalledWith('shared-kit', {
        sync: true,
        branch: null,
        build: true,
        install: true,
        verify: true,
      }),
    );
  });

  it('prepares a multi-library selection as one workflow', async () => {
    mockDesktopApi.listLibraries.mockResolvedValue([
      makeLibrary(),
      makeLibrary({ id: 'shared-utils', display_name: 'shared-utils' }),
    ]);
    const user = await openLibraries();

    await user.click(screen.getByRole('checkbox', { name: 'Select all visible libraries' }));
    await user.click(screen.getByRole('button', { name: 'Prepare selected…' }));
    const dialog = screen.getByRole('dialog', { name: 'Prepare 2 libraries' });
    await user.click(within(dialog).getByRole('button', { name: 'Prepare' }));

    await waitFor(() =>
      expect(mockDesktopApi.prepareLibraries).toHaveBeenCalledWith(
        'demo',
        ['shared-kit', 'shared-utils'],
        expect.objectContaining({ build: true, install: true, verify: true, sync: false }),
      ),
    );
  });
});
