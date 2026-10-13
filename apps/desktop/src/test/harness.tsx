import { act, render, type RenderResult } from '@testing-library/react';

import App from '../App';
import {
  makeAlignment,
  makeBranches,
  makeBulkSummary,
  makeBuildTargets,
  makeConnected,
  makeDaemonStatus,
  makeGitStatus,
  makeGitSyncPreview,
  makeInspection,
  makeLibraries,
  makeLogPage,
  makeOperationOutcome,
  makeRepositories,
  makeServiceDetail,
  makeServices,
  makeTaskView,
  makeVerification,
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
  mockDesktopApi.listRepositories.mockResolvedValue(makeRepositories());
  mockDesktopApi.getGitStatus.mockResolvedValue(makeGitStatus());
  mockDesktopApi.inspectRepositories.mockImplementation(async (workspaceId, serviceIds) =>
    serviceIds.map((serviceId) =>
      makeInspection({
        repository_id: `${workspaceId}:${serviceId}`,
        service_id: serviceId,
        status: makeGitStatus({ service_id: serviceId }),
      }),
    ),
  );
  mockDesktopApi.listBranches.mockResolvedValue(makeBranches());
  mockDesktopApi.previewGitSync.mockResolvedValue(makeGitSyncPreview());
  mockDesktopApi.fetchRepository.mockResolvedValue(makeOperationOutcome({ kind: 'git_fetch' }));
  mockDesktopApi.pullRepository.mockResolvedValue(makeOperationOutcome({ kind: 'git_pull' }));
  mockDesktopApi.checkoutRepository.mockResolvedValue(
    makeOperationOutcome({ kind: 'git_checkout' }),
  );
  mockDesktopApi.syncRepository.mockResolvedValue(makeOperationOutcome({ kind: 'git_sync' }));
  mockDesktopApi.fetchRepositories.mockResolvedValue(
    makeOperationOutcome({ status: 'workflow', task_id: null, workflow_id: 'wf-fetch', kind: 'fetch_repositories' }),
  );
  mockDesktopApi.syncRepositories.mockResolvedValue(
    makeOperationOutcome({ status: 'workflow', task_id: null, workflow_id: 'wf-sync', kind: 'sync_repositories' }),
  );
  mockDesktopApi.checkoutRepositories.mockResolvedValue(
    makeOperationOutcome({ status: 'workflow', task_id: null, workflow_id: 'wf-co', kind: 'checkout_repositories' }),
  );
  mockDesktopApi.listBuildTargets.mockResolvedValue(makeBuildTargets());
  mockDesktopApi.buildService.mockResolvedValue(makeOperationOutcome({ kind: 'build_service' }));
  mockDesktopApi.buildServices.mockResolvedValue(
    makeOperationOutcome({ status: 'workflow', task_id: null, workflow_id: 'wf-build', kind: 'prepare_environment' }),
  );
  mockDesktopApi.listLibraries.mockResolvedValue(makeLibraries());
  mockDesktopApi.getLibraryDependents.mockResolvedValue([
    { service_id: 'orders-api', workspace_id: 'demo', required: true, kind: 'build' },
  ]);
  mockDesktopApi.verifyLibraryArtifact.mockResolvedValue(makeVerification());
  mockDesktopApi.inspectVersionAlignment.mockResolvedValue(makeAlignment());
  mockDesktopApi.buildLibrary.mockResolvedValue(makeOperationOutcome({ kind: 'build_library' }));
  mockDesktopApi.installLibrary.mockResolvedValue(makeOperationOutcome({ kind: 'library_install' }));
  mockDesktopApi.syncLibrary.mockResolvedValue(makeOperationOutcome({ kind: 'git_sync' }));
  mockDesktopApi.prepareLibrary.mockResolvedValue(
    makeOperationOutcome({ status: 'workflow', task_id: null, workflow_id: 'wf-prep', kind: 'prepare_library' }),
  );
  mockDesktopApi.prepareLibraries.mockResolvedValue(
    makeOperationOutcome({ status: 'workflow', task_id: null, workflow_id: 'wf-prep', kind: 'prepare_libraries' }),
  );
  mockDesktopApi.getTask.mockResolvedValue(makeTaskView());
  mockDesktopApi.summarizeWorkflow.mockResolvedValue(makeBulkSummary());
  mockDesktopApi.getTaskLogs.mockResolvedValue(makeLogPage());
  mockDesktopApi.cancelTask.mockResolvedValue(
    makeTaskView({ status: 'cancelled', finished_at: 1_700_000_002 }),
  );
  mockDesktopApi.cancelWorkflow.mockResolvedValue({
    cancelled_tasks: 1,
    already_complete: false,
  });
}
