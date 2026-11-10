use super::*;
use crate::{
    cli::TuiView,
    color::{Theme, ThemePreset},
    mqtt::MqttOptions,
    tui::TuiOptions,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;

fn app(connect: bool) -> (App, UnboundedReceiver<Request>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let mut app = App::new(
        TuiOptions {
            mqtt: MqttOptions::new("tcp://unused:1883"),
            view: TuiView::Music,
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
            music: crate::music::Options {
                address: "127.0.0.1:1".into(),
                ..crate::music::Options::default()
            },
            influx: crate::influx::Options::default(),
            graph_minutes: 60,
            graph_layout: crate::cli::GraphLayout::Table,
            verbose: 0,
            dry_run: false,
        },
        mpsc::unbounded_channel().0,
        mpsc::unbounded_channel().0,
        mpsc::unbounded_channel().0,
        mpsc::unbounded_channel().0,
        mpsc::unbounded_channel().0,
        mpsc::unbounded_channel().0,
    );
    app.music_tx = Some(tx);
    (app, rx)
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn screen(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
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
fn controls_and_stations_send_explicit_requests() {
    let (mut app, mut rx) = app(false);
    app.section = Section::Music;

    press(&mut app, KeyCode::Enter);
    assert_eq!(rx.try_recv().unwrap(), Request::Play);

    app.music.busy = false;
    press(&mut app, KeyCode::End);
    assert_eq!(app.music.focus, STATIONS_START + STATIONS.len() - 1);
    press(&mut app, KeyCode::Enter);
    let Request::Station(station) = rx.try_recv().unwrap() else {
        panic!("expected station request");
    };
    assert_eq!(station.id(), STATIONS.last().unwrap().id);

    app.music.busy = false;
    app.music.status = Some(PlaybackStatus {
        state: "play".into(),
        ..PlaybackStatus::default()
    });
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(rx.try_recv().unwrap(), Request::Pause);

    app.music.busy = false;
    app.music.status = Some(PlaybackStatus {
        state: "pause".into(),
        ..PlaybackStatus::default()
    });
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(rx.try_recv().unwrap(), Request::Play);

    app.music.busy = false;
    press(&mut app, KeyCode::Char('n'));
    assert_eq!(rx.try_recv().unwrap(), Request::Next);
    app.music.busy = false;
    press(&mut app, KeyCode::Char('p'));
    assert_eq!(rx.try_recv().unwrap(), Request::Previous);
}

#[test]
fn status_and_station_catalogue_are_rendered() {
    let (mut app, _rx) = app(false);
    app.section = Section::Music;
    app.music.status = Some(PlaybackStatus {
        state: "play".into(),
        artist: Some("Artist".into()),
        title: Some("Current song".into()),
        elapsed: Some(65.0),
        duration: Some(125.0),
        volume: Some(50),
        ..PlaybackStatus::default()
    });
    app.music.updated = Some(std::time::Instant::now());
    let text = screen(&mut app, 120, 40);
    assert!(text.contains("Music"));
    assert!(text.contains("Playing"));
    assert!(text.contains("Artist - Current song"));
    assert!(text.contains("Groove Salad"));
    assert!(text.contains("1:05 / 2:05"));
}

#[test]
fn completed_status_updates_state_and_errors_are_visible() {
    let (mut app, _rx) = app(false);
    app.section = Section::Music;
    app.music.busy = true;
    app.handle_music_event(Event {
        request: Request::Status,
        result: Ok(Outcome::Status(PlaybackStatus {
            state: "pause".into(),
            name: Some("SomaFM".into()),
            ..PlaybackStatus::default()
        })),
    });
    assert!(!app.music.busy);
    assert_eq!(app.music.status.as_ref().unwrap().display_title(), "SomaFM");
    assert!(app.music.message.starts_with("Paused"));

    app.music.busy = true;
    app.handle_music_event(Event {
        request: Request::Play,
        result: Err("MPD connection failed".into()),
    });
    assert_eq!(app.music.error.as_deref(), Some("MPD connection failed"));
}
