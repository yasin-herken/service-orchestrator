/**
 * Shared table-ordering helpers.
 *
 * Several list screens (services, repositories) sort the same shapes — an
 * optional string (a branch, a path) and an optional number (a port) — with
 * `null` always sorting first. Keeping the comparators in one place means the
 * rule is defined once instead of being re-implemented per query module.
 */

/** Sort direction shared by every sortable table. */
export type SortDirection = 'asc' | 'desc';

/** Orders two nullable strings, with `null` sorting first. */
export function compareNullableString(a: string | null, b: string | null): number {
  if (a === null && b === null) return 0;
  if (a === null) return -1;
  if (b === null) return 1;
  return a.localeCompare(b);
}

/** Orders two nullable numbers, with `null` sorting first. */
export function compareNullableNumber(a: number | null, b: number | null): number {
  if (a === null && b === null) return 0;
  if (a === null) return -1;
  if (b === null) return 1;
  return a - b;
}
