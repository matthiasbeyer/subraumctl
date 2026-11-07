//! Non-blocking climate reads, independent of the MQTT connection.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use super::{
    app::{App, Section, StatusKind},
    traffic::{TrafficDirection, TrafficEntry, now_hms},
};
use crate::{
    climate::Snapshot,
    influx::{Mode, Options},
};

#[derive(Default)]
pub struct State {
    pub mode: Mode,
    pub snapshot: Option<Snapshot>,
    pub updated: Option<Instant>,
    pub attempted: Option<Instant>,
    pub busy: bool,
    pub error: Option<String>,
    pub selected: usize,
}

pub struct Event {
    pub mode: Mode,
    pub result: Result<Option<Snapshot>, String>,
}

pub fn spawn_worker(
    options: Options,
    dry_run: bool,
    mut rx: UnboundedReceiver<Mode>,
    tx: UnboundedSender<Event>,
) {
    tokio::spawn(async move {
        while let Some(mode) = rx.recv().await {
            let result = if dry_run {
                Ok(None)
            } else {
                crate::climate::read(&options, mode)
                    .await
                    .map(Some)
                    .map_err(|e| e.to_string())
            };
            if tx.send(Event { mode, result }).is_err() {
                break;
            }
        }
    });
}

impl App {
    pub(super) fn maybe_refresh_climate(&mut self) {
        if self.graphs.climate.layout != crate::cli::GraphLayout::Table {
            if self.section == Section::Climate {
                self.refresh_graph(super::graphs::Domain::Climate, false);
            }
            return;
        }
        if self.section == Section::Climate
            && self.options.connect
            && !self.climate.busy
            && self
                .climate
                .attempted
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(30))
        {
            self.refresh_climate();
        }
    }

    fn refresh_climate(&mut self) {
        if self.climate.busy {
            return;
        }
        self.climate.attempted = Some(Instant::now());
        self.climate.error = None;
        if self.climate_tx.send(self.climate.mode).is_err() {
            self.climate.error = Some("Climate worker stopped".into());
            return;
        }
        self.climate.busy = true;
        self.traffic.push(traffic(
            if self.options.dry_run {
                TrafficDirection::Dry
            } else {
                TrafficDirection::Tx
            },
            format!("climate {}", self.climate.mode.name()),
        ));
    }

    pub(super) fn handle_climate_key(&mut self, key: KeyEvent) {
        if self.handle_graph_key(super::graphs::Domain::Climate, key) {
            return;
        }
        match key.code {
            KeyCode::Left | KeyCode::Right | KeyCode::Char('m') => {
                self.climate.mode = self.climate.mode.next();
                self.refresh_climate();
            }
            KeyCode::Enter => self.refresh_climate(),
            KeyCode::Up => self.climate.selected = self.climate.selected.saturating_sub(1),
            KeyCode::Down => {
                let len = self.climate.snapshot.as_ref().map_or(0, |s| s.rooms.len());
                self.climate.selected = (self.climate.selected + 1).min(len.saturating_sub(1));
            }
            _ => {}
        }
    }

    pub(super) fn handle_climate_event(&mut self, event: Event) {
        self.climate.busy = false;
        // Switching modes during a read schedules the new mode once it finishes.
        if event.mode != self.climate.mode {
            self.refresh_climate();
            return;
        }
        self.climate.attempted = Some(Instant::now());
        match event.result {
            Ok(snapshot) => {
                self.climate.error = None;
                if let Some(snapshot) = snapshot {
                    self.traffic
                        .push(traffic(TrafficDirection::Rx, snapshot.json().to_string()));
                    self.climate.selected = self
                        .climate
                        .selected
                        .min(snapshot.rooms.len().saturating_sub(1));
                    self.climate.snapshot = Some(snapshot);
                    self.climate.updated = Some(Instant::now());
                    self.set_status("Climate refreshed", StatusKind::Success);
                } else {
                    self.set_status("dry-run: climate read", StatusKind::Info);
                }
            }
            Err(error) => {
                self.traffic
                    .push(traffic(TrafficDirection::Error, error.clone()));
                self.climate.error = Some(error.clone());
                self.set_status(error, StatusKind::Error);
            }
        }
    }
}

