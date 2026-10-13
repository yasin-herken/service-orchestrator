import { describe, expect, it } from 'vitest';

import {
  TERMINAL_TASK_STATUSES,
  isTerminalTaskStatus,
  taskStatusLabel,
} from './taskStatus';

describe('taskStatus', () => {
  it('classifies exactly the settled engine states as terminal', () => {
    expect([...TERMINAL_TASK_STATUSES].sort()).toEqual([
      'cancelled',
      'failed',
      'skipped',
      'succeeded',
    ]);
    for (const settled of ['succeeded', 'failed', 'cancelled', 'skipped']) {
      expect(isTerminalTaskStatus(settled)).toBe(true);
    }
  });

  it('treats in-flight and unknown states as non-terminal', () => {
    for (const active of ['queued', 'running', 'blocked', 'unknown']) {
      expect(isTerminalTaskStatus(active)).toBe(false);
    }
  });

  it('labels a status for display without inventing a verdict', () => {
    expect(taskStatusLabel('succeeded')).toBe('Succeeded');
    expect(taskStatusLabel('running')).toBe('Running');
    expect(taskStatusLabel('unknown')).toBe('Unknown');
  });
});
