use super::*;
use crate::{
    cli::{GraphLayout, TuiView},
    color::{Theme, ThemePreset},
    mqtt::{MqttCommand, MqttOptions},
    tui::TuiOptions,
};
use crossterm::event::KeyModifiers;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc;

use crate::strichliste::test_fixture as fixture;

fn app() -> (
    App,
    UnboundedReceiver<Command>,
    UnboundedReceiver<MqttCommand>,
) {
    let (tx, rx) = mpsc::unbounded_channel();
    let (mqtt_tx, mqtt_rx) = mpsc::unbounded_channel();
    let mut app = App::new(
        TuiOptions {
            mqtt: MqttOptions::new("tcp://unused:1883"),
            view: TuiView::Strichliste,
            connect: false,
            safe_mode: true,
            theme: Theme::preset(ThemePreset::Dark),
            theme_preset: ThemePreset::Dark,
            smoothie_url: String::new(),
            pizzamat: crate::pizzamat::Options::default(),
            heizung: crate::heizung::Options {
                amqp_url: String::new(),
                timeout: Duration::from_secs(1),
            },
            music: crate::music::Options::default(),
            soundboard: crate::soundboard::Options::default(),
            strichliste: Options::default(),
            influx: crate::influx::Options::default(),
            graph_minutes: 60,
            graph_layout: GraphLayout::Table,
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
    app.strichliste.tx = Some(tx);
    (app, rx, mqtt_rx)
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, KeyCode::Char(c));
    }
}

fn book_form(app: &mut App, focus: usize, amount: &str, comment: &str) {
    app.strichliste.focus = focus;
    press(app, KeyCode::Enter);
    type_text(app, amount);
    press(app, KeyCode::Enter);
    type_text(app, comment);
    press(app, KeyCode::Enter);
}

fn screen(app: &mut App) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
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

fn options(url: String) -> Options {
    Options {
        url: Some(url),
        user_id: Some(27),
        timeout: Duration::from_secs(2),
    }
}

fn page(entries: Vec<Transaction>) -> TransactionPage {
    TransactionPage {
        total: Some(entries.len()),
        entries,
        offset: 0,
        next_offset: None,
    }
}

#[test]
fn transactions_menu_can_be_opened_offline_and_empty_history_is_distinct() {
    let (mut app, mut rx, _) = app();
    press(&mut app, KeyCode::End);
    press(&mut app, KeyCode::Enter);
    assert!(app.strichliste.show_transactions);
    assert!(screen(&mut app).contains("Transactions not read"));
    press(&mut app, KeyCode::Enter);
    assert!(app.popup.is_none());
    press(&mut app, KeyCode::Char('r'));
    assert!(matches!(rx.try_recv().unwrap(), Command::Refresh));
    app.handle_strichliste_event(Event {
        command: Command::Refresh,
        result: Ok(Response::Account {
            balance: Ok(0.0),
            rank: Ok((1, 0.0)),
            transactions: Ok(page(vec![])),
        }),
    });
    assert!(screen(&mut app).contains("No recent transactions"));
    press(&mut app, KeyCode::Esc);
    assert!(!app.strichliste.show_transactions);
    assert_eq!(app.section, Section::Strichliste);
    press(&mut app, KeyCode::Char('t'));
    assert!(app.strichliste.show_transactions);
    press(&mut app, KeyCode::Char('q'));
    assert!(!app.strichliste.show_transactions);
    assert_eq!(app.section, Section::Strichliste);
}

