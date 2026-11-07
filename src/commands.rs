//! CLI command implementations.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use clap::CommandFactory;
use eyre::{Result, WrapErr, eyre};

use crate::action::{
    Action, ControlCommand, SmoothieRelay, TOPIC_ALERT_CONDITION, TOPIC_KNOB_GET_PREFIX,
    TOPIC_SMOOTHIE_PREFIX, TOPIC_WHITE_LIGHT_STATUS_PREFIX,
};
use crate::cli::{
    AlertCommand, Cli, CliCommand, CompletionsArgs, GlobalArgs, KnobCommand, ManArgs, MonitorArgs,
    MonitorKind, PublishArgs, SmoothieCommand, SubscribeArgs, SystemCommand, TuiArgs,
    WhiteLightCommand,
};
use crate::color::{Theme, ThemeRole, parse_color};
use crate::decode::{parse_alert, parse_bool};
use crate::knob::{KNOB_SPECS, Knob, format_value};
use crate::mqtt::{self, MqttOptions};
use crate::output;
use crate::smoothiesaur::{
    FLOW_TOPIC, LASER_TOPIC, RELAY_TOPIC_PREFIX, SAFETY_TOPIC, SmoothieStatus, TEMP_TOPICS,
};
use crate::tui::{self, TuiOptions};
use crate::white_light;

pub async fn run(cli: Cli) -> Result<()> {
    let is_tui = matches!(&cli.command, &CliCommand::Tui(_));
    if !is_tui {
        crate::logging::init_cli(cli.global.verbose, cli.global.quiet);
    }

    let global = cli.global.clone();
    match cli.command {
        #[cfg(feature = "mcp")]
        CliCommand::Mcp(args) => crate::mcp::run(args, &global).await,
        CliCommand::Dmx(args) => {
            let command = ControlCommand::Dmx {
                group: args.group,
                animation: args.animation,
            };
            execute_command(&global, &command).await
        }
        CliCommand::WhiteLight(args) => match args.command {
            WhiteLightCommand::Apply(apply) => {
                let cells = if apply.all {
                    white_light::ids().map(str::to_string).collect()
                } else {
                    apply.cells
                };
                let command = ControlCommand::WhiteLight {
                    warm: apply.warm,
                    cold: apply.cold,
                    cells,
                };
                execute_command(&global, &command).await
            }
            WhiteLightCommand::Cells => print_cells(global.json, global.quiet),
            WhiteLightCommand::Status(status) => white_light_status(&global, status.watch).await,
        },
        CliCommand::Outlet(args) => {
            let command = ControlCommand::Outlet {
                outlet: args.outlet,
                on: args.state.is_on(),
            };
            execute_command(&global, &command).await
        }
        CliCommand::Alert(args) => match args.command {
            AlertCommand::Radiation { target } => {
                execute_command(&global, &ControlCommand::Radiation { target }).await
            }
            AlertCommand::Sob { sound } => {
                execute_command(&global, &ControlCommand::Sob { sound }).await
            }
            AlertCommand::Led { watch } => alert_led(&global, watch).await,
        },
        CliCommand::System(args) => match args.command {
            SystemCommand::Telinit { state } => {
                execute_command(&global, &ControlCommand::Telinit { on: state.is_on() }).await
            }
        },
        CliCommand::Knob(args) => match args.command {
            KnobCommand::List => print_knobs(global.json, global.quiet),
            KnobCommand::Get { knob } => knob_get(&global, knob),
            KnobCommand::Set { knob, value } => {
                execute_command(&global, &ControlCommand::KnobSet { knob, value }).await
            }
            KnobCommand::Watch { knobs } => knob_watch(&global, knobs).await,
        },
        CliCommand::Smoothiesaur(args) => match args.command {
            SmoothieCommand::Relay { relay, state } => {
                execute_command(
                    &global,
                    &ControlCommand::SmoothieRelay {
                        relay,
                        on: state.is_on(),
                    },
                )
                .await
            }
            SmoothieCommand::Laser { power } => {
                execute_command(&global, &ControlCommand::SmoothieLaser { power }).await
            }
            SmoothieCommand::Zero => {
                execute_command(&global, &ControlCommand::SmoothieLaserZero).await
            }
            SmoothieCommand::Status { watch } => smoothie_status(&global, watch).await,
        },
        CliCommand::Status => status(&global),
        CliCommand::Heizung(args) => heizung(&global, args.command).await,
        CliCommand::Climate(args) => climate(&global, args.command).await,
        CliCommand::Power(args) => power(&global, args.command).await,
        CliCommand::Publish(args) => publish(&global, args).await,
        CliCommand::Subscribe(args) => subscribe_command(&global, args).await,
        CliCommand::Monitor(args) => monitor(&global, args).await,
        CliCommand::Tui(args) => run_tui(global.clone(), args).await,
        CliCommand::Man(args) => render_man(args),
        CliCommand::Completions(args) => render_completions(args),
    }
}

