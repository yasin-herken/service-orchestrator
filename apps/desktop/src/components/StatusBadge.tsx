import type { ConnectionState, HealthState, ProcessState } from '../types/models';

/** Badge tone. */
export type BadgeTone = 'neutral' | 'positive' | 'warning' | 'negative' | 'info';

const PROCESS_TONES: Record<ProcessState, BadgeTone> = {
  running: 'positive',
  starting: 'info',
  stopping: 'info',
  not_running: 'neutral',
  stopped: 'neutral',
  failed: 'negative',
  unknown: 'neutral',
};

const HEALTH_TONES: Record<HealthState, BadgeTone> = {
  healthy: 'positive',
  degraded: 'warning',
  unhealthy: 'negative',
  unreachable: 'negative',
  checking: 'info',
  unknown: 'neutral',
};

const CONNECTION_TONES: Record<ConnectionState, BadgeTone> = {
  connected: 'positive',
  connecting: 'info',
  reconnecting: 'warning',
  disconnected: 'negative',
  error: 'negative',
};

/** Human-readable labels for the process/health/connection enums. */
function labelize(value: string): string {
  return value.replace(/_/g, ' ');
}

/** A small status pill used across the dashboard and services table. */
export function StatusBadge({
  value,
  tone,
  label,
}: {
  value: string;
  tone: BadgeTone;
  label?: string;
}) {
  return (
    <span className={`badge badge--${tone}`} data-value={value}>
      {label ?? labelize(value)}
    </span>
  );
}

/** Badge for a process state. */
export function ProcessBadge({ state }: { state: ProcessState }) {
  return <StatusBadge value={state} tone={PROCESS_TONES[state]} />;
}

/** Badge for a health state. */
export function HealthBadge({ state }: { state: HealthState }) {
  return <StatusBadge value={state} tone={HEALTH_TONES[state]} />;
}

/** Badge for the daemon connection state. */
export function ConnectionBadge({ state, message }: { state: ConnectionState; message?: string | null }) {
  return (
    <span
      className={`badge badge--${CONNECTION_TONES[state]}`}
      data-value={state}
      title={message ?? undefined}
    >
      <span className={`badge__dot badge__dot--${CONNECTION_TONES[state]}`} aria-hidden="true" />
      {labelize(state)}
    </span>
  );
}
