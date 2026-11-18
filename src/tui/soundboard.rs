//! Explicit, serial soundboard requests. No polling, retries or playback replay.

use crossterm::event::{Event as InputEvent, KeyCode, KeyEvent};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tui_input::{Input, backend::crossterm::EventHandler};

use super::app::{App, Popup, Section, StatusKind};
use super::safe_mode::{ConfirmChoice, PendingCommand};
use super::traffic::{TrafficDirection, TrafficEntry, now_hms};
use crate::soundboard::{Options, Request, Response, transport};

pub struct State {
    pub tx: Option<UnboundedSender<Request>>,
    pub input: Input,
    pub editing: bool,
    /// Zero is the search field, subsequent entries are literal sample names.
    pub focus: usize,
    pub query: Option<String>,
    pub matches: Vec<String>,
    pub truncated: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub message: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            tx: None,
            input: Input::default(),
            editing: true,
            focus: 0,
            query: None,
            matches: Vec::new(),
            truncated: false,
            busy: false,
            error: None,
            message: "Enter a query, then Enter to search".into(),
        }
    }
}

pub struct Event {
    pub request: Request,
    pub result: Result<Response, String>,
}

pub fn spawn_worker(
    options: Options,
    dry_run: bool,
    mut rx: UnboundedReceiver<Request>,
    tx: UnboundedSender<Event>,
) {
    tokio::spawn(async move {
        while let Some(request) = rx.recv().await {
            let result = transport::execute(&options, &request, dry_run)
                .await
                .map_err(|error| error.to_string());
            if tx.send(Event { request, result }).is_err() {
                break;
            }
        }
    });
}

