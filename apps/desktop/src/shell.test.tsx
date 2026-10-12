import { act, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { flush, happyPathDefaults, renderApp } from './test/harness';
import { emitConnection, mockDesktopApi, resetMockDesktopApi } from './test/mocks';
import {
  makeConnected,
  makeDisconnected,
  makeService,
  makeServices,
  makeWorkspace,
} from './test/fixtures';

vi.mock('./services/desktopApi', async () => ({
  desktopApi: (await import('./test/mocks')).mockDesktopApi,
}));

beforeEach(() => {
  resetMockDesktopApi();
  happyPathDefaults();
});

describe('application shell', () => {
  it('renders the sidebar, header, workspace selector, and connection state', async () => {
    await renderApp();

    for (const label of [
      'Dashboard',
      'Services',
      'Git',
      'Builds',
      'Libraries',
      'Migrations',
      'Tasks',
      'Logs',
      'Health',
      'Settings',
    ]) {
      expect(screen.getByRole('button', { name: label })).toBeInTheDocument();
    }

    expect(screen.getByRole('button', { name: 'Dashboard' })).toHaveAttribute(
      'aria-current',
      'page',
    );
    expect(screen.getByRole('heading', { name: 'Dashboard', level: 1 })).toBeInTheDocument();
    expect(screen.getByLabelText('Active workspace')).toHaveValue('demo');
    expect(await screen.findByText('connected')).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('navigates between screens and marks the active page', async () => {
    const user = userEvent.setup();
    await renderApp();

    await user.click(screen.getByRole('button', { name: 'Services' }));
    expect(screen.getByRole('heading', { name: 'Services', level: 1 })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Services' })).toHaveAttribute(
      'aria-current',
      'page',
    );
    // Real rows from the mocked daemon response.
    await screen.findByRole('table');
    expect(screen.getAllByRole('row')).toHaveLength(3);

    await user.click(screen.getByRole('button', { name: 'Dashboard' }));
    expect(screen.getByRole('heading', { name: 'Dashboard', level: 1 })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Dashboard' })).toHaveAttribute(
      'aria-current',
      'page',
    );
  });

  it('shows an honest placeholder for unimplemented screens', async () => {
    const user = userEvent.setup();
    await renderApp();

    await user.click(screen.getByRole('button', { name: 'Git' }));
    expect(screen.getByText('Not implemented')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Git', level: 2 })).toBeInTheDocument();
    expect(screen.getByText(/intentionally not built yet/i)).toBeInTheDocument();
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
  });

  it('scopes services to the selected workspace', async () => {
    mockDesktopApi.listWorkspaces.mockResolvedValue([
      makeWorkspace(),
      makeWorkspace({ id: 'second', name: 'Second Platform', service_count: 1 }),
    ]);
    mockDesktopApi.listServices.mockImplementation(async (workspaceId) =>
      workspaceId === 'demo'
        ? Promise.resolve(makeServices())
        : Promise.resolve([makeService({ id: 'solo-api', display_name: 'Solo API' })]),
    );

    const user = userEvent.setup();
    await renderApp();
    await user.click(screen.getByRole('button', { name: 'Services' }));
    await screen.findByRole('table');

    await user.selectOptions(screen.getByLabelText('Active workspace'), 'second');
    await screen.findByText('Solo API');
    // Only the second workspace's single service remains.
    await waitFor(() => expect(screen.getAllByRole('row')).toHaveLength(2));
    expect(mockDesktopApi.listServices).toHaveBeenCalledWith('second');
  });

  it('retains the workspace selection while navigating', async () => {
    mockDesktopApi.listWorkspaces.mockResolvedValue([
      makeWorkspace(),
      makeWorkspace({ id: 'second', name: 'Second Platform', service_count: 1 }),
    ]);

    const user = userEvent.setup();
    await renderApp();
    await user.selectOptions(screen.getByLabelText('Active workspace'), 'second');

    await user.click(screen.getByRole('button', { name: 'Git' }));
    await user.click(screen.getByRole('button', { name: 'Dashboard' }));
    expect(screen.getByLabelText('Active workspace')).toHaveValue('second');
  });
});

describe('connection status', () => {
  it('shows an actionable banner when the daemon connection is lost', async () => {
    const user = userEvent.setup();
    await renderApp();

    act(() => emitConnection(makeDisconnected('the daemon is not running')));

    expect(await screen.findByRole('alert')).toHaveTextContent('the daemon is not running');
    expect(screen.getByText('disconnected')).toBeInTheDocument();

    const retry = screen.getByRole('button', { name: 'Retry connection' });
    await user.click(retry);
    await waitFor(() => expect(mockDesktopApi.getDaemonStatus).toHaveBeenCalled());
  });

  it('refreshes workspace data after a reconnect', async () => {
    await renderApp();
    await waitFor(() => expect(mockDesktopApi.listWorkspaces).toHaveBeenCalledTimes(1));

    act(() => emitConnection(makeDisconnected('connection lost')));
    await flush();
    act(() => emitConnection(makeConnected()));
    await flush();

    await waitFor(() => expect(mockDesktopApi.listWorkspaces).toHaveBeenCalledTimes(2));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });
});
