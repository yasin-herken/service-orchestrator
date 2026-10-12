import type { ReactNode } from 'react';

import { Dialog } from './Dialog';

/**
 * A reusable confirmation dialog for operations that warrant an explicit
 * acknowledgement. The confirm button is the only way to proceed, and it is
 * disabled while the command is in flight to prevent duplicate submissions
 * (`tasks/019.md` §27, §28).
 */
export function ConfirmationDialog({
  title,
  message,
  confirmLabel,
  tone = 'default',
  busy = false,
  onConfirm,
  onCancel,
  children,
}: {
  title: string;
  message: ReactNode;
  confirmLabel: string;
  tone?: 'default' | 'danger';
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
  children?: ReactNode;
}) {
  return (
    <Dialog
      title={title}
      tone={tone}
      onClose={busy ? () => undefined : onCancel}
      footer={
        <>
          <button type="button" className="button" onClick={onCancel} disabled={busy}>
            Cancel
          </button>
          <button
            type="button"
            className={`button${tone === 'danger' ? ' button--danger' : ' button--primary'}`}
            onClick={onConfirm}
            disabled={busy}
          >
            {busy ? 'Working…' : confirmLabel}
          </button>
        </>
      }
    >
      <p className="dialog__message">{message}</p>
      {children}
    </Dialog>
  );
}
