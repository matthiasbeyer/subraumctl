use super::*;
use crate::{
    cli::TuiView,
    color::{Theme, ThemePreset},
    mqtt::{MqttCommand, MqttOptions},
    tui::{TuiOptions, app::FocusZone},
};
use serde_json::json;
use std::time::Duration;
use tokio::sync::mpsc;

fn app() -> (
    App,
    UnboundedReceiver<Request>,
    UnboundedReceiver<MqttCommand>,
) {
    let (mqtt, mqtt_rx) = mpsc::unbounded_channel();
    let (http, _) = mpsc::unbounded_channel();
    let (pizza, _) = mpsc::unbounded_channel();
    let (heizung, rx) = mpsc::unbounded_channel();
    (
        App::new(
            TuiOptions {
                mqtt: MqttOptions::new("tcp://unused:1883"),
                view: TuiView::Heizung,
                connect: false,
                safe_mode: true,
                theme: Theme::preset(ThemePreset::Dark),
                theme_preset: ThemePreset::Dark,
                smoothie_url: String::new(),
                pizzamat: crate::pizzamat::Options::default(),
                influx: crate::influx::Options::default(),
                graph_minutes: 60,
                graph_layout: crate::cli::GraphLayout::Table,
                heizung: Options {
                    amqp_url: "invalid://PRIVATE".into(),
                    timeout: Duration::from_secs(1),
                },
                music: crate::music::Options::default(),
                verbose: 0,
                dry_run: false,
            },
            mqtt,
            http,
            pizza,
            heizung,
            mpsc::unbounded_channel().0,
            mpsc::unbounded_channel().0,
        ),
        rx,
        mqtt_rx,
    )
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn snapshot(target: f64) -> Value {
    json!({"rooms": Room::ALL.iter().map(|room| json!({
        "room": room.name(), "target_celsius": target, "remaining_seconds": 3600
    })).collect::<Vec<_>>()})
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
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn room_selection_exact_editing_and_safe_mode_use_existing_controls() {
    let (mut app, mut rx, _) = app();
    assert_eq!(app.section, Section::Heizung);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char(' ')); // hack
    for _ in 0..5 {
        press(&mut app, KeyCode::Down);
    }
    assert_eq!(app.heizung.focus, TARGET);
    for ch in "21.5".chars() {
        press(&mut app, KeyCode::Char(ch));
    }
    assert!(screen(&mut app, 100, 30).contains("Target temperature"));
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Down);
    for ch in "30".chars() {
        press(&mut app, KeyCode::Char(ch));
    }
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Down);
    assert_eq!(app.heizung.focus, APPLY);
    press(&mut app, KeyCode::Enter);
    assert!(rx.try_recv().is_err());
    assert!(screen(&mut app, 100, 30).contains("AMQP cmd/heizung set hack 21.5 -d 30"));
    press(&mut app, KeyCode::Enter); // Default button is Cancel, as for device commands.
    assert!(app.popup.is_none());
    assert!(rx.try_recv().is_err());
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('y'));
    let request = rx.try_recv().unwrap();
    assert_eq!(request.args(), "set hack 21.5 -d 30");
    assert!(app.heizung.busy);
    assert!(!app.connection.is_connected());
    assert!(app.heizung.readings.iter().all(Option::is_none));
    press(&mut app, KeyCode::Enter);
    assert!(rx.try_recv().is_err());
    app.handle_heizung_event(Event {
        request,
        result: Ok(Reply {
            value: json!({"acknowledged": true}),
            refresh: Some(Ok(snapshot(20.0))),
        }),
    });
    assert!(!app.heizung.busy);
    assert_eq!(app.heizung.readings[1].unwrap().target, 20.0); // Actual reply, not the edited target.
    assert_eq!(app.heizung.temperature(), 21.5);
    assert!(app.heizung.message.contains("status refreshed"));
    assert_eq!(app.traffic.filtered(Section::Heizung).len(), 4);
    assert!(app.traffic.filtered(Section::Pizzamat).is_empty());
}

