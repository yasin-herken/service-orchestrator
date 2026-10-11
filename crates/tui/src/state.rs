//! UI state and the application-state snapshot.
//!
//! Two kinds of state live here, deliberately separated:
//!
//! - [`UiState`] is *presentation* state — the selected screen, the cursor, the
//!   selection set, dialogs, filters, and scroll positions. It is owned by the
//!   interface and never observed by the application core.
//! - [`Snapshot`] is a read-only cache of *application* state — service
//!   statuses, tasks, health, and migration configuration — refreshed from the
//!   application core. It is never the source of truth; the application core
//!   remains authoritative and the cache is replaced wholesale on refresh.
//!
//! Keeping these apart is what lets the interface stay a pure adapter: the UI
//! never re-derives business meaning, and the application never sees a cursor.

use std::collections::{BTreeMap, BTreeSet};

use service_orchestrator_application::{
    LibraryArtifactVerification, LibraryStatus, LogEntry, LogLevel, MigrationStatus, ServiceHealth,
    ServiceStatus, VersionAlignmentPlan,
};
use service_orchestrator_domain::{
    BranchInfo, GitState, Library, LibraryId, RepositoryStatus, Service, ServiceGroup, ServiceId,
    Task, TaskId, WorkflowId, Workspace, WorkspaceId,
};

use crate::action::Action;

/// The minimum terminal width, in columns, at which the full layout is drawn.
pub(crate) const MIN_WIDTH: u16 = 60;

/// The minimum terminal height, in rows, at which the full layout is drawn.
pub(crate) const MIN_HEIGHT: u16 = 14;

/// A top-level screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Screen {
    /// The workspace overview.
    Dashboard,
    /// The service list.
    Services,
    /// The detail view of one service.
    ServiceDetail,
    /// Git repository management.
    Git,
    /// Build management.
    Builds,
    /// Shared library management.
    Libraries,
    /// Database migration management.
    Migrations,
    /// Task monitoring.
    Tasks,
    /// Log viewing.
    Logs,
    /// Health inspection.
    Health,
    /// Read-only settings.
    Settings,
}

impl Screen {
    /// Every screen reachable from the navigation panel, in display order.
    pub const NAV: [Screen; 10] = [
        Screen::Dashboard,
        Screen::Services,
        Screen::Git,
        Screen::Builds,
        Screen::Libraries,
        Screen::Migrations,
        Screen::Tasks,
        Screen::Logs,
        Screen::Health,
        Screen::Settings,
    ];

    /// The human-readable title.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Screen::Dashboard => "Dashboard",
            Screen::Services => "Services",
            Screen::ServiceDetail => "Service",
            Screen::Git => "Git",
            Screen::Builds => "Builds",
            Screen::Libraries => "Libraries",
            Screen::Migrations => "Migrations",
            Screen::Tasks => "Tasks",
            Screen::Logs => "Logs",
            Screen::Health => "Health",
            Screen::Settings => "Settings",
        }
    }

    /// The navigation entry that owns this screen.
    ///
    /// The service detail view is a sub-view of the service list, so it maps to
    /// [`Screen::Services`].
    #[must_use]
    pub const fn nav_parent(self) -> Screen {
        match self {
            Screen::ServiceDetail => Screen::Services,
            other => other,
        }
    }
}

/// Which panel currently has keyboard focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    /// The navigation panel.
    Nav,
    /// The content panel.
    Content,
}

/// A live search query.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchState {
    /// The text typed so far.
    pub query: String,
    /// Whether the query is being edited.
    pub active: bool,
}

/// A filter over the service list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ServiceFilter {
    /// Show every service.
    #[default]
    All,
    /// Show services whose process is active.
    Running,
    /// Show services whose process is not active.
    Stopped,
    /// Show services whose health is not healthy.
    Unhealthy,
}

