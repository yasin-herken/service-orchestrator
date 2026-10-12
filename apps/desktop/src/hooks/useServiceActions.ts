import { useCallback, useRef, useState } from 'react';

import { desktopApi } from '../services/desktopApi';
import { toDesktopError } from '../services/errors';
import type { DesktopError, OperationOutcome } from '../types/models';

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
export interface ServiceOperation {
  /** Monotonic key for rendering. */
  key: number;
  action: LifecycleAction;
  /** The services the operation targeted. */
  serviceIds: string[];
  /** Human-readable target labels. */
  labels: string[];
  /** The daemon's normalized outcome. */
  outcome: OperationOutcome;
  /** When it was submitted, seconds since the Unix epoch. */
  at: number;
}

/** The most recent action failure, kept for the error dialog. */
export interface ActionErrorState {
  action: LifecycleAction;
  serviceIds: string[];
  labels: string[];
  error: DesktopError;
}

/** The value returned by [`useServiceActions`]. */
export interface ServiceActions {
  /** Service ids with an in-flight command. */
  pending: ReadonlySet<string>;
  /** Submitted operations, newest first. */
  operations: readonly ServiceOperation[];
  /** The most recent failure, or `null`. */
  error: ActionErrorState | null;
  /** Submits a lifecycle action for the given services. */
  execute: (action: LifecycleAction, targets: ServiceTarget[]) => Promise<void>;
  /** Dismisses the error dialog. */
  clearError: () => void;
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
 * The hook never marks an operation successful: it records the daemon's
 * normalized outcome and calls `onSettled` so the caller can reload the
 * authoritative state. `pending` guards against duplicate submissions while a
 * command is in flight; the backend's own coordination guards the rest
 * (`tasks/019.md` §24, §28).
 */
export function useServiceActions(
  workspaceId: string | null,
  onSettled: () => void,
): ServiceActions {
  const [pending, setPending] = useState<ReadonlySet<string>>(new Set());
  const [operations, setOperations] = useState<ServiceOperation[]>([]);
  const [error, setError] = useState<ActionErrorState | null>(null);
  const nextKey = useRef(0);

  const execute = useCallback(
    async (action: LifecycleAction, targets: ServiceTarget[]) => {
      if (!workspaceId || targets.length === 0) {
        return;
      }
      const serviceIds = targets.map((target) => target.id);
      const labels = targets.map((target) => target.display_name);
      setError(null);
      setPending((current) => new Set([...current, ...serviceIds]));
      try {
        const outcome =
          targets.length === 1
            ? await callSingle(action, serviceIds[0])
            : await callBulk(action, workspaceId, serviceIds);
        nextKey.current += 1;
        const operation: ServiceOperation = {
          key: nextKey.current,
          action,
          serviceIds,
          labels,
          outcome,
          at: Math.floor(Date.now() / 1000),
        };
        setOperations((current) => [operation, ...current].slice(0, 20));
        onSettled();
      } catch (cause) {
        setError({
          action,
          serviceIds,
          labels,
          error: toDesktopError(cause),
        });
      } finally {
        setPending((current) => {
          const next = new Set(current);
          for (const id of serviceIds) {
            next.delete(id);
          }
          return next;
        });
      }
    },
    [workspaceId, onSettled],
  );

  const clearError = useCallback(() => setError(null), []);

  return { pending, operations, error, execute, clearError };
}
