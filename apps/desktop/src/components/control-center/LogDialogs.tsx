import { Dialog } from '../Dialog';
import { LogViewer } from '../tasks/LogViewer';
import { UnifiedLogViewer } from '../logs/UnifiedLogViewer';

/** The log scope the Control Center can open without leaving the screen. */
export type ControlCenterLogTarget =
  | { kind: 'task'; taskId: string; label?: string; live?: boolean }
  | { kind: 'service'; serviceId: string; label?: string };

/** A modal wrapping the shared bounded task-log viewer. */
export function TaskLogDialog({
  target,
  onClose,
}: {
  target: Extract<ControlCenterLogTarget, { kind: 'task' }>;
  onClose: () => void;
}) {
  return (
    <Dialog
      title={`Task logs — ${target.label ?? target.taskId}`}
      onClose={onClose}
      footer={
        <button type="button" className="button" onClick={onClose}>
          Close
        </button>
      }
    >
      <p className="dialog__note muted">
        Bounded task logs from the daemon. Following is enabled while the task is
        active; nothing is fetched outside this viewer.
      </p>
      <LogViewer taskId={target.taskId} live={target.live ?? false} />
    </Dialog>
  );
}

/** A modal wrapping the shared bounded service-log viewer. */
export function ServiceLogDialog({
  target,
  workspaceId,
  connected,
  onClose,
}: {
  target: Extract<ControlCenterLogTarget, { kind: 'service' }>;
  workspaceId: string | null;
  connected: boolean;
  onClose: () => void;
}) {
  return (
    <Dialog
      title={`Logs — ${target.label ?? target.serviceId}`}
      onClose={onClose}
      footer={
        <button type="button" className="button" onClick={onClose}>
          Close
        </button>
      }
    >
      <p className="dialog__note muted">
        Realtime and historical logs for this service. The stream is bounded and
        follows only while this dialog is open.
      </p>
      <UnifiedLogViewer
        scope={{
          kind: 'service',
          serviceId: target.serviceId,
          workspaceId,
        }}
        disabled={!connected}
      />
    </Dialog>
  );
}
