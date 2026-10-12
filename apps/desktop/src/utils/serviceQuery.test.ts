import { describe, expect, it } from 'vitest';

import { makeService } from '../test/fixtures';
import { countServices, filterServices, sortServices } from './serviceQuery';

describe('countServices', () => {
  it('derives dashboard counts from real rows', () => {
    const services = [
      makeService({ id: 'a', process_state: 'running', health_state: 'healthy' }),
      makeService({ id: 'b', process_state: 'not_running', health_state: 'unknown' }),
      makeService({ id: 'c', process_state: 'stopped', health_state: 'unhealthy' }),
      makeService({ id: 'd', process_state: 'starting', health_state: 'checking' }),
      makeService({ id: 'e', process_state: 'failed', health_state: 'unreachable' }),
    ];
    expect(countServices(services)).toEqual({
      total: 5,
      running: 1,
      stopped: 3,
      unhealthy: 2,
    });
  });

  it('returns zeroes for an empty list', () => {
    expect(countServices([])).toEqual({ total: 0, running: 0, stopped: 0, unhealthy: 0 });
  });

  it('does not count transitional states as stopped', () => {
    const services = [
      makeService({ id: 'a', process_state: 'starting' }),
      makeService({ id: 'b', process_state: 'stopping' }),
      makeService({ id: 'c', process_state: 'unknown' }),
    ];
    expect(countServices(services).stopped).toBe(0);
  });
});

describe('filterServices', () => {
  const services = [
    makeService({ id: 'orders-api', display_name: 'Orders API', branch: 'main' }),
    makeService({ id: 'admin-ui', display_name: 'Admin UI', kind: 'frontend' }),
    makeService({ id: 'billing', display_name: 'Billing', branch: 'feature/pay' }),
  ];

  it('matches the search text case-insensitively across id, name, and branch', () => {
    expect(filterServices(services, { search: 'ORDERS', kind: 'all' }).map((s) => s.id)).toEqual([
      'orders-api',
    ]);
    expect(filterServices(services, { search: 'admin ui', kind: 'all' }).map((s) => s.id)).toEqual([
      'admin-ui',
    ]);
    expect(filterServices(services, { search: 'feature/', kind: 'all' }).map((s) => s.id)).toEqual([
      'billing',
    ]);
  });

  it('filters by service kind', () => {
    expect(filterServices(services, { search: '', kind: 'frontend' }).map((s) => s.id)).toEqual([
      'admin-ui',
    ]);
  });

  it('combines kind and search filters', () => {
    expect(filterServices(services, { search: 'admin', kind: 'backend' })).toEqual([]);
    expect(filterServices(services, { search: 'admin', kind: 'frontend' }).map((s) => s.id)).toEqual([
      'admin-ui',
    ]);
  });

  it('treats a blank search as a match', () => {
    expect(filterServices(services, { search: '   ', kind: 'all' })).toHaveLength(3);
  });
});

describe('sortServices', () => {
  const services = [
    makeService({ id: 'b', display_name: 'Bravo', port: 8081, branch: 'main' }),
    makeService({ id: 'a', display_name: 'Alpha', port: 3000, branch: null }),
    makeService({ id: 'c', display_name: 'Charlie', port: null, branch: 'dev' }),
  ];

  it('sorts ascending and descending by name', () => {
    expect(sortServices(services, 'name', 'asc').map((s) => s.id)).toEqual(['a', 'b', 'c']);
    expect(sortServices(services, 'name', 'desc').map((s) => s.id)).toEqual(['c', 'b', 'a']);
  });

  it('sorts numeric columns with nulls first in ascending order', () => {
    expect(sortServices(services, 'port', 'asc').map((s) => s.id)).toEqual(['c', 'a', 'b']);
    expect(sortServices(services, 'port', 'desc').map((s) => s.id)).toEqual(['b', 'a', 'c']);
  });

  it('breaks ties deterministically on the id', () => {
    const tied = [
      makeService({ id: 'z', display_name: 'Same' }),
      makeService({ id: 'y', display_name: 'Same' }),
    ];
    expect(sortServices(tied, 'name', 'asc').map((s) => s.id)).toEqual(['y', 'z']);
  });

  it('does not mutate the input list', () => {
    const input = [...services];
    sortServices(input, 'name', 'desc');
    expect(input.map((s) => s.id)).toEqual(['b', 'a', 'c']);
  });
});
