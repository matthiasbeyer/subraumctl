use crossterm::event::{KeyCode, KeyEvent};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tui_input::Input;

use super::app::{App, Section, StatusKind, TextInputMode, TextInputState};
use super::traffic::{TrafficDirection, TrafficEntry, now_hms};
use crate::pizzamat::{Operation, Options, Request, transport};

#[derive(Default)]
pub struct State {
    pub focus: usize,
    pub scroll: u16,
    pub busy: bool,
    pub response: String,
}

pub struct Event {
    request: Request,
    result: Result<String, String>,
}

pub fn spawn_worker(
    options: Options,
    dry_run: bool,
    mut rx: UnboundedReceiver<Request>,
    tx: UnboundedSender<Event>,
) {
    tokio::spawn(async move {
        while let Some(request) = rx.recv().await {
            let result = async {
                let reply = transport::execute(&options, &request, dry_run).await?;
                let mut text = reply["message"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| reply.to_string());
                if !request.operation.read_only() && !dry_run {
                    let list = Request::new(Operation::List, &request.user, &[])?;
                    match transport::execute(&options, &list, false).await {
                        Ok(orders) => {
                            text.push_str("\n\nCurrent orders:\n");
                            text.push_str(orders["message"].as_str().unwrap_or_default());
                        }
                        Err(error) => {
                            text.push_str(&format!("\n\nCould not refresh orders: {error}"))
                        }
                    }
                }
                Ok::<_, eyre::Report>(text)
            }
            .await
            .map_err(|error| error.to_string());
            if tx.send(Event { request, result }).is_err() {
                break;
            }
        }
    });
}

impl App {
    pub(super) fn handle_pizzamat_key(&mut self, key: KeyEvent) {
        let count = Operation::ALL.len() + 1;
        match key.code {
            KeyCode::Up => self.pizzamat.focus = (self.pizzamat.focus + count - 1) % count,
            KeyCode::Down => self.pizzamat.focus = (self.pizzamat.focus + 1) % count,
            KeyCode::PageUp => self.pizzamat.scroll = self.pizzamat.scroll.saturating_sub(5),
            KeyCode::PageDown => {
                self.pizzamat.scroll = self.pizzamat.scroll.saturating_add(5).min(
                    self.pizzamat
                        .response
                        .chars()
                        .count()
                        .min(u16::MAX as usize) as u16,
                )
            }
            KeyCode::Home => self.pizzamat.scroll = 0,
            KeyCode::Enter if !self.pizzamat.busy => {
                if self.pizzamat.focus == 0 {
                    self.text_input = Some(TextInputState {
                        mode: TextInputMode::Pizza {
                            operation: None,
                            fields: vec![],
                            input: Input::new(self.options.pizzamat.user.clone()),
                        },
                    });
                    return;
                }
                let operation = Operation::ALL[self.pizzamat.focus - 1];
                if self.options.pizzamat.user.trim().is_empty() {
                    self.set_status(
                        "Set your Pizzamat identity first (select User and press Enter)",
                        StatusKind::Error,
                    );
                } else if operation.fields().is_empty() {
                    match Request::new(operation, &self.options.pizzamat.user, &[]) {
                        Ok(request) => self.send_pizzamat(request),
                        Err(error) => self.set_status(error.to_string(), StatusKind::Error),
                    }
                } else {
                    self.text_input = Some(TextInputState {
                        mode: TextInputMode::Pizza {
                            operation: Some(operation),
                            fields: vec![],
                            input: Input::default(),
                        },
                    });
                }
            }
            _ => {}
        }
    }

    pub(super) fn send_pizzamat(&mut self, request: Request) {
        if self.pizzamat.busy {
            return;
        }
        let entry = self.pizzamat_traffic(
            &request,
            if self.options.dry_run {
                TrafficDirection::Dry
            } else {
                TrafficDirection::Tx
            },
            request.args.clone(),
        );
        if self.pizza_tx.send(request).is_err() {
            self.set_status("Pizzamat worker stopped", StatusKind::Error);
            return;
        }
        self.traffic.push(entry);
        self.pizzamat.busy = true;
        self.set_status("Waiting for Pizzamat…", StatusKind::Info);
    }

    pub(super) fn handle_pizzamat_event(&mut self, event: Event) {
        self.pizzamat.busy = false;
        self.pizzamat.scroll = 0;
        let (text, direction, kind) = match event.result {
            Ok(text) => (text, TrafficDirection::Rx, StatusKind::Success),
            Err(error) => (error, TrafficDirection::Error, StatusKind::Error),
        };
        self.traffic
            .push(self.pizzamat_traffic(&event.request, direction, text.clone()));
        self.pizzamat.response = text;
        self.set_status(
            if kind == StatusKind::Error {
                "Pizzamat request failed — see response"
            } else {
                "Pizzamat response received"
            },
            kind,
        );
    }

    fn pizzamat_traffic(
        &self,
        request: &Request,
        direction: TrafficDirection,
        payload: String,
    ) -> TrafficEntry {
        TrafficEntry {
            at: now_hms(),
            direction,
            transport: "amqp",
            target: request.command(self.options.pizzamat.debug),
            payload,
            note: Some(format!("user: {}", request.user)),
            sections: Section::Overview.bit() | Section::Pizzamat.bit(),
        }
    }
}

#[cfg(test)]
mod tests;
