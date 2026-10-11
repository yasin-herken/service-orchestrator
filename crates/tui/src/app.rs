//! The application state machine.
//!
//! [`App`] is the single place where an [`Action`] becomes either a UI-state
//! change or an application-core call. It owns the interface's [`UiState`] and a
//! read-only [`Snapshot`] of application state, and it emits [`Effect`]s for the
//! few queries that are genuinely asynchronous (Git inspection, branch listing,
//! artifact inspection, realtime log streaming).
//!
//! Nothing here executes infrastructure. Every mutating operation goes through
//! [`Application`]; every long-running operation returns a task or workflow id
//! that the tasks screen observes through the task engine, never through a task
//! registry of the interface's own.

use std::sync::Arc;

use service_orchestrator_application::{
    Application, ApplicationError, Authorization, HealthCheckScope, LibraryPrepareOptions,
    OperationResult,
};
use service_orchestrator_domain::{
    Branch, LibraryId, Permission, ServiceId, Task, TaskId, WorkspaceId,
};

use crate::action::Action;
use crate::event::{AppEvent, AsyncOutcome, Effect};
use crate::input::{InputEvent, Key};
use crate::observer::TaskObserver;
use crate::state::{
    ConfirmDialog, Dialog, Focus, InputPurpose, LogTarget, LogView, MessageDialog, Screen,
    SelectDialog, SelectOption, Snapshot, Toast, ToastLevel, UiState, LOG_BUFFER_CAPACITY,
};

/// How many finished tasks the snapshot retains for the tasks screen.
const TASK_HISTORY_LIMIT: usize = 200;

/// How many ticks pass between full snapshot refreshes.
///
/// Cheap, in-memory queries refresh more often; this bounds the slightly more
/// expensive snapshot rebuild without polling any repository.
const REFRESH_EVERY_TICKS: u64 = 4;

/// The interface's application state machine.
pub struct App {
    /// The application core this interface drives.
    application: Application,
    /// The read-only view of the task engine.
    observer: Arc<dyn TaskObserver>,
    /// All presentation state.
    pub(crate) state: UiState,
    /// The read-only cache of application state.
    pub(crate) snapshot: Snapshot,
    /// Asynchronous queries the event loop should run.
    pub(crate) effects: Vec<Effect>,
    /// The configuration path, shown on the settings screen.
    config_path: String,
    /// A monotonic tick counter, used to schedule periodic refreshes.
    pub(crate) ticks: u64,
}

impl App {
    /// Creates an interface bound to an application core and a task observer.
    #[must_use]
    pub fn new(
        application: Application,
        observer: Arc<dyn TaskObserver>,
        config_path: impl Into<String>,
    ) -> Self {
        Self {
            application,
            observer,
            state: UiState::new(),
            snapshot: Snapshot::default(),
            effects: Vec::new(),
            config_path: config_path.into(),
            ticks: 0,
        }
    }

    /// Returns the current presentation state.
    #[must_use]
    pub fn state(&self) -> &UiState {
        &self.state
    }

    /// Returns the current application-state snapshot.
    #[must_use]
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// Returns the configuration path shown on the settings screen.
    #[must_use]
    pub fn config_path(&self) -> &str {
        &self.config_path
    }

    /// Returns `true` when the interface should exit.
    #[must_use]
    pub fn should_quit(&self) -> bool {
        self.state.should_quit
    }

    /// Removes and returns the effects the event loop should run.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// Rebuilds the application-state snapshot.
    ///
    /// # Errors
    ///
    /// Returns the application's error when the active workspace cannot be read.
    pub fn refresh(&mut self) -> Result<(), ApplicationError> {
        let (config_version, max_parallel_tasks, default_timeout_seconds) = {
            let config = self.application.config();
            (
                config.version,
                config.execution.max_parallel_tasks,
                config.execution.default_timeout_seconds,
            )
        };
        let workspaces = self.application.list_workspaces();
        let active = self
            .state
            .active_workspace
            .clone()
            .filter(|id| workspaces.iter().any(|workspace| &workspace.id == id))
            .or_else(|| workspaces.first().map(|workspace| workspace.id.clone()));

        let mut snapshot = Snapshot {
            workspaces,
            config_version,
            max_parallel_tasks,
            default_timeout_seconds,
            ..Snapshot::default()
        };
        snapshot.active_workspace = active.clone();

        if let Some(workspace) = active.clone() {
            let status = self.application.get_workspace_status(&workspace)?;
            snapshot.workspace_name = status.name;
            snapshot.workspace_root = Some(status.root.to_string());
            snapshot.services = status.services;
            snapshot.libraries = self.application.list_libraries(&workspace)?;
            snapshot.groups = self.application.list_groups(&workspace)?;
            for service in self.application.list_services(&workspace)? {
                let id = service.id.clone();
                if service.repository.is_some() {
                    snapshot.repositories.insert(id.clone());
                }
                snapshot.services_config.insert(id, service);
            }
            for service in &snapshot.services {
                snapshot
                    .git_states
                    .insert(service.service_id.clone(), service.git_state);
                if let Ok(migration) = self.application.migration_status(&service.service_id) {
                    snapshot
                        .migrations
                        .insert(service.service_id.clone(), migration);
                }
            }
            if let Ok(health) = self.application.list_service_health(&workspace) {
                for record in health {
                    snapshot.health.insert(record.service_id.clone(), record);
                }
            }
        }

        snapshot.tasks = self.collect_tasks();
        self.state.active_workspace = active;
        self.snapshot = snapshot;
        self.clamp_cursor();
        Ok(())
    }