impl ServiceFilter {
    /// The filter's label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            ServiceFilter::All => "all",
            ServiceFilter::Running => "running",
            ServiceFilter::Stopped => "stopped",
            ServiceFilter::Unhealthy => "unhealthy",
        }
    }

    /// The next filter in the cycle.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            ServiceFilter::All => ServiceFilter::Running,
            ServiceFilter::Running => ServiceFilter::Stopped,
            ServiceFilter::Stopped => ServiceFilter::Unhealthy,
            ServiceFilter::Unhealthy => ServiceFilter::All,
        }
    }

    /// Whether `status` matches the filter.
    #[must_use]
    pub fn matches(self, status: &ServiceStatus) -> bool {
        match self {
            ServiceFilter::All => true,
            ServiceFilter::Running => status.process_state.is_active(),
            ServiceFilter::Stopped => !status.process_state.is_active(),
            ServiceFilter::Unhealthy => !status.health_state.is_healthy(),
        }
    }
}

/// A filter over the task list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TaskFilter {
    /// Show every task.
    #[default]
    All,
    /// Show queued, blocked, and running tasks.
    Active,
    /// Show failed tasks.
    Failed,
    /// Show succeeded tasks.
    Succeeded,
}

impl TaskFilter {
    /// The filter's label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            TaskFilter::All => "all",
            TaskFilter::Active => "active",
            TaskFilter::Failed => "failed",
            TaskFilter::Succeeded => "succeeded",
        }
    }

    /// The next filter in the cycle.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            TaskFilter::All => TaskFilter::Active,
            TaskFilter::Active => TaskFilter::Failed,
            TaskFilter::Failed => TaskFilter::Succeeded,
            TaskFilter::Succeeded => TaskFilter::All,
        }
    }

    /// Whether `task` matches the filter.
    #[must_use]
    pub fn matches(self, task: &Task) -> bool {
        use service_orchestrator_domain::TaskStatus;
        match self {
            TaskFilter::All => true,
            TaskFilter::Active => task.status.is_active(),
            TaskFilter::Failed => task.status == TaskStatus::Failed,
            TaskFilter::Succeeded => task.status == TaskStatus::Succeeded,
        }
    }
}

/// What a log view is following.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogTarget {
    /// A service's logs.
    Service(ServiceId),
    /// A task's logs.
    Task(TaskId),
}

impl LogTarget {
    /// A short label naming the target.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            LogTarget::Service(id) => format!("service {id}"),
            LogTarget::Task(id) => format!("task {id}"),
        }
    }
}

/// The capacity of the in-memory log buffer, in entries.
///
/// The TUI bounds its own buffer so a busy service can never grow the interface
/// without limit, independent of the log service's retention.
pub(crate) const LOG_BUFFER_CAPACITY: usize = 2_000;

/// The log viewer's state.
#[derive(Clone, Debug)]
pub struct LogView {
    /// What is being followed.
    pub target: LogTarget,
    /// The entries currently held, oldest first.
    pub entries: Vec<LogEntry>,
    /// The cursor of the newest entry, for incremental paging.
    pub cursor: Option<u64>,
    /// Whether the view auto-follows new entries.
    pub follow: bool,
    /// The minimum severity shown, or every level.
    pub minimum_level: Option<LogLevel>,
    /// A case-insensitive substring the message must contain.
    pub filter: String,
    /// The vertical scroll offset.
    pub scroll: u16,
}

impl LogView {
    /// Creates a new log view for a target.
    #[must_use]
    pub fn new(target: LogTarget) -> Self {
        Self {
            target,
            entries: Vec::new(),
            cursor: None,
            follow: true,
            minimum_level: None,
            filter: String::new(),
            scroll: 0,
        }
    }

    /// Replaces the held entries, keeping only the newest
    /// [`LOG_BUFFER_CAPACITY`] entries.
    pub fn replace(&mut self, entries: Vec<LogEntry>) {
        self.entries = entries;
        self.trim();
    }

    /// Appends entries, keeping only the newest [`LOG_BUFFER_CAPACITY`].
    pub fn append(&mut self, entries: impl IntoIterator<Item = LogEntry>) {
        self.entries.extend(entries);
        self.trim();
    }

    fn trim(&mut self) {
        if self.entries.len() > LOG_BUFFER_CAPACITY {
            let excess = self.entries.len() - LOG_BUFFER_CAPACITY;
            self.entries.drain(0..excess);
        }
    }
}

/// The severity of a transient notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastLevel {
    /// Neutral information.
    Info,
    /// A completed operation.
    Success,
    /// A caution or partial result.
    Warning,
    /// A failure.
    Error,
}

