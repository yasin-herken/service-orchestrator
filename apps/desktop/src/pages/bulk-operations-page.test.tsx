import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { happyPathDefaults, renderApp } from '../test/harness';
import { mockDesktopApi, resetMockDesktopApi } from '../test/mocks';
import { makeWorkflowStatus } from '../test/fixtures';

vi.mock('../services/desktopApi', async () => ({
  desktopApi: (await import('../test/mocks')).mockDesktopApi,
}));

beforeEach(() => {
  resetMockDesktopApi();
  happyPathDefaults();
});

/** Opens the Bulk Operations screen. */
async function openBulkOperations() {
  const user = userEvent.setup();
  await renderApp();
  await user.click(screen.getByRole('button', { name: 'Bulk Operations' }));
  await screen.findByText('Bulk operations');
  return user;
}

describe('bulk operations dashboard', () => {
  it('starts with an empty, safe selection and plans through the daemon', async () => {
    const user = await openBulkOperations();

    expect(screen.getByLabelText('Select orders-api')).not.toBeChecked();
    expect(screen.getByLabelText('Select admin-ui')).not.toBeChecked();
    expect(screen.getByRole('button', { name: 'Preview plan…' })).toBeDisabled();
    // Safe defaults: nothing destructive or long-running is pre-selected.
    expect(screen.getByLabelText('Synchronize repositories')).toBeChecked();
    expect(screen.getByLabelText('Run migrations')).not.toBeChecked();
    expect(screen.getByLabelText('Build services')).not.toBeChecked();
    expect(screen.getByLabelText('Do not change runtime state')).toBeChecked();

    await user.click(screen.getByLabelText('Select orders-api'));
    await user.click(screen.getByRole('button', { name: 'Preview plan…' }));

    expect(await screen.findByRole('region', { name: 'Workflow plan' })).toBeInTheDocument();
    expect(mockDesktopApi.planPrepareEnvironment).toHaveBeenCalledWith(
      expect.objectContaining({
        workspace_id: 'demo',
        services: ['orders-api'],
        sync: true,
        include_dependencies: true,
        build_libraries: false,
        run_liquibase: false,
        build: false,
        start: false,
        restart: false,
        stop: false,
        wait_for_health: false,
      }),
    );
    // A plan is read-only.
    expect(mockDesktopApi.prepareEnvironment).not.toHaveBeenCalled();
  });

  it('loads a saved preset without executing it', async () => {
    const user = await openBulkOperations();

    await user.selectOptions(
      screen.getByLabelText('Saved operation preset'),
      'quick-local',
    );

    expect(
      await screen.findByText(/Loaded “Quick local”/),
    ).toBeInTheDocument();
    expect(screen.getByText('loaded · not executed')).toBeInTheDocument();
    expect(mockDesktopApi.getOperationPresetRequest).toHaveBeenCalledWith(
      'demo',
      'quick-local',
    );

    // The preset's selection and flags are pre-filled…
    expect(screen.getByLabelText('Select orders-api')).toBeChecked();
    expect(screen.getByLabelText('Select admin-ui')).toBeChecked();
    expect(screen.getByLabelText('Restart services')).toBeChecked();
    expect(screen.getByLabelText('Build services')).toBeChecked();
    // …but loading never plans or runs anything.
    expect(mockDesktopApi.planPrepareEnvironment).not.toHaveBeenCalled();
    expect(mockDesktopApi.prepareEnvironment).not.toHaveBeenCalled();
  });

  it('executes only after an explicit confirmation and follows the daemon workflow', async () => {
    mockDesktopApi.getWorkflowStatus.mockResolvedValue(
      makeWorkflowStatus({
        state: 'succeeded',
        succeeded: 3,
        pending: 0,
        failed: 0,
        finished_at: 1_700_000_010,
      }),
    );
    const user = await openBulkOperations();

    await user.click(screen.getByLabelText('Select orders-api'));
    await user.click(screen.getByRole('button', { name: 'Preview plan…' }));
    await screen.findByRole('region', { name: 'Workflow plan' });

    await user.click(screen.getByRole('button', { name: 'Execute operation…' }));
    expect(
      await screen.findByText('Execute this operation?'),
    ).toBeInTheDocument();
    expect(mockDesktopApi.prepareEnvironment).not.toHaveBeenCalled();

    await user.click(screen.getByRole('button', { name: 'Execute operation' }));

    await waitFor(() =>
      expect(mockDesktopApi.prepareEnvironment).toHaveBeenCalledWith(
        expect.objectContaining({ workspace_id: 'demo', services: ['orders-api'] }),
      ),
    );

    expect(
      await screen.findByText('Results — workflow wf-env-1'),
    ).toBeInTheDocument();
    expect(mockDesktopApi.getWorkflowStatus).toHaveBeenCalledWith('wf-env-1');
  });

  it('warns before a destructive reset and never enables it implicitly', async () => {
    const user = await openBulkOperations();

    await user.click(screen.getByLabelText('Select orders-api'));
    expect(screen.queryByText(/drops database objects/)).not.toBeInTheDocument();

    await user.click(screen.getByLabelText('Run migrations'));
    await user.selectOptions(
      screen.getByLabelText('Migration preset'),
      'reset_local',
    );

    expect(screen.getByRole('alert')).toHaveTextContent(/drops database objects/);
    expect(screen.getByRole('alert')).toHaveTextContent(/explicit/i);
    expect(screen.getByLabelText('Migration preset')).toHaveValue('reset_local');
  });

  it('saves the current selection as a preset and reviews the change', async () => {
    const user = await openBulkOperations();

    await user.click(screen.getByLabelText('Select orders-api'));
    await user.click(screen.getByRole('button', { name: 'Save as preset…' }));
    await user.type(screen.getByLabelText('Preset identifier'), 'orders-only');
    await user.type(screen.getByLabelText('Preset name'), 'Orders only');
    await user.click(screen.getByRole('button', { name: 'Review change…' }));

    expect(await screen.findByText('Create operation preset')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() =>
      expect(mockDesktopApi.commitConfiguration).toHaveBeenCalledWith(
        {
          kind: 'operation_preset',
          change: {
            action: 'create',
            entity: expect.objectContaining({
              id: 'orders-only',
              name: 'Orders only',
              workspace: 'demo',
              services: ['orders-api'],
            }),
          },
        },
        'rev-1',
        false,
      ),
    );

    const entity = (
      mockDesktopApi.commitConfiguration.mock.calls[0][0] as unknown as {
        change: { entity: Record<string, unknown> };
      }
    ).change.entity;
    // Presets persist selection and flags only: no credentials, no authorization.
    expect(entity).not.toHaveProperty('credentials');
    expect(entity).not.toHaveProperty('authorized');
    expect(entity).not.toHaveProperty('token');
  });

  it('does not execute a preset automatically when the workspace loads', async () => {
    await openBulkOperations();

    expect(mockDesktopApi.prepareEnvironment).not.toHaveBeenCalled();
    expect(mockDesktopApi.planPrepareEnvironment).not.toHaveBeenCalled();
    expect(screen.getByText(/never executes it/)).toBeInTheDocument();
  });
});
