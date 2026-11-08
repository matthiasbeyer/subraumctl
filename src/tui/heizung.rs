//! Heating state and bounded AMQP worker, independent of the MQTT connection.

use std::future::Future;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use eyre::{Result, ensure, eyre};
use serde_json::Value;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tui_input::Input;

use super::app::{App, Popup, Section, StatusKind, TextInputMode, TextInputState};
use super::safe_mode::{ConfirmChoice, PendingCommand};
use super::traffic::{TrafficDirection, TrafficEntry, now_hms};
use crate::heizung::{
    DEFAULT_DURATION, MAX_DURATION, Options, Request, Room, transport, validate_setting,
};

pub const TARGET: usize = Room::ALL.len();
pub const DURATION: usize = TARGET + 1;
pub const APPLY: usize = DURATION + 1;
pub const REFRESH: usize = APPLY + 1;

#[derive(Clone, Copy, Debug)]
pub enum Field {
    Temperature,
    Duration,
}

impl Field {
    pub fn title(self) -> &'static str {
        match self {
            Self::Temperature => "Target temperature (15..25 °C, steps of 0.1)",
            Self::Duration => "Duration (0..35791 minutes; 0 expires timer)",
        }
    }
}

#[derive(Clone, Copy)]
pub struct Reading {
    pub target: f64,
    pub remaining_seconds: u64,
}

pub struct State {
    pub focus: usize,
    pub selected: [bool; 6],
    pub temperature_tenths: u16,
    pub duration: u32,
    pub readings: [Option<Reading>; 6],
    pub updated: Option<Instant>,
    pub attempted: bool,
    pub busy: bool,
    pub error: bool,
    pub message: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            focus: 0,
            selected: [false; 6],
            temperature_tenths: 200,
            duration: DEFAULT_DURATION,
            readings: [None; 6],
            updated: None,
            attempted: false,
            busy: false,
            error: false,
            message: "Select Refresh status and press Enter.".into(),
        }
    }
}

impl State {
    pub fn temperature(&self) -> f64 {
        f64::from(self.temperature_tenths) / 10.0
    }

    pub fn selected_rooms(&self) -> Vec<Room> {
        Room::ALL
            .into_iter()
            .zip(self.selected)
            .filter_map(|(room, selected)| selected.then_some(room))
            .collect()
    }

    pub fn set_field(&mut self, field: Field, text: &str) -> Result<()> {
        match field {
            Field::Temperature => {
                let temperature = text
                    .trim()
                    .parse()
                    .map_err(|_| eyre!("enter a temperature in Celsius"))?;
                validate_setting(temperature, self.duration)?;
                self.temperature_tenths = (temperature * 10.0).round() as u16;
            }
            Field::Duration => {
                let duration = text
                    .trim()
                    .parse()
                    .map_err(|_| eyre!("enter a whole number of minutes"))?;
                validate_setting(self.temperature(), duration)?;
                self.duration = duration;
            }
        }
        Ok(())
    }

    fn update_readings(&mut self, value: &Value) -> Result<()> {
        let rooms = value["rooms"]
            .as_array()
            .ok_or_else(|| eyre!("invalid heating status"))?;
        ensure!(rooms.len() == Room::ALL.len(), "incomplete heating status");
        let mut readings = [None; 6];
        for room in rooms {
            let index = Room::ALL
                .iter()
                .position(|known| room["room"].as_str() == Some(known.name()))
                .ok_or_else(|| eyre!("unknown heating room"))?;
            ensure!(readings[index].is_none(), "duplicate heating room");
            readings[index] = Some(Reading {
                target: room["target_celsius"]
                    .as_f64()
                    .ok_or_else(|| eyre!("invalid heating target"))?,
                remaining_seconds: room["remaining_seconds"]
                    .as_u64()
                    .ok_or_else(|| eyre!("invalid heating timer"))?,
            });
        }
        self.readings = readings;
        self.updated = Some(Instant::now());
        Ok(())
    }
}

struct Reply {
    value: Value,
    refresh: Option<std::result::Result<Value, String>>,
}

pub struct Event {
    request: Request,
    result: std::result::Result<Reply, String>,
}

// A successful write is followed by a read, never by a retry of the write.
async fn execute_request<F, Fut>(request: &Request, dry_run: bool, mut send: F) -> Result<Reply>
where
    F: FnMut(Request) -> Fut,
    Fut: Future<Output = Result<Value>>,
{
    let value = send(request.clone()).await?;
    let refresh = if !request.read_only() && !dry_run {
        Some(
            send(Request::get(vec![])?)
                .await
                .map_err(|error| error.to_string()),
        )
    } else {
        None
    };
    Ok(Reply { value, refresh })
}

pub fn spawn_worker(
    options: Options,
    dry_run: bool,
    mut rx: UnboundedReceiver<Request>,
    tx: UnboundedSender<Event>,
) {
    tokio::spawn(async move {
        while let Some(request) = rx.recv().await {
            let result = execute_request(&request, dry_run, |request| {
                let options = &options;
                async move { transport::execute(options, &request, dry_run).await }
            })
            .await
            .map_err(|error| error.to_string());
            if tx.send(Event { request, result }).is_err() {
                break;
            }
        }
    });
}

impl App {
    pub(super) fn maybe_refresh_heizung(&mut self) {
        if self.section == Section::Heizung && self.options.connect && !self.heizung.attempted {
            self.refresh_heizung();
        }
    }

    fn refresh_heizung(&mut self) {
        self.send_heizung(Request::get(vec![]).expect("known rooms"));
    }

