import { useCallback, useRef, useState } from 'react';

import { toDesktopError } from '../services/errors';
import type { DesktopError, OperationOutcome } from '../types/models';

/** The minimum an entity must expose to be an operation target. */
export interface OperationTarget {
  id: string;
  /** Human-readable label shown in dialogs and results. */
  label: string;
}

/** An operation submitted to the daemon, newest first. */
export interface SubmittedOperation<Action extends string> {
  /** Monotonic key for rendering. */
  key: number;
  /** The action label (`sync`, `build`, `prepare`, …). */
  action: Action;
  /** The entity ids the operation targeted. */
  entityIds: string[];
  /** Human-readable target labels. */
  labels: string[];
  /** The daemon's normalized outcome. */
  outcome: OperationOutcome;
  /** When it was submitted, seconds since the Unix epoch. */
  at: number;
}

/** The most recent submission failure, kept for the error dialog. */
export interface OperationError<Action extends string> {
  action: Action;
  entityIds: string[];
  labels: string[];
  error: DesktopError;
}

/** The value returned by [`useEntityOperations`]. */
export interface EntityOperations<Action extends string> {
  /** Entity ids with an in-flight command. */
  pending: ReadonlySet<string>;
  /** Submitted operations, newest first. */
  operations: readonly SubmittedOperation<Action>[];
  /** The most recent failure, or `null`. */
  error: OperationError<Action> | null;
  /** Submits an action for the given targets. */
  execute: (action: Action, targets: OperationTarget[]) => Promise<void>;
  /** Dismisses the error dialog. */
  clearError: () => void;
}

/** Dispatches one action for a workspace and a non-empty selection. */
export type SubmitOperation<Action extends string> = (
  action: Action,
  workspaceId: string,
  entityIds: string[],
) => Promise<OperationOutcome>;

/**
 * Submits domain operations through the daemon and tracks their pending state.
 *
 * The hook never marks an operation successful: it records the daemon's
 * normalized outcome (a task id or a workflow id) and calls `onSettled` so the
 * caller can reload authoritative state. `pending` guards duplicate submissions
 * while a command is in flight; the daemon owns coordination beyond that
 * (`tasks/020.md` §27–§31).
 */
export function useEntityOperations<Action extends string>(
  workspaceId: string | null,
  submit: SubmitOperation<Action>,
  onSettled: () => void,
): EntityOperations<Action> {
  const [pending, setPending] = useState<ReadonlySet<string>>(new Set());
  const [operations, setOperations] = useState<SubmittedOperation<Action>[]>([]);
  const [error, setError] = useState<OperationError<Action> | null>(null);
  const nextKey = useRef(0);

  const execute = useCallback(
    async (action: Action, targets: OperationTarget[]) => {
      if (!workspaceId || targets.length === 0) {
        return;
      }
      const entityIds = targets.map((target) => target.id);
      const labels = targets.map((target) => target.label);
      setError(null);
      setPending((current) => new Set([...current, ...entityIds]));
      try {
        const outcome = await submit(action, workspaceId, entityIds);
        nextKey.current += 1;
        const operation: SubmittedOperation<Action> = {
          key: nextKey.current,
          action,
          entityIds,
          labels,
          outcome,
          at: Math.floor(Date.now() / 1000),
        };
        setOperations((current) => [operation, ...current].slice(0, 20));
        onSettled();
      } catch (cause) {
        setError({ action, entityIds, labels, error: toDesktopError(cause) });
      } finally {
        setPending((current) => {
          const next = new Set(current);
          for (const id of entityIds) {
            next.delete(id);
          }
          return next;
        });
      }
    },
    [workspaceId, submit, onSettled],
  );

  const clearError = useCallback(() => setError(null), []);

  return { pending, operations, error, execute, clearError };
}
