import { useCallback, useEffect, useState } from 'react';

import { useConnection } from '../context/ConnectionContext';
import { desktopApi } from '../services/desktopApi';
import { toDesktopError } from '../services/errors';
import type { DesktopError, TaskView, WorkflowStatusView } from '../types/models';
import { isTerminalWorkflowState } from '../types/models';

/** How often a running workflow is re-read while connected. */
const POLL_MS = 1500;

/** The observed state of one daemon workflow. */
export interface WorkflowRun {
  /** The workflow being observed, or `null` before execution. */
  workflowId: string | null;
  /** The daemon's aggregated status; `null` until the first read. */
  status: WorkflowStatusView | null;
  /** The task-engine records that belong to this workflow. */
  tasks: TaskView[];
  /** A structured failure of the status request, if any. */
  error: DesktopError | null;
  /** The wall-clock second observation started, for elapsed time. */
  observedAt: number;
  /** Whether the daemon reports a terminal state. */
  terminal: boolean;
  /** Starts observing a freshly scheduled workflow. */
  begin: (workflowId: string) => void;
  /** Stops observing and forgets the workflow. */
  reset: () => void;
  /** Asks the daemon to cancel; never stops anything locally. */
  cancel: () => Promise<void>;
  /** Switches observation to an existing workflow id. */
  observe: (workflowId: string) => Promise<void>;
}

/**
 * Observes one daemon workflow (`tasks/032.md` §33, §34).
 *
 * Progress and results come only from the daemon's workflow status and the
 * task engine; nothing is estimated or invented locally, and a terminal state
 * is never declared by the GUI. The hook is shared by the Bulk Operations
 * screen so the Workflows screen's behaviour is reused rather than copied.
 */
export function useWorkflowRun(): WorkflowRun {
  const { snapshot } = useConnection();
  const connected = snapshot.state === 'connected';

  const [workflowId, setWorkflowId] = useState<string | null>(null);
  const [status, setStatus] = useState<WorkflowStatusView | null>(null);
  const [tasks, setTasks] = useState<TaskView[]>([]);
  const [error, setError] = useState<DesktopError | null>(null);
  const [observedAt, setObservedAt] = useState(0);

  const terminal = status !== null && isTerminalWorkflowState(status.state);

  const begin = useCallback((id: string) => {
    setWorkflowId(id);
    setStatus(null);
    setTasks([]);
    setError(null);
    setObservedAt(Math.floor(Date.now() / 1000));
  }, []);

  const reset = useCallback(() => {
    setWorkflowId(null);
    setStatus(null);
    setTasks([]);
    setError(null);
    setObservedAt(0);
  }, []);

  const observe = useCallback(async (id: string) => {
    setWorkflowId(id);
    setError(null);
    try {
      const view = await desktopApi.getWorkflowStatus(id);
      setStatus(view);
      const page = await desktopApi.listTasks(200);
      setTasks(page.filter((task) => task.workflow_id === id));
      setObservedAt((current) => current || Math.floor(Date.now() / 1000));
    } catch (cause) {
      setError(toDesktopError(cause));
    }
  }, []);

  const cancel = useCallback(async () => {
    if (!workflowId) {
      return;
    }
    try {
      await desktopApi.cancelWorkflow(workflowId);
    } catch (cause) {
      setError(toDesktopError(cause));
    }
  }, [workflowId]);

  useEffect(() => {
    if (!workflowId || !connected || terminal) {
      return undefined;
    }
    let cancelled = false;
    const load = async () => {
      try {
        const view = await desktopApi.getWorkflowStatus(workflowId);
        if (cancelled) {
          return;
        }
        setStatus(view);
        setError(null);
        const page = await desktopApi.listTasks(200);
        if (cancelled) {
          return;
        }
        setTasks(page.filter((task) => task.workflow_id === workflowId));
      } catch (cause) {
        if (!cancelled) {
          setError(toDesktopError(cause));
        }
      }
    };
    const timer = setInterval(() => void load(), POLL_MS);
    // Read once immediately so the screen shows the daemon's state without
    // waiting for the first poll.
    void load();
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [workflowId, connected, terminal]);

  return {
    workflowId,
    status,
    tasks,
    error,
    observedAt,
    terminal,
    begin,
    reset,
    cancel,
    observe,
  };
}
