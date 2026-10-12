import { useEffect, useRef, type ReactNode } from 'react';

/**
 * A reusable, accessible modal dialog.
 *
 * Renders an overlay and a focused dialog with `role="dialog"` and
 * `aria-modal`. Escape and a backdrop click close it; focus moves into the
 * dialog on open. Confirmation here is a user-interface affordance only — it
 * never bypasses the daemon's authorization policy (`tasks/019.md` §27, §33).
 */
export function Dialog({
  title,
  onClose,
  children,
  footer,
  tone = 'default',
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  tone?: 'default' | 'danger';
}) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        onClose();
      }
    };
    document.addEventListener('keydown', onKeyDown);
    ref.current?.focus();
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [onClose]);

  return (
    <div
      className="dialog-overlay"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <div
        ref={ref}
        className={`dialog dialog--${tone}`}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
      >
        <div className="dialog__header">
          <h2 className="dialog__title">{title}</h2>
        </div>
        <div className="dialog__body">{children}</div>
        {footer ? <div className="dialog__footer">{footer}</div> : null}
      </div>
    </div>
  );
}
