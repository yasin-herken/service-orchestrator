import { taskStatusLabel } from './taskStatus';

/** Formats a Unix timestamp (seconds) as a local `HH:MM` clock string. */
export function formatClock(unixSeconds: number): string {
  const date = new Date(unixSeconds * 1000);
  if (Number.isNaN(date.getTime())) {
    return '—';
  }
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

/** Formats a duration in seconds as a compact `1h 12m` / `4m 05s` string. */
export function formatDuration(totalSeconds: number): string {
  if (!Number.isFinite(totalSeconds) || totalSeconds < 0) {
    return '—';
  }
  const seconds = Math.floor(totalSeconds % 60);
  const minutes = Math.floor((totalSeconds / 60) % 60);
  const hours = Math.floor(totalSeconds / 3600);
  if (hours > 0) {
    return `${hours}h ${minutes}m`;
  }
  if (minutes > 0) {
    return `${minutes}m ${String(seconds).padStart(2, '0')}s`;
  }
  return `${seconds}s`;
}

/** Pluralizes `word` for `count` (simple English plural). */
export function pluralize(count: number, word: string): string {
  return `${count} ${word}${count === 1 ? '' : 's'}`;
}

/** Formats a Unix timestamp (seconds) as a local date and time, or `—`. */
export function formatDateTime(unixSeconds: number | null): string {
  if (unixSeconds === null || unixSeconds === undefined) {
    return '—';
  }
  const date = new Date(unixSeconds * 1000);
  if (Number.isNaN(date.getTime())) {
    return '—';
  }
  return date.toLocaleString([], {
    year: 'numeric',
    month: 'short',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
}

/** Formats a duration in milliseconds as a compact human string. */
export function formatMillis(millis: number | null): string {
  if (millis === null || !Number.isFinite(millis)) {
    return '—';
  }
  if (millis < 1_000) {
    return `${Math.round(millis)} ms`;
  }
  return formatDuration(millis / 1_000);
}

/**
 * A compact "last operation" label for a tracked task: its kind and status
 * (`build_service · Succeeded`).
 *
 * One shared implementation so the services, Git, builds, and libraries tables
 * never drift from each other (`tasks/024.md` §5, §12).
 */
export function operationLabel(task: { kind: string; status: string }): string {
  return `${task.kind} · ${taskStatusLabel(task.status)}`;
}
