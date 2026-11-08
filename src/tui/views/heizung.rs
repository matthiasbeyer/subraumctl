use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    widgets::{List, ListItem, Paragraph},
};

use super::{focused_border, list_state, panel, selection_style};
use crate::heizung::Room;
use crate::tui::app::{App, FocusZone};

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).split(area);
    let state = &app.heizung;
    let mut items: Vec<ListItem> = Room::ALL
        .iter()
        .enumerate()
        .map(|(index, room)| {
            let selection = if state.selected[index] { 'x' } else { ' ' };
            let reading = state.readings[index]
                .map(|reading| {
                    let seconds = reading.remaining_seconds;
                    format!(
                        "{:4.1} °C   {:02}:{:02}:{:02}",
                        reading.target,
                        seconds / 3600,
                        seconds / 60 % 60,
                        seconds % 60
                    )
                })
                .unwrap_or_else(|| "   ? °C          ?".into());
            ListItem::new(format!("[{selection}] {:<7} {reading}", room.name()))
        })
        .collect();
    items.extend([
        ListItem::new(format!("Target: {:.1} °C", state.temperature())),
        ListItem::new(format!("Duration: {} min", state.duration)),
        ListItem::new(format!(
            "Apply to {} selected room(s)",
            state.selected_rooms().len()
        )),
        ListItem::new("Refresh status (all rooms)"),
    ]);
    frame.render_stateful_widget(
        List::new(items)
            .block(
                panel(if state.busy {
                    "Heizung — waiting…"
                } else {
                    "Heizung — target / time remaining"
                })
                .border_style(focused_border(&app.theme, app.zone == FocusZone::Content)),
            )
            .highlight_style(selection_style(&app.theme))
            .highlight_symbol("> "),
        rows[0],
        &mut list_state(Some(state.focus)),
    );
    let age = state
        .updated
        .map(|at| format!("Snapshot {}s ago", at.elapsed().as_secs()))
        .unwrap_or_else(|| "No status received".into());
    frame.render_widget(
        Paragraph::new(format!("{age} — targets, not measured\n{}", state.message)).style(
            Style::default().fg(if state.error {
                app.theme.error
            } else {
                app.theme.muted
            }),
        ),
        rows[1],
    );
}
