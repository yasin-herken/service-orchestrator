import type { ReactNode } from 'react';

import { Dialog } from './Dialog';
import type { DesktopError } from '../types/models';

/**
 * A modal that surfaces a structured operation failure.
 *
 * It preserves the machine code and the human message, names the affected
 * service(s), and never renders a stack trace or secret (`tasks/019.md` §26).
 */
export function ErrorDialog({
  title,
  error,
  labels,
  hint,
  onClose,
}: {
  title: string;
  error: DesktopError;
  labels: readonly string[];
  /** Optional actionable guidance rendered under the reason (e.g. how to satisfy an authorization policy). */
  hint?: ReactNode;
  onClose: () => void;
}) {
  return (
    <Dialog
      title={title}
      tone="danger"
      onClose={onClose}
      footer={
        <button type="button" className="button" onClick={onClose}>
          Close
        </button>
      }
    >
      <dl className="dialog__facts">
        {labels.length > 0 ? (
          <div>
            <dt>{labels.length === 1 ? 'Service' : 'Services'}</dt>
            <dd>{labels.join(', ')}</dd>
          </div>
        ) : null}
        <div>
          <dt>Reason</dt>
          <dd>{error.message}</dd>
        </div>
        <div>
          <dt>Code</dt>
          <dd className="dialog__code">{error.code}</dd>
        </div>
      </dl>
      {hint ? <p className="dialog__note">{hint}</p> : null}
      <p className="dialog__note muted">
        The daemon rejected or could not complete the operation. No state was
        changed optimistically.
      </p>
    </Dialog>
  );
}
