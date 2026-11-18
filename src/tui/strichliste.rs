//! Serial HTTP account operations and cancellable payment forms, independent of MQTT.

use std::time::{Duration, Instant};

use crossterm::event::{Event as InputEvent, KeyCode, KeyEvent};
use eyre::{Result, ensure, eyre};
use ratatui::widgets::ListState;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tui_input::{Input, backend::crossterm::EventHandler};

use super::app::{App, Popup, Section, StatusKind};
use super::safe_mode::{ConfirmChoice, PendingCommand};
use super::traffic::{TrafficDirection, TrafficEntry, now_hms};
use crate::strichliste::{
    Client, Options, Request, TRANSACTION_PAGE_SIZE, Transaction, TransactionPage, User,
    parse_amount, validate_amount,
};

pub const ACTIONS: [&str; 4] = ["Pay", "Add credit", "Send to another user", "Transactions"];
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub enum Command {
    Refresh,
    RefreshAt(usize),
    Transactions(usize),
    LoadRecipients,
    Book {
        request: Request,
        comment: String,
    },
    Send {
        recipient: User,
        amount: f64,
        comment: String,
    },
}

impl Command {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Book { request, .. } => {
                ensure!(request.value()?.is_some(), "expected pay or add")
            }
            Self::Send { amount, .. } => {
                validate_amount(*amount)?;
            }
            Self::LoadRecipients | Self::Refresh | Self::RefreshAt(_) | Self::Transactions(_) => {}
        }
        Ok(())
    }

    pub fn preview_lines(&self) -> Vec<String> {
        match self {
            Self::Refresh | Self::RefreshAt(_) => {
                vec!["Read your balance, purchase rank and recent transactions".into()]
            }
            Self::Transactions(offset) => vec![format!(
                "Read {TRANSACTION_PAGE_SIZE} transactions at offset {offset}"
            )],
            Self::LoadRecipients => vec!["Load recipients for live search".into()],
            Self::Book { request, comment } => vec![
                match request {
                    Request::Pay(amount) => format!("Pay {amount} EUR from your balance"),
                    Request::Add(amount) => format!("Add {amount} EUR to your balance"),
                    _ => "Invalid booking".into(),
                },
                format!("Comment: {}", comment.escape_debug()),
            ],
            Self::Send {
                recipient,
                amount,
                comment,
            } => vec![
                format!(
                    "Send {amount} EUR to {} (id {})",
                    recipient.name.escape_debug(),
                    recipient.id
                ),
                format!("Comment: {}", comment.escape_debug()),
            ],
        }
    }
    fn is_refresh(&self) -> bool {
        matches!(self, Self::Refresh | Self::RefreshAt(_))
    }
}

pub enum Response {
    Account {
        balance: Result<f64, String>,
        rank: Result<(u64, f64), String>,
        transactions: Result<TransactionPage, String>,
    },
    Transactions(TransactionPage),
    Recipients(Vec<User>),
    Submitted,
    DryRun,
}

pub struct Event {
    pub command: Command,
    pub result: Result<Response, String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Pay,
    Add,
    Send,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Search,
    Loading,
    Recipients,
    Amount,
    Comment,
}

pub struct Form {
    pub kind: Kind,
    pub step: Step,
    pub input: Input,
    pub amount: Option<f64>,
    pub recipient: Option<User>,
    pub all_recipients: Vec<User>,
    pub recipients: Vec<User>,
    pub selection: ListState,
}

impl Form {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            step: if kind == Kind::Send {
                Step::Loading
            } else {
                Step::Amount
            },
            input: Input::default(),
            amount: None,
            recipient: None,
            all_recipients: Vec::new(),
            recipients: Vec::new(),
            selection: ListState::default(),
        }
    }

    fn filter_recipients(&mut self) {
        let query = self.input.value().trim().to_lowercase();
        self.recipients = self
            .all_recipients
            .iter()
            .filter(|user| user.name.to_lowercase().contains(&query))
            .cloned()
            .collect();
        self.selection = ListState::default();
        self.selection
            .select((!self.recipients.is_empty()).then_some(0));
    }

    pub fn title(&self) -> &'static str {
        match self.kind {
            Kind::Pay => "Pay",
            Kind::Add => "Add credit",
            Kind::Send => "Send",
        }
    }
}

