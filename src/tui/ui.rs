//! Top-level TUI layout: header, sidebar, content, traffic, log, footer and popups.

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use throbber_widgets_tui::Throbber;
use tui_logger::TuiLoggerSmartWidget;

use crate::color::Theme;

use super::app::{App, FocusZone, Popup, Section, StatusKind, TextInputMode, TextInputState};
use super::keymap::{self, KeyBinding};
use super::safe_mode::ConfirmChoice;
use super::views::{self, selection_style, truncate};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.width < 80 || area.height < 24 {
        draw_too_small(frame, area);
        return;
    }

    frame.buffer_mut().set_style(
        area,
        Style::default()
            .bg(app.theme.background)
            .fg(app.theme.foreground),
    );

    let log_height = if app.show_log {
        (area.height as u32 * app.log_height_pct as u32 / 100) as u16
    } else {
        0
    };
    let traffic_height = (area.height / 6).clamp(5, 10);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(traffic_height),
            Constraint::Length(log_height),
            Constraint::Length(1),
            Constraint::Length(2),
        ])
        .split(area);

    draw_header(frame, chunks[0], app);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(24), Constraint::Min(20)])
        .split(chunks[1]);
    draw_sidebar(frame, body[0], app);
    draw_content(frame, body[1], app);
    draw_traffic(frame, chunks[2], app);
    if app.show_log {
        draw_app_log(frame, chunks[3], app);
    }
    draw_status(frame, chunks[4], app);
    draw_footer(frame, chunks[5], app);
    draw_popups(frame, area, app);
    draw_text_input(frame, area, app);
}

fn draw_too_small(frame: &mut Frame, area: Rect) {
    let text = vec![
        Line::from("subraumctl"),
        Line::from(""),
        Line::from("terminal too small"),
        Line::from(format!(
            "need at least 80x24, have {}x{}",
            area.width, area.height
        )),
        Line::from(""),
        Line::from("Q quits"),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title("subraumctl")),
        area,
    );
}

fn draw_header(frame: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(area);

    if app.connection.is_busy() {
        frame.render_stateful_widget(Throbber::default(), chunks[0], &mut app.throbber);
    }

    let connection = app.connection.label(Instant::now());
    let mut spans = vec![
        Span::styled(
            " subraumctl ",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("● ", Style::default().fg(app.connection_color())),
        Span::styled(connection, Style::default().fg(app.connection_color())),
        Span::raw("  alert: "),
        Span::styled("● ", Style::default().fg(app.alert_color())),
        Span::styled(app.alert_label(), Style::default().fg(app.alert_color())),
        Span::raw("  "),
    ];
    if app.safe_mode {
        spans.push(Span::styled(
            "SAFE",
            Style::default()
                .fg(app.theme.warning)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::styled(
            "UNSAFE",
            Style::default()
                .fg(app.theme.error)
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        app.options.mqtt.broker.as_str(),
        Style::default().fg(app.theme.muted),
    ));
    frame.render_widget(Paragraph::new(Line::from(spans)), chunks[1]);
}

fn draw_sidebar(frame: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = Section::ALL
        .iter()
        .map(|section| ListItem::new(section.title()))
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Sections")
                .border_style(Style::default().fg(if app.zone == FocusZone::Sidebar {
                    app.theme.accent
                } else {
                    app.theme.border
                })),
        )
        .highlight_style(selection_style(&app.theme))
        .highlight_symbol("> ");
    frame.render_stateful_widget(
        list,
        area,
        &mut views::list_state(Some(app.section.index())),
    );
}

fn draw_content(frame: &mut Frame, area: Rect, app: &mut App) {
    let mut content_area = area;
    if let Some(TextInputState {
        mode: TextInputMode::Search { section, input },
    }) = &app.text_input
    {
        if *section == app.section {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(0)])
                .split(area);
            let line = Line::from(vec![
                Span::styled("/ ", Style::default().fg(app.theme.accent)),
                Span::raw(input.value()),
            ]);
            frame.render_widget(Paragraph::new(line), chunks[0]);
            let x = chunks[0].x + 2 + input.visual_cursor() as u16;
            frame.set_cursor_position(ratatui::layout::Position::new(
                x.min(chunks[0].right().saturating_sub(1)),
                chunks[0].y,
            ));
            content_area = chunks[1];
        }
    }
    views::draw(frame, content_area, app);
}