fn traffic(direction: TrafficDirection, payload: String) -> TrafficEntry {
    TrafficEntry {
        at: now_hms(),
        direction,
        transport: "http",
        target: "InfluxDB climate".into(),
        payload,
        note: None,
        sections: Section::Overview.bit() | Section::Climate.bit(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cli::TuiView,
        color::{Theme, ThemePreset},
        mqtt::MqttOptions,
        tui::TuiOptions,
    };
    use crossterm::event::KeyModifiers;
    use tokio::sync::mpsc;

    fn app(connect: bool) -> (App, UnboundedReceiver<Mode>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let app = App::new(
            TuiOptions {
                mqtt: MqttOptions::new("tcp://unused:1883"),
                view: TuiView::Climate,
                connect,
                safe_mode: true,
                theme: Theme::preset(ThemePreset::Dark),
                theme_preset: ThemePreset::Dark,
                smoothie_url: String::new(),
                pizzamat: crate::pizzamat::Options::default(),
                heizung: crate::heizung::Options {
                    amqp_url: "invalid://unused".into(),
                    timeout: Duration::from_secs(1),
                },
                influx: Options::default(),
                graph_minutes: 60,
                graph_layout: crate::cli::GraphLayout::Table,
                verbose: 0,
                dry_run: false,
            },
            mpsc::unbounded_channel().0,
            mpsc::unbounded_channel().0,
            mpsc::unbounded_channel().0,
            mpsc::unbounded_channel().0,
            tx,
            mpsc::unbounded_channel().0,
        );
        (app, rx)
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn snapshot(mode: Mode) -> Snapshot {
        Snapshot {
            mode,
            queried_at_ms: 5000,
            rooms: vec![crate::climate::Reading {
                location: "hack".into(),
                temperature_celsius: Some(21.25),
                humidity_percent: Some(45.5),
                temperature_time_ms: Some(3000),
                humidity_time_ms: Some(4000),
            }],
        }
    }

    #[test]
    fn offline_mode_manual_reads_and_switching_in_flight() {
        let (mut app, mut rx) = app(false);
        app.on_tick();
        assert!(rx.try_recv().is_err());
        press(&mut app, KeyCode::Enter);
        assert_eq!(rx.try_recv().unwrap(), Mode::Average);
        press(&mut app, KeyCode::Enter);
        assert!(rx.try_recv().is_err());
        press(&mut app, KeyCode::Right);
        assert_eq!(app.climate.mode, Mode::Latest);
        assert!(rx.try_recv().is_err());
        app.handle_climate_event(Event {
            mode: Mode::Average,
            result: Ok(Some(snapshot(Mode::Average))),
        });
        assert!(app.climate.snapshot.is_none());
        assert_eq!(rx.try_recv().unwrap(), Mode::Latest);
        app.handle_climate_event(Event {
            mode: Mode::Latest,
            result: Ok(Some(snapshot(Mode::Latest))),
        });
        assert_eq!(app.climate.snapshot.as_ref().unwrap().mode, Mode::Latest);
        assert!(app.popup.is_none());
        assert!(!app.climate.busy);
    }

    #[test]
    fn auto_refresh_is_active_view_only_and_failures_preserve_last_snapshot() {
        let (mut app, mut rx) = app(true);
        assert_eq!(rx.try_recv().unwrap(), Mode::Average);
        app.on_tick();
        assert!(rx.try_recv().is_err());
        app.handle_climate_event(Event {
            mode: Mode::Average,
            result: Ok(Some(snapshot(Mode::Average))),
        });
        app.climate.attempted = Some(Instant::now() - Duration::from_secs(31));
        app.section = Section::Overview;
        app.on_tick();
        assert!(rx.try_recv().is_err());
        app.section = Section::Climate;
        app.on_tick();
        assert_eq!(rx.try_recv().unwrap(), Mode::Average);
        let updated = app.climate.updated;
        app.handle_climate_event(Event {
            mode: Mode::Average,
            result: Err("offline".into()),
        });
        assert_eq!(app.climate.updated, updated);
        assert_eq!(
            app.climate.snapshot.as_ref().unwrap().rooms[0].temperature_celsius,
            Some(21.25)
        );
        assert_eq!(app.climate.error.as_deref(), Some("offline"));
        app.on_tick();
        assert!(rx.try_recv().is_err());
        for (width, height) in [(120, 30), (60, 16), (30, 8)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| crate::tui::views::climate::draw(frame, frame.area(), &app))
                .unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.contains("STALE"));
            if width == 120 {
                assert!(text.contains("21.25"));
                assert!(text.contains("hack"));
            }
        }
    }

    #[tokio::test]
    async fn dry_run_worker_never_connects() {
        let (tx, rx) = mpsc::unbounded_channel();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        spawn_worker(
            Options {
                url: "invalid://unused".into(),
                timeout: Duration::from_millis(1),
            },
            true,
            rx,
            event_tx,
        );
        tx.send(Mode::Latest).unwrap();
        let event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.mode, Mode::Latest);
        assert!(event.result.unwrap().is_none());
    }
}
