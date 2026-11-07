//! Background meter reads, independent of MQTT and room-system switching.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use super::{
    app::{App, Section, StatusKind},
    traffic::{TrafficDirection, TrafficEntry, now_hms},
};
use crate::{
    influx::{Mode, Options},
    power::Snapshot,
};

#[derive(Default)]
pub struct State {
    pub mode: Mode,
    pub snapshot: Option<Snapshot>,
    pub updated: Option<Instant>,
    pub attempted: Option<Instant>,
    pub busy: bool,
    pub error: Option<String>,
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
                crate::power::read(&options, mode)
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
    pub(super) fn maybe_refresh_power(&mut self) {
        if self.graphs.power.layout != crate::cli::GraphLayout::Table {
            if self.section == Section::Power {
                self.refresh_graph(super::graphs::Domain::Power, false);
            }
            return;
        }
        if self.section == Section::Power
            && self.options.connect
            && !self.power.busy
            && self
                .power
                .attempted
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(30))
        {
            self.refresh_power();
        }
    }

    fn refresh_power(&mut self) {
        if self.power.busy {
            return;
        }
        self.power.attempted = Some(Instant::now());
        self.power.error = None;
        if self.power_tx.send(self.power.mode).is_err() {
            self.power.error = Some("Power worker stopped".into());
            return;
        }
        self.power.busy = true;
        self.traffic.push(traffic(
            if self.options.dry_run {
                TrafficDirection::Dry
            } else {
                TrafficDirection::Tx
            },
            format!("power {}", self.power.mode.name()),
        ));
    }

    pub(super) fn handle_power_key(&mut self, key: KeyEvent) {
        if self.handle_graph_key(super::graphs::Domain::Power, key) {
            return;
        }
        match key.code {
            KeyCode::Left | KeyCode::Right | KeyCode::Char('m') => {
                self.power.mode = self.power.mode.next();
                self.refresh_power();
            }
            KeyCode::Enter => self.refresh_power(),
            _ => {}
        }
    }

    pub(super) fn handle_power_event(&mut self, event: Event) {
        self.power.busy = false;
        if event.mode != self.power.mode {
            self.refresh_power();
            return;
        }
        self.power.attempted = Some(Instant::now());
        match event.result {
            Ok(snapshot) => {
                self.power.error = None;
                if let Some(snapshot) = snapshot {
                    self.traffic
                        .push(traffic(TrafficDirection::Rx, snapshot.json().to_string()));
                    self.power.snapshot = Some(snapshot);
                    self.power.updated = Some(Instant::now());
                    self.set_status("Power refreshed", StatusKind::Success);
                } else {
                    self.set_status("dry-run: power read", StatusKind::Info);
                }
            }
            Err(error) => {
                self.traffic
                    .push(traffic(TrafficDirection::Error, error.clone()));
                self.power.error = Some(error.clone());
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
        target: "InfluxDB power".into(),
        payload,
        note: None,
        sections: Section::Overview.bit() | Section::Power.bit(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cli::TuiView,
        color::{Theme, ThemePreset},
        mqtt::{MqttCommand, MqttOptions},
        tui::{TuiOptions, app::FocusZone},
    };
    use crossterm::event::KeyModifiers;
    use tokio::sync::mpsc;

    fn app(connect: bool) -> (App, UnboundedReceiver<Mode>, UnboundedReceiver<MqttCommand>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (mqtt_tx, mqtt_rx) = mpsc::unbounded_channel();
        let app = App::new(
            TuiOptions {
                mqtt: MqttOptions::new("tcp://unused:1883"),
                view: TuiView::Power,
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
            mqtt_tx,
            mpsc::unbounded_channel().0,
            mpsc::unbounded_channel().0,
            mpsc::unbounded_channel().0,
            mpsc::unbounded_channel().0,
            tx,
        );
        (app, rx, mqtt_rx)
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn snapshot(mode: Mode) -> Snapshot {
        Snapshot {
            mode,
            queried_at_ms: 5000,
            sample_time_ms: if mode == Mode::Latest {
                Some(3000)
            } else {
                None
            },
            l1_watts: Some(1234.56),
            l2_watts: Some(-1.0),
            l3_watts: None,
            total_watts: Some(3500.0),
        }
    }

    #[test]
    fn offline_reads_mode_changes_and_navigation_preserve_global_controls() {
        let (mut app, mut rx, mut mqtt_rx) = app(false);
        app.on_tick();
        assert!(rx.try_recv().is_err());
        press(&mut app, KeyCode::Enter);
        assert_eq!(rx.try_recv().unwrap(), Mode::Average);
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('m'));
        assert_eq!(app.power.mode, Mode::Latest);
        assert!(rx.try_recv().is_err());
        app.handle_power_event(Event {
            mode: Mode::Average,
            result: Ok(Some(snapshot(Mode::Average))),
        });
        assert!(app.power.snapshot.is_none());
        assert_eq!(rx.try_recv().unwrap(), Mode::Latest);
        app.handle_power_event(Event {
            mode: Mode::Latest,
            result: Ok(Some(snapshot(Mode::Latest))),
        });
        assert_eq!(app.power.snapshot.as_ref().unwrap().mode, Mode::Latest);
        assert!(app.popup.is_none());
        for (key, expected) in [('c', "connect"), ('d', "disconnect"), ('r', "reconnect")] {
            press(&mut app, KeyCode::Char(key));
            assert!(matches!(
                (expected, mqtt_rx.try_recv().unwrap()),
                ("connect", MqttCommand::Connect)
                    | ("disconnect", MqttCommand::Disconnect)
                    | ("reconnect", MqttCommand::Reconnect)
            ));
            assert!(rx.try_recv().is_err());
        }
        press(&mut app, KeyCode::Char('['));
        assert_eq!(app.section, Section::Climate);
        press(&mut app, KeyCode::Char(']'));
        assert_eq!(app.section, Section::Power);
        press(&mut app, KeyCode::Char('q'));
        app.zone = FocusZone::Sidebar;
        press(&mut app, KeyCode::End);
        assert_eq!(app.section, Section::Power);
    }

    #[test]
    fn active_view_polls_and_failed_reads_keep_labeled_snapshot() {
        let (mut app, mut rx, _) = app(true);
        assert_eq!(rx.try_recv().unwrap(), Mode::Average);
        app.on_tick();
        assert!(rx.try_recv().is_err());
        app.handle_power_event(Event {
            mode: Mode::Average,
            result: Ok(Some(snapshot(Mode::Average))),
        });
        app.power.attempted = Some(Instant::now() - Duration::from_secs(31));
        app.section = Section::Overview;
        app.on_tick();
        assert!(rx.try_recv().is_err());
        app.section = Section::Power;
        app.on_tick();
        assert_eq!(rx.try_recv().unwrap(), Mode::Average);
        app.handle_power_event(Event {
            mode: Mode::Average,
            result: Err("offline".into()),
        });
        assert_eq!(
            app.power.snapshot.as_ref().unwrap().total_watts,
            Some(3500.0)
        );
        assert_eq!(app.power.error.as_deref(), Some("offline"));
        app.on_tick();
        assert!(rx.try_recv().is_err());
        for (width, height) in [(120, 30), (80, 24), (30, 8)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| crate::tui::views::power::draw(frame, frame.area(), &app))
                .unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.contains("STALE"));
            if width >= 80 {
                for expected in [
                    "15-minute average",
                    "1234.56 W",
                    "-1.00 W",
                    "? W",
                    "3500.00 W",
                ] {
                    assert!(text.contains(expected), "{expected}");
                }
            }
        }
        // Changing mode must not relabel cached averages as latest samples.
        press(&mut app, KeyCode::Right);
        assert_eq!(rx.try_recv().unwrap(), Mode::Latest);
        app.handle_power_event(Event {
            mode: Mode::Latest,
            result: Err("offline".into()),
        });
        assert_eq!(app.power.snapshot.as_ref().unwrap().mode, Mode::Average);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| crate::tui::ui::draw(frame, &mut app))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Mode: Latest samples"));
        assert!(text.contains("Meter — 15-minute average"));
    }

    #[tokio::test]
    async fn dry_run_worker_skips_http_without_inventing_readings() {
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
