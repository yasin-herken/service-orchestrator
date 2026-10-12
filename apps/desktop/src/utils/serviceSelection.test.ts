import { describe, expect, it } from 'vitest';

import {
  allSelected,
  emptySelection,
  pruneTo,
  selectedInOrder,
  setIds,
  someSelected,
  toggleId,
} from './serviceSelection';

describe('serviceSelection', () => {
  it('toggles a single id without mutating the input', () => {
    const start = emptySelection();
    const added = toggleId(start, 'orders-api');
    expect([...added]).toEqual(['orders-api']);
    expect(start.size).toBe(0);

    const removed = toggleId(added, 'orders-api');
    expect(removed.size).toBe(0);
  });

  it('sets many ids at once, both adding and removing', () => {
    const base = new Set(['a']);
    const added = setIds(base, ['b', 'c'], true);
    expect([...added].sort()).toEqual(['a', 'b', 'c']);
    const removed = setIds(added, ['a', 'c'], false);
    expect([...removed]).toEqual(['b']);
  });

  it('prunes ids that are no longer available', () => {
    const selected = new Set(['a', 'b', 'c']);
    expect([...pruneTo(selected, ['b', 'c', 'd'])].sort()).toEqual(['b', 'c']);
  });

  it('reports all/some selection over a visible set', () => {
    const selected = new Set(['a', 'b']);
    expect(allSelected(selected, ['a', 'b'])).toBe(true);
    expect(allSelected(selected, ['a', 'b', 'c'])).toBe(false);
    expect(someSelected(selected, ['a', 'b', 'c'])).toBe(true);
    expect(someSelected(selected, ['c'])).toBe(false);
    expect(allSelected(emptySelection(), [])).toBe(false);
  });

  it('returns selected ids in the supplied order', () => {
    const selected = new Set(['c', 'a']);
    expect(selectedInOrder(selected, ['a', 'b', 'c'])).toEqual(['a', 'c']);
  });
});
