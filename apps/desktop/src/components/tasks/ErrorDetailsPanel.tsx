/** One structured error rendered by [`ErrorDetailsPanel`]. */
export interface ErrorDetail {
  /** The affected entity's label, when the error is per-item. */
  label?: string | null;
  /** Stable machine-readable code. */
  code: string;
  /** Human-readable, user-safe message. */
  message: string;
  /** Optional additional detail (for example a process exit code). */
  detail?: string | null;
}

/**
 * Renders one or more structured errors with their codes preserved.
 *
 * It never renders a stack trace or secret, and it never claims a partial
 * success was complete (`tasks/020.md` §30).
 */
export function ErrorDetailsPanel({
  errors,
  emptyLabel = 'No errors.',
}: {
  errors: readonly ErrorDetail[];
  emptyLabel?: string;
}) {
  if (errors.length === 0) {
    return <p className="muted">{emptyLabel}</p>;
  }
  return (
    <ul className="error-details" role="list">
      {errors.map((error, index) => (
        <li className="error-details__item" key={`${error.code}-${index}`}>
          {error.label ? <p className="error-details__label">{error.label}</p> : null}
          <p className="error-details__message">{error.message}</p>
          <p className="error-details__code">
            <code className="dialog__code">{error.code}</code>
            {error.detail ? <span className="muted"> — {error.detail}</span> : null}
          </p>
        </li>
      ))}
    </ul>
  );
}