#[tokio::test]
async fn transaction_history_refreshes_preserves_selection_and_opens_full_details() {
    let (mut app, mut rx, mut mqtt) = app();
    let (url, http) = fixture::server(vec![
        (200, fixture::user()),
        (200, fixture::transactions()),
        (200, fixture::users()),
    ]);
    let response = execute(options(url), &Command::Refresh, false)
        .await
        .unwrap();
    app.handle_strichliste_event(Event {
        command: Command::Refresh,
        result: Ok(response),
    });
    assert_eq!(http.join().unwrap().len(), 3);
    press(&mut app, KeyCode::Char('t'));
    let text = screen(&mut app);
    assert!(text.contains("Recent transactions"));
    assert!(text.contains("Received"));
    press(&mut app, KeyCode::End);
    assert_eq!(app.strichliste.transaction_selection.selected(), Some(3));
    assert!(screen(&mut app).contains("-1.23"));
    press(&mut app, KeyCode::Char('p'));
    assert!(screen(&mut app).contains("-1.2345"));
    press(&mut app, KeyCode::Home);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    let text = screen(&mut app);
    for expected in [
        "Transfer out",
        "to Other: test",
        "-0.01 EUR (full precision)",
    ] {
        assert!(text.contains(expected), "missing {expected}");
    }
    press(&mut app, KeyCode::Esc);
    assert!(app.strichliste.show_transactions);
    let mut transactions = app.strichliste.transactions.clone().unwrap();
    let mut newest = transactions[0].clone();
    newest.id = 105;
    transactions.insert(0, newest);
    app.handle_strichliste_event(Event {
        command: Command::Refresh,
        result: Ok(Response::Account {
            balance: Ok(12.355),
            rank: Ok((2, 10.5)),
            transactions: Ok(page(transactions)),
        }),
    });
    assert_eq!(app.strichliste.transaction_selection.selected(), Some(2)); // Still id 103.
    app.handle_strichliste_event(Event {
        command: Command::Refresh,
        result: Ok(Response::Account {
            balance: Ok(12.355),
            rank: Ok((2, 10.5)),
            transactions: Err("offline".into()),
        }),
    });
    assert_eq!(app.strichliste.transactions.as_ref().unwrap().len(), 5);
    assert!(app.strichliste.transactions_stale);
    assert!(screen(&mut app).contains("(previous)"));
    app.options.connect = true;
    app.strichliste.last_refresh = Some(Instant::now() - Duration::from_secs(11));
    app.on_tick();
    assert!(matches!(rx.try_recv().unwrap(), Command::Refresh));
    assert!(mqtt.try_recv().is_err());
}

#[test]
fn history_comments_are_visible_truncated_by_columns_and_complete_in_details() {
    let (mut app, _, _) = app();
    let comment = "coffee with friends after the weekly meeting";
    let transaction = Transaction {
        id: 1,
        created_at: "2026-11-18 18:45:22".into(),
        value_eur: -1.25,
        comment: Some(comment.into()),
    };
    app.set_transaction_page(page(vec![transaction.clone()]));
    press(&mut app, KeyCode::Char('t'));
    let text = screen(&mut app);
    assert!(text.contains("coffee"));
    assert!(text.contains("..."));
    assert!(!text.contains(comment));
    press(&mut app, KeyCode::Enter);
    assert!(screen(&mut app).contains(comment));
    let mut transaction = transaction;
    transaction.comment = Some("日本語の長いコメント with accents é and emoji 🎉\n".repeat(5));
    for width in [0, 1, 3, 20, 50, 80, 120] {
        let row = crate::tui::views::strichliste::transaction_row(&transaction, width, false);
        assert!(
            ratatui::text::Span::raw(&row).width() <= width,
            "{width}: {row}"
        );
        assert!(!row.contains('\n'));
        if width >= 50 {
            assert!(row.ends_with("..."));
        }
    }
}

