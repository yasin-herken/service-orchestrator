import { act, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { flush } from '../test/harness';
import { emitTaskEvent, resetMockDesktopApi } from '../test/mocks';
import { TaskEventsProvider, useTaskEvents, useTaskRevision } from './TaskEventsContext';

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

  it('does not re-render a revision-only consumer on progress events', async () => {
    // Regression guard for `tasks/025.md` §24: consumers that only reload on a
    // terminal settle must not be re-rendered by every progress frame. Before
    // the context split, `terminalRevision` shared one context value with the
    // per-event task data, so every consumer re-rendered on every event.
    let revisionRenders = 0;
    function RevisionProbe() {
      const revision = useTaskRevision();
      revisionRenders += 1;
      return <span data-testid="revision-only">{revision}</span>;
    }

    render(
      <TaskEventsProvider>
        <Probe />
        <RevisionProbe />
      </TaskEventsProvider>,
    );
    await flush();
    const baseline = revisionRenders;

    act(() =>
      emitTaskEvent({
        TaskCreated: { task: { id: 't9', kind: 'build_service', status: 'running' } },
      }),
    );
    for (let step = 1; step <= 20; step += 1) {
      act(() =>
        emitTaskEvent({
          TaskProgress: { id: 't9', progress: { current: step, total: 20 } },
        }),
      );
    }
    // Twenty progress frames caused zero re-renders of the revision consumer.
    expect(screen.getByTestId('task-t9')).toHaveTextContent('running:20');
    expect(revisionRenders).toBe(baseline);

    act(() => emitTaskEvent({ TaskSucceeded: { id: 't9' } }));
    // Exactly one terminal settle re-renders it, and the revision advanced.
    expect(revisionRenders).toBe(baseline + 1);
    expect(screen.getByTestId('revision-only')).toHaveTextContent('1');
  });
});
