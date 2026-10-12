import { act, render, type RenderResult } from '@testing-library/react';

import App from '../App';
import {
  makeConnected,
  makeDaemonStatus,
  makeOperationOutcome,
  makeServiceDetail,
  makeServices,
  makeWorkspace,
  makeWorkspaceStatus,
} from './fixtures';
import { mockDesktopApi } from './mocks';

/** A promise with externally controlled settlement. */
export interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason: unknown) => void;
}

/** Creates a deferred promise for loading-state tests. */
export function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** Drains pending promise chains (subscriptions, pulls, fetches). */
export async function flush(): Promise<void> {
  await act(async () => {
    // Multiple turns: each round lets one stage of a request → state →
    // effect → request chain settle.
    for (let turn = 0; turn < 4; turn += 1) {
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
  });
}

/** Renders the full application and flushes its initial async work. */
export async function renderApp(): Promise<RenderResult> {
  const view = render(<App />);
  await flush();
  return view;
}

/** Configures the mock API with a connected, fully populated happy path. */
export function happyPathDefaults(): void {
  mockDesktopApi.getConnectionState.mockResolvedValue(makeConnected());
  mockDesktopApi.getDaemonStatus.mockResolvedValue(makeDaemonStatus());
  mockDesktopApi.listWorkspaces.mockResolvedValue([makeWorkspace()]);
  mockDesktopApi.listServices.mockResolvedValue(makeServices());
  mockDesktopApi.getWorkspaceStatus.mockResolvedValue(makeWorkspaceStatus(makeServices()));
  mockDesktopApi.getServiceDetail.mockResolvedValue(makeServiceDetail());
  mockDesktopApi.startService.mockResolvedValue(makeOperationOutcome());
  mockDesktopApi.stopService.mockResolvedValue(
    makeOperationOutcome({ kind: 'stop_service' }),
  );
  mockDesktopApi.restartService.mockResolvedValue(
    makeOperationOutcome({ kind: 'restart_service' }),
  );
  mockDesktopApi.startServices.mockResolvedValue(
    makeOperationOutcome({
      status: 'workflow',
      task_id: null,
      workflow_id: 'wf-1',
      kind: 'start_services',
    }),
  );
  mockDesktopApi.stopServices.mockResolvedValue(
    makeOperationOutcome({
      status: 'workflow',
      task_id: null,
      workflow_id: 'wf-stop',
      kind: 'stop_services',
    }),
  );
  mockDesktopApi.restartServices.mockResolvedValue(
    makeOperationOutcome({
      status: 'workflow',
      task_id: null,
      workflow_id: 'wf-restart',
      kind: 'restart_services',
    }),
  );
  mockDesktopApi.checkServiceHealth.mockResolvedValue(
    makeOperationOutcome({ kind: 'health_check' }),
  );
}
