import { useConnection } from '../context/ConnectionContext';
import { ConnectionBadge } from './StatusBadge';

/**
 * The daemon connection indicator (badge plus, when not connected, an
 * actionable banner with a retry action) (`tasks/018.md` §19).
 */
export function ConnectionStatus() {
  const { snapshot, reconnecting, reconnect } = useConnection();
  const connected = snapshot.state === 'connected';

  return (
    <div className="connection-status">
      <ConnectionBadge state={snapshot.state} message={snapshot.message} />
      {connected ? null : (
        <div
          className={`connection-banner connection-banner--${snapshot.state}`}
          role="alert"
        >
          <span className="connection-banner__text">
            {snapshot.message ?? defaultMessage(snapshot.state)}
          </span>
          <button
            type="button"
            className="button button--small"
            onClick={() => void reconnect()}
            disabled={reconnecting}
          >
            {reconnecting ? 'Connecting…' : 'Retry connection'}
          </button>
        </div>
      )}
    </div>
  );
}

function defaultMessage(state: string): string {
  switch (state) {
    case 'connecting':
      return 'Connecting to the Service Orchestrator daemon…';
    case 'reconnecting':
      return 'Connection to the daemon was lost — reconnecting…';
    case 'disconnected':
      return 'The daemon is not running. Retry to reconnect (this starts it if needed).';
    default:
      return 'The daemon connection is in an error state.';
  }
}
