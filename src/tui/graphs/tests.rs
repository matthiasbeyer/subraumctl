use super::*;
use crate::{
    cli::{Cli, CliCommand, TuiView},
    color::{Theme, ThemePreset},
    mqtt::MqttOptions,
    tui::TuiOptions,
};
use clap::Parser;
use crossterm::event::KeyModifiers;
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use tokio::sync::mpsc;

fn fixture() -> Value {
    json!({"results": [{"statement_id": 0, "series": [{
        "name": "modbus", "columns": ["total", "time", "l2", "l1", "l3"],
        "values": [[5000, 0, -1000, 2000, null], [6000, 100, -2000, 3000, null],
                   [null, 200, null, null, null], [8000, 300, -3000, 4000, null],
                   [9000, 500, -4000, 5000, null]]
    }]}]})
}

#[test]
fn history_scales_power_preserves_gaps_and_authoritative_total() {
    let history = history::decode(&fixture(), Domain::Power, 1, 60_000).unwrap();
    let total = history.series.iter().find(|s| s.name == "Total").unwrap();
    assert_eq!(
        total.segments.iter().map(Vec::len).collect::<Vec<_>>(),
        [2, 1, 1]
    );
    assert_eq!(
        total.segments[0],
        [(-1.0, 5.0), (-59_900.0 / 60_000.0, 6.0)]
    );
    assert!(
        history
            .series
            .iter()
            .find(|s| s.name == "L3")
            .unwrap()
            .segments
            .is_empty()
    );
    assert_eq!(
        history
            .series
            .iter()
            .find(|s| s.name == "L2")
            .unwrap()
            .segments[0][0]
            .1,
        -1.0
    );
}

#[test]
fn climate_matches_statements_and_columns_and_keeps_metrics_separate() {
    let response = json!({"results": [
        {"statement_id": 1, "series": [{"name": "hum", "tags": {"location": "hack"},
            "columns": ["value", "time"], "values": [[40.0, 0], [null, 100], [42.0, 200]]}]},
        {"statement_id": 0, "series": [{"name": "temp", "tags": {"location": "hack"},
            "columns": ["time", "value"], "values": [[0, 20.5], [100, 21.0]]}]}
    ]});
    let h = history::decode(&response, Domain::Climate, 1, 60_000).unwrap();
    assert_eq!(h.series.len(), 2);
    assert!(!h.series[0].humidity);
    assert_eq!(h.series[0].segments[0][0].1, 20.5);
    assert!(h.series[1].humidity);
    assert_eq!(h.series[1].segments.len(), 2);
    assert_eq!(h.series[1].segments[1][0].1, 42.0);
}

#[test]
fn history_rejects_partial_malformed_and_unbounded_data() {
    for (field, value) in [
        ("partial", json!(true)),
        ("columns", json!(["time", "total", "l1", "l1", "l3"])),
        ("values", json!([[1, 60_001, 2, 3, 4]])),
        ("values", json!([[1, 100, 2, 3, 4], [1, 0, 2, 3, 4]])),
        ("values", json!([[1, 0, 2, "NaN", 4]])),
        ("values", json!(vec![vec![0; 5]; 602])),
    ] {
        let mut response = fixture();
        response["results"][0]["series"][0][field] = value;
        assert!(
            history::decode(&response, Domain::Power, 1, 60_000).is_err(),
            "{field}"
        );
    }
    let empty = json!({"results": [{"statement_id": 0}]});
    assert!(
        history::decode(&empty, Domain::Power, 1, 60_000)
            .unwrap()
            .series
            .is_empty()
    );
    assert!(history::decode(&empty, Domain::Climate, 1, 60_000).is_err());
}

#[test]
fn queries_bound_history_and_align_buckets_to_window_start() {
    let query = history::query(Domain::Climate, 1, 1_000_051).unwrap();
    assert!(query.contains("time >= 940051ms AND time <= 1000051ms"));
    assert_eq!(
        query
            .matches("GROUP BY time(100ms, 51ms), \"location\" fill(null)")
            .count(),
        2
    );
    let query = history::query(Domain::Power, 43200, 4_000_000_000).unwrap();
    assert!(query.contains("GROUP BY time(4320000ms,"));
    assert!(query.contains("mean(\"P∑\") AS \"total\""));
    assert!(query.contains("\"name\" = 'stromzähler'"));
}