#[tokio::test]
async fn history_pages_navigate_and_refresh_the_current_offset_without_duplicate_reads() {
    let (mut app, mut rx, _) = app();
    let (url, server) = fixture::server(vec![
        (200, fixture::user()),
        (200, fixture::transaction_page(0, 50, 112)),
        (200, fixture::users()),
    ]);
    let response = execute(options(url), &Command::Refresh, false)
        .await
        .unwrap();
    app.handle_strichliste_event(Event {
        command: Command::Refresh,
        result: Ok(response),
    });
    assert_eq!(app.strichliste.transactions.as_ref().unwrap().len(), 50);
    assert!(
        server.join().unwrap()[1]
            .starts_with("GET /strich-api/user/27/transaction?limit=50&offset=0 ")
    );
    press(&mut app, KeyCode::Char('t'));
    press(&mut app, KeyCode::End);
    assert!(screen(&mut app).contains("Load older"));
    press(&mut app, KeyCode::Enter);
    for offset in [50, 100] {
        let command = rx.try_recv().unwrap();
        assert!(matches!(command, Command::Transactions(actual) if actual == offset));
        press(&mut app, KeyCode::Char('n'));
        assert!(rx.try_recv().is_err());
        let (url, server) =
            fixture::server(vec![(200, fixture::transaction_page(offset, 50, 112))]);
        let response = execute(options(url), &command, false).await.unwrap();
        app.handle_strichliste_event(Event {
            command,
            result: Ok(response),
        });
        assert_eq!(app.strichliste.transaction_offset, offset);
        server.join().unwrap();
        press(&mut app, KeyCode::Char('n'));
    }
    assert!(rx.try_recv().is_err()); // End of history.
    assert_eq!(app.strichliste.transactions.as_ref().unwrap().len(), 12);
    press(&mut app, KeyCode::Char('b'));
    assert!(matches!(rx.try_recv().unwrap(), Command::Transactions(50)));
    app.handle_strichliste_event(Event {
        command: Command::Transactions(50),
        result: Err("offline".into()),
    });
    assert_eq!(app.strichliste.transaction_offset, 100); // Failed navigation keeps the current page.
    press(&mut app, KeyCode::Char('r'));
    assert!(matches!(rx.try_recv().unwrap(), Command::RefreshAt(100)));
    let (url, server) = fixture::server(vec![
        (200, fixture::user()),
        (200, fixture::transaction_page(100, 50, 112)),
        (200, fixture::users()),
    ]);
    let response = execute(options(url), &Command::RefreshAt(100), false)
        .await
        .unwrap();
    app.handle_strichliste_event(Event {
        command: Command::RefreshAt(100),
        result: Ok(response),
    });
    assert_eq!(app.strichliste.transaction_offset, 100);
    assert_eq!(app.strichliste.transaction_previous, [0, 50]);
    assert!(server.join().unwrap()[1].contains("limit=50&offset=100"));
    press(&mut app, KeyCode::Char('b'));
    let command = rx.try_recv().unwrap();
    let (url, server) = fixture::server(vec![(200, fixture::transaction_page(50, 50, 112))]);
    let response = execute(options(url), &command, false).await.unwrap();
    app.handle_strichliste_event(Event {
        command,
        result: Ok(response),
    });
    assert_eq!(app.strichliste.transaction_previous, [0]);
    assert_eq!(app.strichliste.transaction_offset, 50);
    server.join().unwrap();
}

#[test]
fn payment_forms_own_text_keys_validate_amounts_and_obey_safe_mode() {
    for (focus, pay) in [(0, true), (1, false)] {
        let (mut app, mut rx, mut mqtt) = app();
        book_form(&mut app, focus, "0.001", "qQcdr?S[]");
        assert!(!app.should_quit);
        assert!(app.safe_mode);
        assert!(mqtt.try_recv().is_err());
        assert!(matches!(
            app.popup,
            Some(Popup::ConfirmCommand {
                selected: ConfirmChoice::Cancel,
                ..
            })
        ));
        assert!(rx.try_recv().is_err());
        press(&mut app, KeyCode::Enter); // Cancel is the default.
        assert!(rx.try_recv().is_err());

        book_form(&mut app, focus, "0.001", "qQcdr?S[]");
        press(&mut app, KeyCode::Char('y'));
        let Command::Book { request, comment } = rx.try_recv().unwrap() else {
            panic!("booking expected")
        };
        assert_eq!(
            request.value().unwrap(),
            Some(if pay { -0.001 } else { 0.001 })
        );
        assert_eq!(comment, "qQcdr?S[]");
        assert!(app.strichliste.busy);
        press(&mut app, KeyCode::Enter);
        assert!(rx.try_recv().is_err());
    }
    for amount in ["0", "-1", "NaN", "inf"] {
        let (mut app, mut rx, _) = app();
        app.strichliste.focus = 0;
        press(&mut app, KeyCode::Enter);
        type_text(&mut app, amount);
        press(&mut app, KeyCode::Enter);
        assert!(app.strichliste.error.is_some());
        assert!(matches!(
            app.strichliste.form.as_ref().unwrap().step,
            Step::Amount
        ));
        assert!(rx.try_recv().is_err());
        press(&mut app, KeyCode::Esc);
        assert!(app.strichliste.form.is_none());
    }
    let (mut app, mut rx, _) = app();
    app.safe_mode = false;
    book_form(&mut app, 0, "1", "");
    assert!(matches!(
        rx.try_recv().unwrap(),
        Command::Book {
            request: Request::Pay(1.0),
            ..
        }
    ));
    assert!(app.popup.is_none());
}

