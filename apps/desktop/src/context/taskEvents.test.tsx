import { act, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { flush } from '../test/harness';
import { emitTaskEvent, resetMockDesktopApi } from '../test/mocks';
import { TaskEventsProvider, useTaskEvents } from './TaskEventsContext';

vi.mock('../services/desktopApi', async () => ({
  desktopApi: (await import('../test/mocks')).mockDesktopApi,
}));

beforeEach(() => {
  resetMockDesktopApi();
});

/** Renders the tracked task state as text for assertions. */
function Probe() {
  const { tasks, terminalRevision } = useTaskEvents();
  return (
    <div>
      <span data-testid="revision">{terminalRevision}</span>
      <ul>
        {tasks.map((task) => (
          <li key={task.id} data-testid={`task-${task.id}`}>
            {task.status}:{task.progressCurrent}
          </li>
        ))}
      </ul>
    </div>
  );
}

async function renderProvider(): Promise<void> {
  render(
    <TaskEventsProvider>
      <Probe />
    </TaskEventsProvider>,
  );
  await flush();
}

describe('task event tracking', () => {
  it('tracks progress for an in-flight task without bumping the reload revision', async () => {
    await renderProvider();

    act(() =>
      emitTaskEvent({
        TaskCreated: { task: { id: 't3', kind: 'build_service', status: 'queued' } },
      }),
    );
    act(() => emitTaskEvent({ TaskStarted: { id: 't3' } }));
    act(() =>
      emitTaskEvent({
        TaskProgress: { id: 't3', progress: { current: 2, total: 4, message: 'compiling' } },
      }),
    );

    expect(screen.getByTestId('task-t3')).toHaveTextContent('running:2');
    expect(screen.getByTestId('revision')).toHaveTextContent('0');
  });

  it('freezes a settled task against a late progress frame and counts it once', async () => {
    await renderProvider();

    act(() =>
      emitTaskEvent({
        TaskCreated: { task: { id: 't1', kind: 'build_service', status: 'running' } },
      }),
    );
    expect(screen.getByTestId('task-t1')).toHaveTextContent('running:0');
    expect(screen.getByTestId('revision')).toHaveTextContent('0');

    act(() => emitTaskEvent({ TaskSucceeded: { id: 't1' } }));
    expect(screen.getByTestId('task-t1')).toHaveTextContent('succeeded:0');
    expect(screen.getByTestId('revision')).toHaveTextContent('1');

    // An executor can retain a progress reporter and emit after the terminal
    // event; a stale frame must never revive the task or move its progress.
    act(() =>
      emitTaskEvent({
        TaskProgress: { id: 't1', progress: { current: 7, total: 9, message: 'cleanup' } },
      }),
    );
    expect(screen.getByTestId('task-t1')).toHaveTextContent('succeeded:0');

    // A duplicate terminal event must not trigger a second reload.
    act(() => emitTaskEvent({ TaskSucceeded: { id: 't1' } }));
    expect(screen.getByTestId('revision')).toHaveTextContent('1');
  });

  it('does not double-count a task that is created already terminal', async () => {
    await renderProvider();

    act(() =>
      emitTaskEvent({
        TaskCreated: { task: { id: 't2', kind: 'git_sync', status: 'succeeded' } },
      }),
    );
    expect(screen.getByTestId('revision')).toHaveTextContent('1');

    // A replay of the same creation event counts nothing further.
    act(() =>
      emitTaskEvent({
        TaskCreated: { task: { id: 't2', kind: 'git_sync', status: 'succeeded' } },
      }),
    );
    expect(screen.getByTestId('revision')).toHaveTextContent('1');
  });
});