/// A transient notification shown in the footer.
#[derive(Clone, Debug)]
pub struct Toast {
    /// The message text.
    pub message: String,
    /// The severity.
    pub level: ToastLevel,
    /// Remaining ticks before the toast expires.
    pub ttl: u8,
}

impl Toast {
    /// Creates a toast that lives for a fixed number of ticks.
    #[must_use]
    pub fn new(message: impl Into<String>, level: ToastLevel) -> Self {
        Self {
            message: message.into(),
            level,
            ttl: 8,
        }
    }
}

/// One option in a selection dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectOption {
    /// The label shown to the user.
    pub label: String,
    /// The action dispatched when the option is chosen.
    pub action: Action,
}

/// Why an input dialog was opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputPurpose {
    /// Check out a branch in a service's repository.
    CheckoutBranch(ServiceId),
    /// Choose a branch for an environment preparation.
    PrepareBranch(Box<service_orchestrator_application::PrepareEnvironmentRequest>),
}

/// A confirmation dialog for a potentially destructive action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfirmDialog {
    /// The dialog title.
    pub title: String,
    /// The lines of explanatory text.
    pub lines: Vec<String>,
    /// The action to run once confirmed.
    pub action: Action,
    /// Whether the action is destructive.
    pub destructive: bool,
}

/// A read-only information, error, or result dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageDialog {
    /// The dialog title.
    pub title: String,
    /// The lines of text.
    pub lines: Vec<String>,
}

impl MessageDialog {
    /// Creates a dialog from a title and lines.
    #[must_use]
    pub fn new(title: impl Into<String>, lines: Vec<String>) -> Self {
        Self {
            title: title.into(),
            lines,
        }
    }
}

/// A single-line text input dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputDialog {
    /// The prompt shown above the field.
    pub prompt: String,
    /// The text typed so far.
    pub value: String,
    /// Why the dialog was opened.
    pub purpose: InputPurpose,
}

/// A list selection dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectDialog {
    /// The dialog title.
    pub title: String,
    /// The options.
    pub options: Vec<SelectOption>,
    /// The highlighted option.
    pub index: usize,
}

/// The active modal dialog, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialog {
    /// A confirmation prompt.
    Confirm(ConfirmDialog),
    /// An informational message.
    Info(MessageDialog),
    /// An error message.
    Error(MessageDialog),
    /// The help overlay.
    Help,
    /// A text input prompt.
    Input(InputDialog),
    /// A list selection prompt.
    Select(SelectDialog),
}