#[tokio::test]
async fn recipient_search_excludes_self_then_transfers_once_with_the_supplied_contract() {
    let (mut app, mut rx, _) = app();
    app.strichliste.focus = 2;
    press(&mut app, KeyCode::Enter);
    let command = rx.try_recv().unwrap();
    assert!(matches!(command, Command::LoadRecipients));
    type_text(&mut app, "o");
    assert_eq!(app.strichliste.form.as_ref().unwrap().input.value(), "o");
    assert!(rx.try_recv().is_err());
    let (url, http) = fixture::server(vec![(200, fixture::user()), (200, fixture::users())]);
    let result = execute(options(url), &command, false).await.unwrap();
    app.handle_strichliste_event(Event {
        command,
        result: Ok(result),
    });
    let form = app.strichliste.form.as_ref().unwrap();
    assert!(form.recipients.iter().all(|user| user.id != 27));
    assert_eq!(form.recipients.len(), 2);
    assert!(screen(&mut app).contains("live search"));
    type_text(&mut app, "TH");
    assert_eq!(app.strichliste.form.as_ref().unwrap().recipients.len(), 1);
    assert!(screen(&mut app).contains("Other (id 30)"));
    type_text(&mut app, "xyz");
    assert!(app.strichliste.form.as_ref().unwrap().recipients.is_empty());
    press(&mut app, KeyCode::Enter); // No match cannot advance.
    assert!(matches!(
        app.strichliste.form.as_ref().unwrap().step,
        Step::Recipients
    ));
    assert!(screen(&mut app).contains("No matching recipients"));
    for _ in 0..5 {
        press(&mut app, KeyCode::Backspace);
    }
    assert_eq!(app.strichliste.form.as_ref().unwrap().recipients.len(), 2);
    press(&mut app, KeyCode::Char('/'));
    assert_eq!(app.strichliste.form.as_ref().unwrap().recipients.len(), 3);
    type_text(&mut app, "o");
    assert!(rx.try_recv().is_err()); // Filtering uses the loaded list, no per-key HTTP.
    // Alphabetic results: Lower, Other. Navigate to the intended account.
    press(&mut app, KeyCode::End);
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.strichliste
            .form
            .as_ref()
            .unwrap()
            .recipient
            .as_ref()
            .unwrap()
            .id,
        30
    );
    type_text(&mut app, "0.01");
    press(&mut app, KeyCode::Enter);
    type_text(&mut app, "test");
    press(&mut app, KeyCode::Enter);
    assert!(screen(&mut app).contains("Send 0.01 EUR to Other"));
    assert!(rx.try_recv().is_err());
    press(&mut app, KeyCode::Char('y'));
    let command = rx.try_recv().unwrap();
    let (url, http_write) = fixture::server(vec![(200, fixture::user()), (201, json!({}))]);
    assert!(matches!(
        execute(options(url), &command, false).await.unwrap(),
        Response::Submitted
    ));
    let requests = http_write.join().unwrap();
    let body: Value = serde_json::from_str(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(
        body,
        json!({
            "value": -0.01, "comment": "to Other: test", "toUserId": 30,
            "toUserComment": "from Me: test",
        })
    );
    http.join().unwrap();
}

