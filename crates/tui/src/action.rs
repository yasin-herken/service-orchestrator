//! User intents.
//!
//! An [`Action`] is a request produced by the input handler and consumed by the
//! application state machine ([`crate::app::App`]). Some actions only move the
//! cursor or open a dialog; the rest are *dispatched* to the application core as
//! a use case. No action executes infrastructure directly.

use service_orchestrator_application::PrepareEnvironmentRequest;
use service_orchestrator_domain::{LibraryId, ServiceId, TaskId, WorkflowId};

use crate::state::Screen;

/// A single user intent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Exit the interface.
    Quit,
    /// Open the help overlay.
    Help,
    /// Re-read the application snapshot.
    Refresh,
    /// Move to a screen.
    SwitchScreen(Screen),
    /// Switch to the next configured workspace.
    CycleWorkspace,

    /// Move focus to the navigation panel.
    FocusNav,
    /// Move focus to the content panel.
    FocusContent,
    /// Move the cursor up one row.
    MoveUp,
    /// Move the cursor down one row.
    MoveDown,
    /// Move the cursor up one page.
    PageUp,
    /// Move the cursor down one page.
    PageDown,
    /// Move the cursor to the first row.
    First,
    /// Move the cursor to the last row.
    Last,

    /// Toggle the selection of the current row.
    ToggleSelect,
    /// Select every visible row.
    SelectAllVisible,
    /// Clear the current selection.
    ClearSelection,

    /// Open the details of the current row.
    OpenDetails,
    /// Return to the parent view.
    Back,

    /// Begin editing the search query.
    StartSearch,
    /// Commit the current search query.
    CommitSearch,
    /// Cancel the search query.
    CancelSearch,

    /// Close the active dialog without acting.
    CancelDialog,
    /// Confirm the active dialog.
    ConfirmDialog,

    /// Start a service.
    StartService(ServiceId),
    /// Stop a service.
    StopService(ServiceId),
    /// Restart a service.
    RestartService(ServiceId),
    /// Build a service.
    BuildService(ServiceId),
    /// Synchronize a service's repository.
    SyncService(ServiceId),
    /// Fetch a service's repository.
    FetchService(ServiceId),
    /// Pull a service's repository.
    PullService(ServiceId),
    /// Run a health check for a service.
    CheckHealth(ServiceId),
    /// Show a service's logs.
    ViewServiceLogs(ServiceId),
    /// Check out a branch in a service's repository.
    CheckoutBranch(ServiceId, String),
    /// Ask which branch to check out by listing the repository's branches.
    RequestCheckout(ServiceId),
    /// Hard-reset a service's repository (destructive).
    ResetRepositoryHard(ServiceId),
    /// Clean a service's repository (destructive).
    CleanRepository(ServiceId),
    /// Discard a service's local changes (destructive).
    DiscardLocalChanges(ServiceId),
    /// Refresh the cached Git status of a service.
    RefreshGitStatus(ServiceId),

    /// Start every selected service.
    StartSelected,
    /// Stop every selected service.
    StopSelected,
    /// Restart every selected service.
    RestartSelected,
    /// Build every selected service.
    BuildSelected,
    /// Synchronize the selected services' repositories.
    SyncSelected,
    /// Fetch the selected services' repositories.
    FetchSelected,
    /// Run a health check for every selected service.
    CheckHealthSelected,

    /// Cycle the service filter.
    CycleServiceFilter,

    /// Build a library.
    BuildLibrary(LibraryId),
    /// Install a library's artifact locally.
    InstallLibrary(LibraryId),
    /// Prepare a library (sync, build, install, verify).
    PrepareLibrary(LibraryId),
    /// Synchronize a library's repository.
    SyncLibrary(LibraryId),
    /// Verify a library's artifact in the local repository.
    VerifyLibrary(LibraryId),
    /// Inspect a library's consumer version alignment.
    InspectAlignment(LibraryId),
    /// Refresh the cached status of a library.
    RefreshLibraryStatus(LibraryId),

    /// Inspect a service's migration status.
    MigrationStatus(ServiceId),
    /// Inspect a service's pending changesets.
    MigrationPending(ServiceId),
    /// Preview a service's pending migration.
    MigrationPreview(ServiceId),
    /// Validate a service's migration configuration.
    MigrationValidate(ServiceId),
    /// Apply a service's pending migrations.
    MigrationApply(ServiceId),

    /// Select the task under the cursor in the tasks screen.
    SelectTask(TaskId),
    /// Cancel a task.
    CancelTask(TaskId),
    /// Show a task's logs.
    ViewTaskLogs(TaskId),
    /// Cycle the task-status filter.
    CycleTaskFilter,

    /// Toggle log follow mode.
    ToggleLogFollow,
    /// Clear the visible log buffer.
    ClearLogs,
    /// Cycle the minimum log level.
    CycleLogLevel,

    /// Synchronize every repository in the workspace.
    SyncWorkspace,
    /// Build every library and buildable service in the workspace.
    BuildWorkspace,

    /// Open the Prepare Environment options for the active workspace.
    PrepareEnvironment,
    /// Choose a branch for an environment preparation, then plan it.
    PrepareEnvironmentBranch(Box<PrepareEnvironmentRequest>),
    /// Plan (preview) an environment preparation without changing anything.
    PlanPrepareEnvironment(Box<PrepareEnvironmentRequest>),
    /// Execute a reviewed environment preparation plan.
    ConfirmPrepareEnvironment(Box<PrepareEnvironmentRequest>),
    /// Open a chooser for cancelling an active workflow.
    RequestCancelWorkflow,
    /// Cancel every active task in a workflow.
    CancelWorkflow(WorkflowId),
}