async fn climate(global: &GlobalArgs, command: crate::cli::ClimateCommand) -> Result<()> {
    let crate::cli::ClimateCommand::Status {
        mode,
        watch,
        interval,
    } = command;
    if global.dry_run {
        let preview = crate::climate::preview(mode, crate::influx::now_ms()?);
        if global.json {
            println!("{preview}");
        } else if !global.quiet {
            println!(
                "dry-run: InfluxDB climate {}",
                preview["query"].as_str().unwrap()
            );
        }
        return Ok(());
    }
    let options = crate::influx::Options::from_global(global);
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    loop {
        let snapshot = tokio::select! {
            result = crate::climate::read(&options, mode) => result?,
            result = &mut shutdown => { result?; return Ok(()); }
        };
        if global.json {
            println!("{}", snapshot.json());
        } else if !global.quiet {
            println!("Climate — {}", mode.label());
            if snapshot.rooms.is_empty() {
                println!("No climate data");
            }
            for room in &snapshot.rooms {
                let age = |time: Option<u64>| {
                    time.map(|t| {
                        format!(
                            " ({}s old)",
                            snapshot.queried_at_ms.saturating_sub(t) / 1000
                        )
                    })
                    .unwrap_or_default()
                };
                println!(
                    "{}: {}{}, {}{}",
                    room.location,
                    temp_text(room.temperature_celsius),
                    age(room.temperature_time_ms),
                    opt_percent(room.humidity_percent),
                    age(room.humidity_time_ms)
                );
            }
        }
        if !watch {
            return Ok(());
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(interval)) => {},
            result = &mut shutdown => { result?; return Ok(()); }
        }
    }
}

async fn power(global: &GlobalArgs, command: crate::cli::PowerCommand) -> Result<()> {
    let crate::cli::PowerCommand::Status {
        mode,
        watch,
        interval,
    } = command;
    if global.dry_run {
        let preview = crate::power::preview(mode, crate::influx::now_ms()?);
        if global.json {
            println!("{preview}");
        } else if !global.quiet {
            println!(
                "dry-run: InfluxDB power {}",
                preview["query"].as_str().unwrap()
            );
        }
        return Ok(());
    }
    let options = crate::influx::Options::from_global(global);
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    loop {
        let snapshot = tokio::select! {
            result = crate::power::read(&options, mode) => result?,
            result = &mut shutdown => { result?; return Ok(()); }
        };
        if global.json {
            println!("{}", snapshot.json());
        } else if !global.quiet {
            use crate::power::format_watts;
            let age = snapshot.sample_time_ms.map_or(String::new(), |t| {
                format!(
                    " ({}s old)",
                    snapshot.queried_at_ms.saturating_sub(t) / 1000
                )
            });
            println!("Power — {}{age}", mode.label());
            println!(
                "L1: {}, L2: {}, L3: {}, Total: {}",
                format_watts(snapshot.l1_watts),
                format_watts(snapshot.l2_watts),
                format_watts(snapshot.l3_watts),
                format_watts(snapshot.total_watts)
            );
        }
        if !watch {
            return Ok(());
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(interval)) => {},
            result = &mut shutdown => { result?; return Ok(()); }
        }
    }
}

