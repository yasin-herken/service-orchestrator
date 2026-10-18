import { useState } from 'react';

import { Dialog } from '../Dialog';
import { ErrorState } from '../StateViews';
import { desktopApi } from '../../services/desktopApi';
import { toDesktopError } from '../../services/errors';
import type {
  BulkImportOverridesView,
  BulkImportPlanView,
  DesktopError,
  RawTemplateView,
  WorkspaceSummary,
} from '../../types/models';
import { ResolvedEntity } from './ResolvedEntity';

/** Renders one row's structured errors and warnings. */
function RowProblems({ plan }: { plan: BulkImportPlanView }) {
  const row = plan.rows[0];
  const problems = [
    ...(row?.errors ?? []).map((issue) => `${issue.code}: ${issue.message}`),
    ...(row?.warnings ?? []),
    ...(plan.warnings ?? []),
  ];
  if (problems.length === 0) {
    return null;
  }
  return (
    <div className="config-validation" role="alert">
      <ul>
        {problems.map((problem) => (
          <li key={problem}>{problem}</li>
        ))}
      </ul>
    </div>
  );
}

/**
 * Preview of a template's resolved configuration (`tasks/032.md` §5).
 *
 * The preview calls the daemon's real bulk-import planner against a real
 * repository, so the values shown are exactly the ones an import would write —
 * including variable resolution, port assignment, and validation. Nothing is
 * written, cloned, or executed; the daemon inspects the path read-only.
 */
export function TemplatePreviewDialog({
  template,
  workspaces,
  initialWorkspaceId,
  onClose,
}: {
  template: RawTemplateView;
  workspaces: readonly WorkspaceSummary[];
  initialWorkspaceId: string | null;
  onClose: () => void;
}) {
  const [workspaceId, setWorkspaceId] = useState(
    initialWorkspaceId ?? workspaces[0]?.id ?? '',
  );
  const [relativePath, setRelativePath] = useState('');
  const [repositoryUrl, setRepositoryUrl] = useState('');
  const [branch, setBranch] = useState('');
  const [port, setPort] = useState('');
  const [serviceName, setServiceName] = useState('');
  const [plan, setPlan] = useState<BulkImportPlanView | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<DesktopError | null>(null);

  const submit = async () => {
    setLoading(true);
    setError(null);
    setPlan(null);
    const overrides: BulkImportOverridesView = {};
    if (serviceName.trim() !== '') {
      overrides.name = serviceName.trim();
    }
    if (repositoryUrl.trim() !== '') {
      overrides.repository_url = repositoryUrl.trim();
    }
    if (branch.trim() !== '') {
      overrides.default_branch = branch.trim();
    }
    if (port.trim() !== '') {
      const parsed = Number(port);
      if (Number.isInteger(parsed) && parsed > 0) {
        overrides.ports = [parsed];
      }
    }
    try {
      const result = await desktopApi.planBulkImport({
        workspace_id: workspaceId,
        repositories: [
          {
            relative_path: relativePath.trim(),
            template: template.id,
            existing: 'skip',
            overrides,
          },
        ],
        bulk: {},
        port_base: 8080,
      });
      setPlan(result);
    } catch (cause) {
      setError(toDesktopError(cause));
    } finally {
      setLoading(false);
    }
  };

  const row = plan?.rows[0];
  const proposal = row?.proposal;

  return (
    <Dialog
      title={`Preview “${template.name ?? template.id}”`}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="button" onClick={onClose}>
            Close
          </button>
          <button
            type="button"
            className="button button--primary"
            onClick={() => void submit()}
            disabled={loading || workspaceId === '' || relativePath.trim() === ''}
          >
            {loading ? 'Resolving…' : 'Resolve for repository'}
          </button>
        </>
      }
    >
      <p className="dialog__message">
        The daemon resolves the saved template against a real repository. This is read-only:
        no build, Git command, clone, or configuration write runs.
      </p>

      <div className="config-form">
        <label className="config-form__field">
          <span>Workspace</span>
          <select
            className="input input--select"
            value={workspaceId}
            onChange={(event) => setWorkspaceId(event.target.value)}
            aria-label="Preview workspace"
          >
            {workspaces.map((workspace) => (
              <option key={workspace.id} value={workspace.id}>
                {workspace.name}
              </option>
            ))}
          </select>
        </label>
        <label className="config-form__field">
          <span>Repository path</span>
          <input
            className="input"
            value={relativePath}
            onChange={(event) => setRelativePath(event.target.value)}
            aria-label="Preview repository path"
            placeholder="backend/orders-api"
          />
          <span className="muted">Relative to the workspace root.</span>
        </label>
        <label className="config-form__field">
          <span>Repository URL (optional)</span>
          <input
            className="input"
            value={repositoryUrl}
            onChange={(event) => setRepositoryUrl(event.target.value)}
            aria-label="Preview repository URL"
            placeholder="https://example.com/orders-api.git"
          />
          <span className="muted">
            Only needed when the checkout has no detected origin.
          </span>
        </label>
        <label className="config-form__field">
          <span>Branch (optional)</span>
          <input
            className="input"
            value={branch}
            onChange={(event) => setBranch(event.target.value)}
            aria-label="Preview branch"
            placeholder="main"
          />
        </label>
        <label className="config-form__field">
          <span>Service name (optional)</span>
          <input
            className="input"
            value={serviceName}
            onChange={(event) => setServiceName(event.target.value)}
            aria-label="Preview service name"
            placeholder="orders-api"
          />
        </label>
        <label className="config-form__field">
          <span>Port (optional)</span>
          <input
            className="input"
            value={port}
            onChange={(event) => setPort(event.target.value)}
            aria-label="Preview port"
            inputMode="numeric"
            placeholder="8081"
          />
        </label>
      </div>

      {error ? <ErrorState error={error} /> : null}
      {plan ? (
        <div className="template-preview__result">
          <p className="muted" aria-live="polite">
            {plan.valid
              ? 'The daemon resolved and validated this configuration.'
              : 'The daemon reported problems with this configuration.'}
          </p>
          <RowProblems plan={plan} />
          {proposal ? <ResolvedEntity proposal={proposal} /> : null}
          {row?.template_source ? (
            <p className="muted">
              Template source: {row.template_source}
              {row.template ? ` · ${row.template}` : ''}
            </p>
          ) : null}
        </div>
      ) : null}
    </Dialog>
  );
}
