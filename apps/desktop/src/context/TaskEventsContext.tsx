import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react';

import { desktopApi } from '../services/desktopApi';
import type { TaskEvent } from '../types/models';
import { describeTaskEvent, taskEventVariant } from '../utils/taskEvents';
import { isTerminalTaskStatus } from '../utils/taskStatus';

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

/**
 * A task observed on the daemon's task-event stream.
 *
 * This is not fabricated state: every field comes from a forwarded
 * `TaskCreated`/`Task*` event. It lets the GUI show real per-service progress
 * and mark the services of an in-flight task busy without polling
 * (`tasks/019.md` §22, §25, §28).
 */
export interface TrackedTask {
  id: string;
  /** The task kind label (`start_service`, `restart_service`, …). */
  kind: string;
  /** The task lifecycle status (`queued`, `running`, `succeeded`, …). */
  status: string;
  /** The service the task acts on, when any. */
  serviceId: string | null;
  /** The workflow the task belongs to, when any. */
  workflowId: string | null;
  /** The structured failure, when the task failed. */
  failure: { code: string; message: string } | null;
  progressCurrent: number;
  progressTotal: number | null;
  progressMessage: string | null;
  /** When this record was last updated, seconds since the Unix epoch. */
  updatedAt: number;
}

/** A workflow observed on the task-event stream. */
export interface TrackedWorkflow {
  id: string;
  /** The workflow kind label (`start_services`, `StartProfile`, …). */
  kind: string;
  /** The number of steps the workflow declared. */
  stepCount: number;
  /** When this record was last updated, seconds since the Unix epoch. */
  updatedAt: number;
}

/** The value provided by [`TaskEventsProvider`]. */
export interface TaskEventsContextValue {
  /** Most recent task events, newest first. */
  recent: readonly TaskActivityEntry[];
  /** Tracked tasks, most recently updated first. */
  tasks: readonly TrackedTask[];
  /** Tracked workflows, most recently updated first. */
  workflows: readonly TrackedWorkflow[];
  /**
   * Increments whenever a task reaches a terminal state, so views can refresh
   * authoritative state exactly once per settle without polling.
   */
  terminalRevision: number;
}

/**
 * The high-frequency tracking data (`recent`, `tasks`, `workflows`).
 *
 * It changes on every task event, including progress and message frames, so it
 * is kept separate from {@link TaskRevisionContext}: a view that only reacts to
 * terminal settles subscribes to the revision alone and does not re-render for
 * every progress tick (`tasks/025.md` §24, §27).
 */
type TaskEventsData = Pick<TaskEventsContextValue, 'recent' | 'tasks' | 'workflows'>;

/** How many activity entries the feed retains. */
const MAX_ENTRIES = 50;
/** How many task records are retained. */
const MAX_TASKS = 200;
/** How many workflow records are retained. */
const MAX_WORKFLOWS = 50;

/**
 * The tracked-task map plus the reload revision, kept in one state object so a
 * terminal transition and its revision bump are applied atomically and a
 * duplicate terminal event can never bump the revision twice
 * (`tasks/024.md` §20, §24).
 */
interface TaskTrackingState {
  byId: Map<string, TrackedTask>;
  /** Increments once per task that newly reaches a terminal state. */
  revision: number;
}

const TaskEventsContext = createContext<TaskEventsData | null>(null);

/**
 * The low-frequency reload revision, kept in its own context so terminal-settle
 * consumers do not re-render for every progress event.
 */
const TaskRevisionContext = createContext<number | null>(null);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function readString(source: Record<string, unknown>, key: string): string | null {
  const value = source[key];
  return typeof value === 'string' ? value : null;
}