async fn heizung(global: &GlobalArgs, command: crate::cli::HeizungCommand) -> Result<()> {
    use crate::cli::HeizungCommand;
    use crate::heizung::{Options, Request, Room, transport};

    let (request, watch, interval) = match command {
        HeizungCommand::Rooms => {
            for room in Room::ALL {
                if global.json {
                    println!("{}", serde_json::json!({"room": room.name()}));
                } else if !global.quiet {
                    println!("{}", room.name());
                }
            }
            return Ok(());
        }
        HeizungCommand::Status {
            rooms,
            watch,
            interval,
        } => (Request::get(rooms)?, watch, interval),
        HeizungCommand::Set {
            rooms,
            temperature,
            all,
            duration,
        } => (
            Request::set(
                if all { Room::ALL.to_vec() } else { rooms },
                temperature,
                duration,
            )?,
            false,
            0,
        ),
    };
    let options = Options::from_global(global);
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);
    loop {
        let operation = transport::execute(&options, &request, global.dry_run);
        tokio::pin!(operation);
        let result = if watch {
            tokio::select! {
                result = &mut operation => result?,
                signal = &mut shutdown => {
                    signal?;
                    // Finish the bounded exchange so its private queue is closed.
                    let _ = operation.await;
                    return Ok(());
                }
            }
        } else {
            operation.await?
        };
        if global.json {
            println!("{result}");
        } else if !global.quiet {
            if global.dry_run {
                println!("dry-run: AMQP cmd/heizung {}", request.args());
            } else if result["acknowledged"] == true {
                println!("heizung: acknowledged {}", request.args());
            } else if let Some(rooms) = result["rooms"].as_array() {
                for room in rooms {
                    let seconds = room["remaining_seconds"].as_u64().unwrap_or_default();
                    println!(
                        "{}: target {:.1} °C, {:02}:{:02}:{:02} remaining",
                        room["room"].as_str().unwrap_or("?"),
                        room["target_celsius"].as_f64().unwrap_or_default(),
                        seconds / 3600,
                        seconds / 60 % 60,
                        seconds % 60
                    );
                }
            }
        }
        if !watch || global.dry_run {
            return Ok(());
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(interval)) => {},
            result = &mut shutdown => { result?; return Ok(()); }
        }
    }
}

fn options_from_global(global: &GlobalArgs) -> MqttOptions {
    let mut options = MqttOptions::new(global.broker.clone());
    if let Some(client_id) = &global.client_id {
        options.client_id = client_id.clone();
    }
    options.username = global.username.clone();
    options.password = global.password.clone();
    options.keep_alive = Duration::from_secs(global.keep_alive.max(1));
    options.connect_timeout = Duration::from_secs(global.connect_timeout.max(1));
    options.clean_session = !global.no_clean_session;
    options
}

fn wait_deadline(seconds: u64) -> Option<Instant> {
    if seconds == 0 {
        None
    } else {
        Some(Instant::now() + Duration::from_secs(seconds))
    }
}

async fn execute_command(global: &GlobalArgs, command: &ControlCommand) -> Result<()> {
    let actions = command.expand(&global.smoothiesaur_url)?;

    if global.dry_run {
        for action in &actions {
            output::print_action(action, global.json, global.quiet)?;
        }
        return Ok(());
    }

    let mut mqtt_actions = Vec::new();
    let mut http_actions = Vec::new();
    for action in actions {
        match action {
            Action::Publish {
                topic,
                payload,
                retain,
            } => mqtt_actions.push((topic, payload, retain)),
            Action::Http { url, body } => http_actions.push((url, body)),
        }
    }

    if !mqtt_actions.is_empty() {
        let options = options_from_global(global);
        let json = global.json;
        let quiet = global.quiet;
        tokio::task::spawn_blocking(move || -> Result<()> {
            let client = mqtt::connect_once(&options)?;
            let mut result: Result<()> = Ok(());
            for (topic, payload, retain) in mqtt_actions {
                match mqtt::publish(&client, &topic, &payload, retain) {
                    Ok(()) => {
                        if let Err(err) =
                            output::print_published(&topic, &payload, retain, json, quiet)
                        {
                            result = Err(err);
                            break;
                        }
                    }
                    Err(err) => {
                        result = Err(err.into());
                        break;
                    }
                }
            }
            mqtt::disconnect(&client);
            result
        })
        .await??;
    }

    if !http_actions.is_empty() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()?;
        for (url, body) in http_actions {
            let response = client
                .post(&url)
                .header("Content-Type", "text/plain; charset=utf-8")
                .body(body.clone())
                .send()
                .await?;
            if !response.status().is_success() {
                return Err(eyre!(
                    "HTTP request to {url} failed with status {}",
                    response.status()
                ));
            }
            output::print_http_sent(&url, &body, global.json, global.quiet)?;
        }
    }

    Ok(())
}

fn stream_messages<F>(
    options: &MqttOptions,
    topics: &[String],
    deadline: Option<Instant>,
    max_count: Option<usize>,
    mut on_message: F,
) -> Result<()>
where
    F: FnMut(&str, &str) -> Result<()>,
{
    let client = mqtt::connect_once(options)?;
    for topic in topics {
        mqtt::subscribe(&client, topic)?;
    }
    let receiver = client.start_consuming();
    let mut seen = 0usize;

    loop {
        if let Some(max) = max_count {
            if seen >= max {
                break;
            }
        }
        let timeout = match deadline {
            Some(deadline) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                remaining
            }
            None => Duration::from_millis(250),
        };

        match receiver.recv_timeout(timeout) {
            Ok(Some(message)) => {
                let topic = message.topic().to_string();
                let payload = String::from_utf8_lossy(message.payload()).into_owned();
                on_message(&topic, &payload)?;
                seen += 1;
            }
            Ok(None) => break,
            Err(_) => {
                if deadline.is_some() || !client.is_connected() {
                    break;
                }
            }
        }
    }

    mqtt::disconnect(&client);
    Ok(())
}

