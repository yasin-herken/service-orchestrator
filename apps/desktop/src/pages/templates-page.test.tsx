import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { flush, happyPathDefaults, renderApp } from '../test/harness';
import { mockDesktopApi, resetMockDesktopApi } from '../test/mocks';
import {
  makeBulkImportPlan,
  makeConfigurationCommit,
  makeConfigurationPreview,
  makeConfigurationSnapshot,
  makeRawTemplate,
} from '../test/fixtures';

vi.mock('../services/desktopApi', async () => ({
  desktopApi: (await import('../test/mocks')).mockDesktopApi,
}));

beforeEach(() => {
  resetMockDesktopApi();
  happyPathDefaults();
});

/** Opens the Templates screen and waits for the template table. */
async function openTemplates() {
  const user = userEvent.setup();
  await renderApp();
  await user.click(screen.getByRole('button', { name: 'Templates' }));
  await screen.findByRole('table', { name: 'Service templates' });
  return user;
}

describe('template management', () => {
  it('lists the configured templates and validates the configuration', async () => {
    const user = await openTemplates();

    const table = screen.getByRole('table', { name: 'Service templates' });
    expect(within(table).getByText('Spring Boot Backend')).toBeInTheDocument();
    expect(within(table).getByText('spring-boot-backend')).toBeInTheDocument();
    expect(within(table).getByText('spring_boot_backend')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Validate configuration' }));

    await waitFor(() =>
      expect(mockDesktopApi.validateConfiguration).toHaveBeenCalledTimes(1),
    );
    expect(await screen.findByText('Valid')).toBeInTheDocument();
  });

  it('creates a template through the shared configuration review', async () => {
    const user = await openTemplates();

    await user.click(screen.getByRole('button', { name: 'New template' }));
    await user.type(screen.getByLabelText('Template identifier'), 'node-service');
    await user.type(screen.getByLabelText('Template name'), 'Node service');
    await user.click(screen.getByRole('button', { name: 'Review change…' }));

    // The daemon previews the whole proposed file before anything is written.
    expect(await screen.findByText('Create template')).toBeInTheDocument();
    await waitFor(() =>
      expect(mockDesktopApi.previewConfiguration).toHaveBeenCalledWith(
        {
          kind: 'template',
          change: {
            action: 'create',
            entity: expect.objectContaining({
              id: 'node-service',
              name: 'Node service',
            }),
          },
        },
        'rev-1',
      ),
    );

    await user.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() =>
      expect(mockDesktopApi.commitConfiguration).toHaveBeenCalledWith(
        {
          kind: 'template',
          change: {
            action: 'create',
            entity: expect.objectContaining({ id: 'node-service' }),
          },
        },
        'rev-1',
        false,
      ),
    );
    expect(await screen.findByText(/Saved configuration revision/)).toBeInTheDocument();
  });

  it('edits an existing template without renaming its id', async () => {
    const user = await openTemplates();

    await user.click(screen.getByRole('button', { name: 'Edit' }));

    expect(screen.getByLabelText('Template identifier')).toBeDisabled();
    expect(screen.getByLabelText('Template identifier')).toHaveValue(
      'spring-boot-backend',
    );

    const name = screen.getByLabelText('Template name');
    await user.clear(name);
    await user.type(name, 'Backend service');
    await user.click(screen.getByRole('button', { name: 'Review change…' }));

    await screen.findByText('Update template');
    await waitFor(() =>
      expect(mockDesktopApi.previewConfiguration).toHaveBeenCalledWith(
        expect.objectContaining({
          kind: 'template',
          change: expect.objectContaining({
            action: 'update',
            id: 'spring-boot-backend',
          }),
        }),
        'rev-1',
      ),
    );
  });

  it('removes a template with a destructive review', async () => {
    const user = await openTemplates();

    await user.click(screen.getByRole('button', { name: 'Delete' }));
    expect(await screen.findByText('Remove template')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Remove' }));

    await waitFor(() =>
      expect(mockDesktopApi.commitConfiguration).toHaveBeenCalledWith(
        {
          kind: 'template',
          change: { action: 'remove', id: 'spring-boot-backend' },
        },
        'rev-1',
        true,
      ),
    );
  });

  it('previews the resolved configuration through the real planner', async () => {
    const user = await openTemplates();

    await user.click(screen.getByRole('button', { name: 'Preview' }));
    expect(
      await screen.findByText('Preview “Spring Boot Backend”'),
    ).toBeInTheDocument();

    await user.type(screen.getByLabelText('Preview repository path'), 'backend/orders-api');
    await user.click(screen.getByRole('button', { name: 'Resolve for repository' }));

    await waitFor(() =>
      expect(mockDesktopApi.planBulkImport).toHaveBeenCalledWith({
        workspace_id: 'demo',
        repositories: [
          {
            relative_path: 'backend/orders-api',
            template: 'spring-boot-backend',
            existing: 'skip',
            overrides: {},
          },
        ],
        bulk: {},
        port_base: 8080,
      }),
    );

    const resolved = await screen.findByLabelText('Resolved configuration');
    expect(within(resolved).getAllByText(/orders-api/).length).toBeGreaterThan(0);
    // A preview never writes: the commit endpoint stays untouched.
    expect(mockDesktopApi.commitBulkImport).not.toHaveBeenCalled();
    expect(mockDesktopApi.commitConfiguration).not.toHaveBeenCalled();
  });

  it('shows the daemon validation problems for a template', async () => {
    mockDesktopApi.getConfiguration.mockResolvedValue(
      makeConfigurationSnapshot({
        templates: [makeRawTemplate({ id: 'broken', default_branch: '${unknown}' })],
      }),
    );
    mockDesktopApi.validateConfiguration.mockResolvedValue({
      revision: 'rev-1',
      valid: false,
      issues: [
        {
          file: '/Users/dev/.service-orchestrator/config.toml',
          location: 'templates[0].default_branch',
          problem: 'unknown template variable ${unknown}',
          expected: 'a documented variable',
          suggested_fix: 'use one of the documented variables',
        },
      ],
    });
    mockDesktopApi.previewConfiguration.mockResolvedValue(makeConfigurationPreview());
    mockDesktopApi.commitConfiguration.mockResolvedValue(makeConfigurationCommit());
    mockDesktopApi.planBulkImport.mockResolvedValue(makeBulkImportPlan());

    const user = await openTemplates();
    await user.click(screen.getByRole('button', { name: 'Validate' }));

    expect(
      await screen.findByText(/unknown template variable \$\{unknown\}/),
    ).toBeInTheDocument();
    // `flush` keeps the async act warning out of the assertion window.
    await flush();
  });
});
