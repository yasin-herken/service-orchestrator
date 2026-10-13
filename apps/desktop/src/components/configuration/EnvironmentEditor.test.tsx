import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { EnvironmentEditor } from './EnvironmentEditor';

describe('EnvironmentEditor', () => {
  it('masks a secret until it is revealed, then hides it again', async () => {
    const user = userEvent.setup();
    render(
      <EnvironmentEditor
        value={{ TOKEN: { value: 'super-secret', secret: true } }}
        onChange={() => {}}
      />,
    );

    const value = screen.getByLabelText('Environment value 1');
    expect(value).toHaveAttribute('type', 'password');

    await user.click(screen.getByRole('button', { name: 'Reveal environment value 1' }));
    expect(screen.getByLabelText('Environment value 1')).toHaveAttribute('type', 'text');

    await user.click(screen.getByRole('button', { name: 'Hide environment value 1' }));
    expect(screen.getByLabelText('Environment value 1')).toHaveAttribute('type', 'password');
  });

  it('does not leak reveal state onto another row when a row is removed', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <EnvironmentEditor
        value={{
          A: { value: 'secret-a', secret: true },
          B: { value: 'secret-b', secret: true },
        }}
        onChange={onChange}
      />,
    );

    // Reveal only the first secret.
    await user.click(screen.getByRole('button', { name: 'Reveal environment value 1' }));
    expect(screen.getByLabelText('Environment value 1')).toHaveAttribute('type', 'text');
    expect(screen.getByLabelText('Environment value 2')).toHaveAttribute('type', 'password');

    // Removing the first row shifts the remaining one to position 1; its secret
    // must stay masked rather than inheriting the removed row's reveal flag.
    await user.click(screen.getByRole('button', { name: 'Remove environment variable 1' }));

    expect(screen.getByLabelText('Environment value 1')).toHaveAttribute('type', 'password');
    expect(onChange).toHaveBeenLastCalledWith({ B: { value: 'secret-b', secret: true } });
  });

  it('reports edits back in the wire shape, dropping empty names', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<EnvironmentEditor value={{}} onChange={onChange} />);

    await user.click(screen.getByRole('button', { name: 'Add variable' }));
    await user.type(screen.getByLabelText('Environment name 1'), 'API_URL');
    await user.type(screen.getByLabelText('Environment value 1'), 'http://localhost:8081');

    expect(onChange).toHaveBeenLastCalledWith({
      API_URL: 'http://localhost:8081',
    });
  });
});
