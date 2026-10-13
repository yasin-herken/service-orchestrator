import type { ServiceSummary } from '../types/models';
import { compareNullableNumber, compareNullableString, type SortDirection } from './compare';

/** Aggregated counts shown on the dashboard. */
export interface ServiceCounts {
  total: number;
  running: number;
  starting: number;
  stopping: number;
  stopped: number;
  failed: number;
  unhealthy: number;
}

/** Health states that count as "unhealthy" for dashboard reporting. */
const UNHEALTHY_STATES = new Set(['unhealthy', 'unreachable']);

/**
 * Derives dashboard counts from real service rows (never hard-coded).
 *
 * Runtime states stay distinct (`tasks/019.md` §20): `stopped` counts only the
 * resting states, while `starting`, `stopping`, and `failed` are reported
 * separately. Health is counted independently, so a running service can be
 * unhealthy (`tasks/019.md` §21).
 */
export function countServices(services: readonly ServiceSummary[]): ServiceCounts {
  const counts: ServiceCounts = {
    total: services.length,
    running: 0,
    starting: 0,
    stopping: 0,
    stopped: 0,
    failed: 0,
    unhealthy: 0,
  };
  for (const service of services) {
    switch (service.process_state) {
      case 'running':
        counts.running += 1;
        break;
      case 'starting':
        counts.starting += 1;
        break;
      case 'stopping':
        counts.stopping += 1;
        break;
      case 'stopped':
      case 'not_running':
        counts.stopped += 1;
        break;
      case 'failed':
        counts.failed += 1;
        break;
      default:
        break;
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
  /** Process state, or `'all'`/absent for every state. */
  processState?: string;
  /** Health state, or `'all'`/absent for every state. */
  healthState?: string;
}

/** Applies the toolbar filters to a service list. */
export function filterServices(
  services: readonly ServiceSummary[],
  filter: ServiceFilter,
): ServiceSummary[] {
  const needle = filter.search.trim().toLowerCase();
  const processState = filter.processState ?? 'all';
  const healthState = filter.healthState ?? 'all';
  return services.filter((service) => {
    if (filter.kind !== 'all' && service.kind !== filter.kind) {
      return false;
    }
    if (processState !== 'all' && service.process_state !== processState) {
      return false;
    }
    if (healthState !== 'all' && service.health_state !== healthState) {
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
