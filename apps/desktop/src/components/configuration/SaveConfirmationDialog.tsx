import { Dialog } from '../Dialog';
import type { ConfigurationPreviewView, DesktopError } from '../../types/models';
import { ConfigurationDiff } from './ConfigurationDiff';
import {
  AffectedReferences,
  ConfigurationValidationPanel,
  RunningImpact,
} from './ConfigurationValidationPanel';

/**
 * The save-confirmation dialog for a configuration change (`tasks/023.md` §36).
 *
 * It is a review step, not an authorization mechanism: it renders the daemon's
 * preview (diff, validation, affected references, running impact) and forwards
 * the user's confirmation as the request's destructive intent. The daemon still
 * verifies the client kind and peer identity and its policy decision is final
 * (`tasks/023.md` §30, §46).
 *
 * A stale revision (`CONFIG_CONFLICT`) is never overwritten: the dialog
 * explains the conflict and offers a reload instead of saving.
 */
export function SaveConfirmationDialog({
  title,
  message,
  destructive,
  preview,
  loading,
  error,
  conflict,
  saving,
  saveError,
  onConfirm,
  onCancel,
  onRefresh,
}: {
  title: string;
  message: string;
  destructive: boolean;
  preview: ConfigurationPreviewView | null;
  loading: boolean;
  error: DesktopError | null;
  conflict: boolean;
  saving: boolean;
  saveError: DesktopError | null;
  onConfirm: () => void;
  onCancel: () => void;
  onRefresh: () => void;
}) {
  const blocked = conflict || error !== null || preview === null || !preview.valid;

  return (
    <Dialog
      title={title}
      tone={destructive ? 'danger' : 'default'}
      onClose={saving ? () => undefined : onCancel}
      footer={
        <>
          <button type="button" className="button" onClick={onCancel} disabled={saving}>
            Cancel
          </button>
          {conflict ? (
            <button
              type="button"
              className="button button--primary"
              onClick={onRefresh}
              disabled={saving}
            >
              Refresh configuration
            </button>
          ) : (
            <button
              type="button"
              className={`button${destructive ? ' button--danger' : ' button--primary'}`}
              onClick={onConfirm}
              disabled={saving || blocked}
            >
              {saving ? 'Saving…' : destructive ? 'Remove' : 'Save'}
            </button>
          )}
        </>
      }
    >
      <p className="dialog__message">{message}</p>

      {loading ? <p className="muted">Reviewing the change…</p> : null}

      {conflict ? (
        <div className="config-conflict" role="alert">
          <p>
            <span className="badge badge--warning">Conflict</span> The
            configuration changed after this editor loaded. Your change was not
            saved and nothing was overwritten. Review the current file and try
            again.
          </p>
        </div>
      ) : null}

      {error && !conflict ? (
        <div className="config-validation config-validation--error" role="alert">
          <p>{error.message}</p>
        </div>
      ) : null}

      {saveError ? (
        <div className="config-validation config-validation--error" role="alert">
          <p>{saveError.message}</p>
        </div>
      ) : null}

      {preview ? (
        <>
          <ConfigurationValidationPanel
            revision={preview.proposed_revision}
            valid={preview.valid}
            issues={preview.issues}
          />
          <ConfigurationDiff diff={preview.diff} />
          <AffectedReferences references={preview.affected_references} />
          <RunningImpact impact={preview.running_impact} />
        </>
      ) : null}
    </Dialog>
  );
}
