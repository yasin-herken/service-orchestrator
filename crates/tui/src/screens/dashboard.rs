//! The dashboard: an at-a-glance overview of the active workspace.
//!
//! Every number is derived from the application-state snapshot; nothing is
//! hard-coded.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;
use service_orchestrator_domain::TaskStatus;

use crate::app::App;
use crate::components::status;
use crate::theme;

/// Draws the dashboard.
pub(crate) fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let snapshot = &app.snapshot;
    let total = snapshot.services.len();
    let running = snapshot
        .services
        .iter()
        .filter(|service| service.process_state.is_active())
        .count();
    let stopped = total.saturating_sub(running);
    let unhealthy = snapshot
        .services
        .iter()
        .filter(|service| !service.health_state.is_healthy())
        .count();
    let active_tasks = snapshot
        .tasks
        .iter()
        .filter(|task| task.status.is_active())
        .count();
    let failed_tasks = snapshot
        .tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Failed)
        .count();

    let mut lines: Vec<Line<'_>> = Vec::new();
    lines.push(line("Workspace", &snapshot.workspace_name));
    if let Some(root) = &snapshot.workspace_root {
        lines.push(line("Root", root));
    }
    lines.push(Line::from(""));
    lines.push(counts_line(total, running, stopped, unhealthy));
    lines.push(line("Libraries", &snapshot.libraries.len().to_string()));
    lines.push(line(
        "Repositories",
        &snapshot.repositories.len().to_string(),
    ));
    lines.push(line(
        "Migration-enabled",
        &snapshot.migrations.len().to_string(),
    ));
    lines.push(Line::from(""));
    lines.push(line("Active tasks", &active_tasks.to_string()));
    lines.push(line("Failed tasks", &failed_tasks.to_string()));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Recent operations",
        theme::accent(),
    )));
    if snapshot.tasks.is_empty() {
        lines.push(Line::from(Span::styled("  (no tasks yet)", theme::muted())));
    } else {
        for task in snapshot.tasks.iter().rev().take(8) {
            let service = task
                .service
                .as_ref()
                .map_or_else(|| "-".to_owned(), ToString::to_string);
            lines.push(Line::from(vec![
                Span::raw(format!("  {} ", task.id)),
                status::task_status(task.status),
                Span::raw(format!("  {}  {}", task.kind.label(), service)),
            ]));
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(super::panel(app, "Dashboard"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// A `label: value` line.
fn line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<20}"), theme::muted()),
        Span::raw(value.to_owned()),
    ])
}

/// The services summary line.
fn counts_line(total: usize, running: usize, stopped: usize, unhealthy: usize) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{:<20}", "Services"), theme::muted()),
        Span::raw(format!("{total} total   ")),
        Span::styled(format!("{running} running"), theme::success()),
        Span::raw("   "),
        Span::styled(format!("{stopped} stopped"), theme::muted()),
        Span::raw("   "),
        Span::styled(format!("{unhealthy} unhealthy"), theme::warning()),
    ])
}
