import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';

import { desktopApi } from '../services/desktopApi';
import type { ConnectionState, ConnectionSnapshot } from '../types/models';

/** The value provided by [`ConnectionProvider`]. */
export interface ConnectionContextValue {
  /** The adapter's authoritative connection state. */
  snapshot: ConnectionSnapshot;
  /**
   * Increments every time the app (re)gains a daemon connection. Data hooks
   * use it as a reload trigger so a reconnect refreshes stale state
   * (`tasks/018.md` §23).
   */
  epoch: number;
  /** Whether a user-triggered reconnect attempt is running. */
  reconnecting: boolean;
  /** Asks the adapter to connect (which may start the daemon safely). */
  reconnect: () => Promise<void>;
}

const ConnectionContext = createContext<ConnectionContextValue | null>(null);

function initialState(): ConnectionSnapshot {
  return { state: 'connecting', message: null, since_unix: Math.floor(Date.now() / 1000) };
}

/**
 * Tracks the daemon connection for the whole UI.
 *
 * The Rust adapter is the single source of truth: it pushes a
 * `ConnectionSnapshot` on every transition, and the provider pulls the current
 * snapshot once on mount for events emitted before the subscription attached.
 */
export function ConnectionProvider({ children }: { children: ReactNode }) {
  const [snapshot, setSnapshot] = useState<ConnectionSnapshot>(initialState);
  const [epoch, setEpoch] = useState(0);
  const [reconnecting, setReconnecting] = useState(false);
  const previousState = useRef<ConnectionState>('connecting');
  const eventsReceived = useRef(0);

  const apply = useCallback((next: ConnectionSnapshot) => {
    const becameConnected = previousState.current !== 'connected' && next.state === 'connected';
    previousState.current = next.state;
    setSnapshot(next);
    if (becameConnected) {
      setEpoch((value) => value + 1);
    }
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void desktopApi.onConnectionChanged((next) => {
      eventsReceived.current += 1;
      if (!disposed) {
        apply(next);
      }
    }).then(
      (stop) => {
        if (disposed) {
          stop();
        } else {
          unlisten = stop;
        }
      },
      () => {
        // Subscription failed (not running under Tauri, or the event
        // permission is missing): the initial pull below still reports state.
      },
    );

    // Pull the current snapshot in case the adapter transitioned before this
    // provider subscribed; ignore it if a fresher event arrived meanwhile.
    const before = eventsReceived.current;
    void desktopApi.getConnectionState().then(
      (state) => {
        if (!disposed && eventsReceived.current === before) {
          apply(state);
        }
      },
      () => {
        // Reported by the pages that actually call the daemon.
      },
    );

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [apply]);

  const reconnect = useCallback(async () => {
    setReconnecting(true);
    try {
      // The adapter connects (starting the daemon if needed) and pushes the
      // resulting connection state as an event.
      await desktopApi.getDaemonStatus();
    } catch {
      // The adapter already recorded and emitted the failure; page-level
      // error states surface the same structured error.
    } finally {
      setReconnecting(false);
    }
  }, []);

  const value = useMemo<ConnectionContextValue>(
    () => ({ snapshot, epoch, reconnecting, reconnect }),
    [snapshot, epoch, reconnecting, reconnect],
  );

  return <ConnectionContext.Provider value={value}>{children}</ConnectionContext.Provider>;
}

/** Reads the daemon connection state. Must be used under the provider. */
export function useConnection(): ConnectionContextValue {
  const value = useContext(ConnectionContext);
  if (!value) {
    throw new Error('useConnection must be used within a ConnectionProvider');
  }
  return value;
}
