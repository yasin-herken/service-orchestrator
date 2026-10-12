import { describe, expect, it } from 'vitest';

import { ERROR_CODES, isDesktopError, isRetryable, toDesktopError } from './errors';

describe('toDesktopError', () => {
  it('passes structured command errors through unchanged', () => {
    const error = { code: 'DAEMON_UNAVAILABLE', message: 'the daemon is not running' };
    expect(toDesktopError(error)).toEqual(error);
  });

  it('maps Error instances to INTERNAL with their message', () => {
    const result = toDesktopError(new Error('boom'));
    expect(result).toEqual({ code: ERROR_CODES.internal, message: 'boom' });
  });

  it('maps plain strings to INTERNAL', () => {
    expect(toDesktopError('kaboom')).toEqual({
      code: ERROR_CODES.internal,
      message: 'kaboom',
    });
  });

  it('never surfaces stack traces for unrecognized values', () => {
    expect(toDesktopError({ weird: true })).toEqual({
      code: ERROR_CODES.internal,
      message: 'Unexpected desktop API failure.',
    });
  });
});

describe('isDesktopError', () => {
  it('accepts objects with string code and message', () => {
    expect(isDesktopError({ code: 'X', message: 'y' })).toBe(true);
  });

  it('rejects null, primitives, and partial objects', () => {
    expect(isDesktopError(null)).toBe(false);
    expect(isDesktopError('nope')).toBe(false);
    expect(isDesktopError({ code: 'X' })).toBe(false);
    expect(isDesktopError({ message: 'y' })).toBe(false);
  });
});

describe('isRetryable', () => {
  it('treats only daemon-unavailability as retryable', () => {
    expect(isRetryable({ code: 'DAEMON_UNAVAILABLE', message: 'x' })).toBe(true);
    expect(isRetryable({ code: 'PROTOCOL_INCOMPATIBLE', message: 'x' })).toBe(false);
    expect(isRetryable({ code: 'INVALID_INPUT', message: 'x' })).toBe(false);
    expect(isRetryable({ code: 'INTERNAL', message: 'x' })).toBe(false);
  });
});