fn collect_messages(
    options: &MqttOptions,
    topics: &[String],
    deadline: Option<Instant>,
    max_count: Option<usize>,
) -> Result<Vec<(String, String)>> {
    let mut messages = Vec::new();
    stream_messages(options, topics, deadline, max_count, |topic, payload| {
        messages.push((topic.to_string(), payload.to_string()));
        Ok(())
    })?;
    Ok(messages)
}

async fn subscribe_command(global: &GlobalArgs, args: SubscribeArgs) -> Result<()> {
    let deadline = wait_deadline(global.wait);
    let max_count = if args.count == 0 {
        None
    } else {
        Some(args.count)
    };

    if global.dry_run {
        return output::print_info(
            &format!("dry-run: subscribe {}", args.topic),
            global.json,
            global.quiet,
        );
    }

    let options = options_from_global(global);
    let topic = args.topic;
    let json = global.json;
    let quiet = global.quiet;
    tokio::task::spawn_blocking(move || -> Result<()> {
        stream_messages(&options, &[topic], deadline, max_count, |topic, payload| {
            output::print_message(topic, payload, json, quiet)
        })
    })
    .await??;
    Ok(())
}

fn print_cells(json: bool, quiet: bool) -> Result<()> {
    if json {
        for cell in white_light::CELLS {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "cell": cell.id,
                    "group": cell.group.label(),
                }))?
            );
        }
    } else if !quiet {
        println!("{:<28} GROUP", "ID");
        for cell in white_light::CELLS {
            println!("{:<28} {}", cell.id, cell.group.label());
        }
    }
    Ok(())
}

fn print_knobs(json: bool, quiet: bool) -> Result<()> {
    if json {
        for spec in KNOB_SPECS {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "name": spec.name,
                    "min": spec.min,
                    "max": spec.max,
                    "open_min": spec.open_min,
                    "open_max": spec.open_max,
                }))?
            );
        }
    } else if !quiet {
        println!("{:<12} {:>8} {:>8}  OPEN", "NAME", "MIN", "MAX");
        for spec in KNOB_SPECS {
            let open = match (spec.open_min, spec.open_max) {
                (true, true) => "both",
                (true, false) => "min",
                (false, true) => "max",
                (false, false) => "",
            };
            println!(
                "{:<12} {:>8} {:>8}  {}",
                spec.name,
                format_value(spec.min),
                format_value(spec.max),
                open
            );
        }
    }
    Ok(())
}

fn knob_get(global: &GlobalArgs, knob: Knob) -> Result<()> {
    let topic = format!("{TOPIC_KNOB_GET_PREFIX}{}", knob.name());
    if global.dry_run {
        return output::print_info(
            &format!("dry-run: subscribe {topic}"),
            global.json,
            global.quiet,
        );
    }
    let options = options_from_global(global);
    let messages = collect_messages(&options, &[topic], wait_deadline(global.wait), Some(1))?;
    let Some((_, payload)) = messages.first() else {
        return Err(eyre!("no value received for knob {}", knob.name()));
    };
    let value = payload.trim().parse::<f64>().unwrap_or(f64::NAN);
    output::print_decoded(
        &format!("{TOPIC_KNOB_GET_PREFIX}{}", knob.name()),
        &format!("knob {} = {}", knob.name(), format_value(value)),
        global.json,
        global.quiet,
    )
}

async fn knob_watch(global: &GlobalArgs, knobs: Vec<Knob>) -> Result<()> {
    let topics: Vec<String> = if knobs.is_empty() {
        vec![format!("{TOPIC_KNOB_GET_PREFIX}#")]
    } else {
        knobs
            .iter()
            .map(|knob| format!("{TOPIC_KNOB_GET_PREFIX}{}", knob.name()))
            .collect()
    };
    if global.dry_run {
        return output::print_info(
            &format!("dry-run: subscribe {}", topics.join(", ")),
            global.json,
            global.quiet,
        );
    }
    let options = options_from_global(global);
    let json = global.json;
    let quiet = global.quiet;
    tokio::task::spawn_blocking(move || -> Result<()> {
        stream_messages(&options, &topics, None, None, |topic, payload| {
            let name = topic.strip_prefix(TOPIC_KNOB_GET_PREFIX).unwrap_or(topic);
            let value = payload.trim().parse::<f64>().unwrap_or(f64::NAN);
            output::print_decoded(
                topic,
                &format!("knob {name} = {}", format_value(value)),
                json,
                quiet,
            )
        })
    })
    .await??;
    Ok(())
}