impl App {
    pub(super) fn handle_soundboard_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.soundboard.editing = false,
            KeyCode::Enter => match Request::search(self.soundboard.input.value()) {
                Ok(request) => self.send_soundboard(request),
                Err(error) => {
                    self.soundboard.error = Some(error.to_string());
                    self.set_status(error.to_string(), StatusKind::Error);
                }
            },
            _ => {
                self.soundboard.input.handle_event(&InputEvent::Key(key));
            }
        }
    }

    pub(super) fn handle_soundboard_key(&mut self, key: KeyEvent) {
        let count = self.soundboard.matches.len();
        match key.code {
            KeyCode::Char('/') => {
                self.soundboard.focus = 0;
                self.soundboard.editing = true;
            }
            KeyCode::Up => self.soundboard.focus = self.soundboard.focus.saturating_sub(1),
            KeyCode::Down => self.soundboard.focus = (self.soundboard.focus + 1).min(count),
            KeyCode::PageUp => self.soundboard.focus = self.soundboard.focus.saturating_sub(10),
            KeyCode::PageDown => self.soundboard.focus = (self.soundboard.focus + 10).min(count),
            KeyCode::Home => self.soundboard.focus = 0,
            KeyCode::End => self.soundboard.focus = count,
            KeyCode::Enter if self.soundboard.focus == 0 => self.soundboard.editing = true,
            KeyCode::Enter => {
                if self.soundboard.busy {
                    self.set_status("Soundboard is busy", StatusKind::Info);
                    return;
                }
                let Some(sample) = self.soundboard.matches.get(self.soundboard.focus - 1) else {
                    return;
                };
                match Request::play(sample) {
                    Ok(request) if self.safe_mode => {
                        self.popup = Some(Popup::ConfirmCommand {
                            pending: PendingCommand::Soundboard(request),
                            selected: ConfirmChoice::Cancel,
                        });
                    }
                    Ok(request) => self.send_soundboard(request),
                    Err(error) => self.set_status(error.to_string(), StatusKind::Error),
                }
            }
            _ => {}
        }
    }

    pub(super) fn send_soundboard(&mut self, request: Request) {
        if self.soundboard.busy {
            self.set_status("Soundboard is busy", StatusKind::Info);
            return;
        }
        let entry = soundboard_traffic(
            &request,
            if self.options.dry_run {
                TrafficDirection::Dry
            } else {
                TrafficDirection::Tx
            },
            request.args().into(),
        );
        let search = request.read_only();
        let query = search.then(|| request.args().to_owned());
        if !self
            .soundboard
            .tx
            .as_ref()
            .is_some_and(|tx| tx.send(request).is_ok())
        {
            self.soundboard.error = Some("Soundboard worker is not running".into());
            self.set_status("Soundboard worker is not running", StatusKind::Error);
            return;
        }
        self.traffic.push(entry);
        self.soundboard.busy = true;
        self.soundboard.error = None;
        self.soundboard.message = if search {
            "Searching…"
        } else {
            "Publishing sample…"
        }
        .into();
        if search {
            self.soundboard.query = query;
            self.soundboard.matches.clear();
            self.soundboard.truncated = false;
            self.soundboard.focus = 0;
            self.soundboard.editing = false;
        }
    }

    pub(super) fn handle_soundboard_event(&mut self, event: Event) {
        self.soundboard.busy = false;
        self.soundboard.error = None;
        let (direction, message) = match event.result {
            Ok(Response::Search { matches, truncated }) => {
                self.soundboard.focus =
                    usize::from(!self.soundboard.editing && !matches.is_empty());
                self.soundboard.message = if matches.is_empty() {
                    "No matches".into()
                } else {
                    format!(
                        "{} matches{}",
                        matches.len(),
                        if truncated { " (truncated)" } else { "" }
                    )
                };
                self.soundboard.matches = matches;
                self.soundboard.truncated = truncated;
                (TrafficDirection::Rx, self.soundboard.message.clone())
            }
            Ok(Response::PlayPublished) => (
                TrafficDirection::Rx,
                "Broker accepted sample; audible playback is not confirmed".into(),
            ),
            Ok(Response::DryRun { command, args }) => {
                (TrafficDirection::Dry, format!("dry-run: {command} {args}"))
            }
            Err(error) => {
                self.soundboard.error = Some(error.clone());
                (TrafficDirection::Error, error)
            }
        };
        self.traffic.push(soundboard_traffic(
            &event.request,
            direction,
            message.clone(),
        ));
        self.soundboard.message = message.clone();
        self.set_status(
            message,
            if self.soundboard.error.is_some() {
                StatusKind::Error
            } else {
                StatusKind::Info
            },
        );
    }

    #[cfg(feature = "tui-mouse")]
    pub(super) fn handle_soundboard_click(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        area: ratatui::layout::Rect,
        double_click: bool,
    ) {
        let rows = super::views::soundboard::areas(area);
        let point = ratatui::layout::Position::new(mouse.column, mouse.row);
        if rows[0].contains(point) {
            self.soundboard.focus = 0;
            self.soundboard.editing = true;
        } else if rows[3].contains(point)
            && mouse.row > rows[3].y
            && mouse.row < rows[3].bottom().saturating_sub(1)
        {
            let height = rows[3].height.saturating_sub(2) as usize;
            let start = self.soundboard.focus.saturating_sub(height.max(1));
            let index = start + mouse.row.saturating_sub(rows[3].y + 1) as usize;
            if index < self.soundboard.matches.len() {
                self.soundboard.editing = false;
                self.soundboard.focus = index + 1;
                if double_click {
                    self.handle_soundboard_key(KeyEvent::new(
                        KeyCode::Enter,
                        crossterm::event::KeyModifiers::NONE,
                    ));
                }
            }
        }
    }
}

