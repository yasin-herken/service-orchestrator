import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { flush, happyPathDefaults, renderApp } from '../test/harness';
import { emitConnection, mockDesktopApi, resetMockDesktopApi } from '../test/mocks';
import {
  makeConfigurationSnapshot,
  makeDisconnected,
  makeInvalidConfigurationPreview,
  makeProjectSuggestion,
  makeRawService,
} from '../test/fixtures';

vi.mock('../services/desktopApi', async () => ({
  desktopApi: (await import('../test/mocks')).mockDesktopApi,
}));

beforeEach(() => {
  resetMockDesktopApi();
  happyPathDefaults();
});

/** Opens the Configuration screen and waits for the snapshot to render. */
async function openConfiguration() {
  const user = userEvent.setup();
  await renderApp();
  await user.click(screen.getByRole('button', { name: 'Configuration' }));
  await screen.findByText('rev-1');
  return user;
}

/** Opens the editor for a new service and returns the dialog element. */
async function openServiceEditor(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole('button', { name: 'Add service' }));
  return screen.findByRole('dialog');
}

describe('configuration overview', () => {
  it('renders the snapshot revision, entity counts, and rows', async () => {
    await openConfiguration();

    expect(screen.getByText('rev-1')).toBeInTheDocument();
    expect(screen.getByText(/1 workspaces · 1 services · 1 libraries/)).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Edit service orders-api' }),
    ).toBeInTheDocument();
  });

  it('switches between entity tabs', async () => {
    const user = await openConfiguration();

    await user.click(screen.getByRole('tab', { name: 'Libraries' }));
    expect(
      screen.getByRole('button', { name: 'Edit library shared-kit' }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole('tab', { name: 'Profiles' }));
    expect(
      screen.getByRole('button', { name: 'Edit profile full-stack' }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole('tab', { name: 'Groups' }));
    expect(screen.getByRole('button', { name: 'Edit group core' })).toBeInTheDocument();

    await user.click(screen.getByRole('tab', { name: 'Workspaces' }));
    expect(
      screen.getByRole('button', { name: 'Edit workspace demo' }),
    ).toBeInTheDocument();
  });
});

describe('configuration editing', () => {
  it('previews and commits a new service', async () => {
    const user = await openConfiguration();
    const dialog = await openServiceEditor(user);

    await user.type(within(dialog).getByLabelText('Identifier'), 'billing-api');
    await user.click(within(dialog).getByRole('button', { name: 'Review change' }));

    // The daemon's preview renders the diff before anything is written.
    expect(await screen.findByText('Added')).toBeInTheDocument();
    expect(mockDesktopApi.previewConfiguration).toHaveBeenCalledWith(
      expect.objectContaining({ kind: 'service' }),
      'rev-1',
    );

    await user.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() =>
      expect(mockDesktopApi.commitConfiguration).toHaveBeenCalledWith(
        expect.objectContaining({
          kind: 'service',
          change: expect.objectContaining({ action: 'create' }),
        }),
        'rev-1',
        false,
      ),
    );
    expect(await screen.findByText(/create service/)).toBeInTheDocument();
  });

  it('removes an entity only after an explicit destructive confirmation', async () => {
    const user = await openConfiguration();

    await user.click(screen.getByRole('button', { name: 'Remove service orders-api' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText(/Remove service/)).toBeInTheDocument();

    await user.click(within(dialog).getByRole('button', { name: 'Remove' }));
    await waitFor(() =>
      expect(mockDesktopApi.commitConfiguration).toHaveBeenCalledWith(
        { kind: 'service', change: { action: 'remove', id: 'orders-api' } },
        'rev-1',
        true,
      ),
    );
  });

  it('creates a group from the groups tab', async () => {
    const user = await openConfiguration();

    await user.click(screen.getByRole('tab', { name: 'Groups' }));
    await user.click(screen.getByRole('button', { name: 'Add group' }));
    const dialog = await screen.findByRole('dialog');
    await user.type(within(dialog).getByLabelText('Identifier'), 'payments');
    await user.click(within(dialog).getByRole('button', { name: 'Review change' }));

    await waitFor(() =>
      expect(mockDesktopApi.previewConfiguration).toHaveBeenCalledWith(
        { kind: 'group', change: { action: 'create', entity: expect.objectContaining({ id: 'payments' }) } },
        'rev-1',
      ),
    );
  });
});

describe('configuration validation and conflicts', () => {
  it('shows validation problems and blocks saving', async () => {
    mockDesktopApi.previewConfiguration.mockResolvedValue(makeInvalidConfigurationPreview());
    const user = await openConfiguration();
    const dialog = await openServiceEditor(user);

    await user.type(within(dialog).getByLabelText('Identifier'), 'billing-api');
    await user.click(within(dialog).getByRole('button', { name: 'Review change' }));

    expect(await screen.findByText(/1 problem/)).toBeInTheDocument();
    expect(screen.getByText(/unknown workspace 'ghost'/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
  });

  it('reports a stale revision without overwriting it', async () => {
    mockDesktopApi.previewConfiguration.mockRejectedValue({
      code: 'CONFIG_CONFLICT',
      message: 'the configuration changed on disk',
    });
    const user = await openConfiguration();
    const dialog = await openServiceEditor(user);

    await user.type(within(dialog).getByLabelText('Identifier'), 'billing-api');
    await user.click(within(dialog).getByRole('button', { name: 'Review change' }));

    expect(await screen.findByText('Conflict')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Refresh configuration' }),
    ).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Save' })).not.toBeInTheDocument();
  });

  it('validates the active file on demand', async () => {
    const user = await openConfiguration();
    await user.click(screen.getByRole('button', { name: 'Validate' }));

    await waitFor(() => expect(mockDesktopApi.validateConfiguration).toHaveBeenCalled());
    expect(await screen.findByText('Valid')).toBeInTheDocument();
  });

  it('renders a valid result when the daemon omits the empty issues list', async () => {
    // The daemon omits `issues` when the file is valid
    // (`skip_serializing_if = "Vec::is_empty"`); the panel must treat a missing
    // list as empty instead of crashing the whole page.
    mockDesktopApi.validateConfiguration.mockResolvedValue({
      revision: 'rev-1',
      valid: true,
    });
    const user = await openConfiguration();
    await user.click(screen.getByRole('button', { name: 'Validate' }));

    expect(await screen.findByText('Valid')).toBeInTheDocument();
  });
});

describe('repository inspection', () => {
  it('shows read-only detection, including a gradle provider gap', async () => {
    const user = await openConfiguration();
    const dialog = await openServiceEditor(user);

    await user.click(within(dialog).getByRole('button', { name: 'Detect configuration' }));

    expect(await screen.findByText('provider not installed')).toBeInTheDocument();
    expect(
      screen.getByText(/Gradle was detected but its execution provider is not installed/),
    ).toBeInTheDocument();
    expect(mockDesktopApi.detectProjectConfiguration).toHaveBeenCalled();
  });

  it('applies a detected build command to the form', async () => {
    mockDesktopApi.detectProjectConfiguration.mockResolvedValue(makeProjectSuggestion());
    const user = await openConfiguration();
    const dialog = await openServiceEditor(user);

    await user.click(within(dialog).getByRole('button', { name: 'Detect configuration' }));
    await screen.findByText('Suggested');
    await user.click(within(dialog).getByRole('button', { name: 'Apply suggestion' }));

    expect(within(dialog).getByLabelText('Build command')).toHaveValue(
      './mvnw clean install',
    );
  });
});

describe('secret handling', () => {
  it('masks a secret environment value and preserves it', async () => {
    mockDesktopApi.getConfiguration.mockResolvedValue(
      makeConfigurationSnapshot({
        services: [
          makeRawService({
            environment: { API_TOKEN: { value: 's3cr3t', secret: true } },
          }),
        ],
      }),
    );
    const user = await openConfiguration();

    await user.click(screen.getByRole('button', { name: 'Edit service orders-api' }));
    const dialog = await screen.findByRole('dialog');
    const value = within(dialog).getByLabelText('Environment value 1');

    expect(value).toHaveAttribute('type', 'password');
    expect(value).toHaveValue('s3cr3t');

    await user.click(within(dialog).getByLabelText('Reveal environment value 1'));
    expect(value).toHaveAttribute('type', 'text');
  });
});

describe('connection state', () => {
  it('disables editing while the daemon is disconnected', async () => {
    await openConfiguration();

    act(() => emitConnection(makeDisconnected('the daemon is not running')));
    await flush();

    expect(
      await screen.findByText(/Configuration editing is disabled/),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Add service' })).toBeDisabled();
  });
});