async fn alert_led(global: &GlobalArgs, watch: bool) -> Result<()> {
    let topic = TOPIC_ALERT_CONDITION.to_string();
    if global.dry_run {
        return output::print_info(
            &format!("dry-run: subscribe {topic}"),
            global.json,
            global.quiet,
        );
    }
    let options = options_from_global(global);
    let json = global.json;
    let quiet = global.quiet;

    if watch {
        tokio::task::spawn_blocking(move || -> Result<()> {
            stream_messages(&options, &[topic], None, None, |topic, payload| {
                let state = parse_alert(payload);
                output::print_decoded(
                    topic,
                    &format!("alert {}", state.condition.label()),
                    json,
                    quiet,
                )
            })
        })
        .await??;
    } else {
        let messages = collect_messages(&options, &[topic], wait_deadline(global.wait), Some(1))?;
        if let Some((topic, payload)) = messages.first() {
            let state = parse_alert(payload);
            output::print_decoded(
                topic,
                &format!("alert {}", state.condition.label()),
                json,
                quiet,
            )?;
        } else {
            return Err(eyre!("no alert condition received within the timeout"));
        }
    }
    Ok(())
}

async fn white_light_status(global: &GlobalArgs, watch: bool) -> Result<()> {
    let topic = format!("{TOPIC_WHITE_LIGHT_STATUS_PREFIX}#");
    if global.dry_run {
        return output::print_info(
            &format!("dry-run: subscribe {topic}"),
            global.json,
            global.quiet,
        );
    }
    let options = options_from_global(global);
    let json = global.json;
    let quiet = global.quiet;

    if watch {
        tokio::task::spawn_blocking(move || -> Result<()> {
            stream_messages(&options, &[topic], None, None, |topic, payload| {
                let id = topic
                    .strip_prefix(TOPIC_WHITE_LIGHT_STATUS_PREFIX)
                    .unwrap_or(topic);
                let state = if parse_bool(payload) { "on" } else { "off" };
                output::print_decoded(topic, &format!("white-light {id} = {state}"), json, quiet)
            })
        })
        .await??;
    } else {
        let messages = collect_messages(&options, &[topic], wait_deadline(global.wait), None)?;
        let mut states: HashMap<String, String> = HashMap::new();
        for (topic, payload) in messages {
            let id = topic
                .strip_prefix(TOPIC_WHITE_LIGHT_STATUS_PREFIX)
                .unwrap_or(topic.as_str())
                .to_string();
            let state = if parse_bool(&payload) { "on" } else { "off" };
            states.insert(id, state.to_string());
        }
        for cell in white_light::CELLS {
            let state = states.remove(cell.id).unwrap_or_else(|| "?".to_string());
            output::print_decoded(
                cell.id,
                &format!("white-light {} = {state}", cell.id),
                json,
                quiet,
            )?;
        }
        let mut extras: Vec<_> = states.into_iter().collect();
        extras.sort_by(|left, right| left.0.cmp(&right.0));
        for (id, state) in extras {
            output::print_decoded(&id, &format!("white-light {id} = {state}"), json, quiet)?;
        }
    }
    Ok(())
}

async fn smoothie_status(global: &GlobalArgs, watch: bool) -> Result<()> {
    let topic = format!("{TOPIC_SMOOTHIE_PREFIX}#");
    if global.dry_run {
        return output::print_info(
            &format!("dry-run: subscribe {topic}"),
            global.json,
            global.quiet,
        );
    }
    let options = options_from_global(global);
    let json = global.json;
    let quiet = global.quiet;

    if watch {
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut status = SmoothieStatus::default();
            stream_messages(&options, &[topic], None, None, |topic, payload| {
                print_smoothie_message(&mut status, topic, payload, json, quiet)
            })
        })
        .await??;
    } else {
        let messages = collect_messages(&options, &[topic], wait_deadline(global.wait), None)?;
        let mut status = SmoothieStatus::default();
        for (topic, payload) in &messages {
            status.apply(topic, payload, Instant::now());
        }
        print_smoothie_summary(&status, json, quiet)?;
    }
    Ok(())
}

