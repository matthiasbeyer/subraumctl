//! TUI-only history state and background reads. Table sampling stays independent.

mod history;
mod view;
pub use view::draw;

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use super::{
    app::{App, Section},
    traffic::{TrafficDirection, TrafficEntry, now_hms},
};
use crate::{cli::GraphLayout, influx::Options};
use history::History;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    Climate,
    Power,
}

impl Domain {
    fn database(self) -> &'static str {
        match self {
            Self::Climate => crate::climate::DATABASE,
            Self::Power => crate::power::DATABASE,
        }
    }

    fn section(self) -> Section {
        match self {
            Self::Climate => Section::Climate,
            Self::Power => Section::Power,
        }
    }
}

pub struct State {
    pub layout: GraphLayout,
    pub minutes: u64,
    pub humidity: bool,
    pub scroll: usize,
    pub history: Option<History>,
    pub attempted: Option<Instant>,
    pub updated: Option<Instant>,
    pub busy: bool,
    pub error: Option<String>,
}

impl State {
    fn new(minutes: u64, layout: GraphLayout) -> Self {
        Self {
            layout,
            minutes,
            humidity: false,
            scroll: 0,
            history: None,
            attempted: None,
            updated: None,
            busy: false,
            error: None,
        }
    }
}

pub struct Graphs {
    pub climate: State,
    pub power: State,
    pub tx: Option<UnboundedSender<Request>>,
}

impl Graphs {
    pub fn new(minutes: u64, layout: GraphLayout) -> Self {
        Self {
            climate: State::new(minutes, layout),
            power: State::new(minutes, layout),
            tx: None,
        }
    }

    fn state(&self, domain: Domain) -> &State {
        match domain {
            Domain::Climate => &self.climate,
            Domain::Power => &self.power,
        }
    }

    fn state_mut(&mut self, domain: Domain) -> &mut State {
        match domain {
            Domain::Climate => &mut self.climate,
            Domain::Power => &mut self.power,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    domain: Domain,
    minutes: u64,
}

pub struct Event {
    request: Request,
    result: Result<Option<History>, String>,
}

pub fn spawn_worker(
    options: Options,
    dry_run: bool,
    mut rx: UnboundedReceiver<Request>,
    tx: UnboundedSender<Event>,
) {
    tokio::spawn(async move {
        while let Some(request) = rx.recv().await {
            let result = if dry_run {
                Ok(None)
            } else {
                history::read(&options, request.domain, request.minutes)
                    .await
                    .map(Some)
                    .map_err(|e| e.to_string())
            };
            if tx.send(Event { request, result }).is_err() {
                break;
            }
        }
    });
}

impl App {
    pub(super) fn refresh_graph(&mut self, domain: Domain, force: bool) {
        let state = self.graphs.state(domain);
        if state.layout == GraphLayout::Table || state.busy || self.graphs.tx.is_none() {
            return;
        }
        if !force
            && (!self.options.connect
                || state
                    .attempted
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(30)))
        {
            return;
        }
        let request = Request {
            domain,
            minutes: state.minutes,
        };
        let sent = self
            .graphs
            .tx
            .as_ref()
            .expect("worker attached")
            .send(request)
            .is_ok();
        let state = self.graphs.state_mut(domain);
        state.attempted = Some(Instant::now());
        state.busy = sent;
        state.error = if sent {
            None
        } else {
            Some("Graph worker stopped".into())
        };
        if sent {
            self.traffic.push(traffic(
                domain,
                if self.options.dry_run {
                    TrafficDirection::Dry
                } else {
                    TrafficDirection::Tx
                },
                format!("history: last {} minutes", request.minutes),
            ));
        }
    }

    pub(super) fn handle_graph_key(&mut self, domain: Domain, key: KeyEvent) -> bool {
        let state = self.graphs.state_mut(domain);
        if key.code == KeyCode::Char('g') {
            state.layout = match state.layout {
                GraphLayout::Table => GraphLayout::Overlay,
                GraphLayout::Overlay => GraphLayout::Separate,
                GraphLayout::Separate => GraphLayout::Table,
            };
            if state.layout == GraphLayout::Overlay {
                self.refresh_graph(domain, true);
            }
            return true;
        }
        if state.layout == GraphLayout::Table {
            return false;
        }
        match key.code {
            KeyCode::Enter => self.refresh_graph(domain, true),
            KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char('-') => {
                let ranges = [1, 5, 15, 30, 60, 180, 360, 720, 1440, 10080, 43200];
                let minutes = if key.code == KeyCode::Char('-') {
                    ranges
                        .into_iter()
                        .rev()
                        .find(|m| *m < state.minutes)
                        .unwrap_or(1)
                } else {
                    ranges
                        .into_iter()
                        .find(|m| *m > state.minutes)
                        .unwrap_or(43200)
                };
                if state.minutes != minutes {
                    state.minutes = minutes;
                    self.refresh_graph(domain, true);
                }
            }
            KeyCode::Char('t') if domain == Domain::Climate => {
                state.humidity = !state.humidity;
                state.scroll = 0;
            }
            KeyCode::Up => state.scroll = state.scroll.saturating_sub(1),
            KeyCode::Down => {
                let count = state.history.as_ref().map_or(0, |h| {
                    h.series
                        .iter()
                        .filter(|s| s.humidity == state.humidity)
                        .count()
                });
                state.scroll = (state.scroll + 1).min(count.saturating_sub(1));
            }
            _ => {}
        }
        true
    }

    pub(super) fn handle_graph_event(&mut self, event: Event) {
        let domain = event.request.domain;
        let state = self.graphs.state_mut(domain);
        state.busy = false;
        if event.request.minutes != state.minutes {
            state.attempted = None;
            if self.section == domain.section() {
                self.refresh_graph(domain, true);
            }
            return;
        }
        state.attempted = Some(Instant::now());
        let (direction, message) = match event.result {
            Ok(history) => {
                state.error = None;
                if let Some(history) = history {
                    let message = format!(
                        "history: {} series, {} minutes, {}ms buckets",
                        history.series.len(),
                        history.minutes,
                        history.bucket_ms
                    );
                    state.history = Some(history);
                    state.updated = Some(Instant::now());
                    (TrafficDirection::Rx, message)
                } else {
                    (TrafficDirection::Dry, "dry-run: history read".into())
                }
            }
            Err(error) => {
                state.error = Some(error.clone());
                (TrafficDirection::Error, error)
            }
        };
        self.traffic.push(traffic(domain, direction, message));
    }
}

fn traffic(domain: Domain, direction: TrafficDirection, payload: String) -> TrafficEntry {
    TrafficEntry {
        at: now_hms(),
        direction,
        transport: "http",
        target: format!("InfluxDB {} history", domain.database()),
        payload,
        note: None,
        sections: Section::Overview.bit() | domain.section().bit(),
    }
}

#[cfg(test)]
mod tests;