    pub(super) fn handle_heizung_key(&mut self, key: KeyEvent) {
        let focus = self.heizung.focus;
        match key.code {
            KeyCode::Up => self.heizung.focus = (focus + REFRESH) % (REFRESH + 1),
            KeyCode::Down => self.heizung.focus = (focus + 1) % (REFRESH + 1),
            KeyCode::Char('a') => self.heizung.selected.fill(true),
            KeyCode::Char('n') => self.heizung.selected.fill(false),
            KeyCode::Char(' ') if focus < TARGET => self.heizung.selected[focus] ^= true,
            KeyCode::Left | KeyCode::Right => {
                let sign = if key.code == KeyCode::Right { 1 } else { -1 };
                let big = key.modifiers.contains(KeyModifiers::SHIFT);
                if focus == TARGET {
                    let delta = sign * if big { 10 } else { 1 };
                    self.heizung.temperature_tenths =
                        (i32::from(self.heizung.temperature_tenths) + delta).clamp(150, 250) as u16;
                } else if focus == DURATION {
                    let delta = sign * if big { 60 } else { 15 };
                    self.heizung.duration =
                        (self.heizung.duration as i32 + delta).clamp(0, MAX_DURATION as i32) as u32;
                }
            }
            KeyCode::Char(ch)
                if (ch.is_ascii_digit() || ch == '.') && (focus == TARGET || focus == DURATION) =>
            {
                self.edit_heizung(Some(ch.to_string()));
            }
            KeyCode::Enter => match focus {
                0..TARGET => self.heizung.selected[focus] ^= true,
                TARGET | DURATION => self.edit_heizung(None),
                APPLY if !self.heizung.busy => {
                    match Request::set(
                        self.heizung.selected_rooms(),
                        self.heizung.temperature(),
                        self.heizung.duration,
                    ) {
                        Ok(request) if self.safe_mode => {
                            self.popup = Some(Popup::ConfirmCommand {
                                pending: PendingCommand::Heizung(request),
                                selected: ConfirmChoice::Cancel,
                            })
                        }
                        Ok(request) => self.send_heizung(request),
                        Err(error) => self.set_status(error.to_string(), StatusKind::Error),
                    }
                }
                REFRESH => self.refresh_heizung(),
                _ => {}
            },
            _ => {}
        }
    }

    fn edit_heizung(&mut self, text: Option<String>) {
        let (field, value) = if self.heizung.focus == TARGET {
            (
                Field::Temperature,
                format!("{:.1}", self.heizung.temperature()),
            )
        } else {
            (Field::Duration, self.heizung.duration.to_string())
        };
        self.text_input = Some(TextInputState {
            mode: TextInputMode::Heizung {
                field,
                input: Input::new(text.unwrap_or(value)),
            },
        });
    }

    pub(super) fn send_heizung(&mut self, request: Request) {
        if self.heizung.busy {
            return;
        }
        self.heizung.attempted = true;
        let entry = traffic(
            if self.options.dry_run {
                TrafficDirection::Dry
            } else {
                TrafficDirection::Tx
            },
            request.args(),
        );
        if self.heizung_tx.send(request).is_err() {
            self.heizung.error = true;
            self.heizung.message = "Heizung worker stopped".into();
            self.set_status(self.heizung.message.clone(), StatusKind::Error);
            return;
        }
        self.traffic.push(entry);
        self.heizung.busy = true;
        self.heizung.message = "Waiting for Heizung…".into();
        self.set_status(self.heizung.message.clone(), StatusKind::Info);
    }

    pub(super) fn handle_heizung_event(&mut self, event: Event) {
        self.heizung.busy = false;
        self.heizung.error = false;
        let outcome = match event.result {
            Err(error) => Err(error),
            Ok(reply) => {
                self.traffic.push(traffic(
                    if self.options.dry_run {
                        TrafficDirection::Dry
                    } else {
                        TrafficDirection::Rx
                    },
                    reply.value.to_string(),
                ));
                if self.options.dry_run {
                    Ok(format!("dry-run: heizung {}", event.request.args()))
                } else if event.request.read_only() {
                    self.heizung
                        .update_readings(&reply.value)
                        .map(|()| "Heating status received".into())
                        .map_err(|error| error.to_string())
                } else {
                    let snapshot = reply.refresh.expect("live writes refresh status");
                    self.traffic.push(traffic(
                        TrafficDirection::Tx,
                        Request::get(vec![]).expect("known rooms").args(),
                    ));
                    match snapshot {
                        Ok(value) => {
                            self.traffic
                                .push(traffic(TrafficDirection::Rx, value.to_string()));
                            self.heizung
                                .update_readings(&value)
                                .map(|()| "Setting acknowledged; status refreshed".into())
                                .map_err(|error| {
                                    format!("Setting acknowledged; status refresh failed: {error}")
                                })
                        }
                        Err(error) => Err(format!(
                            "Setting acknowledged; status refresh failed: {error}"
                        )),
                    }
                }
            }
        };
        let kind = match outcome {
            Ok(message) => {
                self.heizung.message = message;
                StatusKind::Success
            }
            Err(error) => {
                self.traffic
                    .push(traffic(TrafficDirection::Error, error.clone()));
                self.heizung.message = error;
                self.heizung.error = true;
                StatusKind::Error
            }
        };
        self.set_status(self.heizung.message.clone(), kind);
    }
}

fn traffic(direction: TrafficDirection, payload: String) -> TrafficEntry {
    TrafficEntry {
        at: now_hms(),
        direction,
        transport: "amqp",
        target: "heizung".into(),
        payload,
        note: None,
        sections: Section::Overview.bit() | Section::Heizung.bit(),
    }
}

#[cfg(test)]
mod tests;