fn print_smoothie_message(
    status: &mut SmoothieStatus,
    topic: &str,
    payload: &str,
    json: bool,
    quiet: bool,
) -> Result<()> {
    status.apply(topic, payload, Instant::now());

    if let Some(name) = topic.strip_prefix(RELAY_TOPIC_PREFIX) {
        let relay = SmoothieRelay::from_topic_name(name);
        let label = match relay {
            Some(relay) => relay.label(),
            None => name,
        };
        let state = relay.and_then(|relay| status.relay(relay));
        return output::print_decoded(
            topic,
            &format!("smoothie relay {label} = {}", bool_text(state)),
            json,
            quiet,
        );
    }

    if let Some(index) = TEMP_TOPICS
        .iter()
        .position(|(temp_topic, _)| *temp_topic == topic)
    {
        let label = TEMP_TOPICS[index].1;
        return output::print_decoded(
            topic,
            &format!("smoothie {label} = {}", temp_text(status.temps[index])),
            json,
            quiet,
        );
    }

    if topic == FLOW_TOPIC {
        output::print_decoded(
            topic,
            &format!(
                "smoothie flow raw={} rate={}",
                opt_i64(status.flow_raw),
                opt_f64(status.flow_rate)
            ),
            json,
            quiet,
        )
    } else if topic == SAFETY_TOPIC {
        let safety = status.safety;
        for (_, label, on) in status.safety_bits() {
            let state = if safety.is_some() {
                bool_text(Some(on))
            } else {
                "?"
            };
            output::print_decoded(
                topic,
                &format!("smoothie safety {label} = {state}"),
                json,
                quiet,
            )?;
        }
        Ok(())
    } else if topic == LASER_TOPIC {
        output::print_decoded(
            topic,
            &format!(
                "smoothie laser 8bit={} 16bit={} voltage={}",
                opt_percent(status.laser.control_8),
                opt_percent(status.laser.control_16),
                opt_volts(status.laser.volts)
            ),
            json,
            quiet,
        )
    } else {
        output::print_decoded(topic, &format!("smoothie {topic} = {payload}"), json, quiet)
    }
}

fn print_smoothie_summary(status: &SmoothieStatus, json: bool, quiet: bool) -> Result<()> {
    for relay in SmoothieRelay::ALL {
        output::print_decoded(
            "smoothie/relay",
            &format!(
                "smoothie relay {} = {}",
                relay.label(),
                bool_text(status.relay(relay))
            ),
            json,
            quiet,
        )?;
    }
    for (index, (_, label)) in TEMP_TOPICS.iter().enumerate() {
        output::print_decoded(
            "smoothie/temp",
            &format!("smoothie {label} = {}", temp_text(status.temps[index])),
            json,
            quiet,
        )?;
    }
    output::print_decoded(
        "smoothie/flow",
        &format!(
            "smoothie flow raw={} rate={}",
            opt_i64(status.flow_raw),
            opt_f64(status.flow_rate)
        ),
        json,
        quiet,
    )?;
    let safety = status.safety;
    for (_, label, on) in status.safety_bits() {
        let state = if safety.is_some() {
            bool_text(Some(on))
        } else {
            "?"
        };
        output::print_decoded(
            "smoothie/safety",
            &format!("smoothie safety {label} = {state}"),
            json,
            quiet,
        )?;
    }
    output::print_decoded(
        "smoothie/laser",
        &format!(
            "smoothie laser 8bit={} 16bit={} voltage={}",
            opt_percent(status.laser.control_8),
            opt_percent(status.laser.control_16),
            opt_volts(status.laser.volts)
        ),
        json,
        quiet,
    )?;
    Ok(())
}

