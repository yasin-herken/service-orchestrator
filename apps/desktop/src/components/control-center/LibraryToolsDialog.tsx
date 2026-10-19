import { useMemo, useState } from 'react';

import { Dialog } from '../Dialog';
import { EmptyState, ErrorState, LoadingState } from '../StateViews';
import { useAsync } from '../../hooks/useAsync';
import { desktopApi } from '../../services/desktopApi';
import { toDesktopError } from '../../services/errors';
import type { DesktopError, LibrarySummary } from '../../types/models';

type LibraryAction = 'verify' | 'alignment' | 'install';

/**
 * On-demand library tools for the Workspace Control Center (`tasks/033.md` §7).
 *
 * These are the existing Library Manager operations: verify an artifact,
 * inspect consumer version alignment, and install a library locally. They are
 * deliberately separate from the coordinated environment workflow; the
 * workflow's `build_libraries` flag builds and installs required libraries in
 * dependency order.
 */
export function LibraryToolsDialog({
  workspaceId,
  selectedIds,
  connected,
  onClose,
}: {
  workspaceId: string;
  selectedIds: ReadonlySet<string>;
  connected: boolean;
  onClose: () => void;
}) {
  const [revision, setRevision] = useState(0);
  const [busy, setBusy] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const [failure, setFailure] = useState<DesktopError | null>(null);
  const libraries = useAsync<LibrarySummary[]>(
    () => desktopApi.listLibraries(workspaceId),
    [workspaceId, revision],
    { enabled: connected },
  );

  const { visible, related } = useMemo(() => {
    const all = libraries.data ?? [];
    const relatedLibraries = all.filter((library) =>
      library.dependents.some((dependent) =>
        selectedIds.has(dependent.service_id),
      ),
    );
    return {
      visible: relatedLibraries.length > 0 ? relatedLibraries : all,
      related: relatedLibraries.length > 0,
    };
  }, [libraries.data, selectedIds]);

  const run = async (libraryId: string, action: LibraryAction) => {
    const key = `${libraryId}:${action}`;
    setBusy(key);
    setResult(null);
    setFailure(null);
    try {
      if (action === 'verify') {
        const verification = await desktopApi.verifyLibraryArtifact(libraryId);
        setResult(
          `${verification.artifact.coordinate}: ${
            verification.present ? 'present' : 'missing'
          } — ${verification.message}`,
        );
      } else if (action === 'alignment') {
        const alignment = await desktopApi.inspectVersionAlignment(libraryId);
        setResult(
          `${alignment.library_id}: ${alignment.aligned_count} of ${alignment.total} consumers aligned${
            alignment.fully_aligned ? '' : ' (mismatches present)'
          }.`,
        );
      } else {
        const outcome = await desktopApi.installLibrary(libraryId);
        if (outcome.workflow_id) {
          setResult(
            `Scheduled ${outcome.kind} workflow ${outcome.workflow_id}. Progress is available on the Tasks screen.`,
          );
        } else if (outcome.task_id) {
          setResult(`Scheduled ${outcome.kind} task ${outcome.task_id}.`);
        } else {
          setResult(`Already in the desired state (${outcome.kind}).`);
        }
      }
      setRevision((current) => current + 1);
    } catch (cause) {
      setFailure(toDesktopError(cause));
    } finally {
      setBusy(null);
    }
  };

  return (
    <Dialog
      title="Library tools"
      onClose={onClose}
      footer={
        <button type="button" className="button" onClick={onClose}>
          Close
        </button>
      }
    >
      <p className="dialog__note">
        These actions use the existing Library Manager and schedule real daemon
        tasks. They are independent of the environment workflow; “Build required
        libraries” in the workflow remains the dependency-ordered build.
      </p>

      {libraries.error ? (
        <ErrorState error={libraries.error} onRetry={libraries.reload} />
      ) : libraries.loading && !libraries.data ? (
        <LoadingState label="Loading libraries…" />
      ) : visible.length === 0 ? (
        <EmptyState
          title="No libraries configured"
          description="This workspace declares no libraries."
        />
      ) : (
        <>
          <p className="muted">
            {related
              ? 'Libraries required by the current selection.'
              : 'No selected service declares a library dependency; showing every workspace library.'}
          </p>
          <ul className="control-center__library-list">
            {visible.map((library) => (
              <li key={library.id} className="control-center__library">
                <div className="control-center__library-header">
                  <strong>{library.display_name}</strong>
                  <span className="muted">
                    {library.artifact?.coordinate ?? 'no artifact declared'}
                  </span>
                </div>
                <div className="muted">
                  {library.branch ?? 'no branch'} ·{' '}
                  {library.configured_version ?? 'no configured version'}
                </div>
                <div className="detail-actions">
                  <button
                    type="button"
                    className="button button--small"
                    disabled={!connected || busy !== null}
                    onClick={() => void run(library.id, 'verify')}
                  >
                    {busy === `${library.id}:verify` ? 'Verifying…' : 'Verify'}
                  </button>
                  <button
                    type="button"
                    className="button button--small"
                    disabled={!connected || busy !== null}
                    onClick={() => void run(library.id, 'alignment')}
                  >
                    {busy === `${library.id}:alignment`
                      ? 'Inspecting…'
                      : 'Inspect alignment'}
                  </button>
                  <button
                    type="button"
                    className="button button--small"
                    disabled={!connected || busy !== null}
                    onClick={() => void run(library.id, 'install')}
                  >
                    {busy === `${library.id}:install`
                      ? 'Installing…'
                      : 'Install locally'}
                  </button>
                </div>
              </li>
            ))}
          </ul>
        </>
      )}

      {failure ? (
        <div className="error-details" role="alert">
          <p className="error-details__label">Library action failed</p>
          <p className="error-details__message">{failure.message}</p>
          <p className="error-details__code">{failure.code}</p>
        </div>
      ) : null}
      {result ? (
        <p className="page__notice" role="status">
          {result}
        </p>
      ) : null}
      {!connected ? (
        <p className="muted" role="status">
          Mutating library actions are disabled while the daemon is disconnected.
        </p>
      ) : null}
    </Dialog>
  );
}