fn app(connect: bool) -> (App, UnboundedReceiver<Request>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let mut app = App::new(
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
            graph_minutes: 1,
            graph_layout: GraphLayout::Overlay,
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
    app.graphs.tx = Some(tx);
    (app, rx)
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn range_changes_discard_inflight_response_and_failure_preserves_actual_range() {
    let (mut app, mut rx) = app(false);
    app.on_tick();
    assert!(rx.try_recv().is_err());
    press(&mut app, KeyCode::Enter);
    let old = rx.try_recv().unwrap();
    press(&mut app, KeyCode::Char('+'));
    press(&mut app, KeyCode::Char('+'));
    assert!(rx.try_recv().is_err());
    app.handle_graph_event(Event {
        request: old,
        result: Ok(Some(
            history::decode(&fixture(), Domain::Power, 1, 60_000).unwrap(),
        )),
    });
    assert!(app.graphs.power.history.is_none());
    let request = rx.try_recv().unwrap();
    assert_eq!(request.minutes, 15);
    app.handle_graph_event(Event {
        request,
        result: Ok(Some(
            history::decode(&fixture(), Domain::Power, 15, 60_000).unwrap(),
        )),
    });
    press(&mut app, KeyCode::Char('+'));
    let request = rx.try_recv().unwrap();
    assert_eq!(request.minutes, 30);
    app.handle_graph_event(Event {
        request,
        result: Err("offline".into()),
    });
    assert_eq!(app.graphs.power.history.as_ref().unwrap().minutes, 15);
    assert_eq!(app.graphs.power.error.as_deref(), Some("offline"));
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal
        .draw(|f| draw(f, f.area(), &app, Domain::Power))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("STALE"));
    assert!(text.contains("requested: 30m"));
    assert!(text.contains("last 15m"));
}

#[test]
fn polling_layouts_metrics_and_small_terminals() {
    let (mut app, mut rx) = app(true);
    app.on_tick();
    let request = rx.try_recv().unwrap();
    app.on_tick();
    assert!(rx.try_recv().is_err());
    app.handle_graph_event(Event {
        request,
        result: Ok(Some(
            history::decode(&fixture(), Domain::Power, 1, 60_000).unwrap(),
        )),
    });
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|f| crate::tui::ui::draw(f, &mut app))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    for label in ["L1", "L2", "L3", "Total"] {
        assert!(text.contains(label), "{label}");
    }
    for _ in 0..2 {
        for (width, height) in [(120, 46), (80, 24), (30, 8), (1, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|f| draw(f, f.area(), &app, Domain::Power))
                .unwrap();
            if width == 120 {
                let buffer = terminal.backend().buffer();
                let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
                for label in ["L1", "L2", "L3", "Total"] {
                    assert!(text.contains(label), "{label}");
                }
                assert!(
                    buffer
                        .content
                        .iter()
                        .any(|c| c.fg == ratatui::style::Color::Cyan)
                );
            }
        }
        press(&mut app, KeyCode::Char('g'));
    }
    assert_eq!(app.graphs.power.layout, GraphLayout::Table);
    app.graphs.power.attempted = Some(Instant::now() - Duration::from_secs(31));
    app.on_tick();
    assert!(rx.try_recv().is_err());
    app.section = Section::Overview;
    app.graphs.power.layout = GraphLayout::Overlay;
    app.on_tick();
    assert!(rx.try_recv().is_err());
    app.section = Section::Power;
    app.on_tick();
    assert_eq!(rx.try_recv().unwrap().domain, Domain::Power);
    app.section = Section::Climate;
    press(&mut app, KeyCode::Char('t'));
    assert!(app.graphs.climate.humidity);
    assert!(!app.graphs.power.humidity);
}

#[tokio::test]
async fn dry_run_graph_worker_never_connects() {
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
    tx.send(Request {
        domain: Domain::Climate,
        minutes: 60,
    })
    .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(event.result.unwrap().is_none());
}

#[test]
fn graph_options_are_tui_only_and_validate_range() {
    let cli = Cli::try_parse_from([
        "subraumctl",
        "tui",
        "--graph-minutes",
        "90",
        "--graph-layout",
        "separate",
    ])
    .unwrap();
    let CliCommand::Tui(args) = cli.command else {
        panic!("expected TUI")
    };
    assert_eq!(args.graph_minutes, 90);
    assert_eq!(args.graph_layout, GraphLayout::Separate);
    for minutes in ["0", "43201", "-1"] {
        assert!(Cli::try_parse_from(["subraumctl", "tui", "--graph-minutes", minutes]).is_err());
    }
    assert!(
        Cli::try_parse_from(["subraumctl", "power", "status", "--graph-minutes", "60"]).is_err()
    );
}
