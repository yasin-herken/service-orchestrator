import { useCallback, useMemo } from 'react';

import { desktopApi } from '../services/desktopApi';
import type { OperationOutcome } from '../types/models';
import {
  useEntityOperations,
  type EntityOperations,
  type OperationTarget,
} from './useEntityOperations';

/** The build actions the screen can submit. */
export type BuildAction = 'build';

/** The action labels used in dialogs and result panels. */
export const BUILD_ACTION_LABELS: Record<BuildAction, string> = {
  build: 'Build',
};

/** The value returned by [`useBuildOperations`]. */
export interface BuildOperations
  extends Omit<EntityOperations<BuildAction>, 'execute'> {
  /** Builds the targets (bulk uses one workflow). */
  runBuild: (targets: OperationTarget[]) => Promise<void>;
}

/**
 * Submits service builds over the shared daemon.
 *
 * One service maps to the single build task; a multi-service selection maps to
 * one environment-preparation workflow with only the build phase enabled, so
 * the daemon owns concurrency and dependency ordering (`tasks/020.md` §14, §16).
 */
export function useBuildOperations(
  workspaceId: string | null,
  onSettled: () => void,
): BuildOperations {
  const submit = useCallback(
    async (
      _action: BuildAction,
      workspace: string,
      entityIds: string[],
    ): Promise<OperationOutcome> =>
      entityIds.length > 1
        ? desktopApi.buildServices(workspace, entityIds)
        : desktopApi.buildService(entityIds[0]),
    [],
  );

  const base = useEntityOperations<BuildAction>(workspaceId, submit, onSettled);
  const runBuild = useCallback(
    (targets: OperationTarget[]) => base.execute('build', targets),
    [base],
  );

  return useMemo(
    () => ({
      pending: base.pending,
      operations: base.operations,
      error: base.error,
      clearError: base.clearError,
      runBuild,
    }),
    [base, runBuild],
  );
}
