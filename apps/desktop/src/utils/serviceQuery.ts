import type { ServiceSummary } from '../types/models';

/** Aggregated counts shown on the dashboard. */
export interface ServiceCounts {
  total: number;
  running: number;
  stopped: number;
  unhealthy: number;
}

/** Process states that count as "stopped" for dashboard reporting. */
const STOPPED_STATES = new Set(['not_running', 'stopped', 'failed']);

/** Health states that count as "unhealthy" for dashboard reporting. */
const UNHEALTHY_STATES = new Set(['unhealthy', 'unreachable']);

/** Derives dashboard counts from real service rows (never hard-coded). */
export function countServices(services: readonly ServiceSummary[]): ServiceCounts {
  const counts: ServiceCounts = { total: services.length, running: 0, stopped: 0, unhealthy: 0 };
  for (const service of services) {
    if (service.process_state === 'running') {
      counts.running += 1;
    }
    if (STOPPED_STATES.has(service.process_state)) {
      counts.stopped += 1;
    }
    if (UNHEALTHY_STATES.has(service.health_state)) {
      counts.unhealthy += 1;
    }
  }
  return counts;
}

/** User-facing filters for the services list. */
export interface ServiceFilter {
  /** Case-insensitive substring match over id, name, and branch. */
  search: string;
  /** Service kind, or `'all'`. */
  kind: string;
}

/** Applies the toolbar filters to a service list. */
export function filterServices(
  services: readonly ServiceSummary[],
  filter: ServiceFilter,
): ServiceSummary[] {
  const needle = filter.search.trim().toLowerCase();
  return services.filter((service) => {
    if (filter.kind !== 'all' && service.kind !== filter.kind) {
      return false;
    }
    if (needle.length === 0) {
      return true;
    }
    return (
      service.id.toLowerCase().includes(needle) ||
      service.display_name.toLowerCase().includes(needle) ||
      (service.branch ?? '').toLowerCase().includes(needle)
    );
  });
}

/** Sortable columns of the services table. */
export type ServiceSortKey = 'name' | 'kind' | 'process_state' | 'health_state' | 'branch' | 'port';

/** Sort direction. */
export type SortDirection = 'asc' | 'desc';

function compareNullableString(a: string | null, b: string | null): number {
  if (a === null && b === null) return 0;
  if (a === null) return -1;
  if (b === null) return 1;
  return a.localeCompare(b);
}

function compareNullableNumber(a: number | null, b: number | null): number {
  if (a === null && b === null) return 0;
  if (a === null) return -1;
  if (b === null) return 1;
  return a - b;
}

/** Sorts a service list by one column without mutating the input. */
export function sortServices(
  services: readonly ServiceSummary[],
  key: ServiceSortKey,
  direction: SortDirection,
): ServiceSummary[] {
  const factor = direction === 'asc' ? 1 : -1;
  const sorted = [...services].sort((a, b) => {
    let result: number;
    switch (key) {
      case 'name':
        result = a.display_name.localeCompare(b.display_name);
        break;
      case 'kind':
        result = a.kind.localeCompare(b.kind);
        break;
      case 'process_state':
        result = a.process_state.localeCompare(b.process_state);
        break;
      case 'health_state':
        result = a.health_state.localeCompare(b.health_state);
        break;
      case 'branch':
        result = compareNullableString(a.branch, b.branch);
        break;
      case 'port':
        result = compareNullableNumber(a.port, b.port);
        break;
    }
    if (result === 0) {
      // Stable, deterministic tie-break on the id.
      result = a.id.localeCompare(b.id);
    }
    return result * factor;
  });
  return sorted;
}
