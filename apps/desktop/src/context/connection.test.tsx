import { act, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { deferred, flush, happyPathDefaults, renderApp } from '../test/harness';
import { emitConnection, mockDesktopApi, resetMockDesktopApi } from '../test/mocks';
import { makeConnected, makeDisconnected } from '../test/fixtures';
import type { ConnectionSnapshot, DaemonStatus } from '../types/models';

vi.mock('../services/desktopApi', async () => ({
  desktopApi: (await import('../test/mocks')).mockDesktopApi,
}));

beforeEach(() => {
  resetMockDesktopApi();
  happyPathDefaults();
});

describe('connection context', () => {
  it('reports connecting until the adapter answers', async () => {
    const pending = deferred<ConnectionSnapshot>();
    mockDesktopApi.getConnectionState.mockReturnValue(pending.promise);

    await renderApp();
    expect(screen.getByText('connecting')).toBeInTheDocument();
    expect(screen.getByText('Waiting for the daemon connection…')).toBeInTheDocument();
    // No daemon-dependent data is requested before a connection exists.
    expect(mockDesktopApi.listWorkspaces).not.toHaveBeenCalled();

    pending.resolve(makeConnected());
    await flush();

    expect(screen.getByText('connected')).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    await waitFor(() => expect(mockDesktopApi.listWorkspaces).toHaveBeenCalled());
  });

  it('prefers a pushed transition over a stale initial pull', async () => {
    const pending = deferred<ConnectionSnapshot>();
    mockDesktopApi.getConnectionState.mockReturnValue(pending.promise);

    await renderApp();

    // A transition arrives while the pull is still in flight…
    act(() => emitConnection(makeConnected()));
    expect(screen.getByText('connected')).toBeInTheDocument();

    // …so the older pull result must not downgrade the state.
    pending.resolve({ state: 'connecting', message: null, since_unix: 1 });
    await flush();

    expect(screen.getByText('connected')).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('reconnects on demand and clears the banner when the adapter confirms', async () => {
    const user = userEvent.setup();
    await renderApp();

    act(() => emitConnection(makeDisconnected('socket gone')));
    expect(await screen.findByRole('alert')).toHaveTextContent('socket gone');

    await user.click(screen.getByRole('button', { name: 'Retry connection' }));
    await waitFor(() => expect(mockDesktopApi.getDaemonStatus).toHaveBeenCalled());

    // In the real app the adapter pushes this transition after connecting.
    act(() => emitConnection(makeConnected()));
    await flush();

    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(screen.getByText('connected')).toBeInTheDocument();
  });

  it('disables the retry button while a reconnection attempt runs', async () => {
    const user = userEvent.setup();
    const pending = deferred<DaemonStatus>();
    mockDesktopApi.getDaemonStatus.mockImplementation(() => pending.promise);

    await renderApp();
    act(() => emitConnection(makeDisconnected('socket gone')));

    await user.click(screen.getByRole('button', { name: 'Retry connection' }));

    const running = screen.getByRole('button', { name: 'Connecting…' });
    expect(running).toBeDisabled();

    pending.reject({ code: 'DAEMON_UNAVAILABLE', message: 'still gone' });
    await flush();
    expect(screen.getByRole('button', { name: 'Retry connection' })).not.toBeDisabled();
  });
});