function readNumber(source: Record<string, unknown>, key: string): number | null {
  const value = source[key];
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

/** Reads a snake_case enum that may be a plain string or a tagged object. */
function readKind(value: unknown): string | null {
  if (typeof value === 'string') {
    return value;
  }
  if (isRecord(value)) {
    const inner = Object.values(value)[0];
    if (typeof inner === 'string') {
      return inner;
    }
    const key = Object.keys(value)[0];
    return key ?? null;
  }
  return null;
}

/** The status implied by a task-event variant, when it changes the status. */
function statusForVariant(variant: string): string | null {
  switch (variant) {
    case 'TaskStarted':
      return 'running';
    case 'TaskSucceeded':
      return 'succeeded';
    case 'TaskFailed':
      return 'failed';
    case 'TaskCancelled':
      return 'cancelled';
    case 'TaskSkipped':
      return 'skipped';
    case 'TaskBlocked':
      return 'blocked';
    case 'TaskRetrying':
      return 'running';
    default:
      return null;
  }
}

function nowSeconds(): number {
  return Math.floor(Date.now() / 1000);
}

function parseCreatedTask(task: Record<string, unknown>): TrackedTask | null {
  const id = readString(task, 'id');
  if (!id) {
    return null;
  }
  const progress = isRecord(task.progress) ? task.progress : {};
  const failure = isRecord(task.error) ? task.error : null;
  return {
    id,
    kind: readKind(task.kind) ?? 'unknown',
    status: readString(task, 'status') ?? 'unknown',
    serviceId: readString(task, 'service'),
    workflowId: readString(task, 'workflow'),
    failure: failure
      ? {
          code: readString(failure, 'code') ?? 'ERROR',
          message: readString(failure, 'message') ?? '',
        }
      : null,
    progressCurrent: readNumber(progress, 'current') ?? 0,
    progressTotal: readNumber(progress, 'total'),
    progressMessage: readString(progress, 'message'),
    updatedAt: nowSeconds(),
  };
}

/** Trims a task map to its bounded size, retiring oldest settled tasks first. */
function trimTasks(tasks: Map<string, TrackedTask>): Map<string, TrackedTask> {
  if (tasks.size <= MAX_TASKS) {
    return tasks;
  }
  const settled = [...tasks.values()]
    .filter((task) => isTerminalTaskStatus(task.status))
    .sort((a, b) => a.updatedAt - b.updatedAt);
  const trimmed = new Map(tasks);
  for (const task of settled) {
    if (trimmed.size <= MAX_TASKS) {
      break;
    }
    trimmed.delete(task.id);
  }
  return trimmed;
}

function trimWorkflows(
  workflows: Map<string, TrackedWorkflow>,
): Map<string, TrackedWorkflow> {
  if (workflows.size <= MAX_WORKFLOWS) {
    return workflows;
  }
  const ordered = [...workflows.values()].sort((a, b) => a.updatedAt - b.updatedAt);
  const trimmed = new Map(workflows);
  for (const workflow of ordered) {
    if (trimmed.size <= MAX_WORKFLOWS) {
      break;
    }
    trimmed.delete(workflow.id);
  }
  return trimmed;
}

/**
 * Receives task-engine events forwarded by the Rust adapter and keeps a small
 * in-memory feed plus structured task/workflow tracking.
 *
 * Realtime service state is driven from these real daemon events: when a task
 * settles, views refresh authoritative state once (`tasks/019.md` §22–§25).
 */
export function TaskEventsProvider({ children }: { children: ReactNode }) {
  const [recent, setRecent] = useState<TaskActivityEntry[]>([]);
  const [tracking, setTracking] = useState<TaskTrackingState>(() => ({
    byId: new Map(),
    revision: 0,
  }));
  const [workflows, setWorkflows] = useState<Map<string, TrackedWorkflow>>(new Map());

  const applyEvent = useCallback((event: TaskEvent) => {
    const variant = taskEventVariant(event);
    const body = isRecord(event[variant]) ? event[variant] : {};
    const at = nowSeconds();

    // The bounded activity feed (unchanged behaviour).
    setRecent((current) =>
      [
        {
          key: current.length === 0 ? 1 : current[0].key + 1,
          variant,
          summary: describeTaskEvent(event),
          at,
        },
        ...current,
      ].slice(0, MAX_ENTRIES),
    );

    if (variant === 'WorkflowCreated') {
      const workflow = isRecord(body.workflow) ? body.workflow : null;
      const id = workflow ? readString(workflow, 'id') : null;
      if (workflow && id) {
        const steps = Array.isArray(workflow.steps) ? workflow.steps.length : 0;
        setWorkflows((current) => {
          const next = new Map(current);
          next.set(id, {
            id,
            kind: readKind(workflow.kind) ?? 'workflow',
            stepCount: steps,
            updatedAt: at,
          });
          return trimWorkflows(next);
        });
      }
      return;
    }

    if (variant === 'TaskCreated') {
      const task = isRecord(body.task) ? body.task : null;
      if (!task) {
        return;
      }
      const parsed = parseCreatedTask(task);
      if (!parsed) {
        return;
      }
      setTracking((current) => {
        const existing = current.byId.get(parsed.id);
        const next = new Map(current.byId);
        next.set(parsed.id, parsed);
        // A freshly created task that is already terminal counts once; a
        // duplicate `TaskCreated` for an already-settled task counts nothing.
        const becameTerminal =
          isTerminalTaskStatus(parsed.status) &&
          !(existing !== undefined && isTerminalTaskStatus(existing.status));
        return {
          byId: trimTasks(next),
          revision: current.revision + (becameTerminal ? 1 : 0),
        };
      });
      return;
    }

    const id = readString(body, 'id');
    if (!id) {
      return;
    }
    const status = statusForVariant(variant);
    const progress = isRecord(body.progress) ? body.progress : null;
    const failure = isRecord(body.failure) ? body.failure : null;
    const message =
      variant === 'TaskMessage' ? readString(body, 'message') : null;

    setTracking((current) => {
      const existing = current.byId.get(id);
      // A settled task is frozen: an executor can still emit progress after a
      // terminal event, and a duplicate/out-of-order update must never move a
      // terminal task back to a non-terminal state or trigger a second reload
      // (`tasks/024.md` §20, §24).
      if (existing !== undefined && isTerminalTaskStatus(existing.status)) {
        return current;
      }
      const kind = readKind(body.kind) ?? existing?.kind ?? 'unknown';
      const next = new Map(current.byId);
      next.set(id, {
        id,
        kind,
        status: status ?? existing?.status ?? 'unknown',
        serviceId: existing?.serviceId ?? null,
        workflowId: existing?.workflowId ?? null,
        failure: failure
          ? {
              code: readString(failure, 'code') ?? existing?.failure?.code ?? 'ERROR',
              message: readString(failure, 'message') ?? existing?.failure?.message ?? '',
            }
          : existing?.failure ?? null,
        progressCurrent: progress
          ? readNumber(progress, 'current') ?? existing?.progressCurrent ?? 0
          : existing?.progressCurrent ?? 0,
        progressTotal: progress
          ? readNumber(progress, 'total') ?? existing?.progressTotal ?? null
          : existing?.progressTotal ?? null,
        progressMessage: message ?? (progress ? readString(progress, 'message') : null) ??
          existing?.progressMessage ?? null,
        updatedAt: at,
      });
      const becameTerminal = status !== null && isTerminalTaskStatus(status);
      return {
        byId: trimTasks(next),
        revision: current.revision + (becameTerminal ? 1 : 0),
      };
    });
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void desktopApi
      .onTaskEvent((event: TaskEvent) => {
        if (!disposed) {
          applyEvent(event);
        }
      })
      .then(
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
  }, [applyEvent]);

  const taskList = useMemo(
    () => [...tracking.byId.values()].sort((a, b) => b.updatedAt - a.updatedAt),
    [tracking.byId],
  );
  const workflowList = useMemo(
    () => [...workflows.values()].sort((a, b) => b.updatedAt - a.updatedAt),
    [workflows],
  );
  const revision = tracking.revision;

  const data = useMemo<TaskEventsData>(
    () => ({
      recent,
      tasks: taskList,
      workflows: workflowList,
    }),
    [recent, taskList, workflowList],
  );

  return (
    <TaskEventsContext.Provider value={data}>
      <TaskRevisionContext.Provider value={revision}>
        {children}
      </TaskRevisionContext.Provider>
    </TaskEventsContext.Provider>
  );
}

/**
 * Reads the terminal-state reload revision only.
 *
 * Views that refresh authoritative state when a task settles — but do not
 * render per-task progress — should prefer this over {@link useTaskEvents} so
 * they are not re-rendered by every progress or message frame.
 */
export function useTaskRevision(): number {
  const value = useContext(TaskRevisionContext);
  if (value === null) {
    throw new Error('useTaskRevision must be used within a TaskEventsProvider');
  }
  return value;
}

/** Reads the task-activity feed and tracked tasks. Must be used under the provider. */
export function useTaskEvents(): TaskEventsContextValue {
  const data = useContext(TaskEventsContext);
  const revision = useContext(TaskRevisionContext);
  if (!data || revision === null) {
    throw new Error('useTaskEvents must be used within a TaskEventsProvider');
  }
  return useMemo(
    () => ({ ...data, terminalRevision: revision }),
    [data, revision],
  );
}
