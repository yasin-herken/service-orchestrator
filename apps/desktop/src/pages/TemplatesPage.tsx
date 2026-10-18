import { useState } from 'react';

import { SaveConfirmationDialog } from '../components/configuration/SaveConfirmationDialog';
import { ConfigurationValidationPanel } from '../components/configuration/ConfigurationValidationPanel';
import { Dialog } from '../components/Dialog';
import { ErrorDialog } from '../components/ErrorDialog';
import { EmptyState, ErrorState, LoadingState } from '../components/StateViews';
import { StatusBadge } from '../components/StatusBadge';
import { TemplateEditor } from '../components/template/TemplateEditor';
import { TemplatePreviewDialog } from '../components/template/TemplatePreviewDialog';
import { useConnection } from '../context/ConnectionContext';
import { useWorkspaces } from '../context/WorkspaceContext';
import { useAsync } from '../hooks/useAsync';
import { useConfigurationReview } from '../hooks/useConfigurationReview';
import { desktopApi } from '../services/desktopApi';
import { toDesktopError } from '../services/errors';
import type {
  ConfigurationSnapshotView,
  ConfigurationValidationView,
  DesktopError,
  MigrationPresetSummaryView,
  RawTemplateView,
  ValidationIssueView,
} from '../types/models';

/** The editor state: which template the dialog was opened for. */
type EditorState =
  | { mode: 'create' }
  | { mode: 'edit'; template: RawTemplateView }
  | { mode: 'duplicate'; template: RawTemplateView };

/**
 * Template management (`tasks/032.md` §3–§6).
 *
 * Templates are configuration, so every action goes through the *existing*
 * configuration manager: the daemon previews the whole proposed document, the
 * user reviews the diff and validation issues, and the commit is
 * revision-checked. The resolved-configuration preview calls the real
 * bulk-import planner against a real repository, so it shows exactly what an
 * import would write (`tasks/032.md` §5, §39).
 */
