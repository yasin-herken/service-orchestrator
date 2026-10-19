import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { deferred, flush, happyPathDefaults, renderApp } from '../test/harness';
import { emitConnection, emitTaskEvent, mockDesktopApi, resetMockDesktopApi } from '../test/mocks';
import {
  makeDaemonStatus,
  makeDisconnected,
  makeService,
  makeServices,
  makeWorkspace,
} from '../test/fixtures';
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

/** The checkbox that selects one service row. */
function rowCheckbox(name: string): HTMLInputElement {
  return screen.getByRole('checkbox', { name: `Select ${name}` }) as HTMLInputElement;
}

/** A dialog by its accessible title. */
function dialog(name: string | RegExp): HTMLElement {
  return screen.getByRole('dialog', { name });
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
    const user = userEvent.setup();
    await renderApp();
    const baselineCalls = mockDesktopApi.listServices.mock.calls.length;
    // The default Control Center consumes the initial service listing; the
    // Services screen's own request is the next one.
    mockDesktopApi.listServices.mockRejectedValueOnce({
      code: 'DAEMON_UNAVAILABLE',
      message: 'could not reach the daemon at /tmp/daemon.sock',
    });

    await user.click(screen.getByRole('button', { name: 'Services' }));

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('could not reach the daemon at /tmp/daemon.sock');
    expect(alert).toHaveTextContent('DAEMON_UNAVAILABLE');

    await user.click(within(alert).getByRole('button', { name: 'Retry' }));
    await screen.findByRole('table');
    expect(mockDesktopApi.listServices).toHaveBeenCalledTimes(baselineCalls + 2);
  });

  it('filters services by search text', async () => {
    const user = await openServices();

    await user.type(screen.getByRole('searchbox', { name: 'Search services' }), 'admin');
    await waitFor(() => expect(dataRows()).toHaveLength(1));
    expect(dataRows()[0]).toHaveTextContent('admin-ui');

    expect(screen.getByText('1 service of 2')).toBeInTheDocument();
  });

  it('filters services by type and by runtime state', async () => {
    const user = await openServices();

    await user.selectOptions(screen.getByLabelText('Filter by service type'), 'frontend');
    await waitFor(() => expect(dataRows()).toHaveLength(1));
    expect(dataRows()[0]).toHaveTextContent('frontend');

    await user.selectOptions(screen.getByLabelText('Filter by service type'), 'all');
    await user.selectOptions(screen.getByLabelText('Filter by runtime state'), 'running');
    await waitFor(() => expect(dataRows()).toHaveLength(1));
    expect(dataRows()[0]).toHaveTextContent('orders-api');
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

    expect(dataRows()[0]).toHaveTextContent('admin-ui');

    await user.click(screen.getByRole('button', { name: /^Port/ }));
    await waitFor(() => expect(dataRows()[0]).toHaveTextContent('admin-ui'));
    expect(screen.getByRole('columnheader', { name: /Port/ })).toHaveAttribute(
      'aria-sort',
      'ascending',
    );

    await user.click(screen.getByRole('button', { name: /^Port/ }));
    await waitFor(() => expect(dataRows()[0]).toHaveTextContent('orders-api'));
    expect(screen.getByRole('columnheader', { name: /Port/ })).toHaveAttribute(
      'aria-sort',
      'descending',
    );
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

describe('selection', () => {
  it('selects and deselects a single service by checkbox', async () => {
    const user = await openServices();

    expect(screen.getByText('0')).toBeInTheDocument();
    await user.click(rowCheckbox('orders-api'));
    expect(rowCheckbox('orders-api')).toBeChecked();
    expect(screen.getByRole('toolbar', { name: 'Service selection actions' })).toHaveTextContent(
      '1 selected',
    );

    await user.click(rowCheckbox('orders-api'));
    expect(rowCheckbox('orders-api')).not.toBeChecked();
  });

  it('selects all visible services and clears them', async () => {
    const user = await openServices();

    await user.click(screen.getByRole('checkbox', { name: 'Select all visible services' }));
    expect(rowCheckbox('orders-api')).toBeChecked();
    expect(rowCheckbox('admin-ui')).toBeChecked();

    await user.click(screen.getByRole('button', { name: 'Clear' }));
    expect(rowCheckbox('orders-api')).not.toBeChecked();
    expect(rowCheckbox('admin-ui')).not.toBeChecked();
  });

  it('selects services by type', async () => {
    const user = await openServices();

    await user.selectOptions(screen.getByLabelText('Select services by type'), 'frontend');
    expect(rowCheckbox('admin-ui')).toBeChecked();
    expect(rowCheckbox('orders-api')).not.toBeChecked();
  });

  it('keeps the selection stable when filtering hides rows', async () => {
    const user = await openServices();

    await user.click(rowCheckbox('orders-api'));
    await user.type(screen.getByRole('searchbox', { name: 'Search services' }), 'admin');
    await waitFor(() => expect(dataRows()).toHaveLength(1));
    // orders-api is filtered out but stays selected.
    expect(
      screen.getByRole('toolbar', { name: 'Service selection actions' }),
    ).toHaveTextContent('1 selected');

    await user.click(screen.getAllByRole('button', { name: 'Clear filters' })[0]);
    await waitFor(() => expect(dataRows()).toHaveLength(2));
    expect(rowCheckbox('orders-api')).toBeChecked();
  });
});

describe('single actions', () => {
  it('starts one selected service without confirmation and shows progress', async () => {
    const user = await openServices();

    await user.click(rowCheckbox('orders-api'));
    await user.click(screen.getByRole('button', { name: 'Start selected' }));

    await waitFor(() =>
      expect(mockDesktopApi.startService).toHaveBeenCalledWith('orders-api'),
    );
    const progress = await screen.findByRole('dialog', { name: /Start — orders-api/ });
    expect(progress).toHaveTextContent('task-1');
  });

  it('confirms before stopping and then calls the daemon', async () => {
    const user = await openServices();

    await user.click(rowCheckbox('orders-api'));
    await user.click(screen.getByRole('button', { name: 'Stop selected' }));

    const confirm = dialog('Stop service');
    expect(mockDesktopApi.stopService).not.toHaveBeenCalled();

    await user.click(within(confirm).getByRole('button', { name: 'Stop service' }));
    await waitFor(() =>
      expect(mockDesktopApi.stopService).toHaveBeenCalledWith('orders-api'),
    );
  });

  it('confirms before restarting and then calls the daemon', async () => {
    const user = await openServices();

    await user.click(rowCheckbox('orders-api'));
    await user.click(screen.getByRole('button', { name: 'Restart selected' }));

    const confirm = dialog('Restart service');
    await user.click(within(confirm).getByRole('button', { name: 'Restart service' }));
    await waitFor(() =>
      expect(mockDesktopApi.restartService).toHaveBeenCalledWith('orders-api'),
    );
  });

  it('cancels a confirmation without calling the daemon', async () => {
    const user = await openServices();

    await user.click(rowCheckbox('orders-api'));
    await user.click(screen.getByRole('button', { name: 'Stop selected' }));
    await user.click(within(dialog('Stop service')).getByRole('button', { name: 'Cancel' }));

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(mockDesktopApi.stopService).not.toHaveBeenCalled();
  });

  it('surfaces a structured failure in an error dialog', async () => {
    mockDesktopApi.startService.mockRejectedValueOnce({
      code: 'RUNTIME_PORT_ALREADY_IN_USE',
      message: 'port 8081 is already in use',
    });

    const user = await openServices();
    await user.click(rowCheckbox('orders-api'));
    await user.click(screen.getByRole('button', { name: 'Start selected' }));

    const error = await screen.findByRole('dialog', { name: 'Unable to start service' });
    expect(error).toHaveTextContent('port 8081 is already in use');
    expect(error).toHaveTextContent('RUNTIME_PORT_ALREADY_IN_USE');

    await user.click(within(error).getByRole('button', { name: 'Close' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});

describe('bulk actions', () => {
  it('starts the explicit selection as one workflow after confirmation', async () => {
    const user = await openServices();

    await user.click(screen.getByRole('checkbox', { name: 'Select all visible services' }));
    await user.click(screen.getByRole('button', { name: 'Start selected' }));

    const confirm = dialog('Start 2 services');
    expect(confirm).toHaveTextContent('orders-api');
    expect(confirm).toHaveTextContent('admin-ui');
    await user.click(within(confirm).getByRole('button', { name: 'Start 2 services' }));

    await waitFor(() =>
      expect(mockDesktopApi.startServices).toHaveBeenCalledWith('demo', [
        'admin-ui',
        'orders-api',
      ]),
    );
  });

  it('reports real per-service workflow progress from task events', async () => {
    const user = await openServices();

    await user.click(screen.getByRole('checkbox', { name: 'Select all visible services' }));
    await user.click(screen.getByRole('button', { name: 'Start selected' }));
    await user.click(
      within(dialog('Start 2 services')).getByRole('button', { name: 'Start 2 services' }),
    );

    const progress = await screen.findByRole('dialog', { name: /Start — 2 services/ });
    expect(progress).toHaveTextContent('Waiting for the daemon');

    act(() => {
      emitTaskEvent({
        TaskCreated: {
          task: {
            id: 't1',
            kind: 'start_service',
            status: 'running',
            service: 'orders-api',
            workflow: 'wf-1',
            progress: { current: 0, total: null, message: null },
          },
        },
      });
      emitTaskEvent({
        TaskCreated: {
          task: {
            id: 't2',
            kind: 'start_service',
            status: 'succeeded',
            service: 'admin-ui',
            workflow: 'wf-1',
            progress: { current: 1, total: 1, message: null },
          },
        },
      });
    });
    await flush();

    const updated = dialog(/Start — 2 services/);
    expect(within(updated).getByText('orders-api')).toBeInTheDocument();
    expect(within(updated).getByText('admin-ui')).toBeInTheDocument();
    expect(updated).toHaveTextContent('Succeeded');
    expect(updated).toHaveTextContent('Running');
  });
});

describe('service detail', () => {
  it('opens a detail panel with real configuration and observed state', async () => {
    const user = await openServices();

    await user.click(dataRows()[1]);
    await waitFor(() =>
      expect(mockDesktopApi.getServiceDetail).toHaveBeenCalledWith('demo', 'orders-api'),
    );

    const panel = await screen.findByRole('region', { name: 'Service detail' });
    expect(panel).toHaveTextContent('orders-api');
    expect(panel).toHaveTextContent('java 21');
    expect(panel).toHaveTextContent('maven');
    expect(panel).toHaveTextContent('http://localhost:8081/actuator/health');
    expect(panel).toHaveTextContent('Not inspected yet');
  });

  it('runs a health check from the detail actions', async () => {
    const user = await openServices();

    await user.click(dataRows()[1]);
    const panel = await screen.findByRole('region', { name: 'Service detail' });
    await user.click(within(panel).getByRole('button', { name: 'Check health' }));

    await waitFor(() =>
      expect(mockDesktopApi.checkServiceHealth).toHaveBeenCalledWith('orders-api'),
    );
  });
});

describe('realtime updates', () => {
  it('marks a service busy and refreshes state when tasks settle', async () => {
    await openServices();
    const callsBefore = mockDesktopApi.listServices.mock.calls.length;

    act(() => {
      emitTaskEvent({
        TaskCreated: {
          task: {
            id: 't1',
            kind: 'start_service',
            status: 'running',
            service: 'orders-api',
            workflow: null,
            progress: { current: 0, total: null, message: null },
          },
        },
      });
    });
    await flush();

    expect(screen.getByText('Working…')).toBeInTheDocument();

    act(() => emitTaskEvent({ TaskSucceeded: { id: 't1' } }));
    await flush();

    await waitFor(() =>
      expect(mockDesktopApi.listServices.mock.calls.length).toBeGreaterThan(callsBefore),
    );
  });
});

describe('workspace and connection', () => {
  it('scopes services and clears selection when switching workspaces', async () => {
    mockDesktopApi.listWorkspaces.mockResolvedValue([
      makeWorkspace(),
      makeWorkspace({ id: 'second', name: 'Second Platform', service_count: 1 }),
    ]);
    mockDesktopApi.listServices.mockImplementation(async (workspaceId) =>
      workspaceId === 'demo'
        ? Promise.resolve(makeServices())
        : Promise.resolve([makeService({ id: 'solo-api', display_name: 'Solo API' })]),
    );

    const user = await openServices();
    await user.click(rowCheckbox('orders-api'));

    await user.selectOptions(screen.getByLabelText('Active workspace'), 'second');
    await screen.findByText('Solo API');

    // Selection is scoped to the previous workspace and cleared.
    expect(
      screen.getByRole('toolbar', { name: 'Service selection actions' }),
    ).toHaveTextContent('0 selected');
    expect(mockDesktopApi.listServices).toHaveBeenCalledWith('second');
  });

  it('disables mutating actions while disconnected and keeps loaded state', async () => {
    const user = await openServices();
    await user.click(rowCheckbox('orders-api'));

    act(() => emitConnection(makeDisconnected('the daemon is not running')));
    await flush();

    expect(await screen.findByRole('status')).toHaveTextContent('Disconnected from the daemon');
    expect(screen.getByRole('button', { name: 'Start selected' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Stop selected' })).toBeDisabled();
    // Loaded rows stay on screen.
    expect(rowCheckbox('orders-api')).toBeInTheDocument();

    // Reconnecting flips the connection back and re-enables the actions.
    act(() => emitConnection({ state: 'connected', message: null, since_unix: 2 }));
    await flush();
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Start selected' })).not.toBeDisabled(),
    );
  });

  it('does not request daemon data while disconnected before any workspace', async () => {
    mockDesktopApi.getConnectionState.mockResolvedValue(
      makeDisconnected('the daemon is not running'),
    );
    mockDesktopApi.getDaemonStatus.mockResolvedValue(makeDaemonStatus());

    await renderApp();
    await userEvent.setup().click(screen.getByRole('button', { name: 'Services' }));

    expect(screen.getByText('Waiting for the daemon connection…')).toBeInTheDocument();
    expect(mockDesktopApi.listServices).not.toHaveBeenCalled();
  });
});