fn draw_traffic(frame: &mut Frame, area: Rect, app: &App) {
    let entries = app.traffic.filtered(app.section);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            "Traffic — {} ({})",
            app.section.title(),
            entries.len()
        ))
        .border_style(Style::default().fg(if app.zone == FocusZone::Traffic {
            app.theme.accent
        } else {
            app.theme.border
        }));

    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new("no messages for this section yet").block(block),
            area,
        );
        return;
    }

    let inner_height = area.height.saturating_sub(2) as usize;
    let height = inner_height.max(1);
    let len = entries.len();
    let selected = app.traffic_selected.map(|index| index.min(len - 1));
    let (start, end, state_selected) = if let Some(selected) = selected {
        let start = selected.saturating_sub(height.saturating_sub(1));
        let end = (start + height).min(len);
        (start, end, Some(selected.saturating_sub(start)))
    } else {
        let start = len.saturating_sub(height);
        (start, len, None)
    };

    let items: Vec<ListItem> = entries[start..end]
        .iter()
        .map(|entry| {
            let payload_width = area.width.saturating_sub(54).max(8) as usize;
            let line = Line::from(vec![
                Span::styled(
                    format!("{} ", entry.at),
                    Style::default().fg(app.theme.muted),
                ),
                Span::styled(
                    entry.direction.symbol(),
                    Style::default().fg(entry.direction.color(&app.theme)),
                ),
                Span::raw(" "),
                Span::styled(
                    truncate(&entry.target, 30),
                    Style::default().fg(app.theme.accent),
                ),
                Span::raw(" "),
                Span::raw(truncate(&entry.payload, payload_width)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(selection_style(&app.theme))
        .highlight_symbol("> ");
    let mut state = ListState::default();
    state.select(state_selected);
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_app_log(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Application log (Ctrl+L)")
        .border_style(Style::default().fg(if app.zone == FocusZone::AppLog {
            app.theme.accent
        } else {
            app.theme.border
        }));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(TuiLoggerSmartWidget::default().state(&app.log_state), inner);
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let (text, style) = match &app.status {
        Some(status) => {
            let color = match status.kind {
                StatusKind::Info => app.theme.muted,
                StatusKind::Success => app.theme.success,
                StatusKind::Error => app.theme.error,
            };
            (status.text.clone(), Style::default().fg(color))
        }
        None => (
            "? help   Ctrl+L log   Ctrl+T traffic   S safe mode".to_string(),
            Style::default().fg(app.theme.muted),
        ),
    };
    frame.render_widget(Paragraph::new(text).style(style), area);
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let first = bindings_line(keymap::global_bindings(), &app.theme);
    let context = if app.zone == FocusZone::Content {
        keymap::section_bindings(app.section)
    } else {
        keymap::zone_bindings(app.zone)
    };
    let second = bindings_line(context, &app.theme);
    frame.render_widget(Paragraph::new(vec![first, second]), area);
}

fn bindings_line(bindings: &[KeyBinding], theme: &Theme) -> Line<'static> {
    let mut spans = Vec::new();
    for binding in bindings {
        spans.push(Span::styled(
            binding.keys,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            binding.action,
            Style::default().fg(theme.muted),
        ));
        spans.push(Span::raw("  "));
    }
    Line::from(spans)
}

fn draw_popups(frame: &mut Frame, area: Rect, app: &App) {
    let Some(popup) = &app.popup else {
        return;
    };
    match popup {
        Popup::Quit { selected } => {
            let lines = vec![
                Line::from("Close subraumctl?"),
                Line::from(""),
                button_line(&app.theme, *selected, "Quit"),
                Line::from(""),
                Line::from(Span::styled(
                    "y confirm   n/Esc/q cancel   Q force quit",
                    Style::default().fg(app.theme.muted),
                )),
            ];
            render_popup(frame, area, "Quit", lines, 50, 30, &app.theme);
        }
        Popup::SafeModeOff { selected } => {
            let lines = vec![
                Line::from("Disable safe mode?"),
                Line::from("Commands will be sent without confirmation."),
                Line::from(""),
                button_line(&app.theme, *selected, "Disable"),
                Line::from(""),
                Line::from(Span::styled(
                    "y confirm   n/Esc/q cancel",
                    Style::default().fg(app.theme.muted),
                )),
            ];
            render_popup(frame, area, "Safe mode", lines, 58, 32, &app.theme);
        }
        Popup::ConfirmCommand { pending, selected } => {
            let mut lines: Vec<Line<'static>> = pending
                .preview_lines(4)
                .into_iter()
                .map(owned_line)
                .collect();
            lines.push(Line::from(""));
            lines.push(button_line(&app.theme, *selected, "Send"));
            lines.push(Line::from(Span::styled(
                "y send   Enter activates the selected button   n/Esc/q cancel",
                Style::default().fg(app.theme.muted),
            )));
            render_popup(frame, area, "Confirm command", lines, 80, 50, &app.theme);
        }
        Popup::Help { scroll } => {
            let popup_area = centered_rect(80, 85, area);
            frame.render_widget(Clear, popup_area);
            let block = Block::default()
                .borders(Borders::ALL)
                .title("Help")
                .border_style(Style::default().fg(app.theme.accent))
                .style(
                    Style::default()
                        .bg(app.theme.background)
                        .fg(app.theme.foreground),
                );
            let inner = block.inner(popup_area);
            frame.render_widget(block, popup_area);
            frame.render_widget(
                Paragraph::new(keymap::help_text())
                    .scroll((*scroll, 0))
                    .wrap(Wrap { trim: false }),
                inner,
            );
        }
        Popup::TrafficDetail { entry } => {
            let detail = entry.detail();
            let lines = detail
                .lines()
                .map(|line| Line::from(line.to_string()))
                .collect::<Vec<Line<'static>>>();
            render_popup(frame, area, "Message detail", lines, 72, 48, &app.theme);
        }
        Popup::ColorPicker { picker } => {
            views::theme::draw_picker(frame, centered_rect(75, 80, area), picker, &app.theme);
        }
        Popup::ThemeReset { selected } => {
            let lines = vec![
                Line::from("Reset the whole theme?"),
                Line::from("Every role returns to the selected preset."),
                Line::from(""),
                button_line(&app.theme, *selected, "Reset"),
                Line::from(""),
                Line::from(Span::styled(
                    "y confirm   n/Esc/q cancel",
                    Style::default().fg(app.theme.muted),
                )),
            ];
            render_popup(frame, area, "Reset theme", lines, 56, 32, &app.theme);
        }
    }
}

fn draw_text_input(frame: &mut Frame, area: Rect, app: &App) {
    let Some(TextInputState { mode }) = &app.text_input else {
        return;
    };
    if let TextInputMode::Pizza {
        operation,
        fields,
        input,
    } = mode
    {
        let popup_area = centered_rect(85, 35, area);
        frame.render_widget(Clear, popup_area);
        let title = operation.map_or("Pizzamat identity", |operation| operation.label());
        let block = views::panel(title)
            .border_style(Style::default().fg(app.theme.accent))
            .style(
                Style::default()
                    .bg(app.theme.background)
                    .fg(app.theme.foreground),
            );
        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);
        let rows = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(inner);
        let label = operation.map_or_else(
            || "User identity".to_string(),
            |operation| {
                format!(
                    "{}/{}: {}",
                    fields.len() + 1,
                    operation.fields().len(),
                    operation.fields()[fields.len()]
                )
            },
        );
        frame.render_widget(Paragraph::new(label).wrap(Wrap { trim: false }), rows[0]);
        let scroll = input.visual_scroll(rows[1].width.saturating_sub(1) as usize);
        frame.render_widget(
            Paragraph::new(input.value()).scroll((0, scroll as u16)),
            rows[1],
        );
        frame.render_widget(
            Paragraph::new("Enter next/save   Esc cancel   Ctrl+C quit"),
            rows[2],
        );
        frame.set_cursor_position((
            rows[1].x + input.visual_cursor().saturating_sub(scroll) as u16,
            rows[1].y,
        ));
        return;
    }
    let (title, input) = match mode {
        TextInputMode::Pizza { .. } => return,
        TextInputMode::Search { .. } => return,
        TextInputMode::ExactKnob { knob, input } => {
            (format!("Set knob {} value", knob.name()), input)
        }
        TextInputMode::ExactWarm { input } => ("Warm white value (0..255)".to_string(), input),
        TextInputMode::ExactCold { input } => ("Cold white value (0..255)".to_string(), input),
        TextInputMode::ExactLaser { input } => ("Laser power (0..255)".to_string(), input),
    };

    let popup_area = centered_rect(55, 30, area);
    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(app.theme.accent))
        .style(
            Style::default()
                .bg(app.theme.background)
                .fg(app.theme.foreground),
        );
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);
    let line = Line::from(vec![
        Span::styled("value: ", Style::default().fg(app.theme.muted)),
        Span::raw(input.value()),
    ]);
    frame.render_widget(Paragraph::new(line), inner);
    let x = inner.x + 7 + input.visual_cursor() as u16;
    frame.set_cursor_position(ratatui::layout::Position::new(
        x.min(inner.right().saturating_sub(1)),
        inner.y,
    ));
}

fn owned_line(text: String) -> Line<'static> {
    Line::from(text)
}

fn button_line(theme: &Theme, selected: ConfirmChoice, confirm_label: &str) -> Line<'static> {
    let cancel_style = if selected == ConfirmChoice::Cancel {
        selection_style(theme)
    } else {
        Style::default().fg(theme.muted)
    };
    let confirm_style = if selected == ConfirmChoice::Confirm {
        selection_style(theme)
    } else {
        Style::default().fg(theme.muted)
    };
    Line::from(vec![
        Span::styled("[ Cancel ]", cancel_style),
        Span::raw("   "),
        Span::styled(format!("[ {confirm_label} ]"), confirm_style),
    ])
}

fn render_popup(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    lines: Vec<Line<'static>>,
    percent_x: u16,
    percent_y: u16,
    theme: &Theme,
) {
    let popup_area = centered_rect(percent_x, percent_y, area);
    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title.to_string())
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.background).fg(theme.foreground));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}