#[test]
fn selection_validation_limits_and_cancelled_editing_never_send() {
    let (mut app, mut rx, _) = app();
    app.heizung.focus = APPLY;
    press(&mut app, KeyCode::Enter);
    assert!(app.popup.is_none());
    assert!(app.status.as_ref().unwrap().text.contains("select one"));
    app.heizung.focus = TARGET;
    for value in ["NaN", "26", "21.55", "-1"] {
        app.text_input = Some(TextInputState {
            mode: TextInputMode::Heizung {
                field: Field::Temperature,
                input: Input::new(value.into()),
            },
        });
        press(&mut app, KeyCode::Enter);
        assert!(app.text_input.is_some());
        assert_eq!(app.heizung.temperature(), 20.0);
        press(&mut app, KeyCode::Esc);
    }
    for value in ["-1", "1.5", "35792", "4294967296"] {
        assert!(app.heizung.set_field(Field::Duration, value).is_err());
        assert_eq!(app.heizung.duration, 180);
    }
    press(&mut app, KeyCode::Char('2'));
    press(&mut app, KeyCode::Char('4'));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.heizung.temperature(), 20.0);
    app.heizung.temperature_tenths = 250;
    press(&mut app, KeyCode::Right);
    assert_eq!(app.heizung.temperature_tenths, 250);
    app.heizung.temperature_tenths = 150;
    press(&mut app, KeyCode::Left);
    assert_eq!(app.heizung.temperature_tenths, 150);
    app.heizung.focus = DURATION;
    app.heizung.duration = 0;
    press(&mut app, KeyCode::Left);
    assert_eq!(app.heizung.duration, 0);
    app.heizung.duration = MAX_DURATION;
    press(&mut app, KeyCode::Right);
    assert_eq!(app.heizung.duration, MAX_DURATION);
    assert!(rx.try_recv().is_err());
    app.safe_mode = false;
    app.heizung.focus = APPLY;
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('n'));
    assert!(app.heizung.selected_rooms().is_empty());
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Enter);
    assert!(app.popup.is_none());
    assert_eq!(
        rx.try_recv().unwrap().args(),
        "set make hack cook elab shit storage 15.0 -d 35791"
    );
}

#[test]
fn shared_confirmation_still_dispatches_existing_device_commands() {
    let (mut app, mut heating, mut mqtt) = app();
    app.section = Section::Lighting;
    app.connection = crate::tui::app::ConnectionState::Connected {
        since: Instant::now(),
    };
    press(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.popup,
        Some(Popup::ConfirmCommand {
            pending: PendingCommand::Device { .. },
            ..
        })
    ));
    assert!(mqtt.try_recv().is_err());
    press(&mut app, KeyCode::Enter); // Cancel is selected by default.
    assert!(app.popup.is_none());
    assert!(mqtt.try_recv().is_err());
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('y'));
    match mqtt.try_recv().unwrap() {
        MqttCommand::Publish { topic, .. } => assert_eq!(topic, crate::action::TOPIC_DMX),
        other => panic!("unexpected device command: {other:?}"),
    }
    assert!(heating.try_recv().is_err());
}

#[test]
fn reads_are_independent_of_mqtt_and_preserve_last_snapshot_on_failure() {
    let (mut app, mut rx, _) = app();
    app.heizung.focus = REFRESH;
    press(&mut app, KeyCode::Enter);
    assert!(app.popup.is_none());
    let request = rx.try_recv().unwrap();
    assert_eq!(request.args(), "get make hack cook elab shit storage");
    app.handle_heizung_event(Event {
        request: request.clone(),
        result: Ok(Reply {
            value: snapshot(21.5),
            refresh: None,
        }),
    });
    let updated = app.heizung.updated;
    for result in [
        Err("heizung reply timed out".into()),
        Ok(Reply {
            value: json!({"rooms": []}),
            refresh: None,
        }),
    ] {
        press(&mut app, KeyCode::Enter);
        rx.try_recv().unwrap();
        app.handle_heizung_event(Event {
            request: request.clone(),
            result,
        });
        assert!(app.heizung.error);
        assert_eq!(app.heizung.updated, updated);
        assert_eq!(app.heizung.readings[0].unwrap().target, 21.5);
        assert!(rx.try_recv().is_err());
    }
    let write = Request::set(vec![Room::Hack], 23.0, 180).unwrap();
    app.handle_heizung_event(Event {
        request: write,
        result: Ok(Reply {
            value: json!({"acknowledged": true}),
            refresh: Some(Err("read timed out".into())),
        }),
    });
    assert!(
        app.heizung
            .message
            .contains("Setting acknowledged; status refresh failed")
    );
    assert_eq!(app.heizung.updated, updated);
    assert_eq!(app.heizung.readings[1].unwrap().target, 21.5);
}

