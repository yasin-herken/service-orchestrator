import { act, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { flush, happyPathDefaults, renderApp } from '../test/harness';
import { emitConnection, emitLogEvent, mockDesktopApi, resetMockDesktopApi } from '../test/mocks';
import { makeDisconnected, makeLogEntries, makeLogEntry, makeLogPage } from '../test/fixtures';
import { MAX_LOG_BUFFER } from '../components/logs/UnifiedLogViewer';

vi.mock('../services/desktopApi', async () => ({
  desktopApi: (await import('../test/mocks')).mockDesktopApi,
}));

beforeEach(() => {
  resetMockDesktopApi();
  happyPathDefaults();
});

/** Opens the Logs screen with the workspace scope selected. */
async function openLogs(options: { advanceTimers?: boolean } = {}) {
  const user = userEvent.setup(
    options.advanceTimers ? { advanceTimers: (ms) => vi.advanceTimersByTimeAsync(ms) } : {},
  );
  await renderApp();
  await user.click(screen.getByRole('button', { name: 'Logs' }));
  await screen.findByRole('log');
  return user;
}

/** Switches the viewer to the one-service scope (which subscribes). */
async function selectServiceScope(
  user: ReturnType<typeof userEvent.setup>,
  serviceId = 'orders-api',
) {
  await user.selectOptions(screen.getByLabelText('Log scope kind'), 'service');
  if (serviceId !== 'orders-api') {
    await user.selectOptions(screen.getByLabelText('Log service'), serviceId);
  }
}

/** The subscription id the viewer registered with the adapter. */
function subscriptionId(): string {
  const calls = mockDesktopApi.subscribeServiceLogs.mock.calls;
  expect(calls.length).toBeGreaterThan(0);
  return calls[calls.length - 1][0];
}

describe('log scopes', () => {
  it('queries the bounded workspace scope by default', async () => {
    await openLogs();

    expect(mockDesktopApi.getLogs).toHaveBeenCalledWith(
      expect.objectContaining({ workspaceId: 'demo', limit: 500 }),
    );
  });

  it('renders the returned entries with level, source, and message', async () => {
    await openLogs();

    const viewer = screen.getByRole('log');
    expect(viewer).toHaveTextContent('BUILD FAILURE: missing dependency');
    expect(viewer).toHaveTextContent('error');
  });

  it('scopes to one service and subscribes to its realtime stream', async () => {
    const user = await openLogs();
    await selectServiceScope(user);

    await waitFor(() =>
      expect(mockDesktopApi.subscribeServiceLogs).toHaveBeenCalledWith(
        expect.any(String),
        'orders-api',
      ),
    );
    expect(mockDesktopApi.getLogs).toHaveBeenCalledWith(
      expect.objectContaining({ serviceId: 'orders-api' }),
    );
  });

  it('detaches the subscription when the scope changes', async () => {
    const user = await openLogs();
    await selectServiceScope(user);
    await waitFor(() => expect(mockDesktopApi.subscribeServiceLogs).toHaveBeenCalled());
    const id = subscriptionId();

    await user.selectOptions(screen.getByLabelText('Log scope kind'), 'workspace');

    await waitFor(() =>
      expect(mockDesktopApi.unsubscribeServiceLogs).toHaveBeenCalledWith(id),
    );
  });
});

describe('log streaming', () => {
  it('appends realtime entries pushed by the daemon', async () => {
    await openLogs();
    await selectServiceScope(userEvent.setup());
    await waitFor(() => expect(mockDesktopApi.subscribeServiceLogs).toHaveBeenCalled());
    const id = subscriptionId();

    act(() => {
      emitLogEvent({
        subscription_id: id,
        service_id: 'orders-api',
        kind: 'entry',
        entry: makeLogEntry(42, { message: 'streamed line', level: 'warn' }),
        dropped: null,
        message: null,
      });
    });

    await screen.findByText('streamed line');
  });

  it('reports dropped entries instead of pretending the view is complete', async () => {
    await openLogs();
    await selectServiceScope(userEvent.setup());
    await waitFor(() => expect(mockDesktopApi.subscribeServiceLogs).toHaveBeenCalled());
    const id = subscriptionId();

    act(() => {
      emitLogEvent({
        subscription_id: id,
        service_id: 'orders-api',
        kind: 'lagged',
        entry: null,
        dropped: 25,
        message: null,
      });
    });

    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent(/25 entries were dropped/i));
  });

  it('ignores events belonging to another subscription', async () => {
    await openLogs();
    await selectServiceScope(userEvent.setup());
    await waitFor(() => expect(mockDesktopApi.subscribeServiceLogs).toHaveBeenCalled());

    act(() => {
      emitLogEvent({
        subscription_id: 'someone-else',
        service_id: 'orders-api',
        kind: 'entry',
        entry: makeLogEntry(7, { message: 'not for this view' }),
        dropped: null,
        message: null,
      });
    });
    await flush();

    expect(screen.queryByText('not for this view')).not.toBeInTheDocument();
  });
});

