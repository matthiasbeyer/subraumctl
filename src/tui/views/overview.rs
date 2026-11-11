use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};

use super::super::app::{App, FocusZone, Section};
use super::{focused_border, list_state, panel, selection_style};

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Length(6),
            Constraint::Min(5),
        ])
        .split(area);
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(rows[0]);
    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[1]);

    let now = Instant::now();
    let connection = vec![
        Line::from(vec![
            Span::styled("status: ", Style::default().fg(app.theme.muted)),
            Span::styled(
                app.connection.label(now),
                Style::default().fg(app.connection_color()),
            ),
        ]),
        Line::from(format!("broker: {}", app.options.mqtt.broker)),
        Line::from(format!("client-id: {}", app.options.mqtt.client_id)),
        Line::from(format!("generation: {}", app.mqtt_generation)),
        Line::from(format!(
            "keep-alive: {}s",
            app.options.mqtt.keep_alive.as_secs()
        )),
    ];
    frame.render_widget(
        Paragraph::new(connection).block(
            panel("Connection")
                .border_style(focused_border(&app.theme, app.zone == FocusZone::Content)),
        ),
        top[0],
    );

    let alert = vec![
        Line::from(vec![
            Span::styled("● ", Style::default().fg(app.alert_color())),
            Span::styled(app.alert_label(), Style::default().fg(app.alert_color())),
        ]),
        Line::from(format!("dim: {}", app.alert.dim)),
        Line::from("topic: heimdallr/alert/condition"),
    ];
    frame.render_widget(
        Paragraph::new(alert).block(panel("Alert").border_style(focused_border(&app.theme, false))),
        top[1],
    );

    let safe_color = if app.safe_mode {
        app.theme.warning
    } else {
        app.theme.error
    };
    let safe_lines = vec![
        Line::from(vec![Span::styled(
            if app.safe_mode { "SAFE" } else { "UNSAFE" },
            Style::default().fg(safe_color).add_modifier(Modifier::BOLD),
        )]),
        Line::from("S toggles safe mode"),
        Line::from(if app.safe_mode {
            "Device writes confirmed (including heating)"
        } else {
            "commands are sent immediately"
        }),
    ];
    frame.render_widget(
        Paragraph::new(safe_lines)
            .block(panel("Safe mode").border_style(focused_border(&app.theme, false))),
        middle[0],
    );

    let status = app
        .status
        .as_ref()
        .map(|status| status.text.as_str())
        .unwrap_or("no recent activity");
    let activity = vec![
        Line::from(status),
        Line::from(format!("traffic entries: {}", app.traffic.len())),
        Line::from(format!(
            "safe mode: {}",
            if app.safe_mode { "on" } else { "off" }
        )),
    ];
    frame.render_widget(
        Paragraph::new(activity)
            .block(panel("Last activity").border_style(focused_border(&app.theme, false))),
        middle[1],
    );

    let items: Vec<ListItem> = Section::ALL
        .iter()
        .map(|section| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:<12}", section.title()),
                    Style::default().fg(app.theme.accent),
                ),
                Span::raw(description(*section)),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            panel("Sections")
                .border_style(focused_border(&app.theme, app.zone == FocusZone::Content)),
        )
        .highlight_style(selection_style(&app.theme))
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, rows[2], &mut list_state(Some(app.overview.focus)));
}

fn description(section: Section) -> &'static str {
    match section {
        Section::Overview => "connection, alert and quick links",
        Section::Lighting => "DMX animation commands",
        Section::WhiteLight => "warm/cold white-light cells",
        Section::Outlets => "simple power outlets",
        Section::Alerts => "radiation emitters and sob sounds",
        Section::System => "telinit",
        Section::Animation => "animation knobs",
        Section::Smoothie => "relays, laser and coolant status",
        Section::Audio => "room mute, volume and audio routing",
        Section::Theme => "TUI colors and palette",
        Section::Pizzamat => "shared food orders and delivery services",
        Section::Heizung => "per-room heating targets and timers",
        Section::Climate => "measured room temperature and humidity",
        Section::Power => "meter power by phase and total",
        Section::Music => "MPD playback and radio stations",
    }
}
