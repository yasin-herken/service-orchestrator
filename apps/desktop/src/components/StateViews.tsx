import type { ReactNode } from 'react';

import type { DesktopError } from '../types/models';

/** A non-blocking loading indicator for a panel or page. */
export function LoadingState({ label = 'Loading…' }: { label?: string }) {
  return (
    <div className="state-block" role="status" aria-live="polite">
      <span className="state-block__spinner" aria-hidden="true" />
      <p>{label}</p>
    </div>
  );
}

/** An actionable error block for a failed daemon request. */
export function ErrorState({
  error,
  onRetry,
  retrying = false,
}: {
  error: DesktopError;
  onRetry?: () => void;
  retrying?: boolean;
}) {
  return (
    <div className="state-block state-block--error" role="alert">
      <p className="state-block__title">Something went wrong</p>
      <p className="state-block__message">{error.message}</p>
      <p className="state-block__code">{error.code}</p>
      {onRetry ? (
        <button type="button" className="button" onClick={onRetry} disabled={retrying}>
          {retrying ? 'Retrying…' : 'Retry'}
        </button>
      ) : null}
    </div>
  );
}

/** An empty-state block for lists with nothing to show. */
export function EmptyState({
  title,
  description,
  action,
}: {
  title: string;
  description?: string;
  action?: ReactNode;
}) {
  return (
    <div className="state-block state-block--empty">
      <p className="state-block__title">{title}</p>
      {description ? <p className="state-block__message">{description}</p> : null}
      {action}
    </div>
  );
}