#[test]
fn cancelled_search_does_not_reopen_the_form_or_send_and_global_navigation_recovers() {
    let (mut app, mut rx, _) = app();
    app.strichliste.focus = 2;
    press(&mut app, KeyCode::Enter);
    let command = rx.try_recv().unwrap();
    type_text(&mut app, "qQcdr?S[]");
    assert!(!app.should_quit);
    assert!(rx.try_recv().is_err());
    press(&mut app, KeyCode::Enter);
    assert!(rx.try_recv().is_err());
    press(&mut app, KeyCode::Esc);
    app.handle_strichliste_event(Event {
        command,
        result: Ok(Response::Recipients(vec![])),
    });
    assert!(app.strichliste.form.is_none());
    app.on_tick();
    assert!(rx.try_recv().is_err());
    press(&mut app, KeyCode::Char(']'));
    assert_eq!(app.section, Section::Overview);
    press(&mut app, KeyCode::Char('['));
    assert_eq!(app.section, Section::Strichliste);
    press(&mut app, KeyCode::Enter);
    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.should_quit);
}

#[tokio::test]
async fn failed_pay_and_add_show_a_persistent_status_dialog_without_retrying() {
    for focus in [0, 1] {
        let (mut app, mut rx, _) = app();
        book_form(&mut app, focus, "0.01", "");
        press(&mut app, KeyCode::Char('y'));
        let command = rx.try_recv().unwrap();
        let (url, http) = fixture::server(vec![
            (200, fixture::user()),
            (400, json!({"error": "PRIVATE"})),
        ]);
        let error = execute(options(url), &command, false)
            .await
            .err()
            .unwrap()
            .to_string();
        app.handle_strichliste_event(Event {
            command,
            result: Err(error),
        });
        assert!(matches!(app.popup, Some(Popup::StrichlisteError { .. })));
        let text = screen(&mut app);
        assert!(text.contains("Strichliste transaction error"));
        assert!(text.contains("HTTP 400"));
        assert!(!text.contains("PRIVATE"));
        app.options.connect = true;
        app.strichliste.last_refresh = Some(Instant::now() - Duration::from_secs(11));
        app.on_tick();
        assert!(rx.try_recv().is_err());
        assert!(matches!(app.popup, Some(Popup::StrichlisteError { .. })));
        press(&mut app, KeyCode::Enter);
        assert!(app.popup.is_none());
        assert!(rx.try_recv().is_err());
        assert_eq!(http.join().unwrap().len(), 2);
    }
}

#[test]
fn recipient_load_failure_preserves_query_and_allows_retry() {
    let (mut app, mut rx, _) = app();
    app.strichliste.focus = 2;
    press(&mut app, KeyCode::Enter);
    let command = rx.try_recv().unwrap();
    type_text(&mut app, "Other");
    app.handle_strichliste_event(Event {
        command,
        result: Err("offline".into()),
    });
    assert_eq!(
        app.strichliste.form.as_ref().unwrap().input.value(),
        "Other"
    );
    assert!(screen(&mut app).contains("Enter retries"));
    press(&mut app, KeyCode::Enter);
    assert!(matches!(rx.try_recv().unwrap(), Command::LoadRecipients));
    type_text(&mut app, "x");
    press(&mut app, KeyCode::Esc);
    app.handle_strichliste_event(Event {
        command: Command::LoadRecipients,
        result: Ok(Response::Recipients(vec![])),
    });
    assert!(app.strichliste.form.is_none());
    assert!(rx.try_recv().is_err());
}

