import { useCallback, useMemo } from 'react';

import { desktopApi } from '../services/desktopApi';
import type { OperationOutcome } from '../types/models';
import {
  useEntityOperations,
  type EntityOperations,
  type OperationError,
  type SubmittedOperation,
} from './useEntityOperations';

/** The lifecycle actions the services screen can submit. */
export type LifecycleAction = 'start' | 'stop' | 'restart' | 'check_health';

/** The minimum a service must expose to be a lifecycle target. */
export interface ServiceTarget {
  id: string;
  display_name: string;
}

/** The task kind labels the daemon reports for each action. */
export const ACTION_LABELS: Record<LifecycleAction, string> = {
  start: 'Start',
  stop: 'Stop',
  restart: 'Restart',
  check_health: 'Check health',
};

/** An operation submitted to the daemon, newest first. */
export type ServiceOperation = SubmittedOperation<LifecycleAction>;

/** The most recent action failure, kept for the error dialog. */
export type ActionErrorState = OperationError<LifecycleAction>;

/** The value returned by [`useServiceActions`]. */
export interface ServiceActions extends Omit<EntityOperations<LifecycleAction>, 'execute'> {
  /** Submits a lifecycle action for the given services. */
  execute: (action: LifecycleAction, targets: ServiceTarget[]) => Promise<void>;
}

async function callSingle(
  action: LifecycleAction,
  serviceId: string,
): Promise<OperationOutcome> {
  switch (action) {
    case 'start':
      return desktopApi.startService(serviceId);
    case 'stop':
      return desktopApi.stopService(serviceId);
    case 'restart':
      return desktopApi.restartService(serviceId);
    case 'check_health':
      return desktopApi.checkServiceHealth(serviceId);
  }
}

async function callBulk(
  action: LifecycleAction,
  workspaceId: string,
  serviceIds: string[],
): Promise<OperationOutcome> {
  switch (action) {
    case 'start':
      return desktopApi.startServices(workspaceId, serviceIds);
    case 'stop':
      return desktopApi.stopServices(workspaceId, serviceIds);
    case 'restart':
      return desktopApi.restartServices(workspaceId, serviceIds);
    case 'check_health':
      // Bulk health checks are not offered; only one service is ever targeted.
      return callSingle('check_health', serviceIds[0]);
  }
}

/**
 * Submits service lifecycle actions through the daemon and tracks their
 * pending state.
 *
 * The pending/operations/error bookkeeping is the shared
 * [`useEntityOperations`] state machine; this hook only maps the service-shaped
 * target to the generic `OperationTarget` and dispatches single vs bulk calls.
 * The hook never marks an operation successful: it records the daemon's
 * normalized outcome and calls `onSettled` so the caller can reload the
 * authoritative state (`tasks/019.md` §24, §28).
 */
export function useServiceActions(
  workspaceId: string | null,
  onSettled: () => void,
): ServiceActions {
  const submit = useCallback(
    async (
      action: LifecycleAction,
      workspace: string,
      serviceIds: string[],
    ): Promise<OperationOutcome> =>
      serviceIds.length === 1
        ? callSingle(action, serviceIds[0])
        : callBulk(action, workspace, serviceIds),
    [],
  );

  const base = useEntityOperations<LifecycleAction>(workspaceId, submit, onSettled);
  const execute = useCallback(
    (action: LifecycleAction, targets: ServiceTarget[]) =>
      base.execute(
        action,
        targets.map((target) => ({ id: target.id, label: target.display_name })),
      ),
    [base],
  );

  return useMemo(
    () => ({
      pending: base.pending,
      operations: base.operations,
      error: base.error,
      clearError: base.clearError,
      execute,
    }),
    [base, execute],
  );
}
