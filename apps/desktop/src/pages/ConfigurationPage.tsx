import { useCallback, useEffect, useMemo, useState } from 'react';

import { SaveConfirmationDialog } from '../components/configuration/SaveConfirmationDialog';
import { ConfigurationValidationPanel } from '../components/configuration/ConfigurationValidationPanel';
import { GroupEditor } from '../components/configuration/GroupEditor';
import { LibraryEditor } from '../components/configuration/LibraryEditor';
import { ProfileEditor } from '../components/configuration/ProfileEditor';
import { ServiceEditor } from '../components/configuration/ServiceEditor';
import { WorkspaceEditor } from '../components/configuration/WorkspaceEditor';
import { EmptyState, ErrorState, LoadingState } from '../components/StateViews';
import { useConnection } from '../context/ConnectionContext';
import { useWorkspaces } from '../context/WorkspaceContext';
import { useAsync } from '../hooks/useAsync';
import { desktopApi } from '../services/desktopApi';
import { ERROR_CODES, toDesktopError } from '../services/errors';
import type {
  ConfigChangeView,
  ConfigurationPreviewView,
  ConfigurationSnapshotView,
  ConfigurationValidationView,
  DesktopError,
  EntityKindView,
  RawGroupView,
  RawLibraryView,
  RawProfileView,
  RawServiceView,
  RawWorkspaceView,
} from '../types/models';

/**
 * The entity kinds this screen edits directly.
 *
 * Templates and operation presets have dedicated screens so their domain
 * guidance (variables, resolution preview, preset safety) is not duplicated
 * here; migration presets are declared in the configuration file and validated
 * by the daemon. The narrower union keeps the editor switch exhaustive.
 */
type ConfigTab = Exclude<EntityKindView, 'template' | 'migration_preset' | 'operation_preset'>;

/** Human-readable tab labels. */
const TAB_LABELS: Record<ConfigTab, string> = {
  workspace: 'Workspaces',
  service: 'Services',
  library: 'Libraries',
  profile: 'Profiles',
  group: 'Groups',
};

/** A row shown in the active tab's list. */
interface ConfigRow {
  id: string;
  name: string;
}

/** An open editor dialog. */
type EditorState =
  | { kind: 'workspace'; entity: RawWorkspaceView | null }
  | { kind: 'service'; entity: RawServiceView | null }
  | { kind: 'library'; entity: RawLibraryView | null }
  | { kind: 'profile'; entity: RawProfileView | null }
  | { kind: 'group'; entity: RawGroupView | null };

/** A change awaiting preview and confirmation. */
interface PendingChange {
  change: ConfigChangeView;
  destructive: boolean;
  title: string;
  message: string;
}

/** Filters the workspace-owned entities for the active workspace. */
function scoped<T extends { workspace?: string }>(
  entities: readonly T[],
  workspaceId: string | null,
): T[] {
  if (workspaceId === null) {
    return [];
  }
  return entities.filter(
    (entity) => entity.workspace === undefined || entity.workspace === workspaceId,
  );
}

/**
 * The Configuration Management screen (`tasks/023.md` §10–§41).
 *
 * The GUI parses no TOML and writes no file: it reads a raw snapshot from the
 * shared daemon, builds a typed change, reviews the daemon's diff/validation,
 * and commits it. The daemon is the sole writer and re-validates and re-checks
 * the revision before any atomic save, so a stale editor can never overwrite a
 * newer configuration and a frontend confirmation is never treated as
 * authorization (`tasks/023.md` §30, §38, §47).
 */