    /// Handles one interface event.
    pub fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Input(input) => self.handle_input(input),
            AppEvent::Task(event) => self.handle_task_event(&event),
            AppEvent::Async(outcome) => self.handle_async(*outcome),
            AppEvent::Log(entry) => self.handle_log_entry(entry),
            AppEvent::Tick => self.tick(),
        }
    }

    /// Applies an action.
    pub fn apply(&mut self, action: Action) {
        match action {
            Action::Quit => self.state.should_quit = true,
            Action::Help => self.state.dialog = Some(Dialog::Help),
            Action::Refresh => match self.refresh() {
                Ok(()) => self.toast("refreshed", ToastLevel::Info),
                Err(error) => self.show_error("refresh", &error),
            },
            Action::SwitchScreen(screen) => self.switch_screen(screen),
            Action::CycleWorkspace => self.cycle_workspace(),

            Action::FocusNav => self.state.focus = Focus::Nav,
            Action::FocusContent => self.state.focus = Focus::Content,
            Action::MoveUp => self.move_cursor(-1),
            Action::MoveDown => self.move_cursor(1),
            Action::PageUp => self.page(-1),
            Action::PageDown => self.page(1),
            Action::First => self.select_row(0),
            Action::Last => {
                let last = self.current_row_count().saturating_sub(1);
                self.select_row(last);
            }

            Action::ToggleSelect => self.toggle_select(),
            Action::SelectAllVisible => self.select_all_visible(),
            Action::ClearSelection => self.clear_selection(),

            Action::OpenDetails => self.open_details(),
            Action::Back => self.back(),

            Action::StartSearch => self.start_search(),
            Action::CommitSearch => self.commit_search(),
            Action::CancelSearch => self.cancel_search(),

            Action::CancelDialog => self.state.dialog = None,
            Action::ConfirmDialog => self.confirm_dialog(),

            Action::ViewServiceLogs(id) => self.view_service_logs(&id),
            Action::ViewTaskLogs(id) => self.view_task_logs(&id),
            Action::RequestCheckout(id) => self.request_checkout(&id),
            Action::RefreshGitStatus(id) => self.effects.push(Effect::GitStatus(id)),
            Action::VerifyLibrary(id) => self.effects.push(Effect::VerifyArtifact(id)),
            Action::InspectAlignment(id) => self.effects.push(Effect::Alignment(id)),
            Action::RefreshLibraryStatus(id) => self.effects.push(Effect::LibraryStatus(id)),

            Action::CycleServiceFilter => {
                self.state.service_filter = self.state.service_filter.next();
                self.state.reset_cursor();
                self.toast(
                    format!("filter: {}", self.state.service_filter.label()),
                    ToastLevel::Info,
                );
            }
            Action::CycleTaskFilter => {
                self.state.task_filter = self.state.task_filter.next();
                self.state.reset_cursor();
                self.toast(
                    format!("filter: {}", self.state.task_filter.label()),
                    ToastLevel::Info,
                );
            }

            Action::CycleLogLevel => self.cycle_log_level(),
            Action::ToggleLogFollow => self.toggle_log_follow(),
            Action::ClearLogs => {
                if let Some(view) = &mut self.state.log_view {
                    view.replace(Vec::new());
                    view.scroll = 0;
                }
            }

            Action::SelectTask(id) => self.select_task(&id),
            Action::CancelTask(id) => self.cancel_task(&id),

            Action::StartSelected
            | Action::StopSelected
            | Action::RestartSelected
            | Action::BuildSelected
            | Action::SyncSelected
            | Action::FetchSelected
            | Action::CheckHealthSelected
            | Action::StartService(_)
            | Action::StopService(_)
            | Action::RestartService(_)
            | Action::BuildService(_)
            | Action::SyncService(_)
            | Action::FetchService(_)
            | Action::PullService(_)
            | Action::CheckHealth(_)
            | Action::CheckoutBranch(_, _)
            | Action::ResetRepositoryHard(_)
            | Action::CleanRepository(_)
            | Action::DiscardLocalChanges(_)
            | Action::BuildLibrary(_)
            | Action::InstallLibrary(_)
            | Action::PrepareLibrary(_)
            | Action::SyncLibrary(_)
            | Action::MigrationStatus(_)
            | Action::MigrationPending(_)
            | Action::MigrationPreview(_)
            | Action::MigrationValidate(_)
            | Action::MigrationApply(_)
            | Action::SyncWorkspace
            | Action::BuildWorkspace => self.dispatch_application(action),
        }
    }

    // -- input ---------------------------------------------------------------

    fn handle_input(&mut self, input: InputEvent) {
        match input {
            InputEvent::Resize { .. } => {
                // Layout is recomputed on the next draw; nothing to do here.
            }
            InputEvent::Key(key) => self.handle_key(key),
        }
    }

    fn handle_key(&mut self, key: Key) {
        if key == Key::CtrlC || key == Key::CtrlD {
            self.state.should_quit = true;
            return;
        }
        if matches!(key, Key::Esc) && self.state.dialog.is_some() {
            self.state.dialog = None;
            return;
        }
        if self.state.dialog.is_some() {
            self.handle_dialog_key(key);
            return;
        }
        if self.state.searching() {
            self.handle_search_key(key);
            return;
        }
        // Global keys that work in either panel.
        match key {
            Key::Char('?') => return self.apply(Action::Help),
            Key::Char('q') => return self.apply(Action::Quit),
            Key::Char('/') => return self.apply(Action::StartSearch),
            Key::Char('R') => return self.apply(Action::Refresh),
            Key::Char('W') => return self.apply(Action::CycleWorkspace),
            _ => {}
        }

        if self.state.focus == Focus::Nav {
            self.handle_nav_key(key);
        } else if let Some(action) = self.content_action(key) {
            self.apply(action);
        }
    }

    fn handle_nav_key(&mut self, key: Key) {
        let last = Screen::NAV.len() - 1;
        match key {
            Key::Up | Key::Char('k') => self.nav_move(-1),
            Key::Down | Key::Char('j') => self.nav_move(1),
            Key::Home | Key::Char('g') => self.set_nav(0),
            Key::End | Key::Char('G') => self.set_nav(last),
            Key::Enter | Key::Right | Key::Tab => self.activate_nav(),
            _ => {}
        }
    }

    fn handle_search_key(&mut self, key: Key) {
        match key {
            Key::Esc => self.cancel_search(),
            Key::Enter => self.commit_search(),
            Key::Backspace => {
                if let Some(search) = &mut self.state.search {
                    search.query.pop();
                }
            }
            Key::Char(c) => {
                if let Some(search) = &mut self.state.search {
                    search.query.push(c);
                }
            }
            _ => {}
        }
    }

    fn handle_dialog_key(&mut self, key: Key) {
        if let Some(Dialog::Input(dialog)) = &mut self.state.dialog {
            match key {
                Key::Enter => self.confirm_dialog(),
                Key::Backspace => {
                    dialog.value.pop();
                }
                Key::Char(c) => dialog.value.push(c),
                _ => {}
            }
            return;
        }
        match key {
            Key::Enter => self.confirm_dialog(),
            Key::Up | Key::Char('k') => self.dialog_move(-1),
            Key::Down | Key::Char('j') => self.dialog_move(1),
            _ => {}
        }
    }

    fn content_action(&self, key: Key) -> Option<Action> {
        match key {
            Key::Up | Key::Char('k') => return Some(Action::MoveUp),
            Key::Down | Key::Char('j') => return Some(Action::MoveDown),
            Key::Char(' ') => return Some(Action::ToggleSelect),
            Key::PageUp => return Some(Action::PageUp),
            Key::PageDown => return Some(Action::PageDown),
            Key::Home => return Some(Action::First),
            Key::End => return Some(Action::Last),
            Key::Tab | Key::Left => return Some(Action::FocusNav),
            Key::Esc => return Some(Action::Back),
            Key::Backspace | Key::Delete => {}
            _ => {}
        }
        match self.state.screen {
            Screen::Dashboard => match key {
                Key::Enter | Key::Char('s') => Some(Action::SwitchScreen(Screen::Services)),
                Key::Char('g') => Some(Action::SwitchScreen(Screen::Git)),
                Key::Char('b') => Some(Action::SwitchScreen(Screen::Builds)),
                Key::Char('l') => Some(Action::SwitchScreen(Screen::Libraries)),
                Key::Char('m') => Some(Action::SwitchScreen(Screen::Migrations)),
                Key::Char('t') => Some(Action::SwitchScreen(Screen::Tasks)),
                Key::Char('o') => Some(Action::SwitchScreen(Screen::Logs)),
                Key::Char('h') => Some(Action::SwitchScreen(Screen::Health)),
                _ => None,
            },
            Screen::Services | Screen::ServiceDetail => self.services_action(key),
            Screen::Git => self.git_action(key),
            Screen::Builds => self.builds_action(key),
            Screen::Libraries => self.libraries_action(key),
            Screen::Migrations => self.migrations_action(key),
            Screen::Tasks => self.tasks_action(key),
            Screen::Logs => self.logs_action(key),
            Screen::Health => self.health_action(key),
            Screen::Settings => None,
        }
    }

    fn services_action(&self, key: Key) -> Option<Action> {
        let current = || self.current_service();
        match key {
            Key::Enter => Some(Action::OpenDetails),
            Key::Char('s') => current().map(Action::StartService),
            Key::Char('S') => Some(Action::StartSelected),
            Key::Char('x') => current().map(Action::StopService),
            Key::Char('X') => Some(Action::StopSelected),
            Key::Char('r') => current().map(Action::RestartService),
            Key::Char('B') => Some(Action::BuildSelected),
            Key::Char('b') => current().map(Action::BuildService),
            Key::Char('f') => current().map(Action::FetchService),
            Key::Char('F') => Some(Action::FetchSelected),
            Key::Char('y') => current().map(Action::SyncService),
            Key::Char('Y') => Some(Action::SyncSelected),
            Key::Char('p') => current().map(Action::PullService),
            Key::Char('h') => current().map(Action::CheckHealth),
            Key::Char('H') => Some(Action::CheckHealthSelected),
            Key::Char('l') => current().map(Action::ViewServiceLogs),
            Key::Char('g') => Some(Action::SwitchScreen(Screen::Git)),
            Key::Char('a') => Some(Action::SelectAllVisible),
            Key::Char('c') => Some(Action::ClearSelection),
            Key::Char('t') => Some(Action::CycleServiceFilter),
            _ => None,
        }
    }

    fn git_action(&self, key: Key) -> Option<Action> {
        let current = || self.current_service();
        match key {
            Key::Enter | Key::Char('c') => current().map(Action::RequestCheckout),
            Key::Char('C') => Some(Action::ClearSelection),
            Key::Char('f') => current().map(Action::FetchService),
            Key::Char('F') => Some(Action::FetchSelected),
            Key::Char('y') => current().map(Action::SyncService),
            Key::Char('Y') => Some(Action::SyncSelected),
            Key::Char('p') => current().map(Action::PullService),
            Key::Char('r') => current().map(Action::RefreshGitStatus),
            Key::Char('d') => current().map(Action::ResetRepositoryHard),
            Key::Char('D') => current().map(Action::DiscardLocalChanges),
            Key::Char('n') => current().map(Action::CleanRepository),
            Key::Char('l') => current().map(Action::ViewServiceLogs),
            Key::Char('a') => Some(Action::SelectAllVisible),
            _ => None,
        }
    }

    fn builds_action(&self, key: Key) -> Option<Action> {
        let current = || self.current_service();
        match key {
            Key::Enter | Key::Char('b') => current().map(Action::BuildService),
            Key::Char('B') => Some(Action::BuildSelected),
            Key::Char('l') => current().map(Action::ViewServiceLogs),
            Key::Char('t') => Some(Action::SwitchScreen(Screen::Tasks)),
            Key::Char('c') => Some(Action::ClearSelection),
            _ => None,
        }
    }

    fn libraries_action(&self, key: Key) -> Option<Action> {
        let current = || self.current_library_id();
        match key {
            Key::Enter | Key::Char('b') => current().map(Action::BuildLibrary),
            Key::Char('i') => current().map(Action::InstallLibrary),
            Key::Char('p') => current().map(Action::PrepareLibrary),
            Key::Char('y') => current().map(Action::SyncLibrary),
            Key::Char('v') => current().map(Action::VerifyLibrary),
            Key::Char('a') => current().map(Action::InspectAlignment),
            Key::Char('r') => current().map(Action::RefreshLibraryStatus),
            Key::Char('c') => Some(Action::ClearSelection),
            _ => None,
        }
    }

    fn migrations_action(&self, key: Key) -> Option<Action> {
        let current = || self.current_migration_service();
        match key {
            Key::Enter | Key::Char('s') => current().map(Action::MigrationStatus),
            Key::Char('p') => current().map(Action::MigrationPending),
            Key::Char('P') => current().map(Action::MigrationPreview),
            Key::Char('v') => current().map(Action::MigrationValidate),
            Key::Char('a') => current().map(Action::MigrationApply),
            Key::Char('l') => current().map(Action::ViewServiceLogs),
            _ => None,
        }
    }

    fn tasks_action(&self, key: Key) -> Option<Action> {
        let current = || self.current_task_id();
        match key {
            Key::Enter | Key::Char('l') => current().map(Action::ViewTaskLogs),
            Key::Char('x') => current().map(Action::CancelTask),
            Key::Char('t') => Some(Action::CycleTaskFilter),
            _ => None,
        }
    }

    fn logs_action(&self, key: Key) -> Option<Action> {
        match key {
            Key::Char('f') => Some(Action::ToggleLogFollow),
            Key::Char('c') => Some(Action::ClearLogs),
            Key::Char('v') => Some(Action::CycleLogLevel),
            _ => None,
        }
    }

    fn health_action(&self, key: Key) -> Option<Action> {
        let current = || self.current_service();
        match key {
            Key::Enter | Key::Char('h') => current().map(Action::CheckHealth),
            Key::Char('H') => Some(Action::CheckHealthSelected),
            Key::Char('l') => current().map(Action::ViewServiceLogs),
            Key::Char('c') => Some(Action::ClearSelection),
            _ => None,
        }
    }

    // -- navigation and selection -------------------------------------------

    fn nav_move(&mut self, delta: isize) {
        let last = (Screen::NAV.len() - 1) as isize;
        let next = (self.state.nav_index as isize + delta).clamp(0, last) as usize;
        self.set_nav(next);
    }

    fn set_nav(&mut self, index: usize) {
        self.state.nav_index = index.min(Screen::NAV.len() - 1);
        self.state.screen = Screen::NAV[self.state.nav_index];
        self.state.reset_cursor();
    }

    fn activate_nav(&mut self) {
        self.state.screen = Screen::NAV[self.state.nav_index];
        self.state.focus = Focus::Content;
        self.state.reset_cursor();
    }

    fn switch_screen(&mut self, screen: Screen) {
        if screen == Screen::ServiceDetail {
            self.state.screen = screen;
            return;
        }
        self.state.screen = screen;
        if let Some(index) = Screen::NAV
            .iter()
            .position(|candidate| *candidate == screen.nav_parent())
        {
            self.state.nav_index = index;
        }
        self.state.focus = Focus::Content;
        self.state.reset_cursor();
    }

    fn cycle_workspace(&mut self) {
        if self.snapshot.workspaces.is_empty() {
            self.toast("no workspaces configured", ToastLevel::Warning);
            return;
        }
        let current = self.state.active_workspace.clone();
        let next = self
            .snapshot
            .workspaces
            .iter()
            .position(|workspace| Some(&workspace.id) == current.as_ref())
            .map_or(0, |index| (index + 1) % self.snapshot.workspaces.len());
        self.state.active_workspace = Some(self.snapshot.workspaces[next].id.clone());
        match self.refresh() {
            Ok(()) => self.toast(
                format!("workspace: {}", self.snapshot.workspace_name),
                ToastLevel::Info,
            ),
            Err(error) => self.show_error("switch workspace", &error),
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.state.screen == Screen::Logs {
            if let Some(view) = &mut self.state.log_view {
                let last = view.entries.len().saturating_sub(1);
                view.scroll = (view.scroll as isize + delta).clamp(0, last as isize) as u16;
                view.follow = false;
            }
            return;
        }
        if self.state.screen == Screen::ServiceDetail {
            self.state.detail_scroll = (self.state.detail_scroll as isize + delta).max(0) as u16;
            return;
        }
        let len = self.current_row_count();
        self.state.move_cursor(len, delta);
    }

    fn page(&mut self, direction: isize) {
        let rows = self.state.viewport_rows.max(1) as isize;
        self.move_cursor(direction * rows);
    }

    fn select_row(&mut self, index: usize) {
        let len = self.current_row_count();
        self.state.select_cursor(len, index);
    }

    fn toggle_select(&mut self) {
        match self.state.screen {
            Screen::Libraries => {
                if let Some(id) = self.current_library_id() {
                    if !self.state.library_selection.remove(&id) {
                        self.state.library_selection.insert(id);
                    }
                }
            }
            _ => {
                if let Some(id) = self.current_service() {
                    if !self.state.selection.remove(&id) {
                        self.state.selection.insert(id);
                    }
                }
            }
        }
    }

    fn select_all_visible(&mut self) {
        match self.state.screen {
            Screen::Libraries => {
                let ids: Vec<LibraryId> = self
                    .visible_libraries()
                    .iter()
                    .map(|library| library.id.clone())
                    .collect();
                self.state.library_selection.extend(ids);
            }
            _ => {
                let ids: Vec<ServiceId> = self
                    .visible_services()
                    .iter()
                    .map(|service| service.service_id.clone())
                    .collect();
                self.state.selection.extend(ids);
            }
        }
    }

    fn clear_selection(&mut self) {
        self.state.selection.clear();
        self.state.library_selection.clear();
        self.toast("selection cleared", ToastLevel::Info);
    }

    fn open_details(&mut self) {
        if self.state.screen == Screen::Services && self.current_service().is_some() {
            self.state.screen = Screen::ServiceDetail;
            self.state.detail_scroll = 0;
        }
    }

    fn back(&mut self) {
        if self.state.search.is_some() {
            self.cancel_search();
            return;
        }
        if self.state.screen == Screen::ServiceDetail {
            self.switch_screen(Screen::Services);
            return;
        }
        if !self.state.selection.is_empty() || !self.state.library_selection.is_empty() {
            self.clear_selection();
            return;
        }
        self.state.focus = Focus::Nav;
    }

    fn start_search(&mut self) {
        let mut search = self.state.search.clone().unwrap_or_default();
        search.active = true;
        self.state.search = Some(search);
        self.state.reset_cursor();
    }

    fn commit_search(&mut self) {
        if let Some(search) = &mut self.state.search {
            search.active = false;
            if search.query.is_empty() {
                self.state.search = None;
            }
        }
        self.state.reset_cursor();
    }

    fn cancel_search(&mut self) {
        self.state.search = None;
        self.state.reset_cursor();
    }

    fn cycle_log_level(&mut self) {
        let Some(view) = &mut self.state.log_view else {
            return;
        };
        use service_orchestrator_application::LogLevel;
        view.minimum_level = match view.minimum_level {
            None => Some(LogLevel::Error),
            Some(LogLevel::Error) => Some(LogLevel::Warn),
            Some(LogLevel::Warn) => Some(LogLevel::Info),
            Some(LogLevel::Info) => Some(LogLevel::Debug),
            Some(LogLevel::Debug) => Some(LogLevel::Trace),
            Some(LogLevel::Trace) => None,
        };
    }

    fn toggle_log_follow(&mut self) {
        if let Some(view) = &mut self.state.log_view {
            view.follow = !view.follow;
            if view.follow {
                view.scroll = view.entries.len().saturating_sub(1) as u16;
            }
        }
    }

    fn select_task(&mut self, id: &TaskId) {
        self.switch_screen(Screen::Tasks);
        if let Some(index) = self.visible_tasks().iter().position(|task| &task.id == id) {
            self.select_row(index);
        }
    }

    // -- dialogs -------------------------------------------------------------

    fn dialog_move(&mut self, delta: isize) {
        if let Some(Dialog::Select(select)) = &mut self.state.dialog {
            if !select.options.is_empty() {
                let last = select.options.len() - 1;
                select.index = (select.index as isize + delta).clamp(0, last as isize) as usize;
            }
        }
    }

    fn confirm_dialog(&mut self) {
        let Some(dialog) = self.state.dialog.take() else {
            return;
        };
        match dialog {
            Dialog::Confirm(confirm) => {
                let label = describe_action(&confirm.action);
                let result = self.invoke(&confirm.action, Authorization::allow_destructive());
                self.dispatch(&label, result);
            }
            Dialog::Select(select) => {
                if let Some(option) = select.options.get(select.index).cloned() {
                    self.apply(option.action);
                }
            }
            Dialog::Input(input) => match input.purpose {
                InputPurpose::CheckoutBranch(service) => {
                    self.apply(Action::CheckoutBranch(service, input.value));
                }
            },
            Dialog::Info(_) | Dialog::Error(_) | Dialog::Help => {}
        }
    }

    fn open_confirm(&mut self, action: Action, label: &str, permission: Permission, message: &str) {
        let dialog = ConfirmDialog {
            title: "Confirmation Required".to_owned(),
            lines: vec![
                format!("Operation: {label}"),
                format!("Required permission: {permission}"),
                message.to_owned(),
                String::new(),
                "This action cannot be undone. Confirm to proceed.".to_owned(),
            ],
            action,
            destructive: permission.is_destructive(),
        };
        self.state.dialog = Some(Dialog::Confirm(dialog));
    }

    fn open_branch_dialog(
        &mut self,
        service: ServiceId,
        branches: &[service_orchestrator_domain::BranchInfo],
    ) {
        let options: Vec<SelectOption> = branches
            .iter()
            .map(|branch| SelectOption {
                label: if branch.current {
                    format!("{} (current)", branch.name)
                } else {
                    branch.name.clone()
                },
                action: Action::CheckoutBranch(service.clone(), branch.name.clone()),
            })
            .collect();
        if options.is_empty() {
            self.toast(
                format!("no branches found for {service}"),
                ToastLevel::Warning,
            );
            return;
        }
        self.state.dialog = Some(Dialog::Select(SelectDialog {
            title: format!("Checkout branch — {service}"),
            options,
            index: 0,
        }));
    }

    // -- application dispatch ------------------------------------------------

    fn dispatch_application(&mut self, action: Action) {
        let label = describe_action(&action);

        // Bulk operations that have no single bulk use case are run per service.
        match &action {
            Action::CheckHealthSelected => {
                let ids = self.target_services();
                return self.run_per_service("health check", &ids, Application::health_check);
            }
            Action::BuildSelected => {
                let ids = self.target_services();
                return self.run_per_service("build", &ids, Application::build_service);
            }
            _ => {}
        }

        match self.invoke(&action, Authorization::standard()) {
            Err(ApplicationError::PermissionDenied {
                message,
                permission,
            }) => self.open_confirm(action, &label, permission, &message),
            other => self.dispatch(&label, other),
        }
    }

    fn invoke(
        &self,
        action: &Action,
        authorization: Authorization,
    ) -> Result<OperationResult, ApplicationError> {
        let app = self.application.clone().with_authorization(authorization);
        match action {
            Action::StartService(id) => app.start_service(id),
            Action::StopService(id) => app.stop_service(id),
            Action::RestartService(id) => app.restart_service(id),
            Action::BuildService(id) => app.build_service(id),
            Action::SyncService(id) => app.sync_service(id),
            Action::FetchService(id) => app.fetch_repository(id),
            Action::PullService(id) => app.pull_repository(id),
            Action::CheckHealth(id) => app.check_service_health(id, HealthCheckScope::All),
            Action::CheckoutBranch(id, branch) => {
                let branch = Branch::new(branch.clone()).map_err(|error| {
                    ApplicationError::InvalidOperation {
                        message: error.to_string(),
                    }
                })?;
                app.checkout_repository(id, &branch)
            }
            Action::ResetRepositoryHard(id) => app.reset_repository_hard(id),
            Action::CleanRepository(id) => app.clean_repository(id),
            Action::DiscardLocalChanges(id) => app.discard_local_changes(id),

            Action::StartSelected => {
                let (workspace, ids) = self.workspace_and_targets()?;
                app.start_services(&workspace, &ids)
            }
            Action::StopSelected => {
                let (workspace, ids) = self.workspace_and_targets()?;
                app.stop_services(&workspace, &ids)
            }
            Action::RestartSelected => {
                let (workspace, ids) = self.workspace_and_targets()?;
                app.restart_services(&workspace, &ids)
            }
            Action::SyncSelected => {
                let (workspace, ids) = self.workspace_and_targets()?;
                app.sync_repositories(&workspace, &ids)
            }
            Action::FetchSelected => {
                let (workspace, ids) = self.workspace_and_targets()?;
                app.fetch_repositories(&workspace, &ids)
            }

            Action::BuildLibrary(id) => app.build_library(id),
            Action::InstallLibrary(id) => app.install_library(id),
            Action::PrepareLibrary(id) => app.prepare_library(id, &LibraryPrepareOptions::new()),
            Action::SyncLibrary(id) => app.sync_library(id),

            Action::MigrationStatus(id) => app.get_migration_status(id),
            Action::MigrationPending(id) => app.inspect_migration_pending(id),
            Action::MigrationPreview(id) => app.preview_migration(id),
            Action::MigrationValidate(id) => app.validate_migration(id),
            Action::MigrationApply(id) => app.apply_migration(id),

            Action::SyncWorkspace => {
                let workspace = self.require_workspace()?;
                app.sync_workspace(&workspace)
            }
            Action::BuildWorkspace => {
                let workspace = self.require_workspace()?;
                app.build_workspace(&workspace)
            }

            _ => Err(ApplicationError::InvalidOperation {
                message: "not an application operation".to_owned(),
            }),
        }
    }

    fn dispatch(&mut self, label: &str, result: Result<OperationResult, ApplicationError>) {
        match result {
            Ok(OperationResult::AlreadyInDesiredState { kind }) => {
                self.toast(
                    format!("{label}: already {}", kind.label()),
                    ToastLevel::Info,
                );
            }
            Ok(OperationResult::TaskScheduled { task_id, .. }) => {
                self.toast(
                    format!("{label}: task {task_id} scheduled"),
                    ToastLevel::Success,
                );
                self.refresh_tasks();
            }
            Ok(OperationResult::WorkflowScheduled { workflow_id, .. }) => {
                self.state.pending_workflows.push(workflow_id.clone());
                self.toast(
                    format!("{label}: workflow {workflow_id} scheduled"),
                    ToastLevel::Success,
                );
                self.refresh_tasks();
            }
            Err(error) => self.show_error(label, &error),
        }
    }

    fn run_per_service(
        &mut self,
        label: &str,
        ids: &[ServiceId],
        operation: fn(&Application, &ServiceId) -> Result<OperationResult, ApplicationError>,
    ) {
        let app = self.application.clone();
        let mut scheduled = 0usize;
        let mut failures = 0usize;
        for id in ids {
            match operation(&app, id) {
                Ok(result) if result.is_scheduled() => scheduled += 1,
                Ok(_) => {}
                Err(_) => failures += 1,
            }
        }
        let level = if failures > 0 {
            ToastLevel::Warning
        } else {
            ToastLevel::Success
        };
        self.toast(
            format!("{label}: {scheduled} scheduled, {failures} failed"),
            level,
        );
        self.refresh_tasks();
    }

    fn cancel_task(&mut self, id: &TaskId) {
        match self.application.cancel_task(id) {
            Ok(()) => {
                self.toast(
                    format!("task {id}: cancellation requested"),
                    ToastLevel::Info,
                );
                self.refresh_tasks();
            }
            Err(error) => self.show_error("cancel task", &error),
        }
    }

    // -- logs ----------------------------------------------------------------

    fn view_service_logs(&mut self, id: &ServiceId) {
        self.state.log_view = Some(LogView::new(LogTarget::Service(id.clone())));
        self.open_logs_screen();
        self.effects.push(Effect::StreamLogs(id.clone()));
        self.refresh_logs();
    }

    fn view_task_logs(&mut self, id: &TaskId) {
        self.state.log_view = Some(LogView::new(LogTarget::Task(id.clone())));
        self.open_logs_screen();
        self.effects.push(Effect::StopLogStream);
        self.refresh_logs();
    }

    fn open_logs_screen(&mut self) {
        self.state.screen = Screen::Logs;
        if let Some(index) = Screen::NAV
            .iter()
            .position(|candidate| *candidate == Screen::Logs)
        {
            self.state.nav_index = index;
        }
        self.state.focus = Focus::Content;
        self.state.reset_cursor();
    }

    fn refresh_logs(&mut self) {
        let Some(view) = self.state.log_view.clone() else {
            return;
        };
        let mut request =
            service_orchestrator_application::LogRequest::all().with_limit(LOG_BUFFER_CAPACITY);
        if let Some(level) = view.minimum_level {
            request = request.with_minimum_level(level);
        }
        if !view.filter.is_empty() {
            request = request.matching(view.filter.clone());
        }
        let result = match &view.target {
            LogTarget::Service(id) => self.application.get_service_logs(id, &request),
            LogTarget::Task(id) => self.application.get_task_logs(id, &request),
        };
        if let Some(view) = &mut self.state.log_view {
            if let Ok(page) = result {
                view.replace(page.entries);
                view.cursor = page.next_cursor;
            }
            if view.follow {
                view.scroll = view.entries.len().saturating_sub(1).min(u16::MAX as usize) as u16;
            }
        }
    }

    fn handle_log_entry(&mut self, entry: service_orchestrator_application::LogEntry) {
        let matches_target = match (&self.state.log_view, &entry.service_id) {
            (Some(view), Some(service)) => {
                matches!(&view.target, LogTarget::Service(id) if id == service)
            }
            _ => false,
        };
        if !matches_target {
            return;
        }
        if let Some(view) = &mut self.state.log_view {
            if let Some(minimum) = view.minimum_level {
                if entry.level < minimum {
                    return;
                }
            }
            if !view.filter.is_empty()
                && !entry
                    .message
                    .to_lowercase()
                    .contains(&view.filter.to_lowercase())
            {
                return;
            }
            view.append(std::iter::once(entry));
            if view.follow {
                view.scroll = view.entries.len().saturating_sub(1).min(u16::MAX as usize) as u16;
            }
        }
    }

    // -- task and async events ----------------------------------------------

    fn handle_task_event(&mut self, event: &service_orchestrator_execution::TaskEvent) {
        use service_orchestrator_execution::TaskEvent;
        match event {
            TaskEvent::TaskCreated { .. }
            | TaskEvent::TaskStarted { .. }
            | TaskEvent::TaskProgress { .. }
            | TaskEvent::TaskMessage { .. }
            | TaskEvent::TaskRetrying { .. }
            | TaskEvent::TaskSucceeded { .. }
            | TaskEvent::TaskFailed { .. }
            | TaskEvent::TaskCancelled { .. }
            | TaskEvent::TaskSkipped { .. }
            | TaskEvent::TaskBlocked { .. }
            | TaskEvent::WorkflowCreated { .. } => self.refresh_tasks(),
        }
    }

    fn handle_async(&mut self, outcome: AsyncOutcome) {
        match outcome {
            AsyncOutcome::GitStatus { service, result } => match result {
                Ok(status) => {
                    self.state.git_status.insert(service, status);
                }
                Err(message) => {
                    self.toast(format!("git status: {message}"), ToastLevel::Warning);
                }
            },
            AsyncOutcome::Branches { service, result } => {
                let pending = self.state.pending_branch_service.take();
                match result {
                    Ok(branches) => {
                        self.state
                            .branches
                            .insert(service.clone(), branches.clone());
                        if pending.as_ref() == Some(&service) {
                            self.open_branch_dialog(service, &branches);
                        }
                    }
                    Err(message) => {
                        if pending.as_ref() == Some(&service) {
                            self.show_error(
                                "list branches",
                                &ApplicationError::InvalidOperation { message },
                            );
                        } else {
                            self.toast(format!("branches: {message}"), ToastLevel::Warning);
                        }
                    }
                }
            }
            AsyncOutcome::LibraryStatus { library, result } => match result {
                Ok(status) => {
                    self.state.library_status.insert(library, status);
                }
                Err(message) => {
                    self.toast(format!("library status: {message}"), ToastLevel::Warning)
                }
            },
            AsyncOutcome::VerifyArtifact { library, result } => match result {
                Ok(verification) => {
                    let present = verification.present;
                    self.state
                        .artifact_verification
                        .insert(library, verification);
                    self.toast(
                        format!("artifact {}", if present { "present" } else { "missing" }),
                        if present {
                            ToastLevel::Success
                        } else {
                            ToastLevel::Warning
                        },
                    );
                }
                Err(message) => {
                    self.toast(format!("verify artifact: {message}"), ToastLevel::Warning)
                }
            },
            AsyncOutcome::Alignment { library, result } => match result {
                Ok(plan) => {
                    let aligned = plan.is_fully_aligned();
                    self.state.alignment.insert(library, plan);
                    self.toast(
                        format!(
                            "version alignment: {}",
                            if aligned { "aligned" } else { "drift detected" }
                        ),
                        if aligned {
                            ToastLevel::Success
                        } else {
                            ToastLevel::Warning
                        },
                    );
                }
                Err(message) => {
                    self.toast(format!("version alignment: {message}"), ToastLevel::Warning);
                }
            },
        }
    }

    fn tick(&mut self) {
        self.ticks = self.ticks.wrapping_add(1);

        if let Some(toast) = &mut self.state.toast {
            toast.ttl = toast.ttl.saturating_sub(1);
        }
        if self
            .state
            .toast
            .as_ref()
            .is_some_and(|toast| toast.ttl == 0)
        {
            self.state.toast = None;
        }

        if self.ticks % REFRESH_EVERY_TICKS == 0 {
            let _ = self.refresh();
        } else {
            self.refresh_tasks();
        }

        self.drain_pending_workflows();

        if self.state.screen == Screen::Logs {
            self.refresh_logs();
        }
    }

    fn drain_pending_workflows(&mut self) {
        let workflows = std::mem::take(&mut self.state.pending_workflows);
        let mut remaining = Vec::new();
        for workflow in workflows {
            match self.application.summarize_workflow(&workflow) {
                Ok(summary) if summary.is_complete() => {
                    let level = if summary.failed > 0 {
                        ToastLevel::Warning
                    } else {
                        ToastLevel::Success
                    };
                    self.toast(
                        format!(
                            "workflow {}: {} ok, {} failed, {} cancelled, {} skipped",
                            workflow,
                            summary.succeeded,
                            summary.failed,
                            summary.cancelled,
                            summary.skipped
                        ),
                        level,
                    );
                }
                _ => remaining.push(workflow),
            }
        }
        self.state.pending_workflows = remaining;
    }

    // -- helpers -------------------------------------------------------------

    fn refresh_tasks(&mut self) {
        self.snapshot.tasks = self.collect_tasks();
        self.clamp_cursor();
    }

    fn collect_tasks(&self) -> Vec<Task> {
        let mut tasks = self.observer.running_tasks();
        tasks.sort_by_key(|task| task.created_at.epoch_millis());
        let mut recent = self.observer.recent_tasks(TASK_HISTORY_LIMIT);
        tasks.append(&mut recent);
        tasks
    }

    fn clamp_cursor(&mut self) {
        let len = self.current_row_count();
        self.state.select_cursor(len, self.state.cursor);
    }

    fn current_row_count(&self) -> usize {
        match self.state.screen {
            Screen::Services | Screen::Git | Screen::Builds | Screen::Health => {
                self.visible_services().len()
            }
            Screen::Libraries => self.visible_libraries().len(),
            Screen::Migrations => self.visible_migrations().len(),
            Screen::Tasks => self.visible_tasks().len(),
            _ => 0,
        }
    }

    fn require_workspace(&self) -> Result<WorkspaceId, ApplicationError> {
        self.state
            .active_workspace
            .clone()
            .ok_or_else(|| ApplicationError::InvalidOperation {
                message: "no workspace is selected".to_owned(),
            })
    }

    fn workspace_and_targets(&self) -> Result<(WorkspaceId, Vec<ServiceId>), ApplicationError> {
        let workspace = self.require_workspace()?;
        let targets = self.target_services();
        if targets.is_empty() {
            return Err(ApplicationError::InvalidOperation {
                message: "no services are selected".to_owned(),
            });
        }
        Ok((workspace, targets))
    }

    fn target_services(&self) -> Vec<ServiceId> {
        if !self.state.selection.is_empty() {
            return self.state.selection.iter().cloned().collect();
        }
        self.current_service().into_iter().collect()
    }

    pub(crate) fn visible_services(&self) -> Vec<service_orchestrator_application::ServiceStatus> {
        let query = self.state.query().map(str::to_lowercase);
        self.snapshot
            .services
            .iter()
            .filter(|service| self.state.service_filter.matches(service))
            .filter(|service| match &query {
                Some(query) => service_matches(service, query),
                None => true,
            })
            .cloned()
            .collect()
    }

    fn current_service(&self) -> Option<ServiceId> {
        self.visible_services()
            .get(self.state.cursor)
            .map(|service| service.service_id.clone())
    }

    pub(crate) fn visible_libraries(&self) -> Vec<service_orchestrator_domain::Library> {
        let query = self.state.query().map(str::to_lowercase);
        self.snapshot
            .libraries
            .iter()
            .filter(|library| match &query {
                Some(query) => {
                    library.id.as_str().to_lowercase().contains(query)
                        || library.display_name.to_lowercase().contains(query)
                }
                None => true,
            })
            .cloned()
            .collect()
    }

    fn current_library_id(&self) -> Option<LibraryId> {
        self.visible_libraries()
            .get(self.state.cursor)
            .map(|library| library.id.clone())
    }

    pub(crate) fn visible_migrations(
        &self,
    ) -> Vec<service_orchestrator_application::MigrationStatus> {
        let query = self.state.query().map(str::to_lowercase);
        let mut migrations: Vec<service_orchestrator_application::MigrationStatus> =
            self.snapshot.migrations.values().cloned().collect();
        migrations.sort_by(|left, right| left.service_id.as_str().cmp(right.service_id.as_str()));
        migrations.retain(|migration| match &query {
            Some(query) => migration.service_id.as_str().to_lowercase().contains(query),
            None => true,
        });
        migrations
    }

    fn current_migration_service(&self) -> Option<ServiceId> {
        self.visible_migrations()
            .get(self.state.cursor)
            .map(|migration| migration.service_id.clone())
    }

    pub(crate) fn visible_tasks(&self) -> Vec<Task> {
        let query = self.state.query().map(str::to_lowercase);
        self.snapshot
            .tasks
            .iter()
            .filter(|task| self.state.task_filter.matches(task))
            .filter(|task| match &query {
                Some(query) => {
                    task.id.as_str().to_lowercase().contains(query)
                        || task.kind.label().to_lowercase().contains(query)
                        || task
                            .service
                            .as_ref()
                            .is_some_and(|id| id.as_str().to_lowercase().contains(query))
                }
                None => true,
            })
            .cloned()
            .collect()
    }

    fn current_task_id(&self) -> Option<TaskId> {
        self.visible_tasks()
            .get(self.state.cursor)
            .map(|task| task.id.clone())
    }

    fn request_checkout(&mut self, id: &ServiceId) {
        self.state.pending_branch_service = Some(id.clone());
        self.effects.push(Effect::Branches(id.clone()));
        self.toast(format!("listing branches for {id}"), ToastLevel::Info);
    }

    pub(crate) fn toast(&mut self, message: impl Into<String>, level: ToastLevel) {
        self.state.toast = Some(Toast::new(message, level));
    }

    fn show_error(&mut self, label: &str, error: &ApplicationError) {
        let dialog = present_error(label, error);
        self.state.dialog = Some(Dialog::Error(dialog));
    }
}

