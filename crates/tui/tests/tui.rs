//! Integration tests for the Ratatui interface adapter.
//!
//! These tests drive the real [`App`] state machine over a real
//! [`Application`] core wired to in-memory fakes, and render real frames
//! through Ratatui's [`TestBackend`](ratatui::backend::TestBackend). No Git
//! repository, Maven build, Liquibase migration, or operating-system process is
//! ever started (`tasks/013.md` §36, §37).

use std::sync::Arc;

use service_orchestrator_application::{Application, InMemoryRuntimeStateStore, NullLogService};
use service_orchestrator_config::load_from_str;
use service_orchestrator_domain::{ServiceId, Task, TaskId, TaskKind};
use service_orchestrator_execution::TaskEvent;
use service_orchestrator_tui::testing::{render, FakeTasks};
use service_orchestrator_tui::{
    Action, App, AppEvent, Dialog, Effect, InputEvent, Key, Screen, TaskObserver,
};

/// The fixture configuration shared by these tests.
const CONFIG: &str = include_str!("fixtures/config.toml");

/// The fixture config path reported to the interface.
const CONFIG_PATH: &str = "tests/fixtures/config.toml";

/// An interface under test plus the fake task engine backing it.
struct Harness {
    app: App,
    tasks: Arc<FakeTasks>,
}

/// Builds an interface over the fixture configuration and in-memory fakes.
fn harness() -> Harness {
    let config = load_from_str(CONFIG, "<test>").expect("the fixture configuration is valid");
    let tasks = Arc::new(FakeTasks::new());
    let application = Application::new(
        config,
        tasks.clone(),
        Arc::new(InMemoryRuntimeStateStore::new()),
        Arc::new(NullLogService),
    );
    let observer: Arc<dyn TaskObserver> = tasks.clone();
    let mut app = App::new(application, observer, CONFIG_PATH);
    app.refresh().expect("the fixture workspace reads cleanly");
    Harness { app, tasks }
}

fn service(name: &str) -> ServiceId {
    ServiceId::new(name).expect("test service ids are valid")
}

#[test]
fn startup_lists_the_configured_services() {
    let harness = harness();
    let snapshot = harness.app.snapshot();
    assert_eq!(
        snapshot.services.len(),
        2,
        "two fixture services are listed"
    );
    assert_eq!(snapshot.libraries.len(), 1);
    assert_eq!(snapshot.workspace_name, "Acme Platform");
    assert_eq!(snapshot.config_version, 1);
    assert_eq!(snapshot.max_parallel_tasks, 4);
    assert_eq!(snapshot.default_timeout_seconds, 1800);
    assert_eq!(snapshot.repositories.len(), 2);
}

#[test]
fn dashboard_renders_real_application_state() {
    let harness = harness();
    let text = render(&harness.app, 120, 40);
    assert!(text.contains("SERVICE ORCHESTRATOR"), "{text}");
    assert!(text.contains("Acme Platform"), "{text}");
    assert!(text.contains("2 total"), "{text}");
    assert!(text.contains("Running: 0/2"), "{text}");
}

#[test]
fn a_terminal_below_the_minimum_shows_a_notice_instead_of_panicking() {
    let harness = harness();
    let text = render(&harness.app, 40, 8);
    assert!(text.contains("Terminal too small"), "{text}");
}

#[test]
fn every_workspace_screen_renders_without_panicking() {
    let mut harness = harness();
    for screen in Screen::NAV {
        harness.app.apply(Action::SwitchScreen(screen));
        let text = render(&harness.app, 120, 40);
        assert!(
            !text.is_empty(),
            "screen {screen:?} rendered an empty frame"
        );
    }
    // The service detail sub-view is not part of the navigation list.
    harness.app.apply(Action::SwitchScreen(Screen::Services));
    harness.app.apply(Action::OpenDetails);
    let text = render(&harness.app, 120, 40);
    assert!(text.contains("auth-service"), "{text}");
}