#[derive(Default)]
pub struct State {
    pub tx: Option<UnboundedSender<Command>>,
    pub focus: usize,
    pub balance: Option<f64>,
    pub rank: Option<(u64, f64)>,
    pub balance_stale: bool,
    pub rank_stale: bool,
    pub transactions: Option<Vec<Transaction>>,
    pub transactions_stale: bool,
    pub show_transactions: bool,
    pub transaction_selection: ListState,
    pub transaction_offset: usize,
    pub transaction_total: Option<usize>,
    pub transaction_next: Option<usize>,
    pub transaction_previous: Vec<usize>,
    pub busy: bool,
    pub last_refresh: Option<Instant>,
    pub full_precision: bool,
    pub message: String,
    pub error: Option<String>,
    pub form: Option<Form>,
}

async fn execute(options: Options, command: &Command, dry_run: bool) -> Result<Response> {
    command.validate()?;
    if dry_run {
        return Ok(Response::DryRun);
    }
    let client = Client::new(options)?;
    match command {
        Command::Refresh | Command::RefreshAt(_) => {
            let offset = match command {
                Command::RefreshAt(offset) => *offset,
                _ => 0,
            };
            let (balance, transactions) = match client.account_snapshot_at(offset).await {
                Ok(account) => (
                    account.balance.map_err(|error| error.to_string()),
                    account.transactions.map_err(|error| error.to_string()),
                ),
                Err(error) => (Err(error.to_string()), Err(error.to_string())),
            };
            let rank = client
                .execute(&Request::Rank, "")
                .await
                .and_then(|value| {
                    Ok((
                        value["rank"]
                            .as_u64()
                            .ok_or_else(|| eyre!("missing rank"))?,
                        value["weighted_count_of_purchases"]
                            .as_f64()
                            .ok_or_else(|| eyre!("missing purchase score"))?,
                    ))
                })
                .map_err(|error| error.to_string());
            Ok(Response::Account {
                balance,
                rank,
                transactions,
            })
        }
        Command::Transactions(offset) => Ok(Response::Transactions(
            client.transactions(TRANSACTION_PAGE_SIZE, *offset).await?,
        )),
        Command::LoadRecipients => {
            let current = client.current_user().await?;
            let mut users: Vec<_> = client
                .users()
                .await?
                .into_iter()
                .filter(|user| user.id != current.id)
                .collect();
            users.sort_by(|a, b| {
                a.name
                    .to_lowercase()
                    .cmp(&b.name.to_lowercase())
                    .then(a.id.cmp(&b.id))
            });
            Ok(Response::Recipients(users))
        }
        Command::Book { request, comment } => {
            client.execute(request, comment).await?;
            Ok(Response::Submitted)
        }
        Command::Send {
            recipient,
            amount,
            comment,
        } => {
            client.transfer(recipient, *amount, comment).await?;
            Ok(Response::Submitted)
        }
    }
}

pub fn spawn_worker(
    options: Options,
    dry_run: bool,
    mut rx: UnboundedReceiver<Command>,
    tx: UnboundedSender<Event>,
) {
    tokio::spawn(async move {
        while let Some(command) = rx.recv().await {
            let result = execute(options.clone(), &command, dry_run)
                .await
                .map_err(|e| e.to_string());
            if tx.send(Event { command, result }).is_err() {
                break;
            }
        }
    });
}

impl App {
    pub(super) fn maybe_refresh_strichliste(&mut self) {
        if self.section == Section::Strichliste
            && self.options.connect
            && !self.strichliste.busy
            && self.strichliste.form.is_none()
            && self.popup.is_none()
            && self
                .strichliste
                .last_refresh
                .is_none_or(|at| at.elapsed() >= REFRESH_INTERVAL)
        {
            self.send_strichliste(Command::Refresh);
        }
    }