async fn monitor(global: &GlobalArgs, args: MonitorArgs) -> Result<()> {
    let mut topics = Vec::new();
    match args.kind {
        MonitorKind::Alert => topics.push(TOPIC_ALERT_CONDITION.to_string()),
        MonitorKind::WhiteLight => {
            topics.push(format!("{TOPIC_WHITE_LIGHT_STATUS_PREFIX}#"));
        }
        MonitorKind::Knobs => topics.push(format!("{TOPIC_KNOB_GET_PREFIX}#")),
        MonitorKind::Smoothie => topics.push(format!("{TOPIC_SMOOTHIE_PREFIX}#")),
        MonitorKind::All => {
            topics.push(TOPIC_ALERT_CONDITION.to_string());
            topics.push(format!("{TOPIC_WHITE_LIGHT_STATUS_PREFIX}#"));
            topics.push(format!("{TOPIC_KNOB_GET_PREFIX}#"));
            topics.push(format!("{TOPIC_SMOOTHIE_PREFIX}#"));
        }
    }

    if global.dry_run {
        return output::print_info(
            &format!("dry-run: subscribe {}", topics.join(", ")),
            global.json,
            global.quiet,
        );
    }

    let deadline = args.duration.and_then(|seconds| {
        if seconds == 0 {
            None
        } else {
            Some(Instant::now() + Duration::from_secs(seconds))
        }
    });
    let options = options_from_global(global);
    let json = global.json;
    let quiet = global.quiet;

    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut smoothie = SmoothieStatus::default();
        stream_messages(&options, &topics, deadline, None, |topic, payload| {
            if topic.starts_with(TOPIC_SMOOTHIE_PREFIX) {
                print_smoothie_message(&mut smoothie, topic, payload, json, quiet)
            } else if let Some(name) = topic.strip_prefix(TOPIC_KNOB_GET_PREFIX) {
                let value = payload.trim().parse::<f64>().unwrap_or(f64::NAN);
                output::print_decoded(
                    topic,
                    &format!("knob {name} = {}", format_value(value)),
                    json,
                    quiet,
                )
            } else if let Some(id) = topic.strip_prefix(TOPIC_WHITE_LIGHT_STATUS_PREFIX) {
                let state = if parse_bool(payload) { "on" } else { "off" };
                output::print_decoded(topic, &format!("white-light {id} = {state}"), json, quiet)
            } else if topic == TOPIC_ALERT_CONDITION {
                let state = parse_alert(payload);
                output::print_decoded(
                    topic,
                    &format!("alert {}", state.condition.label()),
                    json,
                    quiet,
                )
            } else {
                output::print_message(topic, payload, json, quiet)
            }
        })
    })
    .await??;
    Ok(())
}

fn status(global: &GlobalArgs) -> Result<()> {
    if global.dry_run {
        return output::print_info(
            &format!(
                "dry-run: broker {} client-id {}",
                global.broker,
                global.client_id.as_deref().unwrap_or("<generated>")
            ),
            global.json,
            global.quiet,
        );
    }

    let options = options_from_global(global);
    let messages = collect_messages(
        &options,
        &[TOPIC_ALERT_CONDITION.to_string()],
        wait_deadline(global.wait),
        Some(1),
    )?;

    output::print_decoded(
        "status",
        &format!("broker: {}", global.broker),
        global.json,
        global.quiet,
    )?;
    output::print_decoded(
        "status",
        &format!("client-id: {}", options.client_id),
        global.json,
        global.quiet,
    )?;
    output::print_decoded("status", "connection: ok", global.json, global.quiet)?;
    if let Some((topic, payload)) = messages.first() {
        let state = parse_alert(payload);
        output::print_decoded(
            topic,
            &format!("alert: {}", state.condition.label()),
            global.json,
            global.quiet,
        )?;
    } else {
        output::print_decoded(
            "status",
            "alert: (none received)",
            global.json,
            global.quiet,
        )?;
    }
    Ok(())
}

async fn publish(global: &GlobalArgs, args: PublishArgs) -> Result<()> {
    let payload = match (args.payload, args.payload_file) {
        (Some(payload), None) => payload,
        (None, Some(path)) => std::fs::read_to_string(&path)
            .wrap_err_with(|| format!("failed to read payload file {}", path.display()))?,
        _ => return Err(eyre!("provide either a payload argument or --payload-file")),
    };
    execute_command(
        global,
        &ControlCommand::RawPublish {
            topic: args.topic,
            payload,
            retain: args.retain,
        },
    )
    .await
}

async fn run_tui(global: GlobalArgs, args: TuiArgs) -> Result<()> {
    let mut theme = Theme::preset(args.theme);
    apply_theme_color(
        &mut theme,
        ThemeRole::Background,
        &args.background_color,
        global.quiet,
    )?;
    apply_theme_color(
        &mut theme,
        ThemeRole::Foreground,
        &args.foreground_color,
        global.quiet,
    )?;
    apply_theme_color(
        &mut theme,
        ThemeRole::Accent,
        &args.accent_color,
        global.quiet,
    )?;
    apply_theme_color(
        &mut theme,
        ThemeRole::Selection,
        &args.selection_color,
        global.quiet,
    )?;
    apply_theme_color(
        &mut theme,
        ThemeRole::Success,
        &args.success_color,
        global.quiet,
    )?;
    apply_theme_color(
        &mut theme,
        ThemeRole::Warning,
        &args.warning_color,
        global.quiet,
    )?;
    apply_theme_color(
        &mut theme,
        ThemeRole::Error,
        &args.error_color,
        global.quiet,
    )?;
    apply_theme_color(
        &mut theme,
        ThemeRole::Muted,
        &args.muted_color,
        global.quiet,
    )?;
    apply_theme_color(
        &mut theme,
        ThemeRole::Border,
        &args.border_color,
        global.quiet,
    )?;
    apply_theme_color(
        &mut theme,
        ThemeRole::Alert,
        &args.alert_color,
        global.quiet,
    )?;

    let options = TuiOptions {
        mqtt: options_from_global(&global),
        view: args.view,
        connect: !args.no_connect,
        safe_mode: !args.no_safe_mode,
        theme,
        theme_preset: args.theme,
        smoothie_url: global.smoothiesaur_url.clone(),
        pizzamat: crate::pizzamat::Options::from_global(&global),
        heizung: crate::heizung::Options::from_global(&global),
        influx: crate::influx::Options::from_global(&global),
        graph_minutes: args.graph_minutes,
        graph_layout: args.graph_layout,
        verbose: global.verbose,
        dry_run: global.dry_run,
    };
    tui::run(options).await
}

