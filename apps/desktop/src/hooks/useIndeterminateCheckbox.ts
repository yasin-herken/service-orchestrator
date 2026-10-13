import { useEffect, useRef } from 'react';

/**
 * A checkbox ref that reflects a tri-state `indeterminate` value.
 *
 * `indeterminate` is a DOM property, not an HTML attribute, so it cannot be set
 * through JSX. Every list table with a "select all" header checkbox had its own
 * ref-and-effect pair to set it; this keeps the rule in one place.
 */
export function useIndeterminateCheckbox(indeterminate: boolean) {
  const ref = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (ref.current) {
      ref.current.indeterminate = indeterminate;
    }
  }, [indeterminate]);

  return ref;
}
