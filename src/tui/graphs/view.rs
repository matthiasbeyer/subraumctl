use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{Axis, Chart, Dataset, GraphType, Paragraph, Wrap},
};

use super::{
    Domain,
    history::{History, Series},
};
use crate::{
    cli::GraphLayout,
    tui::{
        app::{App, FocusZone},
        views::{focused_border, panel},
    },
};

pub fn draw(frame: &mut Frame, area: Rect, app: &App, domain: Domain) {
    let state = app.graphs.state(domain);
    let areas = Layout::vertical([Constraint::Length(6), Constraint::Min(0)]).split(area);
    let layout = if state.layout == GraphLayout::Overlay {
        "overlay"
    } else {
        "separate"
    };
    let age = state.updated.map_or_else(
        || "not yet read".into(),
        |at| format!("refreshed {}s ago", at.elapsed().as_secs()),
    );
    let status = if let Some(error) = &state.error {
        format!("STALE / refresh failed: {error}")
    } else if state.busy {
        "Reading history…".into()
    } else if app.options.dry_run {
        "Dry-run: no HTTP requests".into()
    } else {
        age
    };
    let metric = match domain {
        Domain::Power => "Power (W)",
        Domain::Climate if state.humidity => "Humidity (%)",
        Domain::Climate => "Temperature (°C)",
    };
    frame.render_widget(
        Paragraph::new(format!(
            "{metric} | {layout} | requested: {}m\n{status}\ng: table/overlay/separate  +/-: range\nEnter: refresh  ↑/↓: charts{}",
            state.minutes, if domain == Domain::Climate { "  t: °C/%" } else { "" }
        ))
        .style(Style::default().fg(if state.error.is_some() { app.theme.error } else { app.theme.foreground }))
        .block(panel(format!("{} graphs", domain.section().title())).border_style(focused_border(&app.theme, app.zone == FocusZone::Content))),
        areas[0],
    );
    let Some(history) = &state.history else {
        frame.render_widget(
            Paragraph::new("No history loaded. Press Enter to read.").block(panel("History")),
            areas[1],
        );
        return;
    };
    let lines: Vec<_> = history
        .series
        .iter()
        .filter(|s| s.humidity == state.humidity)
        .collect();
    let title = format!("{metric} — last {}m (bucket means)", history.minutes);
    if lines.is_empty() {
        frame.render_widget(
            Paragraph::new("No history returned for this metric.").block(panel(title)),
            areas[1],
        );
    } else if state.layout == GraphLayout::Overlay {
        chart(frame, areas[1], app, history, &lines, &title);
    } else {
        let start = state.scroll.min(lines.len().saturating_sub(1));
        let count = (areas[1].height as usize / 9)
            .clamp(1, 4)
            .min(lines.len() - start);
        let charts =
            Layout::vertical(vec![Constraint::Ratio(1, count as u32); count]).split(areas[1]);
        for (offset, line) in lines.iter().skip(start).take(count).enumerate() {
            chart(
                frame,
                charts[offset],
                app,
                history,
                &[*line],
                &format!(
                    "{} — {title} [{}/{}]",
                    line.name,
                    start + offset + 1,
                    lines.len()
                ),
            );
        }
    }
}

fn color(history: &History, name: &str) -> Color {
    // Same room color in both metrics and layouts, including missing metrics.
    let names: std::collections::BTreeSet<_> =
        history.series.iter().map(|s| s.name.as_str()).collect();
    let index = names.iter().position(|n| *n == name).unwrap_or(0);
    let palette = [
        Color::Cyan,
        Color::Yellow,
        Color::Magenta,
        Color::Green,
        Color::Blue,
        Color::Red,
    ];
    palette[index % palette.len()]
}

fn chart(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    history: &History,
    lines: &[&Series],
    title: &str,
) {
    if area.width < 20 || area.height < 6 {
        frame.render_widget(
            Paragraph::new("Enlarge terminal for graph").block(panel(title)),
            area,
        );
        return;
    }
    // A horizontal legend stays visible at the TUI's 80x24 minimum, where
    // Chart's floating legend would otherwise disappear or obscure the plot.
    let legend = Paragraph::new(Line::from(
        lines
            .iter()
            .map(|line| {
                Span::styled(
                    format!(
                        " {}{} ",
                        line.name,
                        if line.segments.is_empty() { " (?)" } else { "" }
                    ),
                    Style::default().fg(color(history, &line.name)),
                )
            })
            .collect::<Vec<_>>(),
    ))
    .wrap(Wrap { trim: false });
    let legend_height = (legend.line_count(area.width) as u16).min(area.height.saturating_sub(6));
    let areas =
        Layout::vertical([Constraint::Min(6), Constraint::Length(legend_height)]).split(area);
    frame.render_widget(legend, areas[1]);
    let area = areas[0];
    let values: Vec<_> = lines
        .iter()
        .flat_map(|s| &s.segments)
        .flatten()
        .map(|(_, y)| *y)
        .collect();
    if values.is_empty() {
        frame.render_widget(
            Paragraph::new("No samples in this time window.").block(panel(title)),
            area,
        );
        return;
    }
    let low = values.iter().copied().fold(f64::INFINITY, f64::min);
    let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let padding = ((high - low) * 0.05).max(0.1);
    let bounds = [low - padding, high + padding];
    let mut datasets = Vec::new();
    for line in lines {
        let style = Style::default().fg(color(history, &line.name));
        for segment in &line.segments {
            let dataset = Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(style)
                .data(segment);
            datasets.push(dataset);
        }
    }
    let age = crate::influx::now_ms()
        .unwrap_or(history.end_ms)
        .saturating_sub(history.end_ms)
        / 1000;
    let chart = Chart::new(datasets)
        .block(panel(title))
        .style(
            Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.background),
        )
        .x_axis(
            Axis::default()
                .title(format!("minutes (end: {age}s ago)"))
                .bounds([-(history.minutes as f64), 0.0])
                .labels([
                    format!("-{}", history.minutes),
                    format!("-{:.1}", history.minutes as f64 / 2.0),
                    "0".into(),
                ]),
        )
        .y_axis(
            Axis::default()
                .bounds(bounds)
                .labels([format!("{:.1}", bounds[0]), format!("{:.1}", bounds[1])]),
        )
        .legend_position(None);
    frame.render_widget(chart, area);
}