    pub(super) fn handle_strichliste_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('t') {
            self.strichliste.show_transactions = !self.strichliste.show_transactions;
            return;
        }
        if self.strichliste.show_transactions {
            let entries = self.strichliste.transactions.as_ref().map_or(0, Vec::len);
            let count = entries
                + usize::from(self.strichliste.transaction_next.is_some())
                + usize::from(!self.strichliste.transaction_previous.is_empty());
            let selected = self
                .strichliste
                .transaction_selection
                .selected()
                .unwrap_or(0);
            let index = match key.code {
                KeyCode::Up => selected.saturating_sub(1),
                KeyCode::Down => (selected + 1).min(count.saturating_sub(1)),
                KeyCode::PageUp => selected.saturating_sub(10),
                KeyCode::PageDown => (selected + 10).min(count.saturating_sub(1)),
                KeyCode::Home => 0,
                KeyCode::End => count.saturating_sub(1),
                _ => selected,
            };
            self.strichliste
                .transaction_selection
                .select((count > 0).then_some(index));
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.strichliste.show_transactions = false,
                KeyCode::Char('r') => self.send_strichliste(Command::Refresh),
                KeyCode::Char('n') => {
                    if let Some(offset) = self.strichliste.transaction_next {
                        self.send_strichliste(Command::Transactions(offset));
                    }
                }
                KeyCode::Char('b') => {
                    if let Some(offset) = self.strichliste.transaction_previous.last().copied() {
                        self.send_strichliste(Command::Transactions(offset));
                    }
                }
                KeyCode::Char('p') => {
                    self.strichliste.full_precision = !self.strichliste.full_precision
                }
                KeyCode::Enter => {
                    if index >= entries {
                        let offset =
                            if index == entries && self.strichliste.transaction_next.is_some() {
                                self.strichliste.transaction_next
                            } else {
                                self.strichliste.transaction_previous.last().copied()
                            };
                        if let Some(offset) = offset {
                            self.send_strichliste(Command::Transactions(offset));
                        }
                        return;
                    }
                    if let Some(transaction) = self
                        .strichliste
                        .transactions
                        .as_ref()
                        .and_then(|items| items.get(index))
                    {
                        self.popup = Some(Popup::StrichlisteTransaction {
                            transaction: transaction.clone(),
                            scroll: 0,
                        });
                    }
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Up => {
                self.strichliste.focus =
                    (self.strichliste.focus + ACTIONS.len() - 1) % ACTIONS.len()
            }
            KeyCode::Down => self.strichliste.focus = (self.strichliste.focus + 1) % ACTIONS.len(),
            KeyCode::Home => self.strichliste.focus = 0,
            KeyCode::End => self.strichliste.focus = ACTIONS.len() - 1,
            KeyCode::Char('r') => self.send_strichliste(Command::Refresh),
            KeyCode::Char('p') => {
                self.strichliste.full_precision = !self.strichliste.full_precision
            }
            KeyCode::Enter if !self.strichliste.busy => {
                if self.strichliste.focus == 3 {
                    self.strichliste.show_transactions = true;
                    return;
                }
                let kind = match self.strichliste.focus {
                    0 => Kind::Pay,
                    1 => Kind::Add,
                    _ => Kind::Send,
                };
                self.strichliste.form = Some(Form::new(kind));
                self.strichliste.error = None;
                if kind == Kind::Send {
                    self.send_strichliste(Command::LoadRecipients);
                }
            }
            _ => {}
        }
    }

    pub(super) fn handle_strichliste_input(&mut self, key: KeyEvent) {
        let Some(mut form) = self.strichliste.form.take() else {
            return;
        };
        if key.code == KeyCode::Esc {
            self.strichliste.message = "Form cancelled".into();
            return;
        }
        match form.step {
            Step::Loading => {
                form.input.handle_event(&InputEvent::Key(key));
            }
            Step::Recipients => {
                let count = form.recipients.len();
                let selected = form.selection.selected().unwrap_or(0);
                let index = match key.code {
                    KeyCode::Up => selected.saturating_sub(1),
                    KeyCode::Down => (selected + 1).min(count.saturating_sub(1)),
                    KeyCode::PageUp => selected.saturating_sub(10),
                    KeyCode::PageDown => (selected + 10).min(count.saturating_sub(1)),
                    KeyCode::Home => 0,
                    KeyCode::End => count.saturating_sub(1),
                    _ => selected,
                };
                form.selection.select((count > 0).then_some(index));
                match key.code {
                    KeyCode::Char('/') => {
                        form.input = Input::default();
                        form.filter_recipients();
                    }
                    KeyCode::Enter if count > 0 => {
                        form.recipient = form.recipients.get(index).cloned();
                        form.step = Step::Amount;
                        form.input = Input::default();
                    }
                    _ => {
                        let before = form.input.value().to_owned();
                        form.input.handle_event(&InputEvent::Key(key));
                        if form.input.value() != before {
                            form.filter_recipients();
                        }
                    }
                }
            }
            _ if key.code == KeyCode::Enter => match form.step {
                Step::Search => {
                    form.step = Step::Loading;
                    self.strichliste.form = Some(form);
                    self.send_strichliste(Command::LoadRecipients);
                    return;
                }
                Step::Amount => match parse_amount(form.input.value()) {
                    Ok(amount) => {
                        form.amount = Some(amount);
                        form.input = Input::default();
                        form.step = Step::Comment;
                        self.strichliste.error = None;
                    }
                    Err(error) => self.strichliste.error = Some(error),
                },
                Step::Comment => {
                    let Some(amount) = form.amount else { return };
                    let comment = form.input.value().to_owned();
                    let command = match form.kind {
                        Kind::Pay => Command::Book {
                            request: Request::Pay(amount),
                            comment,
                        },
                        Kind::Add => Command::Book {
                            request: Request::Add(amount),
                            comment,
                        },
                        Kind::Send => {
                            let Some(recipient) = form.recipient else {
                                return;
                            };
                            Command::Send {
                                recipient,
                                amount,
                                comment,
                            }
                        }
                    };
                    if self.safe_mode {
                        self.popup = Some(Popup::ConfirmCommand {
                            pending: PendingCommand::Strichliste(command),
                            selected: ConfirmChoice::Cancel,
                        });
                    } else {
                        self.send_strichliste(command);
                    }
                    return;
                }
                _ => {}
            },
            _ => {
                form.input.handle_event(&InputEvent::Key(key));
            }
        }
        self.strichliste.form = Some(form);
    }

