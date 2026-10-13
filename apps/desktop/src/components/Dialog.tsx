import { useEffect, useRef, type ReactNode } from 'react';

/**
 * Selector for the elements a dialog may move focus to.
 *
 * Disabled controls are excluded so the Tab ring never lands on an element the
 * user cannot act on.
 */
const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** Returns the focusable descendants of `node`, in document order. */
function focusableWithin(node: HTMLElement | null): HTMLElement[] {
  if (!node) {
    return [];
  }
  return Array.from(node.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR));
}

/**
 * A reusable, accessible modal dialog.
 *
 * Renders an overlay and a focused dialog with `role="dialog"` and
 * `aria-modal`. Escape and a backdrop click close it. On open, focus moves to
 * the first focusable control and Tab is trapped inside the dialog so a
 * keyboard user cannot tab behind the modal; closing restores focus to the
 * element that opened it (`tasks/024.md` §9, §34). Confirmation here is a
 * user-interface affordance only — it never bypasses the daemon's
 * authorization policy (`tasks/019.md` §27, §33).
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
  const restoreFocusRef = useRef<HTMLElement | null>(null);

  // Move focus into the dialog on open and restore it on close.
  useEffect(() => {
    const previous = document.activeElement;
    restoreFocusRef.current = previous instanceof HTMLElement ? previous : null;

    const node = ref.current;
    const focusables = focusableWithin(node);
    if (focusables.length > 0) {
      focusables[0].focus();
    } else {
      node?.focus();
    }

    return () => {
      const previousElement = restoreFocusRef.current;
      if (previousElement && previousElement.isConnected) {
        previousElement.focus();
      }
    };
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        onClose();
        return;
      }
      if (event.key !== 'Tab') {
        return;
      }
      const node = ref.current;
      if (!node) {
        return;
      }
      const focusables = focusableWithin(node);
      if (focusables.length === 0) {
        event.preventDefault();
        node.focus();
        return;
      }
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      const active = document.activeElement;
      if (event.shiftKey && (active === first || active === node)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener('keydown', onKeyDown);
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
