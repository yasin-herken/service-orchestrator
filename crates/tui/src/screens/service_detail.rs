//! The single-service detail view.
//!
//! Everything shown is read from the snapshot: the service's validated
//! configuration, its last observed status, its last observed health, and the
//! most recent task that targeted it. The view never probes anything.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::components::status;
use crate::theme;

/// Draws the detail view of the service under the cursor.
pub(crate) fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(service) = app.visible_services().get(app.state.cursor).cloned() else {
        frame.render_widget(
            Paragraph::new("No service selected").block(super::panel(app, "Service")),
            area,
        );
        return;
    };

    let id = &service.service_id;
    let config = app.snapshot.service_config(id);
    let mut lines: Vec<Line<'_>> = vec![
        Line::from(Span::styled("Identity", theme::accent())),
        field("Service", &service.display_name),
        field("Id", id.as_str()),
        field("Type", service.kind.label()),
        field("Workspace", &app.snapshot.workspace_name),
        Line::from(""),
        Line::from(Span::styled("Repository", theme::accent())),
    ];

    match config.and_then(|service| service.repository.as_ref()) {
        Some(repository) => {
            lines.push(field("Path", repository.local_path.as_str()));
            lines.push(field("URL", &repository.url));
            lines.push(field("Default branch", repository.default_branch.as_str()));
        }
        None => lines.push(muted("  no repository configured")),
    }
    if let Some(config) = config {
        if let Some(path) = &config.path {
            lines.push(field("Module path", path.as_str()));
        }
    }
    let branch = app
        .state
        .git_status
        .get(id)
        .and_then(|status| status.branch.as_ref())
        .map(|branch| branch.as_str().to_owned())
        .or_else(|| {
            service
                .branch
                .as_ref()
                .map(|branch| branch.as_str().to_owned())
        });
    lines.push(field("Branch", branch.as_deref().unwrap_or("-")));
    let commit = app
        .state
        .git_status
        .get(id)
        .and_then(|status| status.commit.as_deref())
        .unwrap_or("-");
    lines.push(field("Commit", commit));
    lines.push(field("Git state", &service.git_state.to_string()));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Observed state", theme::accent())));
    lines.push(spanned("Process", status::process(service.process_state)));
    lines.push(spanned("Health", status::health(service.health_state)));
    if let Some(health) = app.snapshot.health.get(id) {
        if let Some(checked) = health.checked_at {
            lines.push(field("Last checked", &checked.to_string()));
        }
        if health.consecutive_failures > 0 {
            lines.push(field(
                "Consecutive failures",
                &health.consecutive_failures.to_string(),
            ));
        }
        for check in &health.checks {
            let target = check.target.as_deref().unwrap_or("-");
            let message = check.message.as_deref().unwrap_or("");
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<18}", check.check_type), theme::muted()),
                Span::styled(
                    check.status.label().to_owned(),
                    theme::health_style(check.status),
                ),
                Span::raw(format!("  {target}  {message}")),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Configuration", theme::accent())));
    if let Some(config) = config {
        let runtimes = if config.runtimes.is_empty() {
            "-".to_owned()
        } else {
            config
                .runtimes
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        lines.push(field("Runtimes", &runtimes));
        let build = config
            .build_system
            .map_or_else(|| "-".to_owned(), |system| system.label().to_owned());
        lines.push(field("Build system", &build));
        let ports = if config.ports.is_empty() {
            "-".to_owned()
        } else {
            config
                .ports
                .iter()
                .map(|port| format!(":{}", port.value()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        lines.push(field("Ports", &ports));
        lines.push(field(
            "Start command",
            if config.has_start_command() {
                "configured"
            } else {
                "none"
            },
        ));
        lines.push(field(
            "Build command",
            if config.has_build_command() {
                "configured"
            } else {
                "none"
            },
        ));

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Dependencies", theme::accent())));
        if config.dependencies.is_empty() {
            lines.push(muted("  none declared"));
        } else {
            for dependency in &config.dependencies {
                lines.push(Line::from(vec![
                    Span::styled("  ", theme::muted()),
                    Span::raw(dependency.to_string()),
                ]));
            }
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("Last task", theme::accent())));
    match latest_task(app, id) {
        Some(task) => {
            lines.push(Line::from(vec![
                Span::raw(format!("  {} ", task.id)),
                status::task_status(task.status),
                Span::raw(format!("  {}", task.kind.label())),
            ]));
        }
        None => lines.push(muted("  no tasks observed")),
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(super::panel(
                app,
                format!("Service — {}", service.display_name),
            ))
            .scroll((app.state.detail_scroll, 0))
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// The most recent task that targeted `id`, newest first.
fn latest_task<'a>(
    app: &'a App,
    id: &service_orchestrator_domain::ServiceId,
) -> Option<&'a service_orchestrator_domain::Task> {
    app.snapshot
        .tasks
        .iter()
        .filter(|task| task.service.as_ref() == Some(id))
        .max_by_key(|task| task.created_at.epoch_millis())
}

/// A `label: value` line.
fn field(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<18}"), theme::muted()),
        Span::raw(value.to_owned()),
    ])
}

/// A `label: span` line.
fn spanned(label: &str, value: Span<'static>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<18}"), theme::muted()),
        value,
    ])
}

/// A muted placeholder line.
fn muted(text: &str) -> Line<'static> {
    Line::from(Span::styled(text.to_owned(), theme::muted()))
}