/// Returns `true` when a service status matches a lower-cased query.
fn service_matches(service: &service_orchestrator_application::ServiceStatus, query: &str) -> bool {
    service.display_name.to_lowercase().contains(query)
        || service.service_id.as_str().to_lowercase().contains(query)
        || service.kind.label().to_lowercase().contains(query)
        || service.process_state.to_string().contains(query)
        || service.health_state.label().contains(query)
        || service
            .branch
            .as_ref()
            .is_some_and(|branch| branch.as_str().to_lowercase().contains(query))
        || service.workspace_id.as_str().to_lowercase().contains(query)
}

/// Renders an application error into an actionable dialog.
fn present_error(label: &str, error: &ApplicationError) -> MessageDialog {
    let mut lines = vec![format!("Operation: {label}")];
    match error {
        ApplicationError::ServiceNotFound(id) => lines.push(format!("Service: {id}")),
        ApplicationError::LibraryNotFound(id) => lines.push(format!("Library: {id}")),
        ApplicationError::WorkspaceNotFound(id) => lines.push(format!("Workspace: {id}")),
        ApplicationError::TaskNotFound(id) => lines.push(format!("Task: {id}")),
        ApplicationError::WorkflowNotFound(id) => lines.push(format!("Workflow: {id}")),
        ApplicationError::GroupNotFound(id) => lines.push(format!("Group: {id}")),
        ApplicationError::ProfileNotFound(id) => lines.push(format!("Profile: {id}")),
        ApplicationError::Git { code, message }
        | ApplicationError::Build { code, message }
        | ApplicationError::ArtifactInspection { code, message }
        | ApplicationError::ServiceRuntime { code, message }
        | ApplicationError::Migration { code, message }
        | ApplicationError::Health { code, message } => {
            lines.push(format!("Code: {code}"));
            lines.push("Reason:".to_owned());
            lines.push(message.clone());
        }
        ApplicationError::PermissionDenied {
            permission,
            message,
            ..
        } => {
            lines.push(format!("Permission: {permission}"));
            lines.push("Reason:".to_owned());
            lines.push(message.clone());
        }
        ApplicationError::DependencyNotFound { owner, target } => {
            lines.push(format!("Owner: {owner}"));
            lines.push(format!("Missing dependency: {target}"));
        }
        other => {
            lines.push("Reason:".to_owned());
            lines.push(other.to_string());
        }
    }
    lines.push(String::new());
    lines.push("[Esc] Close".to_owned());
    MessageDialog::new("Operation Failed", lines)
}