    pub(super) fn send_strichliste(&mut self, command: Command) {
        if self.strichliste.busy {
            self.set_status("Strichliste is busy", StatusKind::Info);
            return;
        }
        let command =
            if matches!(command, Command::Refresh) && self.strichliste.transaction_offset > 0 {
                Command::RefreshAt(self.strichliste.transaction_offset)
            } else {
                command
            };
        if command.is_refresh() {
            self.strichliste.last_refresh = Some(Instant::now());
        }
        let error = command.validate().err().map(|error| error.to_string());
        let message = command.preview_lines().join(" | ");
        let error = error.or_else(|| {
            (!self
                .strichliste
                .tx
                .as_ref()
                .is_some_and(|tx| tx.send(command.clone()).is_ok()))
            .then(|| "Strichliste worker is not running".to_owned())
        });
        if let Some(error) = error {
            self.strichliste.error = Some(error.clone());
            if let Some(form) = &mut self.strichliste.form {
                if form.step == Step::Loading {
                    form.step = Step::Search;
                }
            }
            self.set_status(error, StatusKind::Error);
            return;
        }
        if !self.options.dry_run && matches!(command, Command::Book { .. } | Command::Send { .. }) {
            self.strichliste.balance_stale = true;
            self.strichliste.rank_stale = true;
            self.strichliste.transactions_stale = true;
        }
        self.strichliste.busy = true;
        self.strichliste.error = None;
        self.strichliste.message = message.clone();
        self.traffic.push(traffic(
            if self.options.dry_run {
                TrafficDirection::Dry
            } else {
                TrafficDirection::Tx
            },
            message,
        ));
    }

    pub(super) fn handle_strichliste_event(&mut self, event: Event) {
        self.strichliste.busy = false;
        if event.command.is_refresh() {
            self.strichliste.last_refresh = Some(Instant::now());
        }
        self.strichliste.error = None;
        let mut refresh = false;
        let message = match event.result {
            Ok(Response::Account {
                balance,
                rank,
                transactions,
            }) => {
                let mut errors = Vec::new();
                self.strichliste.balance_stale = balance.is_err();
                self.strichliste.rank_stale = rank.is_err();
                self.strichliste.transactions_stale = transactions.is_err();
                match transactions {
                    Ok(page) => self.set_transaction_page(page),
                    Err(error) => errors.push(error),
                }
                match balance {
                    Ok(value) => self.strichliste.balance = Some(value),
                    Err(error) => errors.push(error),
                }
                match rank {
                    Ok(value) => self.strichliste.rank = Some(value),
                    Err(error) => errors.push(error),
                }
                if errors.is_empty() {
                    "Account refreshed".into()
                } else {
                    self.strichliste.error = Some(errors.join("; "));
                    "Refresh incomplete; previous values retained".into()
                }
            }
            Ok(Response::Transactions(page)) => {
                self.set_transaction_page(page);
                "Transactions loaded".into()
            }
            Ok(Response::Recipients(users)) => {
                if let Some(form) = &mut self.strichliste.form {
                    if form.step == Step::Loading {
                        form.step = Step::Recipients;
                        form.all_recipients = users;
                        form.filter_recipients();
                    }
                }
                "Type to filter recipients; Up/Down select, Enter continues".into()
            }
            Ok(Response::Submitted) => {
                refresh = true;
                "Transaction submitted".into()
            }
            Ok(Response::DryRun) => {
                if let Some(form) = &mut self.strichliste.form {
                    if form.step == Step::Loading {
                        form.step = Step::Search;
                    }
                }
                format!("dry-run: {}", event.command.preview_lines().join(" | "))
            }
            Err(error) => {
                if matches!(event.command, Command::Book { .. } | Command::Send { .. })
                    && self.popup.is_none()
                {
                    self.popup = Some(Popup::StrichlisteError {
                        message: error.clone(),
                    });
                }
                if event.command.is_refresh() {
                    self.strichliste.balance_stale = true;
                    self.strichliste.rank_stale = true;
                    self.strichliste.transactions_stale = true;
                }
                if matches!(event.command, Command::Transactions(_)) {
                    self.strichliste.transactions_stale = true;
                }
                if let Some(form) = &mut self.strichliste.form {
                    if form.step == Step::Loading {
                        form.step = Step::Search;
                    }
                }
                self.strichliste.error = Some(error.clone());
                error
            }
        };
        self.strichliste.message = message.clone();
        self.traffic.push(traffic(
            if self.strichliste.error.is_some() {
                TrafficDirection::Error
            } else {
                TrafficDirection::Rx
            },
            message.clone(),
        ));
        self.set_status(
            message,
            if self.strichliste.error.is_some() {
                StatusKind::Error
            } else {
                StatusKind::Info
            },
        );
        if refresh {
            self.strichliste.transaction_offset = 0;
            self.strichliste.transaction_previous.clear();
            self.send_strichliste(Command::Refresh);
        }
    }

