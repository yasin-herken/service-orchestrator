import { describe, expect, it } from 'vitest';

import type { TrackedTask } from '../context/TaskEventsContext';
import {
  busyServiceIds,
  isTerminalStatus,
  lastTaskByService,
  summarizeTasks,
  tasksForWorkflow,
} from './taskProgress';

function task(overrides: Partial<TrackedTask> = {}): TrackedTask {
  return {
    id: 'task-1',
    kind: 'start_service',
    status: 'running',
    serviceId: 'orders-api',
    workflowId: null,
    failure: null,
    progressCurrent: 0,
    progressTotal: null,
    progressMessage: null,
    updatedAt: 1,
    ...overrides,
  };
}

describe('taskProgress', () => {
  it('classifies terminal statuses', () => {
    expect(isTerminalStatus('succeeded')).toBe(true);
    expect(isTerminalStatus('failed')).toBe(true);
    expect(isTerminalStatus('running')).toBe(false);
  });

  it('collects services with non-terminal tasks', () => {
    const tasks = [
      task({ id: 'a', serviceId: 'x', status: 'running' }),
      task({ id: 'b', serviceId: 'y', status: 'succeeded' }),
      task({ id: 'c', serviceId: 'z', status: 'queued' }),
    ];
    expect([...busyServiceIds(tasks)].sort()).toEqual(['x', 'z']);
  });

  it('keeps the most recent task per service', () => {
    const tasks = [
      task({ id: 'old', serviceId: 'x', updatedAt: 1 }),
      task({ id: 'new', serviceId: 'x', updatedAt: 5 }),
      task({ id: 'other', serviceId: 'y', updatedAt: 3 }),
    ];
    const latest = lastTaskByService(tasks);
    expect(latest.get('x')?.id).toBe('new');
    expect(latest.get('y')?.id).toBe('other');
  });

  it('groups workflow tasks and aggregates real outcomes', () => {
    const tasks = [
      task({ id: 'a', workflowId: 'wf', serviceId: 'x', status: 'succeeded' }),
      task({ id: 'b', workflowId: 'wf', serviceId: 'y', status: 'failed' }),
      task({ id: 'c', workflowId: 'other', status: 'running' }),
    ];
    const grouped = tasksForWorkflow(tasks, 'wf');
    expect(grouped.map((t) => t.id)).toEqual(['a', 'b']);
    const summary = summarizeTasks(grouped);
    expect(summary).toMatchObject({
      total: 2,
      succeeded: 1,
      failed: 1,
      running: 0,
      pending: 0,
      complete: true,
    });
  });

  it('marks a summary incomplete while any task is pending or running', () => {
    const summary = summarizeTasks([
      task({ id: 'a', status: 'succeeded' }),
      task({ id: 'b', status: 'running' }),
      task({ id: 'c', status: 'queued' }),
    ]);
    expect(summary.complete).toBe(false);
    expect(summary.pending).toBe(1);
    expect(summary.running).toBe(1);
  });
});
