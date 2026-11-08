use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::action::AudioRoom;
use crate::tui::app::App;

use super::{focused_border, selection_style};

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);
    draw_maximum_toggle(frame, sections[0], app);

    let cards = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25); 4])
        .split(sections[1]);
    for (index, room) in AudioRoom::ALL.iter().enumerate() {
        draw_room(frame, cards[index], app, index, room.as_str());
    }
}

fn draw_maximum_toggle(frame: &mut Frame, area: Rect, app: &App) {
    let focused = app.audio.focus == AudioRoom::ALL.len() * 4;
    let checked = if app.audio.raise_maximum_volume {
        "[x]"
    } else {
        "[ ]"
    };
    let style = if focused {
        selection_style(&app.theme)
    } else {
        Style::default().fg(app.theme.foreground)
    };
    frame.render_widget(
        Paragraph::new(format!("{checked} Raise maximum volume (allow up to 200%)"))
            .style(style)
            .alignment(Alignment::Left),
        area,
    );
}

fn draw_vertical_slider(
    frame: &mut Frame,
    area: Rect,
    volume: u8,
    focused: bool,
    expanded: bool,
    theme: &crate::color::Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Volume")
        .border_style(focused_border(theme, focused));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height < 2 {
        return;
    }

    let label = Paragraph::new(format!("{volume}%"))
        .alignment(Alignment::Center)
        .style(
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        );
    let label_area = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(label, label_area);

    let track_y = inner.y.saturating_add(1);
    let track_height = inner.height.saturating_sub(1);
    let scale = if expanded { 200 } else { 100 };
    let filled = (track_height as u32 * volume.min(scale) as u32 / scale as u32) as u16;
    let filled_start = track_y + track_height.saturating_sub(filled);
    let track_style = Style::default().fg(theme.border);
    let fill_style = if focused {
        selection_style(theme)
    } else {
        Style::default().fg(theme.accent)
    };
    let center_x = inner.x + inner.width / 2;
    for offset in 0..track_height {
        let y = track_y + offset;
        let filled_cell = y >= filled_start && filled > 0;
        let symbol = if filled_cell { "█" } else { "│" };
        let style = if filled_cell { fill_style } else { track_style };
        frame.buffer_mut().set_string(center_x, y, symbol, style);
    }
}

fn draw_room(frame: &mut Frame, area: Rect, app: &App, index: usize, name: &str) {
    let focused_volume = app.audio.focus == index * 4;
    let focused_mute = app.audio.focus == index * 4 + 1;
    let focused_attach = app.audio.focus == index * 4 + 2;
    let focused_detach = app.audio.focus == index * 4 + 3;
    let inner = Block::default()
        .borders(Borders::ALL)
        .title(name)
        .border_style(focused_border(
            &app.theme,
            focused_volume || focused_mute || focused_attach || focused_detach,
        ))
        .style(
            Style::default()
                .bg(app.theme.background)
                .fg(app.theme.foreground),
        );
    let content = inner.inner(area);
    frame.render_widget(inner, area);

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(content);
    let volume = app.audio.volumes[index];
    draw_vertical_slider(
        frame,
        parts[0],
        volume,
        focused_volume,
        app.audio.raise_maximum_volume,
        &app.theme,
    );

    let mute_label = if app.audio.muted[index] {
        "[ MUTED ]"
    } else {
        "[ MUTE ]"
    };
    let mute_style = if focused_mute {
        selection_style(&app.theme)
    } else if app.audio.muted[index] {
        Style::default()
            .fg(app.theme.warning)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(app.theme.foreground)
    };
    frame.render_widget(
        Paragraph::new(mute_label)
            .alignment(Alignment::Center)
            .style(mute_style),
        parts[1],
    );

    let attach_style = if focused_attach {
        selection_style(&app.theme)
    } else if !app.audio.detached[index] {
        Style::default().fg(app.theme.success)
    } else {
        Style::default().fg(app.theme.muted)
    };
    let detach_style = if focused_detach {
        selection_style(&app.theme)
    } else if app.audio.detached[index] {
        Style::default().fg(app.theme.warning)
    } else {
        Style::default().fg(app.theme.muted)
    };
    frame.render_widget(
        Paragraph::new("[A] Attach")
            .alignment(Alignment::Center)
            .style(attach_style),
        parts[2],
    );
    frame.render_widget(
        Paragraph::new("[D] Detach")
            .alignment(Alignment::Center)
            .style(detach_style),
        parts[3],
    );
}