export function ConfigurationPage({
  onNavigate,
}: {
  onNavigate?: (page: 'templates' | 'bulk-operations') => void;
} = {}) {
  const { snapshot: connection } = useConnection();
  const connected = connection.state === 'connected';
  const {
    activeWorkspaceId,
    reload: reloadWorkspaces,
  } = useWorkspaces();

  const [reloadNonce, setReloadNonce] = useState(0);
  const snapshotResource = useAsync<ConfigurationSnapshotView>(
    () => desktopApi.getConfiguration(),
    [reloadNonce],
    { enabled: connected },
  );
  const snapshot = snapshotResource.data;

  const [tab, setTab] = useState<ConfigTab>('service');
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [pending, setPending] = useState<PendingChange | null>(null);

  const [preview, setPreview] = useState<ConfigurationPreviewView | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewError, setPreviewError] = useState<DesktopError | null>(null);
  const [conflict, setConflict] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<DesktopError | null>(null);

  const [validation, setValidation] = useState<ConfigurationValidationView | null>(null);
  const [validating, setValidating] = useState(false);
  const [validationError, setValidationError] = useState<string | null>(null);

  const [status, setStatus] = useState<string | null>(null);

  const refresh = useCallback(() => {
    setReloadNonce((value) => value + 1);
    reloadWorkspaces();
  }, [reloadWorkspaces]);

  // The daemon broadcasts every committed change; reload the snapshot and the
  // workspace list so the whole GUI converges on the same configuration.
  useEffect(() => {
    if (!connected) {
      return undefined;
    }
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void desktopApi
      .onConfigurationChanged(() => {
        if (!cancelled) {
          refresh();
        }
      })
      .then((stop) => {
        if (cancelled) {
          stop();
        } else {
          unlisten = stop;
        }
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [connected, refresh]);

  // Preview the pending change against the current revision; stale replies are
  // discarded when the pending change or revision moves on.
  useEffect(() => {
    if (!pending) {
      setPreview(null);
      setPreviewError(null);
      setConflict(false);
      setPreviewLoading(false);
      return undefined;
    }
    let cancelled = false;
    setPreviewLoading(true);
    setPreview(null);
    setPreviewError(null);
    setConflict(false);
    desktopApi.previewConfiguration(pending.change, snapshot?.revision ?? '').then(
      (result) => {
        if (!cancelled) {
          setPreview(result);
          setPreviewLoading(false);
        }
      },
      (cause: unknown) => {
        if (!cancelled) {
          const error = toDesktopError(cause);
          setPreviewError(error);
          setConflict(error.code === ERROR_CODES.configConflict);
          setPreviewLoading(false);
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, [pending, snapshot?.revision]);

  const workspaceRoot =
    snapshot?.workspaces.find((workspace) => workspace.id === activeWorkspaceId)?.root ?? '';

  const rows = useMemo<ConfigRow[]>(() => {
    if (!snapshot) {
      return [];
    }
    switch (tab) {
      case 'workspace':
        return snapshot.workspaces.map((w) => ({ id: w.id, name: w.name ?? w.id }));
      case 'service':
        return scoped(snapshot.services, activeWorkspaceId).map((s) => ({
          id: s.id,
          name: s.name ?? s.id,
        }));
      case 'library':
        return scoped(snapshot.libraries, activeWorkspaceId).map((l) => ({
          id: l.id,
          name: l.name ?? l.id,
        }));
      case 'profile':
        return scoped(snapshot.profiles, activeWorkspaceId).map((p) => ({
          id: p.id,
          name: p.name ?? p.id,
        }));
      case 'group':
      default:
        return scoped(snapshot.groups, activeWorkspaceId).map((g) => ({
          id: g.id,
          name: g.name ?? g.id,
        }));
    }
  }, [snapshot, tab, activeWorkspaceId]);

  const openEditor = useCallback(
    (kind: ConfigTab, id: string | null) => {
      if (!snapshot) {
        return;
      }
      switch (kind) {
        case 'workspace':
          setEditor({
            kind,
            entity: snapshot.workspaces.find((w) => w.id === id) ?? null,
          });
          break;
        case 'service':
          setEditor({
            kind,
            entity: snapshot.services.find((s) => s.id === id) ?? null,
          });
          break;
        case 'library':
          setEditor({
            kind,
            entity: snapshot.libraries.find((l) => l.id === id) ?? null,
          });
          break;
        case 'profile':
          setEditor({
            kind,
            entity: snapshot.profiles.find((p) => p.id === id) ?? null,
          });
          break;
        case 'group':
        default:
          setEditor({
            kind: 'group',
            entity: snapshot.groups.find((g) => g.id === id) ?? null,
          });
          break;
      }
    },
    [snapshot],
  );

  const reviewChange = useCallback((change: ConfigChangeView, options: { destructive: boolean; title: string; message: string }) => {
    setEditor(null);
    setSaveError(null);
    setPending({ change, ...options });
  }, []);

  const remove = useCallback(
    (kind: ConfigTab, id: string, name: string) => {
      reviewChange(
        { kind, change: { action: 'remove', id } } as ConfigChangeView,
        {
          destructive: true,
          title: `Remove ${kind} “${name}”`,
          message:
            'This removes the declaration from the configuration file. Running processes are left untouched and any entity that still references it is reported instead of being silently changed. The daemon verifies the request independently; your confirmation is intent, not authorization.',
        },
      );
    },
    [reviewChange],
  );

  const confirmSave = useCallback(async () => {
    if (!pending || !snapshot) {
      return;
    }
    setSaving(true);
    setSaveError(null);
    try {
      const result = await desktopApi.commitConfiguration(
        pending.change,
        snapshot.revision,
        pending.destructive,
      );
      setPending(null);
      setStatus(`${result.description} · revision ${result.revision}`);
      refresh();
    } catch (cause) {
      const error = toDesktopError(cause);
      setSaveError(error);
      if (error.code === ERROR_CODES.configConflict) {
        setConflict(true);
      }
    } finally {
      setSaving(false);
    }
  }, [pending, snapshot, refresh]);

  const runValidation = useCallback(async () => {
    setValidating(true);
    setValidationError(null);
    try {
      setValidation(await desktopApi.validateConfiguration());
    } catch (cause) {
      setValidationError(toDesktopError(cause).message);
    } finally {
      setValidating(false);
    }
  }, []);

  const runReload = useCallback(async () => {
    setStatus(null);
    try {
      const result = await desktopApi.reloadConfiguration();
      setStatus(`Reloaded from disk · revision ${result.revision}`);
      refresh();
    } catch (cause) {
      setValidationError(toDesktopError(cause).message);
    }
  }, [refresh]);

  const dismissPending = useCallback(() => {
    setPending(null);
    setSaveError(null);
    setConflict(false);
  }, []);

  if (snapshotResource.error && !snapshot) {
    return <ErrorState error={snapshotResource.error} onRetry={snapshotResource.reload} />;
  }

  return (
    <div className="page">
      <div className="config-toolbar">
        <div className="config-toolbar__meta">
          <span className="config-toolbar__revision" aria-live="polite">
            Revision <code>{snapshot?.revision ?? '—'}</code>
          </span>
          {snapshot ? (
            <span className="config-toolbar__counts muted">
              {snapshot.workspaces.length} workspaces · {snapshot.services.length} services ·{' '}
              {snapshot.libraries.length} libraries · {snapshot.profiles.length} profiles ·{' '}
              {snapshot.groups.length} groups
            </span>
          ) : null}
        </div>
        <div className="config-toolbar__actions">
          <button
            type="button"
            className="button button--small"
            onClick={() => void runValidation()}
            disabled={!connected || validating}
          >
            {validating ? 'Validating…' : 'Validate'}
          </button>
          <button
            type="button"
            className="button button--small"
            onClick={() => void runReload()}
            disabled={!connected}
          >
            Reload
          </button>
        </div>
      </div>

      {!connected ? (
        <div className="page__notice" role="status">
          Disconnected from the daemon. Configuration editing is disabled until
          the connection returns; the loaded snapshot is kept and nothing is
          written locally.
        </div>
      ) : null}

      {status ? (
        <div className="config-status" role="status">
          <span className="badge badge--positive">Saved</span> {status}
        </div>
      ) : null}

      {validation ? (
        <ConfigurationValidationPanel
          revision={validation.revision}
          valid={validation.valid}
          issues={validation.issues}
        />
      ) : null}

      {validationError ? (
        <div className="config-validation config-validation--error" role="alert">
          {validationError}
        </div>
      ) : null}

      <div className="config-tabs" role="tablist" aria-label="Configuration sections">
        {(Object.keys(TAB_LABELS) as ConfigTab[]).map((kind) => (
          <button
            key={kind}
            type="button"
            role="tab"
            aria-selected={tab === kind}
            className={`config-tabs__tab${tab === kind ? ' config-tabs__tab--active' : ''}`}
            onClick={() => setTab(kind)}
          >
            {TAB_LABELS[kind]}
          </button>
        ))}
      </div>

      <div className="card card--table">
        {snapshotResource.loading && !snapshot ? (
          <LoadingState label="Loading configuration…" />
        ) : snapshot === null ? (
          <LoadingState
            label={connected ? 'Loading configuration…' : 'Waiting for the daemon connection…'}
          />
        ) : tab !== 'workspace' && activeWorkspaceId === null ? (
          <EmptyState
            title="No workspace selected"
            description="Select or declare a workspace before editing services, libraries, profiles, and groups."
          />
        ) : (
          <>
            <div className="config-list__header">
              <span className="toolbar__count" aria-live="polite">
                {rows.length} {TAB_LABELS[tab].toLowerCase()}
              </span>
              <button
                type="button"
                className="button button--small button--primary"
                onClick={() => openEditor(tab, null)}
                disabled={!connected || (tab !== 'workspace' && activeWorkspaceId === null)}
              >
                Add {tab}
              </button>
            </div>
            {rows.length === 0 ? (
              <EmptyState
                title={`No ${TAB_LABELS[tab].toLowerCase()} configured`}
                description={`The configuration file declares no ${TAB_LABELS[tab].toLowerCase()} for this workspace.`}
              />
            ) : (
              <ul className="config-list">
                {rows.map((row) => (
                  <li key={row.id} className="config-list__row">
                    <span className="config-list__name">{row.name}</span>
                    <code className="config-list__id muted">{row.id}</code>
                    <span className="config-list__actions">
                      <button
                        type="button"
                        className="button button--small"
                        aria-label={`Edit ${tab} ${row.id}`}
                        onClick={() => openEditor(tab, row.id)}
                        disabled={!connected}
                      >
                        Edit
                      </button>
                      <button
                        type="button"
                        className="button button--small button--danger"
                        aria-label={`Remove ${tab} ${row.id}`}
                        onClick={() => remove(tab, row.id, row.name)}
                        disabled={!connected}
                      >
                        Remove
                      </button>
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </>
        )}
      </div>

      <div className="card">
        <div className="card__header">
          <div>
            <h3 className="card__title">Templates and presets</h3>
            <p className="muted">
              These configuration entities are declared in the same file and committed through
              the same revision-checked manager, but they have dedicated screens.
            </p>
          </div>
        </div>
        <dl className="detail-grid">
          <div>
            <dt>Service templates</dt>
            <dd>{snapshot?.templates.length ?? 0}</dd>
          </div>
          <div>
            <dt>Migration presets</dt>
            <dd>{snapshot?.migration_presets.length ?? 0}</dd>
          </div>
          <div>
            <dt>Operation presets</dt>
            <dd>
              {snapshot?.operation_presets.filter(
                (preset) =>
                  preset.workspace === undefined || preset.workspace === activeWorkspaceId,
              ).length ?? 0}
            </dd>
          </div>
        </dl>
        {onNavigate ? (
          <div className="detail-actions">
            <button
              type="button"
              className="button button--small"
              onClick={() => onNavigate('templates')}
            >
              Open Templates
            </button>
            <button
              type="button"
              className="button button--small"
              onClick={() => onNavigate('bulk-operations')}
            >
              Open Bulk Operations
            </button>
          </div>
        ) : null}
      </div>

      {editor?.kind === 'workspace' ? (
        <WorkspaceEditor
          initial={editor.entity}
          onClose={() => setEditor(null)}
          onSave={(entity) =>
            reviewChange(
              {
                kind: 'workspace',
                change: editor.entity
                  ? { action: 'update', id: entity.id, entity }
                  : { action: 'create', entity },
              },
              {
                destructive: false,
                title: editor.entity ? `Update workspace “${entity.id}”` : `Add workspace “${entity.id}”`,
                message: 'Review the change before it is written to the configuration file.',
              },
            )
          }
        />
      ) : null}

      {editor?.kind === 'service' ? (
        <ServiceEditor
          initial={editor.entity}
          workspaceId={activeWorkspaceId ?? ''}
          workspaceRoot={workspaceRoot}
          onClose={() => setEditor(null)}
          onSave={(entity) =>
            reviewChange(
              {
                kind: 'service',
                change: editor.entity
                  ? { action: 'update', id: entity.id, entity }
                  : { action: 'create', entity },
              },
              {
                destructive: false,
                title: editor.entity ? `Update service “${entity.id}”` : `Add service “${entity.id}”`,
                message: 'Review the change before it is written to the configuration file.',
              },
            )
          }
        />
      ) : null}

      {editor?.kind === 'library' ? (
        <LibraryEditor
          initial={editor.entity}
          workspaceId={activeWorkspaceId ?? ''}
          onClose={() => setEditor(null)}
          onSave={(entity) =>
            reviewChange(
              {
                kind: 'library',
                change: editor.entity
                  ? { action: 'update', id: entity.id, entity }
                  : { action: 'create', entity },
              },
              {
                destructive: false,
                title: editor.entity ? `Update library “${entity.id}”` : `Add library “${entity.id}”`,
                message: 'Review the change before it is written to the configuration file.',
              },
            )
          }
        />
      ) : null}

      {editor?.kind === 'profile' ? (
        <ProfileEditor
          initial={editor.entity}
          workspaceId={activeWorkspaceId ?? ''}
          onClose={() => setEditor(null)}
          onSave={(entity) =>
            reviewChange(
              {
                kind: 'profile',
                change: editor.entity
                  ? { action: 'update', id: entity.id, entity }
                  : { action: 'create', entity },
              },
              {
                destructive: false,
                title: editor.entity ? `Update profile “${entity.id}”` : `Add profile “${entity.id}”`,
                message: 'Review the change before it is written to the configuration file.',
              },
            )
          }
        />
      ) : null}

      {editor?.kind === 'group' ? (
        <GroupEditor
          initial={editor.entity}
          workspaceId={activeWorkspaceId ?? ''}
          onClose={() => setEditor(null)}
          onSave={(entity) =>
            reviewChange(
              {
                kind: 'group',
                change: editor.entity
                  ? { action: 'update', id: entity.id, entity }
                  : { action: 'create', entity },
              },
              {
                destructive: false,
                title: editor.entity ? `Update group “${entity.id}”` : `Add group “${entity.id}”`,
                message: 'Review the change before it is written to the configuration file.',
              },
            )
          }
        />
      ) : null}

      {pending ? (
        <SaveConfirmationDialog
          title={pending.title}
          message={pending.message}
          destructive={pending.destructive}
          preview={preview}
          loading={previewLoading}
          error={previewError}
          conflict={conflict}
          saving={saving}
          saveError={saveError}
          onConfirm={() => void confirmSave()}
          onCancel={dismissPending}
          onRefresh={() => {
            dismissPending();
            refresh();
          }}
        />
      ) : null}
    </div>
  );
}