#[test]
fn reads_respect_no_connect_refresh_after_submission_and_preserve_stale_values_on_error() {
    let (mut app, mut rx, mut mqtt) = app();
    app.maybe_refresh_strichliste();
    assert!(rx.try_recv().is_err());
    press(&mut app, KeyCode::Char('r'));
    assert!(matches!(rx.try_recv().unwrap(), Command::Refresh));
    assert!(mqtt.try_recv().is_err());
    app.handle_strichliste_event(Event {
        command: Command::Refresh,
        result: Ok(Response::Account {
            balance: Ok(12.345),
            rank: Ok((2, 10.5)),
            transactions: Ok(page(vec![])),
        }),
    });
    let text = screen(&mut app);
    for expected in [
        "Balance: 12.35 EUR",
        "Rank: 2",
        "weighted purchases: 10.5",
        "Add credit",
        "Send to another user",
    ] {
        assert!(text.contains(expected), "missing {expected}");
    }
    app.options.connect = true;
    app.maybe_refresh_strichliste();
    app.on_tick();
    assert!(rx.try_recv().is_err()); // No repeat before the refresh interval.
    app.handle_strichliste_event(Event {
        command: Command::Book {
            request: Request::Pay(0.01),
            comment: String::new(),
        },
        result: Ok(Response::Submitted),
    });
    assert!(matches!(rx.try_recv().unwrap(), Command::Refresh));
    app.handle_strichliste_event(Event {
        command: Command::Refresh,
        result: Ok(Response::Account {
            balance: Err("offline".into()),
            rank: Err("offline".into()),
            transactions: Err("offline".into()),
        }),
    });
    assert_eq!(app.strichliste.balance, Some(12.345));
    assert_eq!(app.strichliste.rank, Some((2, 10.5)));
    assert!(app.strichliste.balance_stale && app.strichliste.rank_stale);
    assert!(screen(&mut app).contains("(previous)"));
    app.handle_strichliste_event(Event {
        command: Command::Book {
            request: Request::Pay(0.01),
            comment: String::new(),
        },
        result: Err("outcome unknown".into()),
    });
    app.on_tick();
    assert!(rx.try_recv().is_err());
    assert!(screen(&mut app).contains("outcome unknown"));
    assert_eq!(Section::Strichliste.bit(), 0x10000);
}

#[tokio::test]
async fn background_worker_reads_balance_rank_and_dry_run_never_connects() {
    let (url, http) = fixture::server(vec![
        (200, fixture::user()),
        (200, fixture::transactions()),
        (200, fixture::users()),
    ]);
    let (tx, rx) = mpsc::unbounded_channel();
    let (events, mut replies) = mpsc::unbounded_channel();
    spawn_worker(options(url), false, rx, events);
    tx.send(Command::Refresh).unwrap();
    drop(tx);
    let event = tokio::time::timeout(Duration::from_secs(3), replies.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event.result,
        Ok(Response::Account {
            balance: Ok(12.345),
            rank: Ok((2, 10.5)),
            transactions: Ok(_),
        })
    ));
    assert!(replies.recv().await.is_none());
    http.join().unwrap();

    let (tx, rx) = mpsc::unbounded_channel();
    let (events, mut replies) = mpsc::unbounded_channel();
    spawn_worker(
        options("invalid://PRIVATE:SECRET@unused".into()),
        true,
        rx,
        events,
    );
    for command in [
        Command::Refresh,
        Command::LoadRecipients,
        Command::Book {
            request: Request::Pay(0.01),
            comment: "TUI".into(),
        },
        Command::Book {
            request: Request::Add(1.0),
            comment: "TUI".into(),
        },
    ] {
        tx.send(command).unwrap();
    }
    drop(tx);
    for _ in 0..4 {
        let event = tokio::time::timeout(Duration::from_secs(1), replies.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(event.result, Ok(Response::DryRun)));
        assert!(!event.command.preview_lines().join(" ").contains("SECRET"));
    }
    assert!(replies.recv().await.is_none());
}

