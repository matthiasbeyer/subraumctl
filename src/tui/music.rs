//! MPD playback state and background worker, independent of MQTT.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use super::app::{App, Section, StatusKind};
use super::traffic::{TrafficDirection, TrafficEntry, now_hms};
use crate::music::{self, Options, Outcome, PlaybackStatus, Request, STATIONS, StationId};

pub const PLAY: usize = 0;
pub const PAUSE: usize = 1;
pub const PREVIOUS: usize = 2;
pub const NEXT: usize = 3;
pub const REFRESH: usize = 4;
pub const STATIONS_START: usize = REFRESH + 1;
const POLL_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct State {
    pub focus: usize,
    pub status: Option<PlaybackStatus>,
    pub updated: Option<Instant>,
    pub last_poll: Option<Instant>,
    pub busy: bool,
    pub error: Option<String>,
    pub message: String,
}

pub struct Event {
    pub request: Request,
    pub result: Result<Outcome, String>,
}

pub fn spawn_worker(
    options: Options,
    dry_run: bool,
    mut rx: UnboundedReceiver<Request>,
    tx: UnboundedSender<Event>,
) {
    tokio::spawn(async move {
        while let Some(request) = rx.recv().await {
            let result = music::execute(&options, &request, dry_run)
                .await
                .map_err(|error| error.to_string());
            if tx.send(Event { request, result }).is_err() {
                break;
            }
        }
    });
}

impl App {
    pub(super) fn maybe_refresh_music(&mut self) {
        if self.section != Section::Music || !self.options.connect {
            return;
        }
        if self.music.busy
            || self
                .music
                .last_poll
                .is_some_and(|at| at.elapsed() < POLL_INTERVAL)
        {
            return;
        }
        self.request_music(Request::Status, false);
    }

    pub(super) fn poll_music(&mut self) {
        self.maybe_refresh_music();
    }

    pub(super) fn handle_music_key(&mut self, key: KeyEvent) {
        let count = STATIONS_START + STATIONS.len();
        match key.code {
            KeyCode::Up => self.music.focus = (self.music.focus + count - 1) % count,
            KeyCode::Down => self.music.focus = (self.music.focus + 1) % count,
            KeyCode::PageUp => self.music.focus = self.music.focus.saturating_sub(5),
            KeyCode::PageDown => self.music.focus = (self.music.focus + 5).min(count - 1),
            KeyCode::Home => self.music.focus = 0,
            KeyCode::End => self.music.focus = count - 1,
            KeyCode::Char(' ') => self.toggle_music(),
            KeyCode::Char('n') => self.request_music(Request::Next, true),
            KeyCode::Char('p') => self.request_music(Request::Previous, true),
            KeyCode::Enter => match self.music.focus {
                PLAY => self.request_music(Request::Play, true),
                PAUSE => self.request_music(Request::Pause, true),
                PREVIOUS => self.request_music(Request::Previous, true),
                NEXT => self.request_music(Request::Next, true),
                REFRESH => self.request_music(Request::Status, true),
                station => {
                    let index = station - STATIONS_START;
                    let Some(id) = STATIONS
                        .get(index)
                        .and_then(|station| StationId::from_id(station.id))
                    else {
                        return;
                    };
                    self.request_music(Request::Station(id), true);
                }
            },
            _ => {}
        }
    }

    fn toggle_music(&mut self) {
        let playing = self
            .music
            .status
            .as_ref()
            .is_some_and(|status| status.state == "play");
        self.request_music(
            if playing {
                Request::Pause
            } else {
                Request::Play
            },
            true,
        );
    }

    fn request_music(&mut self, request: Request, announce: bool) {
        if self.music.busy {
            if announce {
                self.set_status("Music is busy", StatusKind::Info);
            }
            return;
        }
        let Some(tx) = &self.music_tx else {
            self.set_status("Music worker is not running", StatusKind::Error);
            return;
        };
        let direction = if self.options.dry_run {
            TrafficDirection::Dry
        } else {
            TrafficDirection::Tx
        };
        let entry = self.music_traffic(&request, direction, request.command_preview());
        if tx.send(request).is_err() {
            self.music.error = Some("Music worker stopped".into());
            self.set_status("Music worker stopped", StatusKind::Error);
            return;
        }
        self.traffic.push(entry);
        self.music.busy = true;
        self.music.last_poll = Some(Instant::now());
        if announce {
            self.music.message = "Talking to MPD…".into();
            self.set_status("Talking to MPD…", StatusKind::Info);
        }
    }

    pub(super) fn handle_music_event(&mut self, event: Event) {
        self.music.busy = false;
        match event.result {
            Ok(Outcome::Status(status)) => {
                self.traffic.push(self.music_traffic(
                    &event.request,
                    TrafficDirection::Rx,
                    status.display_title(),
                ));
                self.music.error = None;
                self.music.updated = Some(Instant::now());
                self.music.message =
                    format!("{} — {}", status.state_label(), status.display_title());
                self.music.status = Some(status);
            }
            Ok(Outcome::Submitted) => {
                self.traffic.push(self.music_traffic(
                    &event.request,
                    TrafficDirection::Rx,
                    "OK".into(),
                ));
                self.music.error = None;
                self.music.message = "Command sent".into();
                self.set_status("music command sent", StatusKind::Success);
                self.request_music(Request::Status, false);
            }
            Ok(Outcome::DryRun { command }) => {
                self.music.message = format!("dry-run: {command}");
                self.set_status(format!("dry-run: {command}"), StatusKind::Info);
            }
            Err(error) => {
                self.traffic.push(self.music_traffic(
                    &event.request,
                    TrafficDirection::Error,
                    error.clone(),
                ));
                self.music.error = Some(error.clone());
                self.music.message = error.clone();
                self.set_status(format!("music error: {error}"), StatusKind::Error);
            }
        }
    }

    fn music_traffic(
        &self,
        request: &Request,
        direction: TrafficDirection,
        payload: String,
    ) -> TrafficEntry {
        TrafficEntry {
            at: now_hms(),
            direction,
            transport: "mpd",
            target: self.options.music.address.clone(),
            payload,
            note: Some(request.command_preview()),
            sections: Section::Overview.bit() | Section::Music.bit(),
        }
    }
}

#[cfg(test)]
mod tests;
