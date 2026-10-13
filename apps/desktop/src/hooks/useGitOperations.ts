import { useCallback, useMemo, useRef } from 'react';

import { desktopApi } from '../services/desktopApi';
import type { OperationOutcome } from '../types/models';
import {
  useEntityOperations,
  type EntityOperations,
  type OperationTarget,
} from './useEntityOperations';

/** The Git actions the repository screen can submit. */
export type GitAction = 'fetch' | 'pull' | 'checkout' | 'sync';

/** The action labels used in dialogs and result panels. */
export const GIT_ACTION_LABELS: Record<GitAction, string> = {
  fetch: 'Fetch',
  pull: 'Pull',
  checkout: 'Checkout',
  sync: 'Sync',
};

/** The value returned by [`useGitOperations`]. */
export interface GitOperations
  extends Omit<EntityOperations<GitAction>, 'execute'> {
  /** Fetches the targets (bulk uses one workflow). */
  runFetch: (targets: OperationTarget[]) => Promise<void>;
  /** Fast-forward pulls a single target. */
  runPull: (targets: OperationTarget[]) => Promise<void>;
  /** Checks out `branch` for the targets (bulk uses one workflow). */
  runCheckout: (targets: OperationTarget[], branch: string) => Promise<void>;
  /** Synchronizes the targets (bulk uses one workflow). */
  runSync: (targets: OperationTarget[]) => Promise<void>;
}

/**
 * Submits Git operations over the shared daemon.
 *
 * A single repository maps to the corresponding single-repository task; a
 * multi-repository selection maps to the coordinated bulk workflow, so the GUI
 * never issues an uncoordinated loop of requests (`tasks/020.md` §6, §7).
 * Pull is single-repository only because the core exposes no bulk pull.
 */
export function useGitOperations(
  workspaceId: string | null,
  onSettled: () => void,
): GitOperations {
  const branchRef = useRef('');

  const submit = useCallback(
    async (
      action: GitAction,
      workspace: string,
      entityIds: string[],
    ): Promise<OperationOutcome> => {
      const bulk = entityIds.length > 1;
      switch (action) {
        case 'fetch':
          return bulk
            ? desktopApi.fetchRepositories(workspace, entityIds)
            : desktopApi.fetchRepository(entityIds[0]);
        case 'pull':
          return desktopApi.pullRepository(entityIds[0]);
        case 'checkout':
          return bulk
            ? desktopApi.checkoutRepositories(workspace, entityIds, branchRef.current)
            : desktopApi.checkoutRepository(entityIds[0], branchRef.current);
        case 'sync':
          return bulk
            ? desktopApi.syncRepositories(workspace, entityIds)
            : desktopApi.syncRepository(entityIds[0]);
      }
    },
    [],
  );

  const base = useEntityOperations<GitAction>(workspaceId, submit, onSettled);

  const runFetch = useCallback(
    (targets: OperationTarget[]) => base.execute('fetch', targets),
    [base],
  );
  const runPull = useCallback(
    (targets: OperationTarget[]) => base.execute('pull', targets),
    [base],
  );
  const runCheckout = useCallback(
    (targets: OperationTarget[], branch: string) => {
      branchRef.current = branch;
      return base.execute('checkout', targets);
    },
    [base],
  );
  const runSync = useCallback(
    (targets: OperationTarget[]) => base.execute('sync', targets),
    [base],
  );

  return useMemo(
    () => ({
      pending: base.pending,
      operations: base.operations,
      error: base.error,
      clearError: base.clearError,
      runFetch,
      runPull,
      runCheckout,
      runSync,
    }),
    [base, runFetch, runPull, runCheckout, runSync],
  );
}
