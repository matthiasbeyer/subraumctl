use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph, Wrap},
};

use super::{focused_border, list_state, panel, selection_style, truncate};
use crate::music::STATIONS;
use crate::tui::app::{App, FocusZone};
use crate::tui::music::{PAUSE, PLAY};

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let rows = Layout::vertical([Constraint::Length(6), Constraint::Min(3)]).split(area);
    let state = &app.music;

    let (state_label, state_style) = match state.status.as_ref().map(|s| s.state.as_str()) {
        Some("play") => ("Playing", Style::default().fg(app.theme.success)),
        Some("pause") => ("Paused", Style::default().fg(app.theme.warning)),
        Some("stop") => ("Stopped", Style::default().fg(app.theme.muted)),
        Some(_) => ("Unknown", Style::default().fg(app.theme.muted)),
        None => ("Connecting", Style::default().fg(app.theme.muted)),
    };
    let title = state
        .status
        .as_ref()
        .map(|status| status.display_title())
        .unwrap_or_else(|| "No status received".into());
    let time = state
        .status
        .as_ref()
        .map(|status| status.time_label())
        .unwrap_or_default();
    let volume = state
        .status
        .as_ref()
        .and_then(|status| status.volume)
        .map(|volume| format!("volume {volume}"))
        .unwrap_or_default();
    let age = state
        .updated
        .map(|at| format!("updated {}s ago", at.elapsed().as_secs()))
        .unwrap_or_else(|| "not read yet".into());
    let message = if let Some(error) = &state.error {
        Line::from(Span::styled(
            error.clone(),
            Style::default().fg(app.theme.error),
        ))
    } else if state.busy {
        Line::from(Span::styled(
            "Talking to MPD…",
            Style::default().fg(app.theme.muted),
        ))
    } else {
        let mut spans = vec![Span::styled(age, Style::default().fg(app.theme.muted))];
        if !time.is_empty() {
            spans.push(Span::raw("   "));
            spans.push(Span::styled(time, Style::default().fg(app.theme.muted)));
        }
        if !volume.is_empty() {
            spans.push(Span::raw("   "));
            spans.push(Span::styled(volume, Style::default().fg(app.theme.muted)));
        }
        Line::from(spans)
    };
    let lines = vec![
        Line::from(vec![
            Span::styled("● ", state_style),
            Span::styled(state_label, state_style.add_modifier(Modifier::BOLD)),
            Span::raw("   "),
            Span::styled(
                truncate(&title, area.width.saturating_sub(24) as usize),
                Style::default()
                    .fg(app.theme.foreground)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        message,
    ];
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            panel("Music — now playing")
                .border_style(focused_border(&app.theme, app.zone == FocusZone::Content)),
        ),
        rows[0],
    );

    let controls = [
        ("Play", "resume playback"),
        ("Pause", "pause playback"),
        ("Previous", "previous queue entry"),
        ("Next", "next queue entry"),
        ("Refresh", "read MPD status"),
    ];
    let mut items: Vec<ListItem> = controls
        .iter()
        .enumerate()
        .map(|(index, (label, help))| {
            let active = state.status.as_ref().map(|status| status.state.as_str());
            let button_style = match index {
                PLAY if active == Some("play") => Style::default().fg(app.theme.success),
                PAUSE if active == Some("pause") => Style::default().fg(app.theme.warning),
                _ => Style::default().fg(app.theme.foreground),
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("[ {} ]", label), button_style),
                Span::raw("  "),
                Span::styled(*help, Style::default().fg(app.theme.muted)),
            ]))
        })
        .collect();
    items.extend(STATIONS.iter().map(|station| {
        ListItem::new(Line::from(vec![
            Span::styled(
                format!("{:<16}", station.id),
                Style::default().fg(app.theme.accent),
            ),
            Span::raw(station.name),
        ]))
    }));
    frame.render_stateful_widget(
        List::new(items)
            .block(
                panel("Controls and radio stations")
                    .border_style(focused_border(&app.theme, app.zone == FocusZone::Content)),
            )
            .highlight_style(selection_style(&app.theme))
            .highlight_symbol("> "),
        rows[1],
        &mut list_state(Some(state.focus)),
    );
}
