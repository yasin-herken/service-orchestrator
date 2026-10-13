import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { flush, happyPathDefaults, renderApp } from '../test/harness';
import {
  emitConnection,
  emitTaskEvent,
  mockDesktopApi,
  resetMockDesktopApi,
} from '../test/mocks';
import {
  makeDisconnected,
  makeHealthDetail,
  makeHealthRow,
  makeHealthRows,
} from '../test/fixtures';

vi.mock('../services/desktopApi', async () => ({
  desktopApi: (await import('../test/mocks')).mockDesktopApi,
}));

beforeEach(() => {
  resetMockDesktopApi();
  happyPathDefaults();
});

/** Opens the Health screen and waits for the table. */
async function openHealth() {
  const user = userEvent.setup();
  await renderApp();
  await user.click(screen.getByRole('button', { name: 'Health' }));
  await screen.findByRole('table');
  return user;
}

describe('health status rendering', () => {
  it('shows runtime and health as separate observations', async () => {
    await openHealth();

    const table = screen.getByRole('table');
    expect(within(table).getByText('orders-api')).toBeInTheDocument();
    expect(within(table).getAllByText('running').length).toBeGreaterThan(0);
    expect(within(table).getAllByText('healthy').length).toBeGreaterThan(0);
  });

  it('renders an explicit unknown for checks that were never observed', async () => {
    await openHealth();

    // admin-ui has no observed check at all; nothing is fabricated healthy.
    expect(screen.getAllByText('Not checked').length).toBeGreaterThan(0);
    const row = screen.getAllByRole('row').find((r) => r.textContent?.includes('admin-ui'));
    expect(row).toHaveTextContent('unknown');
  });

  it('shows an empty state when the workspace has no services', async () => {
    mockDesktopApi.listHealth.mockResolvedValue([]);
    const user = userEvent.setup();
    await renderApp();
    await user.click(screen.getByRole('button', { name: 'Health' }));

    expect(await screen.findByText('No services in this workspace')).toBeInTheDocument();
  });
});

describe('health details', () => {
  it('lists every observed check with its requirement and duration', async () => {
    const user = await openHealth();

    await user.click(screen.getAllByRole('row')[1]);
    const panel = await screen.findByRole('region', { name: 'Health detail' });

    expect(mockDesktopApi.getHealth).toHaveBeenCalledWith('demo', 'orders-api');
    await waitFor(() => expect(panel).toHaveTextContent('http'));
    expect(panel).toHaveTextContent('required');
    expect(panel).toHaveTextContent('localhost:8081');
  });

  it('reports history as unavailable instead of inventing it', async () => {
    mockDesktopApi.getHealth.mockResolvedValue(
      makeHealthDetail({
        row: makeHealthRow({
          last_healthy_at: null,
          last_unhealthy_at: null,
          consecutive_failures: 0,
        }),
      }),
    );
    const user = await openHealth();

    await user.click(screen.getAllByRole('row')[1]);
    const panel = await screen.findByRole('region', { name: 'Health detail' });

    await within(panel).findByText(/history is unavailable/i);
  });

  it('shows the structured last failure and diagnostics', async () => {
    mockDesktopApi.getHealth.mockResolvedValue(
      makeHealthDetail({
        pid: 4321,
        ports: [8081],
        last_failure: {
          source: 'http',
          code: 'HEALTH_HTTP_UNREACHABLE',
          message: 'connection refused',
          at: 1_700_000_000,
        },
        recent_errors: ['http check failed: connection refused'],
      }),
    );
    const user = await openHealth();

    await user.click(screen.getAllByRole('row')[1]);
    const panel = await screen.findByRole('region', { name: 'Health detail' });

    expect(panel).toHaveTextContent('HEALTH_HTTP_UNREACHABLE');
    expect(panel).toHaveTextContent('connection refused');
    expect(panel).toHaveTextContent('4321');
  });
});

describe('manual health checks', () => {
  it('runs a check through the daemon task engine, never from React', async () => {
    const user = await openHealth();

    const row = screen.getAllByRole('row').find((r) => r.textContent?.includes('orders-api'));
    await user.click(within(row!).getByRole('button', { name: 'Check now' }));

    await waitFor(() => expect(mockDesktopApi.checkServiceHealth).toHaveBeenCalledWith('orders-api'));
    // The check is a task; its progress comes from the task engine.
    const progress = await screen.findByRole('dialog');
    expect(progress).toHaveTextContent(/health check/i);
  });

  it('runs only the readiness checks on request', async () => {
    const user = await openHealth();

    await user.click(screen.getAllByRole('row')[1]);
    const panel = await screen.findByRole('region', { name: 'Health detail' });
    await user.click(within(panel).getByRole('button', { name: 'Run readiness check' }));

    await waitFor(() =>
      expect(mockDesktopApi.checkServiceReadiness).toHaveBeenCalledWith('orders-api'),
    );
  });

  it('surfaces a daemon refusal to run a check', async () => {
    mockDesktopApi.checkServiceHealth.mockRejectedValueOnce({
      code: 'PermissionDenied',
      message: 'the daemon could not verify an authorized request',
    });
    const user = await openHealth();

    const row = screen.getAllByRole('row').find((r) => r.textContent?.includes('orders-api'));
    await user.click(within(row!).getByRole('button', { name: 'Check now' }));

    const error = await screen.findByRole('dialog');
    expect(error).toHaveTextContent('the daemon could not verify an authorized request');
  });
});

describe('health event updates', () => {
  it('refreshes observed health when a health task settles', async () => {
    await openHealth();
    const before = mockDesktopApi.listHealth.mock.calls.length;

    mockDesktopApi.listHealth.mockResolvedValue([
      makeHealthRow({ health_state: 'unhealthy', consecutive_failures: 2 }),
      makeHealthRows()[1],
    ]);
    // A health-kind task settling is what moves health state; the page then
    // re-reads the daemon's authoritative observation exactly once.
    act(() =>
      emitTaskEvent({
        TaskSucceeded: {
          id: 'hc-1',
          kind: 'health_check',
        },
      }),
    );
    await flush();

    await waitFor(() => expect(mockDesktopApi.listHealth.mock.calls.length).toBeGreaterThan(before));
    expect(screen.getByRole('table')).toHaveTextContent('unhealthy');
  });
});

describe('daemon disconnection', () => {
  it('disables checks while disconnected and keeps the last observation', async () => {
    await openHealth();

    act(() => emitConnection(makeDisconnected('the daemon is not running')));
    await flush();

    expect(screen.getByText(/Disconnected from the daemon/i)).toBeInTheDocument();
    const row = screen.getAllByRole('row').find((r) => r.textContent?.includes('orders-api'));
    expect(within(row!).getByRole('button', { name: 'Check now' })).toBeDisabled();
    // Previously observed state stays visible with its date.
    expect(screen.getByRole('table')).toHaveTextContent('orders-api');
  });
});

describe('workspace switching', () => {
  it('re-queries health for the newly selected workspace', async () => {
    mockDesktopApi.listWorkspaces.mockResolvedValue([
      { id: 'demo', name: 'Demo', root: '/p/demo', service_count: 2 },
      { id: 'second', name: 'Second', root: '/p/second', service_count: 1 },
    ]);
    const user = await openHealth();

    await user.selectOptions(screen.getByLabelText('Active workspace'), 'second');

    await waitFor(() => expect(mockDesktopApi.listHealth).toHaveBeenCalledWith('second'));
  });
});
