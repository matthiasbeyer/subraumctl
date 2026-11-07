//! Section rendering.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, ListState};

use super::app::{App, Section};
use crate::color::Theme;

pub mod alerts;
pub mod animation;
pub mod climate;
pub mod heizung;
pub mod lighting;
pub mod outlets;
pub mod overview;
pub mod pizzamat;
pub mod smoothie;
pub mod system;
pub mod theme;
pub mod white_light;

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    match app.section {
        Section::Overview => overview::draw(frame, area, app),
        Section::Lighting => lighting::draw(frame, area, app),
        Section::WhiteLight => white_light::draw(frame, area, app),
        Section::Outlets => outlets::draw(frame, area, app),
        Section::Alerts => alerts::draw(frame, area, app),
        Section::System => system::draw(frame, area, app),
        Section::Animation => animation::draw(frame, area, app),
        Section::Smoothie => smoothie::draw(frame, area, app),
        Section::Theme => theme::draw(frame, area, app),
        Section::Pizzamat => pizzamat::draw(frame, area, app),
        Section::Heizung => heizung::draw(frame, area, app),
        Section::Climate => climate::draw(frame, area, app),
    }
}

pub(crate) fn panel(title: impl Into<String>) -> Block<'static> {
    Block::default().borders(Borders::ALL).title(title.into())
}

pub(crate) fn selection_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.foreground)
        .bg(theme.selection)
        .add_modifier(Modifier::BOLD)
}

pub(crate) fn focused_border(theme: &Theme, focused: bool) -> Style {
    Style::default().fg(if focused { theme.accent } else { theme.border })
}

pub(crate) fn list_state(selected: Option<usize>) -> ListState {
    let mut state = ListState::default();
    state.select(selected);
    state
}

pub(crate) fn status_text(state: Option<bool>) -> &'static str {
    match state {
        Some(true) => "on",
        Some(false) => "off",
        None => "?",
    }
}

pub(crate) fn status_style(theme: &Theme, state: Option<bool>) -> Style {
    match state {
        Some(true) => Style::default().fg(theme.success),
        Some(false) => Style::default().fg(theme.error),
        None => Style::default().fg(theme.muted),
    }
}

pub(crate) fn truncate(text: &str, max: usize) -> String {
    let sanitized = text.replace('\n', "⏎");
    if max == 0 {
        return String::new();
    }
    if sanitized.chars().count() <= max {
        sanitized
    } else {
        let mut result: String = sanitized.chars().take(max.saturating_sub(1)).collect();
        result.push('…');
        result
    }
}