fn apply_theme_color(
    theme: &mut Theme,
    role: ThemeRole,
    value: &Option<String>,
    quiet: bool,
) -> Result<()> {
    if let Some(value) = value {
        let parsed = parse_color(value)?;
        if parsed.alpha_ignored && !quiet {
            eprintln!("warning: alpha channel in '{value}' is ignored (terminals have no alpha)");
        }
        theme.set(role, parsed.color);
    }
    Ok(())
}

fn render_man(args: ManArgs) -> Result<()> {
    let command = Cli::command();

    if let Some(name) = args.name {
        let subcommand = command
            .get_subcommands()
            .find(|subcommand| subcommand.get_name() == name.as_str())
            .ok_or_else(|| eyre!("unknown subcommand '{name}'"))?;
        let buffer = render_man_page(subcommand)?;
        if let Some(directory) = args.output_dir {
            std::fs::create_dir_all(&directory)
                .wrap_err_with(|| format!("failed to create {}", directory.display()))?;
            let path = directory.join(format!("subraumctl-{name}.1"));
            std::fs::write(&path, buffer)
                .wrap_err_with(|| format!("failed to write {}", path.display()))?;
        } else {
            std::io::stdout().write_all(&buffer)?;
        }
        return Ok(());
    }

    if let Some(directory) = args.output_dir {
        std::fs::create_dir_all(&directory)
            .wrap_err_with(|| format!("failed to create {}", directory.display()))?;
        write_man_tree(&command, "", &directory)?;
    } else {
        let buffer = render_man_page(&command)?;
        std::io::stdout().write_all(&buffer)?;
    }
    Ok(())
}

fn render_man_page(command: &clap::Command) -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    clap_mangen::Man::new(command.clone()).render(&mut buffer)?;
    Ok(buffer)
}

fn write_man_tree(command: &clap::Command, prefix: &str, directory: &Path) -> Result<()> {
    let file_name = if prefix.is_empty() {
        "subraumctl".to_string()
    } else {
        format!("subraumctl-{prefix}")
    };
    let buffer = render_man_page(command)?;
    std::fs::write(directory.join(format!("{file_name}.1")), buffer)
        .wrap_err_with(|| format!("failed to write {file_name}.1"))?;

    for subcommand in command.get_subcommands() {
        if subcommand.get_name() == "help" {
            continue;
        }
        let next_prefix = if prefix.is_empty() {
            subcommand.get_name().to_string()
        } else {
            format!("{prefix}-{}", subcommand.get_name())
        };
        write_man_tree(subcommand, &next_prefix, directory)?;
    }
    Ok(())
}

fn render_completions(args: CompletionsArgs) -> Result<()> {
    let mut command = Cli::command();
    let shell = args.shell.into_shell();
    clap_complete::generate(shell, &mut command, "subraumctl", &mut std::io::stdout());
    Ok(())
}

fn bool_text(state: Option<bool>) -> &'static str {
    match state {
        Some(true) => "on",
        Some(false) => "off",
        None => "?",
    }
}

fn temp_text(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{value:.2} °C"),
        None => "?".to_string(),
    }
}

fn opt_f64(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{value:.2}"),
        None => "?".to_string(),
    }
}

fn opt_i64(value: Option<i64>) -> String {
    match value {
        Some(value) => value.to_string(),
        None => "?".to_string(),
    }
}

fn opt_percent(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{value:.2}%"),
        None => "?".to_string(),
    }
}

fn opt_volts(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{value:.4} V"),
        None => "?".to_string(),
    }
}
