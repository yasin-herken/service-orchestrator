import type { DesktopError } from '../types/models';

/** Codes the frontend treats specially when rendering or reacting to errors. */
export const ERROR_CODES = {
  /** The daemon is unreachable; retrying may (re)start it. */
  daemonUnavailable: 'DAEMON_UNAVAILABLE',
  /** Client and daemon speak different IPC protocol versions. */
  protocolIncompatible: 'PROTOCOL_INCOMPATIBLE',
  /** The command received input the domain rejected. */
  invalidInput: 'INVALID_INPUT',
  /** A requested entity does not exist. */
  notFound: 'NOT_FOUND',
  /** Unexpected adapter-side failure. */
  internal: 'INTERNAL',
} as const;

/** Type guard for the structured error shape produced by Tauri commands. */
export function isDesktopError(value: unknown): value is DesktopError {
  if (typeof value !== 'object' || value === null) {
    return false;
  }
  const candidate = value as Partial<DesktopError>;
  return typeof candidate.code === 'string' && typeof candidate.message === 'string';
}

/**
 * Normalizes anything thrown by the desktop API into a `DesktopError`.
 *
 * Tauri rejects with whatever `Serialize` value the command returned as its
 * error, so the structured shape arrives intact; plain `Error`s and strings
 * are mapped to `INTERNAL` without leaking stack traces to the UI.
 */
export function toDesktopError(error: unknown): DesktopError {
  if (isDesktopError(error)) {
    return { code: error.code, message: error.message };
  }
  if (error instanceof Error) {
    return { code: ERROR_CODES.internal, message: error.message };
  }
  if (typeof error === 'string') {
    return { code: ERROR_CODES.internal, message: error };
  }
  return { code: ERROR_CODES.internal, message: 'Unexpected desktop API failure.' };
}

/** Returns `true` when retrying the failed operation may succeed. */
export function isRetryable(error: DesktopError): boolean {
  return error.code === ERROR_CODES.daemonUnavailable;
}
