use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    widgets::{Paragraph, Row, Table},
};

use super::super::app::{App, FocusZone};
use super::{focused_border, panel};
use crate::{influx::WINDOW_MS, power::format_watts};

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let state = &app.power;
    let areas = Layout::vertical([Constraint::Length(5), Constraint::Min(3)]).split(area);
    let refreshed = state.updated.map_or_else(
        || "not yet read".into(),
        |at| format!("refreshed {}s ago", at.elapsed().as_secs()),
    );
    let sample_age = state
        .snapshot
        .as_ref()
        .and_then(|snapshot| {
            snapshot.sample_time_ms.map(|time| {
                let age = crate::influx::now_ms()
                    .unwrap_or(snapshot.queried_at_ms)
                    .saturating_sub(time);
                format!(
                    " | sample {}s old{}",
                    age / 1000,
                    if age > WINDOW_MS { " (>15m)" } else { "" }
                )
            })
        })
        .unwrap_or_default();
    let status = if let Some(error) = &state.error {
        format!("STALE / refresh failed: {error}")
    } else if state.busy {
        "Reading…".into()
    } else if app.options.dry_run {
        "Dry-run: no HTTP requests".into()
    } else {
        "←/→ or m: mode   Enter: refresh".into()
    };
    frame.render_widget(
        Paragraph::new(format!(
            "Mode: {}\n{refreshed}{sample_age}\n{status}",
            state.mode.label()
        ))
        .style(Style::default().fg(if state.error.is_some() {
            app.theme.error
        } else {
            app.theme.foreground
        }))
        .block(
            panel("Power").border_style(focused_border(&app.theme, app.zone == FocusZone::Content)),
        ),
        areas[0],
    );
    let Some(snapshot) = &state.snapshot else {
        frame.render_widget(
            Paragraph::new("No power snapshot. Press Enter to read.").block(panel("Meter")),
            areas[1],
        );
        return;
    };
    let readings = [
        ("L1", snapshot.l1_watts),
        ("L2", snapshot.l2_watts),
        ("L3", snapshot.l3_watts),
        ("Total", snapshot.total_watts),
    ];
    let empty = readings.iter().all(|(_, value)| value.is_none());
    let label = format!(
        "Meter — {}{}",
        snapshot.mode.label(),
        if empty { " — no data" } else { "" }
    );
    let rows = readings
        .into_iter()
        .map(|(phase, value)| Row::new(vec![phase.to_string(), format_watts(value)]));
    let table = Table::new(
        rows,
        [Constraint::Percentage(35), Constraint::Percentage(65)],
    )
    .header(Row::new(["Phase", "Power"]).style(Style::default().fg(app.theme.accent)))
    .block(panel(label));
    frame.render_widget(table, areas[1]);
}
