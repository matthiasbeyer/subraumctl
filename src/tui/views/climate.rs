use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    widgets::{Paragraph, Row, Table, TableState},
};

use super::super::app::{App, FocusZone};
use super::{focused_border, panel, selection_style};
use crate::influx::{Mode, WINDOW_MS};

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let state = &app.climate;
    let areas = Layout::vertical([Constraint::Length(4), Constraint::Min(3)]).split(area);
    let age = state.updated.map_or_else(
        || "not yet read".into(),
        |at| format!("refreshed {}s ago", at.elapsed().as_secs()),
    );
    let status = if let Some(error) = &state.error {
        format!("STALE / refresh failed: {error}")
    } else if state.busy {
        "Reading…".into()
    } else if app.options.dry_run {
        "Dry-run: no HTTP requests".into()
    } else {
        "←/→ or m: mode   Enter: refresh   ↑/↓: scroll".into()
    };
    frame.render_widget(
        Paragraph::new(format!("Mode: {} | {age}\n{status}", state.mode.label()))
            .style(Style::default().fg(if state.error.is_some() {
                app.theme.error
            } else {
                app.theme.foreground
            }))
            .block(
                panel("Climate")
                    .border_style(focused_border(&app.theme, app.zone == FocusZone::Content)),
            ),
        areas[0],
    );

    let Some(snapshot) = &state.snapshot else {
        frame.render_widget(
            Paragraph::new("No climate snapshot. Press Enter to read.").block(panel("Rooms")),
            areas[1],
        );
        return;
    };
    if snapshot.rooms.is_empty() {
        frame.render_widget(
            Paragraph::new("No climate data returned.").block(panel(snapshot.mode.label())),
            areas[1],
        );
        return;
    }
    let now = crate::influx::now_ms().unwrap_or(snapshot.queried_at_ms);
    let sample = |value: Option<f64>, time: Option<u64>, unit: &str| {
        value.map_or_else(
            || "?".into(),
            |value| {
                let age = time.map_or(String::new(), |t| {
                    let elapsed = now.saturating_sub(t);
                    format!(
                        " ({}s{})",
                        elapsed / 1000,
                        if elapsed > WINDOW_MS { ", old" } else { "" }
                    )
                });
                format!("{value:.2} {unit}{age}")
            },
        )
    };
    let rows = snapshot.rooms.iter().map(|room| {
        Row::new(vec![
            room.location.clone(),
            sample(room.temperature_celsius, room.temperature_time_ms, "°C"),
            sample(room.humidity_percent, room.humidity_time_ms, "%"),
        ])
    });
    let label = format!(
        "Rooms — {}{}",
        snapshot.mode.label(),
        if snapshot.mode == Mode::Latest {
            " (sample ages; old >15m)"
        } else {
            ""
        }
    );
    let table = Table::new(
        rows,
        [
            Constraint::Percentage(24),
            Constraint::Percentage(38),
            Constraint::Percentage(38),
        ],
    )
    .header(
        Row::new(["Location", "Temperature", "Humidity"])
            .style(Style::default().fg(app.theme.accent)),
    )
    .block(panel(label))
    .row_highlight_style(selection_style(&app.theme));
    frame.render_stateful_widget(
        table,
        areas[1],
        &mut TableState::default().with_selected(state.selected),
    );
}