#[test]
fn auto_refresh_runs_every_ten_seconds_only_when_the_tab_is_idle_and_visible() {
    let (mut app, mut rx, mut mqtt) = app();
    app.options.connect = true;
    app.on_tick();
    assert!(matches!(rx.try_recv().unwrap(), Command::Refresh));
    app.on_tick();
    press(&mut app, KeyCode::Char('r'));
    assert!(rx.try_recv().is_err()); // An in-flight read is never queued twice.
    app.handle_strichliste_event(Event {
        command: Command::Refresh,
        result: Ok(Response::Account {
            balance: Ok(1.0),
            rank: Ok((1, 1.0)),
            transactions: Ok(page(vec![])),
        }),
    });
    app.strichliste.last_refresh = Some(Instant::now() - Duration::from_secs(9));
    app.on_tick();
    assert!(rx.try_recv().is_err());
    app.strichliste.last_refresh = Some(Instant::now() - Duration::from_secs(11));
    app.section = Section::Overview;
    app.on_tick();
    assert!(rx.try_recv().is_err());
    app.section = Section::Strichliste;
    press(&mut app, KeyCode::Enter);
    app.on_tick();
    assert!(rx.try_recv().is_err()); // Editing takes priority over automatic reads.
    press(&mut app, KeyCode::Esc);
    app.popup = Some(Popup::Quit {
        selected: ConfirmChoice::Cancel,
    });
    app.on_tick();
    assert!(rx.try_recv().is_err());
    press(&mut app, KeyCode::Esc);
    app.on_tick();
    assert!(matches!(rx.try_recv().unwrap(), Command::Refresh));
    app.handle_strichliste_event(Event {
        command: Command::Refresh,
        result: Err("offline".into()),
    });
    app.on_tick();
    assert!(rx.try_recv().is_err()); // Errors are also throttled.
    press(&mut app, KeyCode::Char('r'));
    assert!(matches!(rx.try_recv().unwrap(), Command::Refresh)); // Explicit refresh bypasses timer.
    assert!(mqtt.try_recv().is_err()); // r did not reconnect MQTT.
}

#[test]
fn balance_is_rounded_by_default_and_precision_toggle_only_changes_presentation() {
    let (mut app, mut rx, mut mqtt) = app();
    let value = -2.4181945088546075;
    app.strichliste.balance = Some(value);
    let text = screen(&mut app);
    assert!(text.contains("Balance: -2.42 EUR"));
    assert!(text.contains("rounded to cents"));
    assert!(!text.contains("Refresh balance and rank"));
    assert!(text.contains("r refresh"));
    press(&mut app, KeyCode::Char('p'));
    let text = screen(&mut app);
    assert!(text.contains("Balance: -2.4181945088546075 EUR"));
    assert!(text.contains("full precision"));
    assert_eq!(app.strichliste.balance, Some(value));
    press(&mut app, KeyCode::Char('p'));
    assert!(screen(&mut app).contains("Balance: -2.42 EUR"));
    app.strichliste.balance = Some(-0.001);
    assert!(screen(&mut app).contains("Balance: 0.00 EUR"));
    assert!(rx.try_recv().is_err());
    assert!(mqtt.try_recv().is_err());

    press(&mut app, KeyCode::Enter); // Pay is now the first action.
    type_text(&mut app, "0.01");
    press(&mut app, KeyCode::Enter);
    type_text(&mut app, "pr");
    assert_eq!(app.strichliste.form.as_ref().unwrap().input.value(), "pr");
    assert!(!app.strichliste.full_precision);
    assert!(rx.try_recv().is_err());
    assert!(mqtt.try_recv().is_err());
}

#[cfg(feature = "tui-mouse")]
#[test]
fn mouse_payment_confirmation_uses_the_same_guarded_dispatch() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let (mut app, mut rx, _) = app();
    book_form(&mut app, 1, "1", "cash");
    app.handle_mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 25,
            row: 14,
            modifiers: KeyModifiers::NONE,
        },
        ratatui::layout::Rect::new(0, 0, 100, 40),
    );
    assert!(matches!(
        rx.try_recv().unwrap(),
        Command::Book {
            request: Request::Add(1.0),
            ..
        }
    ));
}
