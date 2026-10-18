import { useCallback, useEffect, useRef, useState } from 'react';

import { desktopApi } from '../services/desktopApi';
import { ERROR_CODES, toDesktopError } from '../services/errors';
import type {
  ConfigChangeView,
  ConfigurationCommitView,
  ConfigurationPreviewView,
  DesktopError,
} from '../types/models';

/** A configuration change awaiting the daemon's preview and the user's review. */
export interface PendingConfigurationChange {
  /** The dialog title, for example "Save template". */
  title: string;
  /** A short explanation of what the change does. */
  message: string;
  /** Whether the change removes or overwrites hand-written configuration. */
  destructive: boolean;
  /** The exact change submitted to the daemon. */
  change: ConfigChangeView;
  /** The revision the editor was based on. */
  expectedRevision: string;
}

/** The state and actions of a configuration review dialog. */
export interface ConfigurationReview {
  pending: PendingConfigurationChange | null;
  preview: ConfigurationPreviewView | null;
  loading: boolean;
  error: DesktopError | null;
  conflict: boolean;
  saving: boolean;
  saveError: DesktopError | null;
  /** Requests the daemon preview; this never writes anything. */
  review: (pending: PendingConfigurationChange) => Promise<void>;
  /** Commits the reviewed change and forwards the user's destructive intent. */
  confirm: () => Promise<void>;
  /** Closes the dialog without writing. */
  cancel: () => void;
  /** Closes the dialog after a stale revision so the caller can reload. */
  refresh: () => void;
}

/**
 * Drives the shared configuration review flow (`tasks/032.md` §5, §38).
 *
 * Both the Templates screen and the saved Bulk Operations presets persist
 * through the *existing* configuration manager: the daemon previews the whole
 * proposed document, the user reviews the diff and validation issues, and the
 * commit is revision-checked. A stale revision is never overwritten — the
 * dialog offers a reload instead (`tasks/023.md` §33–§36).
 *
 * `confirmed` forwards the user's explicit destructive intent. It is not an
 * authorization mechanism: the daemon independently verifies the client kind
 * and peer identity, and its policy decision is final.
 */
export function useConfigurationReview(options: {
  onCommitted: (commit: ConfigurationCommitView) => void;
  onRefresh: () => void;
}): ConfigurationReview {
  const [pending, setPending] = useState<PendingConfigurationChange | null>(null);
  const [preview, setPreview] = useState<ConfigurationPreviewView | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<DesktopError | null>(null);
  const [conflict, setConflict] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<DesktopError | null>(null);

  // The callbacks are read through a ref so `review`/`confirm` keep stable
  // identities while always calling the latest caller-provided handlers.
  const callbacks = useRef(options);
  useEffect(() => {
    callbacks.current = options;
  }, [options]);

  const review = useCallback(async (next: PendingConfigurationChange) => {
    setPending(next);
    setPreview(null);
    setError(null);
    setConflict(false);
    setSaveError(null);
    setLoading(true);
    try {
      const result = await desktopApi.previewConfiguration(next.change, next.expectedRevision);
      setPreview(result);
    } catch (cause) {
      const failure = toDesktopError(cause);
      setConflict(failure.code === ERROR_CODES.configConflict);
      setError(failure);
    } finally {
      setLoading(false);
    }
  }, []);

  const confirm = useCallback(async () => {
    if (!pending) {
      return;
    }
    setSaving(true);
    setSaveError(null);
    try {
      const commit = await desktopApi.commitConfiguration(
        pending.change,
        pending.expectedRevision,
        pending.destructive,
      );
      setPending(null);
      setPreview(null);
      callbacks.current.onCommitted(commit);
    } catch (cause) {
      const failure = toDesktopError(cause);
      if (failure.code === ERROR_CODES.configConflict) {
        setConflict(true);
      } else {
        setSaveError(failure);
      }
    } finally {
      setSaving(false);
    }
  }, [pending]);

  const cancel = useCallback(() => {
    setPending(null);
    setPreview(null);
    setError(null);
    setConflict(false);
    setSaveError(null);
  }, []);

  const refresh = useCallback(() => {
    cancel();
    callbacks.current.onRefresh();
  }, [cancel]);

  return {
    pending,
    preview,
    loading,
    error,
    conflict,
    saving,
    saveError,
    review,
    confirm,
    cancel,
    refresh,
  };
}