fn soundboard_traffic(
    request: &Request,
    direction: TrafficDirection,
    payload: String,
) -> TrafficEntry {
    TrafficEntry {
        at: now_hms(),
        direction,
        transport: "amqp",
        target: format!("cmd/{}", request.command()),
        payload,
        note: Some("Soundboard EDI".into()),
        sections: Section::Overview.bit() | Section::Soundboard.bit(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cli::TuiView,
        color::{Theme, ThemePreset},
        mqtt::{MqttCommand, MqttOptions},
        tui::TuiOptions,
    };
    use crossterm::event::KeyModifiers;
    use tokio::sync::mpsc;

    fn app() -> (
        App,
        UnboundedReceiver<Request>,
        UnboundedReceiver<MqttCommand>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (mqtt_tx, mqtt_rx) = mpsc::unbounded_channel();
        let mut app = App::new(
            TuiOptions {
                mqtt: MqttOptions::new("tcp://unused:1883"),
                view: TuiView::Soundboard,
                connect: false,
                safe_mode: true,
                theme: Theme::preset(ThemePreset::Dark),
                theme_preset: ThemePreset::Dark,
                smoothie_url: String::new(),
                pizzamat: crate::pizzamat::Options::default(),
                heizung: crate::heizung::Options {
                    amqp_url: crate::heizung::DEFAULT_AMQP_URL.into(),
                    timeout: std::time::Duration::from_secs(15),
                },
                music: crate::music::Options::default(),
                soundboard: Options {
                    amqp_url: "invalid://PRIVATE:SECRET@unused".into(),
                    timeout: std::time::Duration::ZERO,
                },
                influx: crate::influx::Options::default(),
                graph_minutes: 60,
                #[cfg(feature = "strichliste")]
                strichliste: crate::strichliste::Options::default(),
                graph_layout: crate::cli::GraphLayout::Table,
                verbose: 0,
                dry_run: false,
            },
            mqtt_tx,
            mpsc::unbounded_channel().0,
            mpsc::unbounded_channel().0,
            mpsc::unbounded_channel().0,
            mpsc::unbounded_channel().0,
            mpsc::unbounded_channel().0,
        );
        app.soundboard.tx = Some(tx);
        (app, rx, mqtt_rx)
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn results(app: &mut App) {
        app.soundboard.editing = false;
        app.handle_soundboard_event(Event {
            request: Request::search("qQcdr?S[]/").unwrap(),
            result: Ok(Response::Search {
                matches: (0..25).map(|i| format!(" sample {i} ")).collect(),
                truncated: true,
            }),
        });
    }

    fn screen(app: &mut App) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| crate::tui::ui::draw(frame, app))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn input_owns_global_keys_until_escape_and_enter_alone_searches() {
        let (mut app, mut rx, mut mqtt) = app();
        for c in "qQcdr?S[]/".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        assert_eq!(app.soundboard.input.value(), "qQcdr?S[]/");
        assert!(!app.should_quit);
        assert!(app.popup.is_none());
        assert!(rx.try_recv().is_err());
        assert!(mqtt.try_recv().is_err());
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            rx.try_recv().unwrap(),
            Request::search("qQcdr?S[]/").unwrap()
        );
        assert!(!app.soundboard.editing);
        assert!(app.soundboard.busy);
        assert!(app.popup.is_none());
        assert!(screen(&mut app).contains("Busy:"));
        results(&mut app);
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('x'));
        assert_eq!(app.soundboard.query.as_deref(), Some("qQcdr?S[]/"));
        press(&mut app, KeyCode::Esc);
        assert!(!app.soundboard.editing);
        assert_eq!(app.section, Section::Soundboard);
        press(&mut app, KeyCode::Enter);
        assert!(app.soundboard.editing);
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn result_navigation_safe_mode_cancel_and_confirm_preserve_literal_sample() {
        let (mut app, mut rx, _) = app();
        app.soundboard.editing = false;
        results(&mut app);
        assert_eq!(app.soundboard.focus, 1);
        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.soundboard.focus, 11);
        press(&mut app, KeyCode::PageUp);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Enter);
        assert!(
            matches!(&app.popup, Some(Popup::ConfirmCommand { pending: PendingCommand::Soundboard(request), selected: ConfirmChoice::Cancel }) if request.args() == " sample 0 ")
        );
        assert!(rx.try_recv().is_err());
        press(&mut app, KeyCode::Enter);
        assert!(app.popup.is_none());
        assert!(rx.try_recv().is_err());
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Esc);
        assert!(rx.try_recv().is_err());
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(rx.try_recv().unwrap(), Request::play(" sample 0 ").unwrap());
        press(&mut app, KeyCode::Enter);
        assert!(rx.try_recv().is_err());
        app.handle_soundboard_event(Event {
            request: Request::play(" sample 0 ").unwrap(),
            result: Err("execution status unknown; no retry".into()),
        });
        app.on_tick();
        assert!(rx.try_recv().is_err());
        assert!(screen(&mut app).contains("Error:"));
        app.safe_mode = false;
        press(&mut app, KeyCode::Enter);
        assert_eq!(rx.try_recv().unwrap(), Request::play(" sample 0 ").unwrap());
        assert!(app.popup.is_none());
    }

    #[test]
    fn minimum_size_renders_queries_results_empty_errors_and_truncation_without_credentials() {
        let (mut app, _, _) = app();
        app.soundboard.query = Some("visible query".into());
        app.soundboard.editing = false;
        results(&mut app);
        let text = screen(&mut app);
        for expected in [
            "Soundboard",
            "Queried: visible query",
            "sample 0",
            "truncated",
        ] {
            assert!(text.contains(expected), "missing {expected}");
        }
        assert!(!text.contains("SECRET"));
        app.handle_soundboard_event(Event {
            request: Request::search("empty").unwrap(),
            result: Ok(Response::Search {
                matches: vec![],
                truncated: false,
            }),
        });
        assert!(screen(&mut app).contains("No matches"));
        assert_eq!(app.soundboard.focus, 0);
        assert!(!app.soundboard.truncated);
        assert_eq!(Section::Soundboard.bit(), 0x8000);
        assert_eq!(
            Section::ALL.len(),
            16 + cfg!(feature = "strichliste") as usize
        );
        #[cfg(not(feature = "strichliste"))]
        assert_eq!(Section::Soundboard.next(), Section::Overview);
        #[cfg(feature = "strichliste")]
        assert_eq!(Section::Soundboard.next(), Section::Strichliste);
        let entry = soundboard_traffic(
            &Request::play("literal").unwrap(),
            TrafficDirection::Tx,
            "literal".into(),
        );
        assert_eq!(entry.transport, "amqp");
        assert_eq!(entry.sections, 0x8001);
        assert!(!entry.detail().contains("SECRET"));
        assert_eq!(
            super::super::traffic::sections_for_topic(crate::action::TOPIC_SOB)
                & Section::Soundboard.bit(),
            0
        );
    }

    #[tokio::test]
    async fn worker_is_serial_dry_run_has_no_io_and_errors_are_not_retried() {
        let (tx, rx) = mpsc::unbounded_channel();
        let (events, mut replies) = mpsc::unbounded_channel();
        let options = Options {
            amqp_url: "invalid://PRIVATE:SECRET@unused".into(),
            timeout: std::time::Duration::ZERO,
        };
        spawn_worker(options.clone(), true, rx, events);
        for request in [
            Request::search("query").unwrap(),
            Request::play("sample").unwrap(),
        ] {
            tx.send(request).unwrap();
        }
        drop(tx);
        for command in ["sobfind", "sob"] {
            let event = replies.recv().await.unwrap();
            assert_eq!(event.request.command(), command);
            assert!(matches!(event.result, Ok(Response::DryRun { .. })));
        }
        assert!(replies.recv().await.is_none());
        let (tx, rx) = mpsc::unbounded_channel();
        let (events, mut replies) = mpsc::unbounded_channel();
        spawn_worker(options, false, rx, events);
        tx.send(Request::play("sample").unwrap()).unwrap();
        let event = replies.recv().await.unwrap();
        assert!(event.result.is_err());
        assert!(!event.result.unwrap_err().contains("SECRET"));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), replies.recv())
                .await
                .is_err()
        );
        drop(tx);
        assert!(replies.recv().await.is_none());
    }

    #[cfg(feature = "tui-mouse")]
    #[test]
    fn mouse_confirmation_dispatches_and_cancel_sends_nothing() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let (mut app, mut rx, _) = app();
        app.soundboard.editing = false;
        results(&mut app);
        let area = ratatui::layout::Rect::new(0, 0, 100, 40);
        for (column, sends) in [(15, false), (25, true)] {
            press(&mut app, KeyCode::Enter);
            app.handle_mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column,
                    row: 14,
                    modifiers: KeyModifiers::NONE,
                },
                area,
            );
            if sends {
                assert_eq!(rx.try_recv().unwrap(), Request::play(" sample 0 ").unwrap());
            } else {
                assert!(rx.try_recv().is_err());
            }
        }
    }
}
