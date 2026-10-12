import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { deferred, flush, happyPathDefaults, renderApp } from '../test/harness';
import { mockDesktopApi, resetMockDesktopApi } from '../test/mocks';
import { makeServices } from '../test/fixtures';
import type { ServiceSummary } from '../types/models';

vi.mock('../services/desktopApi', async () => ({
  desktopApi: (await import('../test/mocks')).mockDesktopApi,
}));

beforeEach(() => {
  resetMockDesktopApi();
  happyPathDefaults();
});

/** Opens the Services screen. */
async function openServices() {
  const user = userEvent.setup();
  await renderApp();
  await user.click(screen.getByRole('button', { name: 'Services' }));
  await screen.findByRole('table');
  return user;
}

/** The data rows of the services table (header excluded). */
function dataRows(): HTMLElement[] {
  return screen.getAllByRole('row').slice(1);
}

describe('services list', () => {
  it('renders service rows with state, health, branch, and port', async () => {
    await openServices();

    const rows = dataRows();
    expect(rows).toHaveLength(2);
    const first = rows[0];
    expect(first).toHaveTextContent('admin-ui');
    within(first).getByText('frontend');
    expect(first).toHaveTextContent('Demo Platform');
    expect(first).toHaveTextContent('not running');
    within(first).getByText('main');
    within(first).getByText('3000');
  });

  it('shows a loading state while the request is in flight', async () => {
    const pending = deferred<ServiceSummary[]>();
    mockDesktopApi.listServices.mockReturnValue(pending.promise);

    const user = userEvent.setup();
    await renderApp();
    await user.click(screen.getByRole('button', { name: 'Services' }));

    expect(screen.getByRole('status')).toHaveTextContent('Loading services…');
    expect(screen.queryByRole('table')).not.toBeInTheDocument();

    pending.resolve(makeServices());
    await flush();
    await screen.findByRole('table');
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('shows an actionable error and retries on demand', async () => {
    mockDesktopApi.listServices.mockRejectedValueOnce({
      code: 'DAEMON_UNAVAILABLE',
      message: 'could not reach the daemon at /tmp/daemon.sock',
    });

    const user = userEvent.setup();
    await renderApp();
    await user.click(screen.getByRole('button', { name: 'Services' }));

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('could not reach the daemon at /tmp/daemon.sock');
    expect(alert).toHaveTextContent('DAEMON_UNAVAILABLE');

    await user.click(within(alert).getByRole('button', { name: 'Retry' }));
    await screen.findByRole('table');
    expect(mockDesktopApi.listServices).toHaveBeenCalledTimes(2);
  });

  it('filters services by search text', async () => {
    const user = await openServices();

    await user.type(screen.getByRole('searchbox', { name: 'Search services' }), 'admin');
    await waitFor(() => expect(dataRows()).toHaveLength(1));
    expect(dataRows()[0]).toHaveTextContent('admin-ui');

    expect(screen.getByText('1 service of 2')).toBeInTheDocument();
  });

  it('filters services by type', async () => {
    const user = await openServices();

    await user.selectOptions(screen.getByLabelText('Filter by service type'), 'frontend');
    await waitFor(() => expect(dataRows()).toHaveLength(1));
    expect(dataRows()[0]).toHaveTextContent('frontend');
  });

  it('shows a clearable empty state when filters match nothing', async () => {
    const user = await openServices();

    await user.type(screen.getByRole('searchbox', { name: 'Search services' }), 'zzz');
    await screen.findByText('No services match your filters');

    const clearButtons = screen.getAllByRole('button', { name: 'Clear filters' });
    expect(clearButtons.length).toBeGreaterThan(0);
    await user.click(clearButtons[0]);
    await waitFor(() => expect(dataRows()).toHaveLength(2));
  });

  it('sorts by column, toggling direction', async () => {
    const user = await openServices();

    // Default: name ascending.
    expect(dataRows()[0]).toHaveTextContent('admin-ui');

    await user.click(screen.getByRole('button', { name: /^Port/ }));
    await waitFor(() => expect(dataRows()[0]).toHaveTextContent('admin-ui'));
    expect(
      screen.getByRole('columnheader', { name: /Port/ }),
    ).toHaveAttribute('aria-sort', 'ascending');

    await user.click(screen.getByRole('button', { name: /^Port/ }));
    await waitFor(() => expect(dataRows()[0]).toHaveTextContent('orders-api'));
    expect(
      screen.getByRole('columnheader', { name: /Port/ }),
    ).toHaveAttribute('aria-sort', 'descending');
  });

  it('selects a row by mouse and by keyboard', async () => {
    const user = await openServices();

    const target = dataRows()[1];
    await user.click(target);
    expect(target).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByText(/Selected:/)).toHaveTextContent('orders-api');

    const other = dataRows()[0];
    other.focus();
    await user.keyboard('{Enter}');
    expect(other).toHaveAttribute('aria-selected', 'true');
    expect(target).toHaveAttribute('aria-selected', 'false');
    expect(screen.getByText(/Selected:/)).toHaveTextContent('admin-ui');
  });

  it('shows an empty state when the workspace has no services', async () => {
    mockDesktopApi.listServices.mockResolvedValue([]);

    const user = userEvent.setup();
    await renderApp();
    await user.click(screen.getByRole('button', { name: 'Services' }));

    await screen.findByText('No services configured');
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
  });
});