    fn set_transaction_page(&mut self, page: TransactionPage) {
        let state = &mut self.strichliste;
        let previous_id = (page.offset == state.transaction_offset)
            .then(|| {
                state
                    .transaction_selection
                    .selected()
                    .and_then(|index| state.transactions.as_ref()?.get(index))
                    .map(|transaction| transaction.id)
            })
            .flatten();
        if page.offset > state.transaction_offset {
            state.transaction_previous.push(state.transaction_offset);
        } else if page.offset < state.transaction_offset {
            state
                .transaction_previous
                .retain(|offset| *offset < page.offset);
        }
        let selected = previous_id
            .and_then(|id| {
                page.entries
                    .iter()
                    .position(|transaction| transaction.id == id)
            })
            .or_else(|| {
                (!page.entries.is_empty()
                    || page.next_offset.is_some()
                    || !state.transaction_previous.is_empty())
                .then_some(0)
            });
        if page.offset != state.transaction_offset {
            state.transaction_selection = ListState::default();
        }
        state.transaction_selection.select(selected);
        state.transaction_offset = page.offset;
        state.transaction_total = page.total;
        state.transaction_next = page.next_offset;
        state.transactions_stale = false;
        state.transactions = Some(page.entries);
    }

    #[cfg(feature = "tui-mouse")]
    pub(super) fn handle_strichliste_click(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        area: ratatui::layout::Rect,
        double_click: bool,
    ) {
        let rows = super::views::strichliste::areas(area);
        let list = if self.strichliste.form.is_some() {
            super::views::strichliste::search_areas(rows[2])[1]
        } else {
            rows[2]
        };
        if mouse.row <= list.y || mouse.row >= list.bottom().saturating_sub(1) {
            return;
        }
        let row = mouse.row.saturating_sub(list.y + 1) as usize;
        if self.strichliste.show_transactions {
            let index = self.strichliste.transaction_selection.offset() + row;
            let count = self.strichliste.transactions.as_ref().map_or(0, Vec::len)
                + usize::from(self.strichliste.transaction_next.is_some())
                + usize::from(!self.strichliste.transaction_previous.is_empty());
            if index < count {
                self.strichliste.transaction_selection.select(Some(index));
                if double_click {
                    self.handle_strichliste_key(KeyEvent::from(KeyCode::Enter));
                }
            }
            return;
        }
        if let Some(form) = &mut self.strichliste.form {
            if form.step != Step::Recipients {
                return;
            }
            let index = form.selection.offset() + row;
            if index >= form.recipients.len() {
                return;
            }
            form.selection.select(Some(index));
            if double_click {
                self.handle_strichliste_input(KeyEvent::from(KeyCode::Enter));
            }
        } else if row < ACTIONS.len() {
            self.strichliste.focus = row;
            if double_click {
                self.handle_strichliste_key(KeyEvent::from(KeyCode::Enter));
            }
        }
    }
}

fn traffic(direction: TrafficDirection, payload: String) -> TrafficEntry {
    TrafficEntry {
        at: now_hms(),
        direction,
        transport: "http",
        target: "Strichliste".into(),
        payload,
        note: None,
        sections: Section::Overview.bit() | Section::Strichliste.bit(),
    }
}

#[cfg(test)]
mod tests;
