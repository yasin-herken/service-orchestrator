import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react';

import { desktopApi } from '../services/desktopApi';
import type { TaskEvent } from '../types/models';
import { describeTaskEvent, taskEventVariant } from '../utils/taskEvents';

/** One entry in the bounded task-activity feed. */
export interface TaskActivityEntry {
  /** Monotonic key for rendering. */
  key: number;
  /** Task-event variant name (`TaskCreated`, `TaskFailed`, …). */
  variant: string;
  /** One-line summary. */
  summary: string;
  /** When the event arrived, seconds since the Unix epoch. */
  at: number;
}

/** How many activity entries the feed retains. */
const MAX_ENTRIES = 50;

/** The value provided by [`TaskEventsProvider`]. */
export interface TaskEventsContextValue {
  /** Most recent task events, newest first. */
  recent: readonly TaskActivityEntry[];
}

const TaskEventsContext = createContext<TaskEventsContextValue | null>(null);

/**
 * Receives task-engine events forwarded by the Rust adapter and keeps a small
 * in-memory feed.
 *
 * This establishes the event subscription architecture for later tasks
 * (`tasks/018.md` §22); only the dashboard renders it so far.
 */
export function TaskEventsProvider({ children }: { children: ReactNode }) {
  const [recent, setRecent] = useState<TaskActivityEntry[]>([]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let nextKey = 0;

    void desktopApi.onTaskEvent((event: TaskEvent) => {
      if (disposed) {
        return;
      }
      const entry: TaskActivityEntry = {
        key: (nextKey += 1),
        variant: taskEventVariant(event),
        summary: describeTaskEvent(event),
        at: Math.floor(Date.now() / 1000),
      };
      setRecent((current) => [entry, ...current].slice(0, MAX_ENTRIES));
    }).then(
      (stop) => {
        if (disposed) {
          stop();
        } else {
          unlisten = stop;
        }
      },
      () => {
        // No event stream outside Tauri; the feed simply stays empty.
      },
    );

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const value = useMemo<TaskEventsContextValue>(() => ({ recent }), [recent]);
  return <TaskEventsContext.Provider value={value}>{children}</TaskEventsContext.Provider>;
}

/** Reads the task-activity feed. Must be used under the provider. */
export function useTaskEvents(): TaskEventsContextValue {
  const value = useContext(TaskEventsContext);
  if (!value) {
    throw new Error('useTaskEvents must be used within a TaskEventsProvider');
  }
  return value;
}
