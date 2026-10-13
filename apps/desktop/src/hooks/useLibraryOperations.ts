import { useCallback, useMemo, useRef } from 'react';

import { desktopApi, type PrepareOptions } from '../services/desktopApi';
import type { OperationOutcome } from '../types/models';
import {
  useEntityOperations,
  type EntityOperations,
  type OperationTarget,
} from './useEntityOperations';

/** The library actions the screen can submit. */
export type LibraryAction = 'build' | 'install' | 'sync' | 'prepare';

/** The action labels used in dialogs and result panels. */
export const LIBRARY_ACTION_LABELS: Record<LibraryAction, string> = {
  build: 'Build',
  install: 'Install',
  sync: 'Sync',
  prepare: 'Prepare',
};

/** The value returned by [`useLibraryOperations`]. */
export interface LibraryOperations
  extends Omit<EntityOperations<LibraryAction>, 'execute'> {
  /** Builds the targets. */
  runBuild: (targets: OperationTarget[]) => Promise<void>;
  /** Installs the targets' artifacts locally. */
  runInstall: (targets: OperationTarget[]) => Promise<void>;
  /** Synchronizes the targets' repositories. */
  runSync: (targets: OperationTarget[]) => Promise<void>;
  /** Runs the prepare workflow for the targets with explicit options. */
  runPrepare: (targets: OperationTarget[], options: PrepareOptions) => Promise<void>;
}

/** The prepare options that express a single phase of the workflow. */
function phaseOptions(phase: Partial<PrepareOptions>): PrepareOptions {
  return { sync: false, branch: null, build: false, install: false, verify: false, ...phase };
}

/**
 * Submits library operations over the shared daemon.
 *
 * One library maps to the dedicated build/install/sync task or the prepare
 * workflow; a multi-library selection maps onto one prepare workflow with only
 * the requested phases enabled, so the daemon owns dependency ordering rather
 * than the GUI looping (`tasks/020.md` §21, §25, §26).
 */
export function useLibraryOperations(
  workspaceId: string | null,
  onSettled: () => void,
): LibraryOperations {
  const optionsRef = useRef<PrepareOptions>(phaseOptions({}));

  const submit = useCallback(
    async (
      action: LibraryAction,
      workspace: string,
      entityIds: string[],
    ): Promise<OperationOutcome> => {
      const bulk = entityIds.length > 1;
      if (action === 'prepare') {
        return bulk
          ? desktopApi.prepareLibraries(workspace, entityIds, optionsRef.current)
          : desktopApi.prepareLibrary(entityIds[0], optionsRef.current);
      }
      if (!bulk) {
        switch (action) {
          case 'build':
            return desktopApi.buildLibrary(entityIds[0]);
          case 'install':
            return desktopApi.installLibrary(entityIds[0]);
          case 'sync':
            return desktopApi.syncLibrary(entityIds[0]);
        }
      }
      // Bulk build/install/sync reuse the prepare workflow with one phase on.
      const options =
        action === 'build'
          ? phaseOptions({ build: true })
          : action === 'install'
            ? phaseOptions({ install: true })
            : phaseOptions({ sync: true });
      return desktopApi.prepareLibraries(workspace, entityIds, options);
    },
    [],
  );

  const base = useEntityOperations<LibraryAction>(workspaceId, submit, onSettled);

  const runBuild = useCallback(
    (targets: OperationTarget[]) => base.execute('build', targets),
    [base],
  );
  const runInstall = useCallback(
    (targets: OperationTarget[]) => base.execute('install', targets),
    [base],
  );
  const runSync = useCallback(
    (targets: OperationTarget[]) => base.execute('sync', targets),
    [base],
  );
  const runPrepare = useCallback(
    (targets: OperationTarget[], options: PrepareOptions) => {
      optionsRef.current = options;
      return base.execute('prepare', targets);
    },
    [base],
  );

  return useMemo(
    () => ({
      pending: base.pending,
      operations: base.operations,
      error: base.error,
      clearError: base.clearError,
      runBuild,
      runInstall,
      runSync,
      runPrepare,
    }),
    [base, runBuild, runInstall, runSync, runPrepare],
  );
}