#[test]
fn navigation_and_quit_are_keyboard_driven() {
    let mut harness = harness();
    harness.app.apply(Action::SwitchScreen(Screen::Services));
    assert_eq!(harness.app.state().screen, Screen::Services);

    harness
        .app
        .handle_event(AppEvent::Input(InputEvent::ch('j')));
    assert_eq!(harness.app.state().cursor, 1);
    harness
        .app
        .handle_event(AppEvent::Input(InputEvent::ch('k')));
    assert_eq!(harness.app.state().cursor, 0);

    // Tab moves focus to the navigation panel and Enter re-activates a screen.
    harness
        .app
        .handle_event(AppEvent::Input(InputEvent::key(Key::Tab)));
    assert_eq!(
        harness.app.state().focus,
        service_orchestrator_tui::Focus::Nav
    );

    harness
        .app
        .handle_event(AppEvent::Input(InputEvent::ch('q')));
    assert!(harness.app.should_quit());
}

#[test]
fn selection_is_by_typed_id_and_survives_filtering() {
    let mut harness = harness();
    harness.app.apply(Action::SwitchScreen(Screen::Services));
    let first = harness.app.snapshot().services[0].service_id.clone();

    harness.app.apply(Action::ToggleSelect);
    assert!(harness.app.state().selection.contains(&first));
    harness.app.apply(Action::ToggleSelect);
    assert!(!harness.app.state().selection.contains(&first));

    // Re-select and change the filter: the typed selection must persist.
    harness.app.apply(Action::ToggleSelect);
    harness.app.apply(Action::CycleServiceFilter);
    assert!(harness.app.state().selection.contains(&first));
    assert_eq!(harness.app.state().selection.len(), 1);

    harness.app.apply(Action::ClearSelection);
    assert!(harness.app.state().selection.is_empty());
}

#[test]
fn typing_a_query_narrows_the_visible_services() {
    let mut harness = harness();
    harness.app.apply(Action::SwitchScreen(Screen::Services));
    harness.app.apply(Action::StartSearch);
    for character in "web".chars() {
        harness
            .app
            .handle_event(AppEvent::Input(InputEvent::ch(character)));
    }
    harness.app.apply(Action::CommitSearch);

    // "web" matches only web-ui, so the first visible row is web-ui.
    harness.app.apply(Action::ToggleSelect);
    assert!(harness.app.state().selection.contains(&service("web-ui")));
    assert_eq!(harness.app.state().selection.len(), 1);
}

#[test]
fn service_actions_are_dispatched_as_tasks_through_the_engine() {
    let mut harness = harness();
    harness
        .app
        .apply(Action::StartService(service("auth-service")));
    assert_eq!(
        harness.tasks.task_ids().len(),
        1,
        "a start task was submitted"
    );

    harness
        .app
        .apply(Action::BuildService(service("auth-service")));
    assert_eq!(
        harness.tasks.task_ids().len(),
        2,
        "a build task was submitted"
    );

    // The refresh after dispatch surfaces the new tasks in the snapshot.
    assert!(harness.app.snapshot().tasks.len() >= 2);
}

#[test]
fn bulk_actions_target_the_selection() {
    let mut harness = harness();
    harness.app.apply(Action::SwitchScreen(Screen::Services));
    harness.app.apply(Action::SelectAllVisible);
    assert_eq!(harness.app.state().selection.len(), 2);

    harness.app.apply(Action::FetchSelected);
    assert_eq!(
        harness.tasks.task_ids().len(),
        2,
        "one fetch task per service"
    );
}

#[test]
fn cancel_schedules_a_cancellation_through_the_application() {
    let mut harness = harness();
    harness.tasks.insert(
        Task::new(TaskId::new("task-7").expect("valid id"), TaskKind::GitFetch)
            .with_service(service("auth-service")),
    );
    harness.app.apply(Action::SwitchScreen(Screen::Tasks));
    harness.app.handle_event(AppEvent::Tick);
    let id = TaskId::new("task-7").expect("valid id");
    harness.app.apply(Action::CancelTask(id.clone()));
    let cancelled = harness
        .app
        .snapshot()
        .tasks
        .iter()
        .find(|task| task.id == id)
        .expect("the task is still listed");
    assert_eq!(
        cancelled.status,
        service_orchestrator_domain::TaskStatus::Cancelled
    );
}