#[test]
fn initial_read_respects_no_connect_and_does_not_retry_in_background() {
    let (mut app, mut rx, _) = app();
    app.maybe_refresh_heizung();
    assert!(rx.try_recv().is_err());
    app.options.connect = true;
    app.maybe_refresh_heizung();
    let request = rx.try_recv().unwrap();
    app.maybe_refresh_heizung();
    assert!(rx.try_recv().is_err());
    app.handle_heizung_event(Event {
        request,
        result: Err("offline".into()),
    });
    app.maybe_refresh_heizung();
    app.on_tick();
    assert!(rx.try_recv().is_err());
}

#[test]
fn global_shortcuts_sidebar_and_status_rendering_remain_consistent() {
    let (mut app, mut rx, mut mqtt) = app();
    assert!(screen(&mut app, 80, 24).contains("Refresh status"));
    assert!(screen(&mut app, 80, 24).contains("? °C"));
    app.heizung.update_readings(&snapshot(21.5)).unwrap();
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("21.5 °C"));
    assert!(text.contains("01:00:00"));
    assert!(text.contains("targets, not measured"));
    assert!(text.contains("Pizzamat"));
    for (key, expected) in [('c', "connect"), ('d', "disconnect"), ('r', "reconnect")] {
        press(&mut app, KeyCode::Char(key));
        let command = mqtt.try_recv().unwrap();
        assert!(matches!(
            (expected, command),
            ("connect", MqttCommand::Connect)
                | ("disconnect", MqttCommand::Disconnect)
                | ("reconnect", MqttCommand::Reconnect)
        ));
        assert!(rx.try_recv().is_err());
    }
    press(&mut app, KeyCode::Char('?'));
    assert!(matches!(app.popup, Some(Popup::Help { .. })));
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('['));
    assert_eq!(app.section, Section::Pizzamat);
    press(&mut app, KeyCode::Char(']'));
    assert_eq!(app.section, Section::Heizung);
    press(&mut app, KeyCode::Char('q'));
    assert_eq!(app.section, Section::Overview);
    app.zone = FocusZone::Sidebar;
    press(&mut app, KeyCode::End);
    assert_eq!(app.section, Section::Music);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.section, Section::Power);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.section, Section::Climate);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.section, Section::Heizung);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.zone, FocusZone::Content);
    app.show_log = true;
    app.heizung.focus = REFRESH;
    assert!(screen(&mut app, 80, 24).contains("Refresh status"));
}

#[tokio::test]
async fn writes_refresh_once_and_failures_never_replay_settings() {
    let request = Request::set(vec![Room::Hack], 21.5, 120).unwrap();
    let mut sent = Vec::new();
    let reply = execute_request(&request, false, |request| {
        sent.push(request.args());
        std::future::ready(if request.read_only() {
            Err(eyre!("read failed"))
        } else {
            Ok(json!({"acknowledged": true}))
        })
    })
    .await
    .unwrap();
    assert_eq!(
        sent,
        [
            "set hack 21.5 -d 120",
            "get make hack cook elab shit storage"
        ]
    );
    assert!(reply.refresh.unwrap().is_err());
    sent.clear();
    assert!(
        execute_request(&request, false, |request| {
            sent.push(request.args());
            std::future::ready(Err(eyre!("reply lost")))
        })
        .await
        .is_err()
    );
    assert_eq!(sent, ["set hack 21.5 -d 120"]);
}

#[tokio::test]
async fn dry_run_worker_previews_without_connecting_or_faking_status() {
    let (mut app, _, _) = app();
    app.options.dry_run = true;
    let (tx, rx) = mpsc::unbounded_channel();
    let (event_tx, mut events) = mpsc::unbounded_channel();
    spawn_worker(app.options.heizung.clone(), true, rx, event_tx);
    for request in [
        Request::get(vec![]).unwrap(),
        Request::set(vec![Room::Hack], 21.0, 180).unwrap(),
    ] {
        tx.send(request.clone()).unwrap();
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(event.result.as_ref().unwrap().refresh.is_none());
        app.handle_heizung_event(event);
        assert!(!app.heizung.error);
        assert!(app.heizung.message.contains(&request.args()));
        assert!(app.heizung.updated.is_none());
        assert!(app.heizung.readings.iter().all(Option::is_none));
    }
    drop(tx);
    assert!(
        tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .is_none()
    );
}
