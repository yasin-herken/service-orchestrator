import { describe, expect, it } from 'vitest';

import { operationLabel } from './format';

describe('operationLabel', () => {
  it('combines the task kind with a capitalized status', () => {
    expect(operationLabel({ kind: 'build_service', status: 'succeeded' })).toBe(
      'build_service · Succeeded',
    );
    expect(operationLabel({ kind: 'git_sync', status: 'running' })).toBe(
      'git_sync · Running',
    );
  });

  it('uses the shared task-status sources so tables never drift', () => {
    // `operationLabel` must round-trip every engine status through the one
    // label source (`tasks/024.md` §5, §12).
    for (const status of ['queued', 'running', 'succeeded', 'failed', 'cancelled', 'skipped', 'blocked']) {
      expect(operationLabel({ kind: 'x', status })).toBe(
        `x · ${status.charAt(0).toUpperCase()}${status.slice(1)}`,
      );
    }
  });
});
