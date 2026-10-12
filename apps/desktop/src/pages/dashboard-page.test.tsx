import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { flush, happyPathDefaults, renderApp } from '../test/harness';
import { emitTaskEvent, mockDesktopApi, resetMockDesktopApi } from '../test/mocks';
import {
  makeDisconnected,
  makeService,
  makeWorkspaceStatus,
} from '../test/fixtures';

vi.mock('../services/desktopApi', async () => ({
  desktopApi: (await import('../test/mocks')).mockDesktopApi,
}));

beforeEach(() => {
  resetMockDesktopApi();
  happyPathDefaults();
});

/** Returns the dashboard stat card for a label. */
function statCard(container: HTMLElement, label: string): HTMLElement {
  const card = container.querySelector(`[data-stat="${label}"]`);
  if (!card) {
    throw new Error(`stat card not found: ${label}`);
  }
  return card as HTMLElement;
}

describe('dashboard', () => {
  it('derives counts from real workspace status data', async () => {
    const view = await renderApp();

    expect(statCard(view.container, 'Active workspace')).toHaveTextContent('Demo Platform');
    expect(statCard(view.container, 'Active workspace')).toHaveTextContent(
      '/Users/dev/Projects/demo',
    );
    expect(statCard(view.container, 'Total services')).toHaveTextContent('2');
    expect(statCard(view.container, 'Running')).toHaveTextContent('1');
    expect(statCard(view.container, 'Stopped')).toHaveTextContent('1');
    expect(statCard(view.container, 'Unhealthy')).toHaveTextContent('0');
  });

  it('shows real daemon status details', async () => {
    const view = await renderApp();

    expect(view.container).toHaveTextContent('4242');
    expect(view.container).toHaveTextContent('v1');
    expect(view.container).toHaveTextContent(
      '/Users/dev/.service-orchestrator/run/daemon.sock',
    );
    // Daemon card values come from the mocked status (1 client, 1 managed
    // service, 0 active tasks).
    const daemonCard = screen.getByText('Daemon').closest('.card') as HTMLElement;
    expect(within(daemonCard).getByText('Clients')).toBeInTheDocument();
    expect(within(daemonCard).getByText('Clients').parentElement).toHaveTextContent('1');
    expect(within(daemonCard).getByText('Managed services').parentElement).toHaveTextContent(
      '1',
    );
  });

  it('counts unhealthy services from the workspace status', async () => {
    mockDesktopApi.getWorkspaceStatus.mockResolvedValue(
      makeWorkspaceStatus([
        makeService({
          id: 'sick',
          display_name: 'Sick API',
          health_state: 'unhealthy',
          process_state: 'failed',
        }),
        makeService({ id: 'fine', display_name: 'Fine API' }),
      ]),
    );

    const view = await renderApp();
    await waitFor(() =>
      expect(statCard(view.container, 'Unhealthy')).toHaveTextContent('1'),
    );
    expect(statCard(view.container, 'Running')).toHaveTextContent('1');
  });

  it('reports workspace-status failures with an actionable retry', async () => {
    mockDesktopApi.getWorkspaceStatus.mockRejectedValueOnce({
      code: 'WorkspaceNotFound',
      message: "workspace 'demo' does not exist",
    });

    const user = userEvent.setup();
    await renderApp();

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent("workspace 'demo' does not exist");
    expect(alert).toHaveTextContent('WorkspaceNotFound');

    await user.click(within(alert).getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(mockDesktopApi.getWorkspaceStatus).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByRole('alert')).not.toBeInTheDocument());
  });

  it('surfaces daemon status failures with their code', async () => {
    mockDesktopApi.getDaemonStatus.mockRejectedValueOnce({
      code: 'DAEMON_UNAVAILABLE',
      message: 'the daemon is not running',
    });

    const user = userEvent.setup();
    await renderApp();

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('the daemon is not running');
    expect(alert).toHaveTextContent('DAEMON_UNAVAILABLE');

    await user.click(within(alert).getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(mockDesktopApi.getDaemonStatus).toHaveBeenCalledTimes(2));
  });

  it('shows an empty state when no workspace is configured', async () => {
    mockDesktopApi.listWorkspaces.mockResolvedValue([]);

    await renderApp();
    expect(await screen.findByText('No workspaces configured')).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('waits instead of showing stale numbers while disconnected', async () => {
    mockDesktopApi.getConnectionState.mockResolvedValue(
      makeDisconnected('the daemon is not running'),
    );

    await renderApp();
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'the daemon is not running',
    );
    expect(screen.getByText('Waiting for the daemon connection…')).toBeInTheDocument();
    // No fake data while the daemon is unreachable.
    expect(mockDesktopApi.getWorkspaceStatus).not.toHaveBeenCalled();
    expect(mockDesktopApi.getDaemonStatus).not.toHaveBeenCalled();
  });

  it('renders live task activity forwarded from the daemon', async () => {
    await renderApp();

    act(() => emitTaskEvent({ TaskStarted: { id: 'task-1', kind: 'start_service' } }));
    await flush();

    expect(screen.getByText('TaskStarted')).toBeInTheDocument();
    expect(screen.getByText('Task started (start_service)')).toBeInTheDocument();
  });

  it('renders failure details in the activity feed', async () => {
    await renderApp();

    act(() =>
      emitTaskEvent({
        TaskFailed: { id: 'task-1', failure: { code: 'MAVEN_BUILD_FAILED', message: 'exit 1' } },
      }),
    );
    await flush();

    expect(screen.getByText('Task failed: exit 1')).toBeInTheDocument();
  });
});
