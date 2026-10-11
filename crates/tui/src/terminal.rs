//! Terminal lifecycle and the asynchronous event loop.
//!
//! This module owns everything the rest of the interface deliberately does not:
//! raw mode, the alternate screen, the input thread, the tick, the task-event
//! subscription, realtime log streaming, and the dispatch of [`Effect`]s. The
//! state machine ([`App`]) stays pure and testable because none of that leaks
//! into it.
//!
//! # Lifecycle
//!
//! Raw mode and the alternate screen are entered once and restored by the
//! [`TerminalSession`]'s `Drop`, which also runs during unwinding. A panic hook
//! is installed so a panic that aborts the loop still leaves the developer's
//! terminal usable.
//!
//! # Event loop
//!
//! Terminal input, task events, realtime log entries, and ticks are funnelled
//! into one [`AppEvent`] channel. The loop draws, then awaits exactly one event;
//! nothing blocks the renderer on Git, Maven, npm, Liquibase, processes, or
//! health checks, because those run as tasks or as spawned queries whose results
//! arrive back on the same channel.

use std::io::{self, Stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::Terminal;
use service_orchestrator_application::LogStreamError;
use service_orchestrator_client::OrchestratorClient;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::app::App;
use crate::error::TuiError;
use crate::event::{AppEvent, AsyncOutcome, Effect};
use crate::input::translate;
use crate::observer::TaskObserver;
use crate::router;
use crate::state::ToastLevel;

/// How often the interface receives a periodic tick.
const TICK: Duration = Duration::from_millis(250);

/// How long the input thread waits for a key before re-checking the stop flag.
const INPUT_POLL: Duration = Duration::from_millis(50);

/// Runs the interface until the developer quits.
///
/// # Errors
///
/// Returns [`TuiError`] when the terminal cannot be initialized, read, or drawn.
pub async fn run(
    application: Arc<dyn OrchestratorClient>,
    observer: Arc<dyn TaskObserver>,
    config_path: impl Into<String>,
) -> Result<(), TuiError> {
    install_panic_hook();

    let mut app = App::new(application.clone(), observer.clone(), config_path);
    // A failure here is not fatal: the interface still renders, with an empty
    // snapshot, which is more useful than refusing to start.
    let _ = app.refresh();

    let mut session = TerminalSession::enter()?;
    let mut events = EventBus::start(&observer);

    // The input thread is separate because `crossterm`'s reader is blocking; it
    // forwards translated events and stops when asked.
    let stop = Arc::new(AtomicBool::new(false));
    spawn_input_thread(events.sender().clone(), Arc::clone(&stop));

    let mut log_task: Option<JoinHandle<()>> = None;

    let outcome = event_loop(
        &mut session.terminal,
        &mut app,
        &application,
        &mut events,
        &mut log_task,
    )
    .await;

    stop.store(true, Ordering::Relaxed);
    if let Some(handle) = log_task.take() {
        handle.abort();
    }
    // `session` drops here, restoring the terminal even on the error path.
    outcome
}

/// The draw/await loop.
async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    application: &Arc<dyn OrchestratorClient>,
    events: &mut EventBus,
    log_task: &mut Option<JoinHandle<()>>,
) -> Result<(), TuiError> {
    loop {
        let effects = app.take_effects();
        for effect in effects {
            dispatch_effect(effect, application, events.sender(), log_task, app);
        }

        if let Ok(size) = terminal.size() {
            app.state.viewport_rows = router::list_rows(size.height);
        }
        terminal.draw(|frame| router::draw(frame, app))?;

        if app.should_quit() {
            return Ok(());
        }

        match events.receiver().recv().await {
            Some(event) => app.handle_event(event),
            None => return Ok(()),
        }
    }
}

