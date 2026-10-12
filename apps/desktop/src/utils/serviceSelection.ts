/**
 * Pure selection helpers for the services table.
 *
 * Selection is always a `Set<string>` of stable service identifiers — never
 * array indices — so it survives filtering, sorting, detail navigation, and
 * realtime row updates (`tasks/019.md` §7, §8).
 */

/** Returns a copy of `current` with `id` toggled. */
export function toggleId(current: ReadonlySet<string>, id: string): Set<string> {
  const next = new Set(current);
  if (next.has(id)) {
    next.delete(id);
  } else {
    next.add(id);
  }
  return next;
}

/** Returns a copy of `current` with every id in `ids` set to `selected`. */
export function setIds(
  current: ReadonlySet<string>,
  ids: Iterable<string>,
  selected: boolean,
): Set<string> {
  const next = new Set(current);
  for (const id of ids) {
    if (selected) {
      next.add(id);
    } else {
      next.delete(id);
    }
  }
  return next;
}

/** Returns a copy of `current` containing only ids still present. */
export function pruneTo(current: ReadonlySet<string>, available: Iterable<string>): Set<string> {
  const keep = new Set(available);
  const next = new Set<string>();
  for (const id of current) {
    if (keep.has(id)) {
      next.add(id);
    }
  }
  return next;
}

/** Whether every id in `ids` is selected (and `ids` is non-empty). */
export function allSelected(current: ReadonlySet<string>, ids: readonly string[]): boolean {
  return ids.length > 0 && ids.every((id) => current.has(id));
}

/** Whether at least one but not all of `ids` is selected. */
export function someSelected(current: ReadonlySet<string>, ids: readonly string[]): boolean {
  return ids.some((id) => current.has(id)) && !allSelected(current, ids);
}

/** Returns the ids selected, in the order they are given. */
export function selectedInOrder(
  current: ReadonlySet<string>,
  ids: readonly string[],
): string[] {
  return ids.filter((id) => current.has(id));
}

/** Returns a new empty selection. */
export function emptySelection(): Set<string> {
  return new Set<string>();
}
