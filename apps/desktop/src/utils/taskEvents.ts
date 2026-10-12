import type { TaskEvent } from '../types/models';

/** Returns the externally tagged variant name of a task event. */
export function taskEventVariant(event: TaskEvent): string {
  const key = Object.keys(event)[0];
  return key ?? 'Unknown';
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function readString(source: Record<string, unknown>, key: string): string | null {
  const value = source[key];
  return typeof value === 'string' ? value : null;
}

/** A human-readable one-line summary of a task event. */
export function describeTaskEvent(event: TaskEvent): string {
  const variant = taskEventVariant(event);
  const body = event[variant];

  if (variant === 'Lagged' && isRecord(body)) {
    const dropped = typeof body.dropped === 'number' ? body.dropped : 0;
    return `${dropped} task event(s) dropped while the view was behind`;
  }
  if (!isRecord(body)) {
    return variant;
  }

  const task = isRecord(body.task) ? body.task : null;
  const failure = isRecord(body.failure) ? body.failure : null;
  const kind =
    readString(body, 'kind') ?? (task ? readString(task, 'kind') : null) ?? null;
  const detail = failure
    ? readString(failure, 'message') ?? readString(failure, 'code')
    : null;

  const label = variant.replace(/^Task/, '').replace(/([a-z])([A-Z])/g, '$1 $2').toLowerCase();
  const suffix = kind ? ` (${kind})` : '';
  const failureSuffix = detail ? `: ${detail}` : '';
  return `Task ${label}${suffix}${failureSuffix}`;
}