/// All presentation state.
#[derive(Clone, Debug)]
pub struct UiState {
    /// The active screen.
    pub screen: Screen,
    /// The index into [`Screen::NAV`] highlighted in the navigation panel.
    pub nav_index: usize,
    /// Which panel has focus.
    pub focus: Focus,
    /// The cursor row within the content list.
    pub cursor: usize,
    /// The first visible content row.
    pub list_offset: usize,
    /// The number of content rows visible in the last frame.
    pub viewport_rows: usize,
    /// The selected services, by typed identifier.
    pub selection: BTreeSet<ServiceId>,
    /// The selected libraries, by typed identifier.
    pub library_selection: BTreeSet<LibraryId>,
    /// The active dialog, if any.
    pub dialog: Option<Dialog>,
    /// The active search query, if any.
    pub search: Option<SearchState>,
    /// The service-list filter.
    pub service_filter: ServiceFilter,
    /// The task-list filter.
    pub task_filter: TaskFilter,
    /// The last observed Git status per service, populated on demand.
    pub git_status: BTreeMap<ServiceId, RepositoryStatus>,
    /// The last listed branches per service, populated on demand.
    pub branches: BTreeMap<ServiceId, Vec<BranchInfo>>,
    /// The service whose branches were most recently requested.
    pub pending_branch_service: Option<ServiceId>,
    /// The last observed status per library, populated on demand.
    pub library_status: BTreeMap<LibraryId, LibraryStatus>,
    /// The last artifact verification per library, populated on demand.
    pub artifact_verification: BTreeMap<LibraryId, LibraryArtifactVerification>,
    /// The last version-alignment plan per library, populated on demand.
    pub alignment: BTreeMap<LibraryId, VersionAlignmentPlan>,
    /// The active log view, if any.
    pub log_view: Option<LogView>,
    /// The active transient notification, if any.
    pub toast: Option<Toast>,
    /// The vertical scroll of a long detail pane.
    pub detail_scroll: u16,
    /// The workspace whose state the snapshot reflects, when one is selected.
    ///
    /// The interface remembers this independently of the snapshot so cycling
    /// workspaces survives a wholesale snapshot replacement.
    pub active_workspace: Option<WorkspaceId>,
    /// Workflows submitted by a bulk operation whose partial-failure summary has
    /// not yet been reported.
    pub pending_workflows: Vec<WorkflowId>,
    /// Whether the interface should exit.
    pub should_quit: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            screen: Screen::Dashboard,
            nav_index: 0,
            focus: Focus::Content,
            cursor: 0,
            list_offset: 0,
            viewport_rows: 0,
            selection: BTreeSet::new(),
            library_selection: BTreeSet::new(),
            dialog: None,
            search: None,
            service_filter: ServiceFilter::All,
            task_filter: TaskFilter::All,
            git_status: BTreeMap::new(),
            branches: BTreeMap::new(),
            pending_branch_service: None,
            library_status: BTreeMap::new(),
            artifact_verification: BTreeMap::new(),
            alignment: BTreeMap::new(),
            log_view: None,
            toast: None,
            detail_scroll: 0,
            active_workspace: None,
            pending_workflows: Vec::new(),
            should_quit: false,
        }
    }
}

impl UiState {
    /// Creates default UI state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current search query, if searching.
    #[must_use]
    pub fn query(&self) -> Option<&str> {
        self.search
            .as_ref()
            .filter(|search| !search.query.is_empty())
            .map(|search| search.query.as_str())
    }

    /// Whether a search is currently being edited.
    #[must_use]
    pub fn searching(&self) -> bool {
        self.search.as_ref().is_some_and(|search| search.active)
    }

    /// Moves the cursor by `delta` within a list of `len` rows.
    pub fn move_cursor(&mut self, len: usize, delta: isize) {
        if len == 0 {
            self.cursor = 0;
            self.list_offset = 0;
            return;
        }
        let last = len - 1;
        let next = (self.cursor as isize + delta).clamp(0, last as isize) as usize;
        self.cursor = next;
        self.ensure_visible();
    }

    /// Moves the cursor to an absolute index, clamped to the list.
    pub fn select_cursor(&mut self, len: usize, index: usize) {
        if len == 0 {
            self.cursor = 0;
            self.list_offset = 0;
            return;
        }
        self.cursor = index.min(len - 1);
        self.ensure_visible();
    }

    fn ensure_visible(&mut self) {
        let rows = self.viewport_rows.max(1);
        if self.cursor < self.list_offset {
            self.list_offset = self.cursor;
        } else if self.cursor >= self.list_offset + rows {
            self.list_offset = self.cursor + 1 - rows;
        }
    }

    /// Resets the cursor and scroll when the active list changes.
    pub(crate) fn reset_cursor(&mut self) {
        self.cursor = 0;
        self.list_offset = 0;
        self.detail_scroll = 0;
    }
}

/// A read-only cache of application state.
///
/// It is replaced wholesale on refresh; partial merges would risk showing a
/// state the application core never produced.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Every configured workspace.
    pub workspaces: Vec<Workspace>,
    /// The workspace currently in focus.
    pub active_workspace: Option<WorkspaceId>,
    /// The active workspace's display name.
    pub workspace_name: String,
    /// The active workspace's root path.
    pub workspace_root: Option<String>,
    /// The configuration schema version.
    pub config_version: u32,
    /// The configured maximum number of tasks executed in parallel.
    pub max_parallel_tasks: usize,
    /// The configured default operation timeout, in seconds.
    pub default_timeout_seconds: u64,
    /// The status of every service in the active workspace.
    pub services: Vec<ServiceStatus>,
    /// The validated configuration of every service in the active workspace,
    /// keyed by identifier.
    ///
    /// Statuses carry observed runtime state; this carries the static
    /// configuration (repository, path, dependencies, build system) the detail
    /// view and build view need.
    pub services_config: BTreeMap<ServiceId, Service>,
    /// The libraries in the active workspace.
    pub libraries: Vec<Library>,
    /// The service groups in the active workspace.
    pub groups: Vec<ServiceGroup>,
    /// The configured migration status of every migration-enabled service.
    pub migrations: BTreeMap<ServiceId, MigrationStatus>,
    /// The last observed health of every service.
    pub health: BTreeMap<ServiceId, ServiceHealth>,
    /// Active tasks first, then recently finished tasks, newest first.
    pub tasks: Vec<Task>,
    /// The last observed Git state per service, taken from the runtime store.
    pub git_states: BTreeMap<ServiceId, GitState>,
    /// The services in the active workspace that have a configured repository.
    pub repositories: BTreeSet<ServiceId>,
}

