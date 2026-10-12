import { useCallback, useEffect, useRef, useState, type DependencyList } from 'react';

import { toDesktopError } from '../services/errors';
import type { DesktopError } from '../types/models';

/** The value returned by [`useAsync`]. */
export interface AsyncResource<T> {
  /** Last successful result, retained across reloads. */
  data: T | null;
  /** Whether a request is in flight. */
  loading: boolean;
  /** Structured failure of the last request. */
  error: DesktopError | null;
  /** Re-runs the loader (e.g. from a Retry button). */
  reload: () => void;
}

/**
 * Loads server state from the daemon with loading, error, and stale-guard
 * semantics (`tasks/018.md` §21, §23).
 *
 * - The loader only runs when `enabled` is true; disabling (e.g. while the
 *   daemon is unreachable) keeps the last result but stops showing a spinner.
 * - Re-enabling or changing `deps` refetches exactly once — a reconnect
 *   therefore refreshes state without duplicate requests.
 * - Responses that arrive after the inputs changed or after unmount are
 *   discarded, so a slow IPC reply can never overwrite fresher state.
 */
export function useAsync<T>(
  loader: () => Promise<T>,
  deps: DependencyList,
  options: { enabled?: boolean } = {},
): AsyncResource<T> {
  const enabled = options.enabled ?? true;
  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState(enabled);
  const [error, setError] = useState<DesktopError | null>(null);
  const [nonce, setNonce] = useState(0);

  // Keep the latest loader without making its identity part of the effect
  // dependencies: callers pass inline closures that change every render.
  const loaderRef = useRef(loader);
  useEffect(() => {
    loaderRef.current = loader;
  });

  useEffect(() => {
    if (!enabled) {
      setLoading(false);
      return undefined;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    loaderRef.current().then(
      (result) => {
        if (!cancelled) {
          setData(result);
          setLoading(false);
        }
      },
      (cause: unknown) => {
        if (!cancelled) {
          setError(toDesktopError(cause));
          setLoading(false);
        }
      },
    );
    return () => {
      cancelled = true;
    };
    // The loader itself is tracked via loaderRef; `deps` are the inputs.
  }, [enabled, nonce, ...deps]);

  const reload = useCallback(() => setNonce((value) => value + 1), []);

  return { data, loading, error, reload };
}