export function TemplatesPage() {
  const { snapshot } = useConnection();
  const connected = snapshot.state === 'connected';
  const { workspaces, activeWorkspaceId } = useWorkspaces();

  const [reloadToken, setReloadToken] = useState(0);
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [preview, setPreview] = useState<RawTemplateView | null>(null);
  const [validation, setValidation] = useState<ConfigurationValidationView | null>(null);
  const [validating, setValidating] = useState(false);
  const [validationTarget, setValidationTarget] = useState<{
    template: RawTemplateView;
    issues: ValidationIssueView[];
  } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [failure, setFailure] = useState<{ title: string; error: DesktopError } | null>(null);

  const configuration = useAsync<ConfigurationSnapshotView>(
    () => desktopApi.getConfiguration(),
    [connected, reloadToken],
    { enabled: connected },
  );
  const presets = useAsync<MigrationPresetSummaryView[]>(
    () => desktopApi.listMigrationPresets(),
    [connected, reloadToken],
    { enabled: connected },
  );

  const review = useConfigurationReview({
    onCommitted: (commit) => {
      setNotice(
        `Saved configuration revision ${commit.revision} (${commit.counts.workspaces} workspaces, ` +
          `${commit.counts.services} services, ${commit.counts.libraries} libraries).`,
      );
      setValidation(null);
      setReloadToken((token) => token + 1);
    },
    onRefresh: () => setReloadToken((token) => token + 1),
  });

  const templates = configuration.data?.templates ?? [];
  const revision = configuration.data?.revision;

  const runValidation = async () => {
    setValidating(true);
    try {
      setValidation(await desktopApi.validateConfiguration());
    } catch (cause) {
      setFailure({ title: 'Unable to validate the configuration', error: toDesktopError(cause) });
    } finally {
      setValidating(false);
    }
  };

  const validateOne = async (template: RawTemplateView, index: number) => {
    setValidating(true);
    try {
      const result = await desktopApi.validateConfiguration();
      const prefix = `templates[${index}]`;
      setValidationTarget({
        template,
        issues: (result.issues ?? []).filter((issue) => issue.location.startsWith(prefix)),
      });
    } catch (cause) {
      setFailure({ title: 'Unable to validate the template', error: toDesktopError(cause) });
    } finally {
      setValidating(false);
    }
  };

  const save = (template: RawTemplateView) => {
    if (revision === undefined) {
      return;
    }
    const editing = editor?.mode === 'edit';
    setEditor(null);
    void review.review({
      title: editing ? 'Update template' : 'Create template',
      message: editing
        ? `Updates template “${template.id}”. Existing services keep their own values; only future imports and explicit updates use the template.`
        : `Adds template “${template.id}”. Nothing is imported or executed.`,
      destructive: false,
      expectedRevision: revision,
      change: {
        kind: 'template',
        change: editing
          ? { action: 'update', id: template.id, entity: template }
          : { action: 'create', entity: template },
      },
    });
  };

  const remove = (template: RawTemplateView) => {
    if (revision === undefined) {
      return;
    }
    void review.review({
      title: 'Remove template',
      message: `Removes template “${template.id}” from the configuration. Services keep their resolved values; future imports cannot select it.`,
      destructive: true,
      expectedRevision: revision,
      change: { kind: 'template', change: { action: 'remove', id: template.id } },
    });
  };

  if (!connected) {
    return <LoadingState label="Waiting for the daemon connection…" />;
  }
  if (configuration.loading && configuration.data === null) {
    return <LoadingState label="Loading templates…" />;
  }
  if (configuration.error) {
    return <ErrorState error={configuration.error} onRetry={configuration.reload} />;
  }

  return (
    <div className="page">
      <div className="card">
        <div className="card__header">
          <div>
            <h2 className="card__title">Service templates</h2>
            <p className="muted">
              Reusable defaults for the existing service model. Templates never create a
              second configuration source: the daemon resolves them into validated services
              only when a repository is imported.
            </p>
          </div>
          <div className="detail-actions">
            <button
              type="button"
              className="button"
              onClick={() => void runValidation()}
              disabled={validating}
            >
              {validating ? 'Validating…' : 'Validate configuration'}
            </button>
            <button
              type="button"
              className="button button--primary"
              onClick={() => setEditor({ mode: 'create' })}
            >
              New template
            </button>
          </div>
        </div>

        {notice ? (
          <p className="page__notice" role="status">
            {notice}
          </p>
        ) : null}

        {validation ? (
          <ConfigurationValidationPanel
            revision={validation.revision}
            valid={validation.valid}
            issues={validation.issues ?? []}
          />
        ) : null}

        {templates.length === 0 ? (
          <EmptyState
            title="No templates configured"
            description="Add a template to describe the defaults for a Spring Boot backend, a React frontend, or a shared Maven library."
          />
        ) : (
          <div className="table-wrap">
            <table className="service-table" aria-label="Service templates">
              <thead>
                <tr>
                  <th scope="col">Template</th>
                  <th scope="col">Category</th>
                  <th scope="col">Type</th>
                  <th scope="col">Branch</th>
                  <th scope="col">Build</th>
                  <th scope="col">Migration</th>
                  <th scope="col">Actions</th>
                </tr>
              </thead>
              <tbody>
                {templates.map((template, index) => (
                  <tr key={template.id} data-template={template.id}>
                    <td>
                      <div className="service-table__name">{template.name ?? template.id}</div>
                      <div className="muted">{template.id}</div>
                    </td>
                    <td>
                      <StatusBadge
                        value={template.category ?? 'custom'}
                        tone="info"
                        label={template.category ?? 'custom'}
                      />
                    </td>
                    <td>{template.type ?? 'from detection'}</td>
                    <td>{template.default_branch ?? '—'}</td>
                    <td>
                      {template.build_system ?? '—'}
                      {template.build_operation ? ` · ${template.build_operation}` : ''}
                    </td>
                    <td>
                      {template.liquibase?.enabled === false
                        ? 'disabled'
                        : template.liquibase?.preset
                          ? template.liquibase.preset
                          : 'per service'}
                    </td>
                    <td>
                      <div className="row-actions">
                        <button
                          type="button"
                          className="button button--small"
                          onClick={() => setPreview(template)}
                        >
                          Preview
                        </button>
                        <button
                          type="button"
                          className="button button--small"
                          onClick={() => void validateOne(template, index)}
                          disabled={validating}
                        >
                          Validate
                        </button>
                        <button
                          type="button"
                          className="button button--small"
                          onClick={() => setEditor({ mode: 'edit', template })}
                        >
                          Edit
                        </button>
                        <button
                          type="button"
                          className="button button--small"
                          onClick={() => setEditor({ mode: 'duplicate', template })}
                        >
                          Duplicate
                        </button>
                        <button
                          type="button"
                          className="button button--small button--danger"
                          onClick={() => remove(template)}
                        >
                          Delete
                        </button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {editor ? (
        <TemplateEditor
          initial={editor.mode === 'create' ? null : editor.template}
          mode={editor.mode}
          presets={presets.data ?? []}
          onSave={save}
          onClose={() => setEditor(null)}
        />
      ) : null}

      {preview ? (
        <TemplatePreviewDialog
          template={preview}
          workspaces={workspaces}
          initialWorkspaceId={activeWorkspaceId}
          onClose={() => setPreview(null)}
        />
      ) : null}

      {validationTarget ? (
        <Dialog
          title={`Validation: ${validationTarget.template.name ?? validationTarget.template.id}`}
          onClose={() => setValidationTarget(null)}
          footer={
            <button type="button" className="button" onClick={() => setValidationTarget(null)}>
              Close
            </button>
          }
        >
          <p className="dialog__message">
            The daemon validated the active configuration file. Only problems at this
            template’s location are shown.
          </p>
          <ConfigurationValidationPanel
            revision={revision ?? ''}
            valid={validationTarget.issues.length === 0}
            issues={validationTarget.issues}
          />
        </Dialog>
      ) : null}

      {review.pending ? (
        <SaveConfirmationDialog
          title={review.pending.title}
          message={review.pending.message}
          destructive={review.pending.destructive}
          preview={review.preview}
          loading={review.loading}
          error={review.error}
          conflict={review.conflict}
          saving={review.saving}
          saveError={review.saveError}
          onConfirm={() => void review.confirm()}
          onCancel={review.cancel}
          onRefresh={review.refresh}
        />
      ) : null}

      {failure ? (
        <ErrorDialog
          title={failure.title}
          error={failure.error}
          labels={[]}
          onClose={() => setFailure(null)}
        />
      ) : null}
    </div>
  );
}