impl Snapshot {
    /// The status of a service by id, if present.
    #[must_use]
    pub fn service(&self, id: &ServiceId) -> Option<&ServiceStatus> {
        self.services.iter().find(|status| &status.service_id == id)
    }

    /// The configuration of a service by id, if present.
    #[must_use]
    pub fn service_config(&self, id: &ServiceId) -> Option<&Service> {
        self.services_config.get(id)
    }

    /// The library by id, if present.
    #[must_use]
    pub fn library(&self, id: &LibraryId) -> Option<&Library> {
        self.libraries.iter().find(|library| &library.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_detail_is_a_navigable_sub_view_of_services() {
        assert_eq!(Screen::ServiceDetail.nav_parent(), Screen::Services);
        assert_eq!(Screen::Services.nav_parent(), Screen::Services);
        assert_eq!(Screen::Tasks.nav_parent(), Screen::Tasks);
    }

    #[test]
    fn cursors_are_clamped_to_the_list_bounds() {
        let mut state = UiState::new();
        state.viewport_rows = 3;

        state.move_cursor(0, 1);
        assert_eq!(state.cursor, 0);

        state.move_cursor(5, 10);
        assert_eq!(state.cursor, 4);
        state.move_cursor(5, -10);
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn cursor_scrolling_keeps_the_selection_visible() {
        let mut state = UiState::new();
        state.viewport_rows = 3;
        state.move_cursor(20, 19);
        assert_eq!(state.cursor, 19);
        assert_eq!(state.list_offset, 17);
        // The selected row must be within [offset, offset + rows).
        assert!(state.cursor >= state.list_offset);
        assert!(state.cursor < state.list_offset + state.viewport_rows);
    }

    #[test]
    fn service_filters_cycle_through_every_variant() {
        assert_eq!(ServiceFilter::All.next(), ServiceFilter::Running);
        assert_eq!(ServiceFilter::Running.next(), ServiceFilter::Stopped);
        assert_eq!(ServiceFilter::Stopped.next(), ServiceFilter::Unhealthy);
        assert_eq!(ServiceFilter::Unhealthy.next(), ServiceFilter::All);
    }

    #[test]
    fn task_filters_cycle_through_every_variant() {
        assert_eq!(TaskFilter::All.next(), TaskFilter::Active);
        assert_eq!(TaskFilter::Active.next(), TaskFilter::Failed);
        assert_eq!(TaskFilter::Failed.next(), TaskFilter::Succeeded);
        assert_eq!(TaskFilter::Succeeded.next(), TaskFilter::All);
    }

    #[test]
    fn log_view_keeps_only_the_newest_entries() {
        let mut view = LogView::new(LogTarget::Service(
            ServiceId::new("auth-service").expect("valid id"),
        ));
        let entries = (0..(LOG_BUFFER_CAPACITY + 50))
            .map(|index| {
                let mut entry = LogEntry::new(
                    service_orchestrator_application::LogLevel::Info,
                    format!("entry {index}"),
                );
                entry.sequence = index as u64;
                entry
            })
            .collect::<Vec<_>>();
        view.append(entries);
        assert_eq!(view.entries.len(), LOG_BUFFER_CAPACITY);
        // The oldest entries are the ones that were dropped.
        assert_eq!(view.entries[0].message, "entry 50");
    }
}
