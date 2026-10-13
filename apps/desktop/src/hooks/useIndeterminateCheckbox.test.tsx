import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';

import { useIndeterminateCheckbox } from './useIndeterminateCheckbox';

function Harness({ indeterminate }: { indeterminate: boolean }) {
  const ref = useIndeterminateCheckbox(indeterminate);
  return <input type="checkbox" ref={ref} aria-label="select all" />;
}

describe('useIndeterminateCheckbox', () => {
  it('reflects the indeterminate DOM property, not the checked attribute', () => {
    const { rerender } = render(<Harness indeterminate />);
    const checkbox = screen.getByLabelText('select all') as HTMLInputElement;

    expect(checkbox.indeterminate).toBe(true);
    expect(checkbox.checked).toBe(false);

    rerender(<Harness indeterminate={false} />);
    expect(checkbox.indeterminate).toBe(false);
  });
});
