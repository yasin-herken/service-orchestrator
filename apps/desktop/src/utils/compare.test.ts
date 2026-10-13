import { describe, expect, it } from 'vitest';

import { compareNullableNumber, compareNullableString } from './compare';

describe('compareNullableString', () => {
  it('sorts null before any value', () => {
    expect(compareNullableString(null, 'a')).toBeLessThan(0);
    expect(compareNullableString('a', null)).toBeGreaterThan(0);
    expect(compareNullableString(null, null)).toBe(0);
  });

  it('orders two values lexically', () => {
    expect(compareNullableString('a', 'b')).toBeLessThan(0);
    expect(compareNullableString('b', 'a')).toBeGreaterThan(0);
    expect(compareNullableString('a', 'a')).toBe(0);
  });
});

describe('compareNullableNumber', () => {
  it('sorts null before any value', () => {
    expect(compareNullableNumber(null, 1)).toBeLessThan(0);
    expect(compareNullableNumber(1, null)).toBeGreaterThan(0);
    expect(compareNullableNumber(null, null)).toBe(0);
  });

  it('orders two values numerically', () => {
    expect(compareNullableNumber(1, 2)).toBeLessThan(0);
    expect(compareNullableNumber(2, 1)).toBeGreaterThan(0);
    expect(compareNullableNumber(2, 2)).toBe(0);
  });
});