describe('log filtering', () => {
  it('re-queries with the selected level', async () => {
    const user = await openLogs();

    await user.selectOptions(screen.getByLabelText('Filter by log level'), 'error');

    await waitFor(() =>
      expect(mockDesktopApi.getLogs).toHaveBeenLastCalledWith(
        expect.objectContaining({ minimumLevel: 'error' }),
      ),
    );
  });

  it('re-queries with the selected source and search text', async () => {
    const user = await openLogs();

    await user.selectOptions(screen.getByLabelText('Filter by log source'), 'build');
    await user.type(screen.getByLabelText('Search logs'), 'dependency');

    await waitFor(() =>
      expect(mockDesktopApi.getLogs).toHaveBeenLastCalledWith(
        expect.objectContaining({ source: 'build', text: 'dependency' }),
      ),
    );
  });
});

describe('log pagination', () => {
  it('continues from the cursor instead of reloading the first page', async () => {
    mockDesktopApi.getLogs.mockResolvedValueOnce(
      makeLogPage({ entries: makeLogEntries(2), next_cursor: 2, truncated: true }),
    );
    const user = await openLogs();

    expect(screen.getByRole('group', { name: 'Log controls' })).toHaveTextContent(
      /older entries exist/i,
    );

    await user.click(screen.getByRole('button', { name: 'Fetch newer' }));

    await waitFor(() =>
      expect(mockDesktopApi.getLogs).toHaveBeenLastCalledWith(
        expect.objectContaining({ after: 2 }),
      ),
    );
  });

  it('deduplicates entries that arrive twice with the same sequence', async () => {
    mockDesktopApi.getLogs.mockResolvedValueOnce(
      makeLogPage({ entries: makeLogEntries(2), next_cursor: 2 }),
    );
    const user = await openLogs();

    // Re-fetching without a cursor returns the same page; lines are not
    // duplicated in the buffer.
    await user.click(screen.getByRole('button', { name: 'Fetch newer' }));
    await flush();

    expect(screen.getAllByText('line 1')).toHaveLength(1);
  });
});

describe('log follow mode', () => {
  it('polls the cursor while following a non-service scope', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      // A non-null cursor makes the follow poll continue from the last page.
      mockDesktopApi.getLogs.mockResolvedValue(
        makeLogPage({ entries: makeLogEntries(2), next_cursor: 2 }),
      );
      await openLogs();
      const before = mockDesktopApi.getLogs.mock.calls.length;

      await act(async () => {
        await vi.advanceTimersByTimeAsync(1_200);
      });

      expect(mockDesktopApi.getLogs.mock.calls.length).toBeGreaterThan(before);
      expect(mockDesktopApi.getLogs).toHaveBeenLastCalledWith(
        expect.objectContaining({ after: expect.any(Number) }),
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it('stops following when the toggle is switched off', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const user = await openLogs({ advanceTimers: true });
      const follow = screen.getByRole('checkbox', { name: /follow/i });
      expect(follow).toBeChecked();

      await user.click(follow);
      const before = mockDesktopApi.getLogs.mock.calls.length;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2_500);
      });

      expect(mockDesktopApi.getLogs.mock.calls.length).toBe(before);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('log buffer limits', () => {
  it('keeps only the newest entries and says so honestly', async () => {
    mockDesktopApi.getLogs.mockResolvedValue(
      makeLogPage({
        entries: Array.from({ length: MAX_LOG_BUFFER + 50 }, (_, index) =>
          makeLogEntry(index + 1),
        ),
        next_cursor: null,
      }),
    );
    await openLogs();

    await waitFor(() =>
      expect(screen.getByText(new RegExp(`Buffer limit reached \\(${MAX_LOG_BUFFER}`))).toBeInTheDocument(),
    );
    expect(screen.getByRole('log').childElementCount).toBeLessThanOrEqual(MAX_LOG_BUFFER);
  });

  it('clearing the view clears only the visible buffer', async () => {
    const user = await openLogs();
    expect(screen.getByRole('log')).toHaveTextContent('BUILD FAILURE: missing dependency');

    await user.click(screen.getByRole('button', { name: 'Clear view' }));

    expect(screen.getByRole('log')).toHaveTextContent('No log output matches this view.');
    // Stored logs are never touched from React: no destructive call exists.
    expect(mockDesktopApi.getLogs).toHaveBeenCalled();
  });
});

describe('workspace switching', () => {
  it('re-queries logs for the newly selected workspace', async () => {
    mockDesktopApi.listWorkspaces.mockResolvedValue([
      { id: 'demo', name: 'Demo', root: '/p/demo', service_count: 2 },
      { id: 'second', name: 'Second', root: '/p/second', service_count: 1 },
    ]);
    const user = await openLogs();

    await user.selectOptions(screen.getByLabelText('Active workspace'), 'second');

    await waitFor(() =>
      expect(mockDesktopApi.getLogs).toHaveBeenLastCalledWith(
        expect.objectContaining({ workspaceId: 'second' }),
      ),
    );
  });
});

describe('daemon disconnection', () => {
  it('pauses follow and detaches the subscription while disconnected', async () => {
    const user = await openLogs();
    await selectServiceScope(user);
    await waitFor(() => expect(mockDesktopApi.subscribeServiceLogs).toHaveBeenCalled());
    const id = subscriptionId();

    act(() => emitConnection(makeDisconnected('the daemon is not running')));
    await flush();

    expect(screen.getAllByText(/Disconnected from the daemon/i).length).toBeGreaterThan(0);
    await waitFor(() =>
      expect(mockDesktopApi.unsubscribeServiceLogs).toHaveBeenCalledWith(id),
    );
  });
});
