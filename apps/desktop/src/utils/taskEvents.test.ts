import { describe, expect, it } from 'vitest';

import { describeTaskEvent, taskEventVariant } from './taskEvents';

describe('taskEventVariant', () => {
  it('reads the externally tagged variant name', () => {
    expect(taskEventVariant({ TaskCreated: { task: {} } })).toBe('TaskCreated');
    expect(taskEventVariant({ Lagged: { dropped: 3 } })).toBe('Lagged');
  });

  it('degrades gracefully for an empty payload', () => {
    expect(taskEventVariant({})).toBe('Unknown');
  });
});

describe('describeTaskEvent', () => {
  it('describes task creation with its kind', () => {
    const summary = describeTaskEvent({
      TaskCreated: { task: { id: 'task-1', kind: 'start_service', status: 'queued' } },
    });
    expect(summary).toBe('Task created (start_service)');
  });

  it('describes a started task', () => {
    expect(describeTaskEvent({ TaskStarted: { id: 'task-1', kind: 'git_sync' } })).toBe(
      'Task started (git_sync)',
    );
  });

  it('describes a failure with its message', () => {
    const summary = describeTaskEvent({
      TaskFailed: { id: 'task-1', failure: { code: 'GIT_FETCH_FAILED', message: 'auth denied' } },
    });
    expect(summary).toBe('Task failed: auth denied');
  });

  it('describes a success', () => {
    expect(describeTaskEvent({ TaskSucceeded: { id: 'task-1' } })).toBe('Task succeeded');
  });

  it('describes dropped events when the subscription lagged', () => {
    expect(describeTaskEvent({ Lagged: { dropped: 7 } })).toBe(
      '7 task event(s) dropped while the view was behind',
    );
  });
});