/// Runs one effect.
fn dispatch_effect(
    effect: Effect,
    application: &Arc<dyn OrchestratorClient>,
    sender: &mpsc::UnboundedSender<AppEvent>,
    log_task: &mut Option<JoinHandle<()>>,
    app: &mut App,
) {
    match effect {
        Effect::GitStatus(service) => {
            let application = application.clone();
            let sender = sender.clone();
            tokio::spawn(async move {
                let result = application
                    .get_git_status(&service)
                    .await
                    .map_err(|error| error.to_string());
                let _ = sender.send(AppEvent::Async(Box::new(AsyncOutcome::GitStatus {
                    service,
                    result,
                })));
            });
        }
        Effect::Branches(service) => {
            let application = application.clone();
            let sender = sender.clone();
            tokio::spawn(async move {
                let result = application
                    .list_branches(&service)
                    .await
                    .map_err(|error| error.to_string());
                let _ = sender.send(AppEvent::Async(Box::new(AsyncOutcome::Branches {
                    service,
                    result,
                })));
            });
        }
        Effect::LibraryStatus(library) => {
            let application = application.clone();
            let sender = sender.clone();
            tokio::spawn(async move {
                let result = application
                    .get_library_status(&library)
                    .await
                    .map_err(|error| error.to_string());
                let _ = sender.send(AppEvent::Async(Box::new(AsyncOutcome::LibraryStatus {
                    library,
                    result,
                })));
            });
        }
        Effect::VerifyArtifact(library) => {
            let application = application.clone();
            let sender = sender.clone();
            tokio::spawn(async move {
                let result = application
                    .verify_library_artifact(&library)
                    .await
                    .map_err(|error| error.to_string());
                let _ = sender.send(AppEvent::Async(Box::new(AsyncOutcome::VerifyArtifact {
                    library,
                    result,
                })));
            });
        }
        Effect::Alignment(library) => {
            let application = application.clone();
            let sender = sender.clone();
            tokio::spawn(async move {
                let result = application
                    .inspect_library_version_alignment(&library)
                    .await
                    .map_err(|error| error.to_string());
                let _ = sender.send(AppEvent::Async(Box::new(AsyncOutcome::Alignment {
                    library,
                    result,
                })));
            });
        }
        Effect::StreamLogs(service) => {
            if let Some(handle) = log_task.take() {
                handle.abort();
            }
            match application.subscribe_service_logs(&service) {
                Ok(subscription) => {
                    let sender = sender.clone();
                    *log_task = Some(tokio::spawn(stream_logs(subscription, sender)));
                }
                Err(error) => {
                    app.toast(
                        format!("log stream unavailable: {error}"),
                        ToastLevel::Warning,
                    );
                }
            }
        }
        Effect::StopLogStream => {
            if let Some(handle) = log_task.take() {
                handle.abort();
            }
        }
    }
}

/// Consumes a log subscription until it closes or the receiver goes away.
async fn stream_logs(
    mut subscription: Box<dyn service_orchestrator_application::LogSubscription>,
    sender: mpsc::UnboundedSender<AppEvent>,
) {
    loop {
        match subscription.recv().await {
            Ok(entry) => {
                if sender.send(AppEvent::Log(entry)).is_err() {
                    return;
                }
            }
            Err(LogStreamError::Lagged { .. }) => continue,
            Err(LogStreamError::Closed) => return,
        }
    }
}

/// The multiplexed event channel and its producer tasks.
struct EventBus {
    sender: mpsc::UnboundedSender<AppEvent>,
    receiver: mpsc::UnboundedReceiver<AppEvent>,
}

impl EventBus {
    /// Creates the channel and starts the task-event forwarder and the ticker.
    fn start(observer: &Arc<dyn TaskObserver>) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        spawn_task_forwarder(observer, sender.clone());
        spawn_ticker(sender.clone());
        Self { sender, receiver }
    }

    fn sender(&self) -> &mpsc::UnboundedSender<AppEvent> {
        &self.sender
    }

    fn receiver(&mut self) -> &mut mpsc::UnboundedReceiver<AppEvent> {
        &mut self.receiver
    }
}

/// Forwards task-engine events onto the interface channel.
fn spawn_task_forwarder(observer: &Arc<dyn TaskObserver>, sender: mpsc::UnboundedSender<AppEvent>) {
    let mut receiver = observer.subscribe();
    tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if sender.send(AppEvent::Task(event)).is_err() {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });
}

/// Sends a periodic tick, skipping missed ticks under load.
fn spawn_ticker(sender: mpsc::UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if sender.send(AppEvent::Tick).is_err() {
                return;
            }
        }
    });
}

/// Reads terminal events on a blocking thread and forwards them.
fn spawn_input_thread(sender: mpsc::UnboundedSender<AppEvent>, stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match event::poll(INPUT_POLL) {
                Ok(true) => match event::read() {
                    Ok(event) => forward(&sender, event),
                    Err(_) => return,
                },
                Ok(false) => {}
                Err(_) => return,
            }
        }
    });
}

/// Translates and forwards one backend event.
fn forward(sender: &mpsc::UnboundedSender<AppEvent>, event: Event) {
    if let Some(input) = translate(event) {
        let _ = sender.send(AppEvent::Input(input));
    }
}

/// Owns the terminal and restores it on drop.
struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalSession {
    /// Enables raw mode and the alternate screen, returning the terminal.
    fn enter() -> Result<Self, TuiError> {
        enable_raw_mode().map_err(TuiError::Terminal)?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(TuiError::Terminal(error));
        }
        let backend = CrosstermBackend::new(stdout);
        match Terminal::new(backend) {
            Ok(mut terminal) => {
                let _ = terminal.clear();
                Ok(Self { terminal })
            }
            Err(error) => {
                let _ = disable_raw_mode();
                let _ = execute!(io::stdout(), LeaveAlternateScreen);
                Err(TuiError::Terminal(error))
            }
        }
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

/// Installs a panic hook that restores the terminal before the default hook.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = io::stdout().flush();
        previous(info);
    }));
}