/// Renders a human-readable label for an action, used in dialogs and toasts.
fn describe_action(action: &Action) -> String {
    match action {
        Action::StartService(id) => format!("Start service {id}"),
        Action::StopService(id) => format!("Stop service {id}"),
        Action::RestartService(id) => format!("Restart service {id}"),
        Action::BuildService(id) => format!("Build service {id}"),
        Action::SyncService(id) => format!("Synchronize service {id}"),
        Action::FetchService(id) => format!("Fetch repository {id}"),
        Action::PullService(id) => format!("Pull repository {id}"),
        Action::CheckHealth(id) => format!("Check health {id}"),
        Action::CheckoutBranch(id, branch) => format!("Checkout {branch} in {id}"),
        Action::ResetRepositoryHard(id) => format!("Hard reset {id}"),
        Action::CleanRepository(id) => format!("Clean repository {id}"),
        Action::DiscardLocalChanges(id) => format!("Discard local changes in {id}"),
        Action::StartSelected => "Start selected services".to_owned(),
        Action::StopSelected => "Stop selected services".to_owned(),
        Action::RestartSelected => "Restart selected services".to_owned(),
        Action::BuildSelected => "Build selected services".to_owned(),
        Action::SyncSelected => "Synchronize selected repositories".to_owned(),
        Action::FetchSelected => "Fetch selected repositories".to_owned(),
        Action::CheckHealthSelected => "Check selected services".to_owned(),
        Action::BuildLibrary(id) => format!("Build library {id}"),
        Action::InstallLibrary(id) => format!("Install library {id}"),
        Action::PrepareLibrary(id) => format!("Prepare library {id}"),
        Action::SyncLibrary(id) => format!("Synchronize library {id}"),
        Action::VerifyLibrary(id) => format!("Verify artifact {id}"),
        Action::InspectAlignment(id) => format!("Inspect version alignment {id}"),
        Action::RefreshLibraryStatus(id) => format!("Refresh library status {id}"),
        Action::MigrationStatus(id) => format!("Inspect migration status {id}"),
        Action::MigrationPending(id) => format!("Inspect pending migrations {id}"),
        Action::MigrationPreview(id) => format!("Preview migration {id}"),
        Action::MigrationValidate(id) => format!("Validate migration {id}"),
        Action::MigrationApply(id) => format!("Apply migration {id}"),
        Action::SyncWorkspace => "Synchronize workspace".to_owned(),
        Action::BuildWorkspace => "Build workspace".to_owned(),
        Action::CancelTask(id) => format!("Cancel task {id}"),
        other => format!("{other:?}"),
    }
}
