//! The Tauri 2 desktop GUI adapter for Service Orchestrator.
//!
//! This crate is a **thin adapter**: it exposes a small set of Tauri commands —
//! service read and lifecycle operations — and pushes connection/task events
//! into the React webview in `apps/desktop`. All real behaviour — configuration
//! parsing, the task engine, process management, daemon transport — stays in the
//! shared workspace crates and the single local daemon, exactly like the Ratatui
//! TUI and MCP adapters (`tasks/018.md` §10; `tasks/019.md` §3; `AGENTS.md` §1,
//! §21).
//!
//! ```text
//! React webview  ──invoke──▶  Tauri commands  ──▶  OrchestratorClient  ──▶  daemon IPC
//! React webview  ◀──emit────  connection/task events ◀── tracker / subscription
//! ```
//!
//! The webview has no shell, filesystem, or process capabilities: the only
//! things it can do are the commands registered in [`run`] and the two events
//! emitted by [`state::AppState`].

pub mod build;
mod commands;
pub mod connection;
mod daemon_bin;
pub mod dto;
mod error;
mod events;
pub mod git;
pub mod health;
pub mod library;
pub mod log_fanout;
pub mod logs;
pub mod migration;
pub mod mutations;
pub mod queries;
mod state;
pub mod tasks;
mod validation;
pub mod workflow;

use tauri::{Manager, Url};

#[cfg(target_os = "macos")]
use tauri::RunEvent;

use state::AppState;

/// The port the Vite dev server listens on during `npm run tauri dev`.
const DEV_SERVER_PORT: u16 = 5173;

/// The initial window size, in logical pixels.
const WINDOW_WIDTH: f64 = 1280.0;
/// The initial window height, in logical pixels.
const WINDOW_HEIGHT: f64 = 840.0;
/// The smallest usable window width, in logical pixels.
const MIN_WINDOW_WIDTH: f64 = 1024.0;
/// The smallest usable window height, in logical pixels.
const MIN_WINDOW_HEIGHT: f64 = 680.0;

/// Runs the desktop application.
///
/// Creates the main window (with navigation restricted to the application
/// itself), sets up the shared [`AppState`] (which resolves the daemon binary
/// and starts the initial connect plus the passive monitor), and registers
/// the read-only commands.
///
/// # Panics
///
/// Panics when the Tauri context cannot be built, the main window cannot be
/// created, or the event loop fails to start — packaging/installation errors
/// the user cannot recover from at runtime.
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // The window is created in code, not from tauri.conf.json, so the
            // navigation filter is attached to it (`tasks/018.md` §17).
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("Service Orchestrator")
            .inner_size(WINDOW_WIDTH, WINDOW_HEIGHT)
            .min_inner_size(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT)
            .center()
            .on_navigation(navigation_allowed)
            .build()
            .expect("failed to create the main window");

            let handle = app.handle().clone();
            let state = AppState::new(handle.clone());
            app.manage(state);
            // The first connect attempt and the passive monitor both run in
            // the background: startup never blocks the window on the daemon.
            state::spawn_initial_connect(handle.clone());
            state::spawn_monitor(handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_daemon_status,
            commands::list_workspaces,
            commands::list_services,
            commands::get_workspace_status,
            commands::get_service_status,
            commands::get_service_detail,
            commands::start_service,
            commands::stop_service,
            commands::restart_service,
            commands::start_services,
            commands::stop_services,
            commands::restart_services,
            commands::check_service_health,
            commands::get_connection_state,
            commands::list_repositories,
            commands::get_git_status,
            commands::inspect_repositories,
            commands::list_branches,
            commands::preview_git_sync,
            commands::fetch_repository,
            commands::pull_repository,
            commands::checkout_repository,
            commands::sync_repository,
            commands::fetch_repositories,
            commands::sync_repositories,
            commands::checkout_repositories,
            commands::list_build_targets,
            commands::get_build_capabilities,
            commands::build_service,
            commands::build_services,
            commands::list_libraries,
            commands::get_library_dependents,
            commands::verify_library_artifact,
            commands::inspect_version_alignment,
            commands::build_library,
            commands::install_library,
            commands::sync_library,
            commands::prepare_library,
            commands::prepare_libraries,
            commands::get_task,
            commands::summarize_workflow,
            commands::get_task_logs,
            commands::cancel_task,
            commands::cancel_workflow,
            commands::list_tasks,
            commands::get_workflow,
            commands::list_profiles,
            commands::get_profile,
            commands::list_groups,
            commands::get_group,
            commands::plan_prepare_environment,
            commands::prepare_environment,
            commands::get_workflow_status,
            commands::list_workflows,
            commands::list_migrations,
            commands::get_migration_status,
            commands::validate_migration,
            commands::inspect_migration_status,
            commands::inspect_migration_pending,
            commands::preview_migration,
            commands::apply_migration,
            commands::apply_selected_migrations,
            commands::list_health,
            commands::get_health,
            commands::check_service_readiness,
            commands::get_logs,
            commands::subscribe_service_logs,
            commands::unsubscribe_service_logs,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build the desktop application")
        .run(|app, event| {
            // macOS: clicking the Dock icon with every window closed must
            // bring the window back instead of leaving a headless app
            // (`tasks/018.md` §16).
            #[cfg(target_os = "macos")]
            if let RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } = event
            {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}

/// Restricts webview navigation to the application itself.
///
/// Production content is served from the `tauri` scheme; development content
/// comes from the local Vite dev server. Everything else — external links,
/// arbitrary hosts — is refused so the webview cannot wander off
/// (`tasks/018.md` §17).
fn navigation_allowed(url: &Url) -> bool {
    match url.scheme() {
        "tauri" => true,
        "http" | "https" => {
            cfg!(debug_assertions)
                && url.host_str() == Some("localhost")
                && url.port_or_known_default() == Some(DEV_SERVER_PORT)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_navigation_stays_in_the_app_scheme() {
        assert!(navigation_allowed(
            &Url::parse("tauri://localhost").unwrap()
        ));
        assert!(navigation_allowed(
            &Url::parse("tauri://localhost/index.html").unwrap()
        ));
    }

    #[test]
    fn external_navigation_is_refused() {
        assert!(!navigation_allowed(
            &Url::parse("https://example.com").unwrap()
        ));
        assert!(!navigation_allowed(
            &Url::parse("http://localhost:9999").unwrap()
        ));
        assert!(!navigation_allowed(
            &Url::parse("http://127.0.0.1:5173").unwrap()
        ));
        assert!(!navigation_allowed(
            &Url::parse("file:///etc/passwd").unwrap()
        ));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn debug_navigation_allows_only_the_dev_server() {
        assert!(navigation_allowed(
            &Url::parse("http://localhost:5173").unwrap()
        ));
        assert!(navigation_allowed(
            &Url::parse("http://localhost:5173/assets/app.js").unwrap()
        ));
        assert!(!navigation_allowed(
            &Url::parse("https://example.com").unwrap()
        ));
    }
}
