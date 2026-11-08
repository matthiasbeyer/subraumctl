use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    widgets::{List, ListItem, Paragraph, Wrap},
};

use super::{focused_border, list_state, panel, selection_style};
use crate::pizzamat::Operation;
use crate::tui::app::{App, FocusZone};

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let columns =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).split(area);
    let user = &app.options.pizzamat.user;
    let mut actions = vec![ListItem::new(format!(
        "User: {}",
        if user.is_empty() {
            "(set identity)"
        } else {
            user
        }
    ))];
    actions.extend(
        Operation::ALL
            .iter()
            .map(|operation| ListItem::new(operation.label())),
    );
    frame.render_stateful_widget(
        List::new(actions)
            .block(
                panel("Pizzamat actions")
                    .border_style(focused_border(&app.theme, app.zone == FocusZone::Content)),
            )
            .highlight_style(selection_style(&app.theme))
            .highlight_symbol("> "),
        columns[0],
        &mut list_state(Some(app.pizzamat.focus)),
    );
    let text = if app.pizzamat.response.is_empty() {
        "Select Show orders or Show delivery services and press Enter.\n\nStart order chooses a service. Add food, then Finish my order. End ordering marks the shared list ordered; it does not order from the restaurant.\n\nEnter edits fields; Esc cancels. PgUp/PgDn scroll replies."
    } else {
        &app.pizzamat.response
    };
    let title = if app.pizzamat.busy {
        "Waiting for Pizzamat…"
    } else {
        "Pizzamat response"
    };
    let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
    let max_scroll = paragraph
        .line_count(columns[1].width.saturating_sub(2))
        .saturating_sub(columns[1].height.saturating_sub(2) as usize)
        .min(u16::MAX as usize) as u16;
    app.pizzamat.scroll = app.pizzamat.scroll.min(max_scroll);
    frame.render_widget(
        paragraph
            .scroll((app.pizzamat.scroll, 0))
            .block(panel(title)),
        columns[1],
    );
}
