import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { flush, happyPathDefaults, renderApp } from '../test/harness';
import { emitTaskEvent, mockDesktopApi, resetMockDesktopApi } from '../test/mocks';
import { makeBuildTarget } from '../test/fixtures';

vi.mock('../services/desktopApi', async () => ({
  desktopApi: (await import('../test/mocks')).mockDesktopApi,
}));

beforeEach(() => {
  resetMockDesktopApi();
  happyPathDefaults();
});

/** Opens the Builds screen. */
async function openBuilds() {
  const user = userEvent.setup();
  await renderApp();
  await user.click(screen.getByRole('button', { name: 'Builds' }));
  await screen.findByRole('table');
  return user;
}

/** The data rows of the build-target table (header excluded). */
function dataRows(): HTMLElement[] {
  return screen.getAllByRole('row').slice(1);
}

/** Emits a build task for one service. */
function emitBuildTask(
  id: string,
  service: string,
  status: string,
  workflow: string | null = null,
) {
  emitTaskEvent({
    TaskCreated: {
      task: {
        id,
        kind: 'build_service',
        status,
        service,
        workflow,
        progress: { current: 1, total: 1, message: null },
      },
    },
  });
}

describe('build targets', () => {
  it('renders provider-agnostic targets with their configured build system', async () => {
    await openBuilds();

    const rows = dataRows();
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent('orders-api');
    expect(rows[0]).toHaveTextContent('maven');
    expect(rows[0]).toHaveTextContent('java 21');
    expect(rows[0]).toHaveTextContent('Build');
    expect(rows[1]).toHaveTextContent('npm');
    expect(rows[1]).toHaveTextContent('node 20.x');
  });

  it('shows provider capabilities with the core explanation for the rest', async () => {
    const user = await openBuilds();

    await user.click(dataRows()[1]);
    const panel = await screen.findByRole('region', { name: 'Build target detail' });

    const capabilities = within(panel).getByRole('region', { name: 'Build capabilities' });
    expect(capabilities).toHaveTextContent('Build');
    expect(capabilities).toHaveTextContent('Available');
    expect(capabilities).toHaveTextContent('Clean');
    expect(capabilities).toHaveTextContent('Unsupported');
    expect(capabilities).toHaveTextContent('Install dependencies');
    expect(capabilities).toHaveTextContent('Not exposed');
    expect(capabilities).toHaveTextContent(/not exposed by the application core/i);
  });

  it('disables the bulk build and explains when a selection is unsupported', async () => {
    mockDesktopApi.listBuildTargets.mockResolvedValue([
      makeBuildTarget(),
      makeBuildTarget({
        service_id: 'docs-site',
        display_name: 'docs-site',
        build_system: null,
        capability: null,
        capability_error: 'no build system is configured for this service',
      }),
    ]);
    const user = await openBuilds();

    await user.click(screen.getByRole('checkbox', { name: 'Select all visible services' }));
    expect(screen.getByRole('button', { name: 'Build selected' })).toBeDisabled();
    expect(screen.getByText(/expose no build operation/)).toHaveTextContent('docs-site');
  });
});

describe('build operations', () => {
  it('builds one service through the single build task', async () => {
    const user = await openBuilds();

    const firstRow = dataRows()[0];
    await user.click(within(firstRow).getByRole('button', { name: 'Build' }));

    await waitFor(() =>
      expect(mockDesktopApi.buildService).toHaveBeenCalledWith('orders-api'),
    );
    await screen.findByRole('dialog', { name: /Build — orders-api/ });
  });

  it('builds the explicit selection as one workflow', async () => {
    const user = await openBuilds();

    await user.click(screen.getByRole('checkbox', { name: 'Select all visible services' }));
    await user.click(screen.getByRole('button', { name: 'Build selected' }));

    await waitFor(() =>
      expect(mockDesktopApi.buildServices).toHaveBeenCalledWith('demo', [
        'admin-ui',
        'orders-api',
      ]),
    );
  });

  it('reports real per-service bulk build progress from task events', async () => {
    const user = await openBuilds();

    await user.click(screen.getByRole('checkbox', { name: 'Select all visible services' }));
    await user.click(screen.getByRole('button', { name: 'Build selected' }));

    const progress = await screen.findByRole('dialog', { name: /Build — 2 services/ });
    expect(progress).toHaveTextContent('Waiting for the daemon');

    act(() => {
      emitBuildTask('b1', 'orders-api', 'succeeded', 'wf-build');
      emitBuildTask('b2', 'admin-ui', 'failed', 'wf-build');
    });
    await flush();

    const updated = screen.getByRole('dialog', { name: /Build — 2 services/ });
    expect(updated).toHaveTextContent('Succeeded');
    expect(updated).toHaveTextContent('Failed');
  });

  it('surfaces a structured build failure', async () => {
    mockDesktopApi.buildService.mockRejectedValueOnce({
      code: 'BUILD_MAVEN_MISSING',
      message: 'the maven executable is unavailable',
    });
    const user = await openBuilds();

    await user.click(within(dataRows()[0]).getByRole('button', { name: 'Build' }));

    const error = await screen.findByRole('dialog', { name: 'Unable to build service' });
    expect(error).toHaveTextContent('the maven executable is unavailable');
    expect(error).toHaveTextContent('BUILD_MAVEN_MISSING');
  });
});

describe('build logs', () => {
  it('shows the real logs of the last build for a selected service', async () => {
    const user = await openBuilds();

    act(() => emitBuildTask('b1', 'orders-api', 'failed'));
    await flush();

    await user.click(dataRows()[0]);
    const logs = await screen.findByRole('region', { name: 'Build logs' });

    await within(logs).findByText('Compiling 3 source files');
    expect(logs).toHaveTextContent('BUILD FAILURE: missing dependency');
    expect(mockDesktopApi.getTaskLogs).toHaveBeenCalledWith(
      'b1',
      expect.objectContaining({ limit: 500 }),
    );
  });
});