#[test]
fn task_events_refresh_the_snapshot() {
    let mut harness = harness();
    let task = Task::new(
        TaskId::new("task-1").expect("valid id"),
        TaskKind::MavenBuild,
    )
    .with_service(service("auth-service"));
    harness.tasks.insert(task.clone());
    harness
        .app
        .handle_event(AppEvent::Task(TaskEvent::TaskSucceeded {
            id: task.id.clone(),
        }));
    assert!(harness
        .app
        .snapshot()
        .tasks
        .iter()
        .any(|listed| listed.id == task.id));
}

#[test]
fn asynchronous_effects_are_emitted_rather_than_executed() {
    let mut harness = harness();
    harness
        .app
        .apply(Action::RefreshGitStatus(service("auth-service")));
    let effects = harness.app.take_effects();
    assert!(effects.contains(&Effect::GitStatus(service("auth-service"))));

    harness
        .app
        .apply(Action::RequestCheckout(service("auth-service")));
    let effects = harness.app.take_effects();
    assert!(effects.contains(&Effect::Branches(service("auth-service"))));
    // Branch listing also opens a toast; the dialog is still closed.
    assert!(harness.app.state().dialog.is_none());
}

#[test]
fn destructive_actions_require_confirmation() {
    let mut harness = harness();
    harness
        .app
        .apply(Action::ResetRepositoryHard(service("auth-service")));

    match harness.app.state().dialog.as_ref() {
        Some(Dialog::Confirm(confirm)) => {
            assert!(confirm.destructive, "a hard reset is destructive");
        }
        other => panic!("expected a confirmation dialog, found {other:?}"),
    }
    assert!(
        harness.tasks.task_ids().is_empty(),
        "nothing is submitted before confirmation"
    );

    // Confirming re-runs the action with destructive authorization.
    harness.app.apply(Action::ConfirmDialog);
    assert_eq!(harness.tasks.task_ids().len(), 1);
    assert!(!matches!(
        harness.app.state().dialog,
        Some(Dialog::Confirm(_))
    ));
}

#[test]
fn application_errors_are_presented_actionably() {
    let mut harness = harness();
    harness
        .app
        .apply(Action::StartService(service("ghost-service")));
    match harness.app.state().dialog.as_ref() {
        Some(Dialog::Error(message)) => {
            assert!(message.title.contains("Failed"), "{:?}", message.title);
            assert!(
                message
                    .lines
                    .iter()
                    .any(|line| line.contains("ghost-service")),
                "{:?}",
                message.lines
            );
        }
        other => panic!("expected an error dialog, found {other:?}"),
    }
}

#[test]
fn the_help_overlay_is_reachable_and_renderable() {
    let mut harness = harness();
    harness
        .app
        .handle_event(AppEvent::Input(InputEvent::ch('?')));
    assert!(matches!(harness.app.state().dialog, Some(Dialog::Help)));
    let text = render(&harness.app, 120, 40);
    assert!(text.contains("Help"), "{text}");
    assert!(text.contains("Global"), "{text}");

    harness
        .app
        .handle_event(AppEvent::Input(InputEvent::key(Key::Esc)));
    assert!(harness.app.state().dialog.is_none());
}

#[test]
fn quitting_stops_the_interface_without_touching_the_engine() {
    let mut harness = harness();
    harness
        .app
        .handle_event(AppEvent::Input(InputEvent::key(Key::CtrlC)));
    assert!(harness.app.should_quit());
    // The fake engine never sees a submission from a quit.
    assert!(harness.tasks.task_ids().is_empty());
}
