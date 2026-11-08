//! TUI state, key precedence and message handling.

use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::style::Color;
use throbber_widgets_tui::ThrobberState;
use tokio::sync::mpsc::UnboundedSender;
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;
use tui_logger::TuiWidgetState;

use crate::action::{
    Action, ControlCommand, DmxAnimation, DmxGroup, Outlet, RadiationTarget, SmoothieRelay,
    SobSound, TOPIC_ALERT_CONDITION, TOPIC_KNOB_GET_PREFIX, TOPIC_KNOB_SET_PREFIX,
    TOPIC_SMOOTHIE_PREFIX, TOPIC_WHITE_LIGHT_STATUS_PREFIX, Transport,
};
use crate::cli::TuiView;
use crate::color::{Theme, ThemePreset, ThemeRole};
use crate::decode::{AlertCondition, AlertState, parse_alert, parse_bool};
use crate::knob::{KNOB_SPECS, Knob, format_value};
use crate::mqtt::{MqttCommand, MqttEvent};
use crate::smoothiesaur::{HttpCommand, HttpEvent, SmoothieStatus};
use crate::tui::TuiOptions;
use crate::tui::safe_mode::{ConfirmChoice, PendingCommand};
use crate::tui::theme::{ColorPicker, PickerOutcome};
use crate::tui::traffic::{TrafficDirection, TrafficEntry, TrafficLog, sections_for_action};
use crate::white_light;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Section {
    Overview,
    Lighting,
    WhiteLight,
    Outlets,
    Alerts,
    System,
    Animation,
    Smoothie,
    Theme,
    Pizzamat,
}

impl Section {
    pub const ALL: [Self; 10] = [
        Self::Overview,
        Self::Lighting,
        Self::WhiteLight,
        Self::Outlets,
        Self::Alerts,
        Self::System,
        Self::Animation,
        Self::Smoothie,
        Self::Theme,
        Self::Pizzamat,
    ];

    pub fn index(self) -> usize {
        match self {
            Self::Overview => 0,
            Self::Lighting => 1,
            Self::WhiteLight => 2,
            Self::Outlets => 3,
            Self::Alerts => 4,
            Self::System => 5,
            Self::Animation => 6,
            Self::Smoothie => 7,
            Self::Theme => 8,
            Self::Pizzamat => 9,
        }
    }

    pub fn bit(self) -> u16 {
        1u16 << self.index()
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Lighting => "Lighting",
            Self::WhiteLight => "White Light",
            Self::Outlets => "Outlets",
            Self::Alerts => "Alerts",
            Self::System => "System",
            Self::Animation => "Animation",
            Self::Smoothie => "Smoothiesaur",
            Self::Theme => "Theme",
            Self::Pizzamat => "Pizzamat",
        }
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn from_tui_view(view: TuiView) -> Self {
        match view {
            TuiView::Overview => Self::Overview,
            TuiView::Lighting => Self::Lighting,
            TuiView::WhiteLight => Self::WhiteLight,
            TuiView::Outlets => Self::Outlets,
            TuiView::Alerts => Self::Alerts,
            TuiView::System => Self::System,
            TuiView::Animation => Self::Animation,
            TuiView::Smoothie => Self::Smoothie,
            TuiView::Theme => Self::Theme,
            TuiView::Pizzamat => Self::Pizzamat,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FocusZone {
    Sidebar,
    Content,
    Traffic,
    AppLog,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StatusKind {
    Info,
    Success,
    Error,
}

#[derive(Clone, Debug)]
pub struct StatusMessage {
    pub text: String,
    pub kind: StatusKind,
    pub at: Instant,
}

#[derive(Copy, Clone, Debug)]
pub enum ConnectionState {
    Disconnected,
    Connecting { attempt: u32 },
    Connected { since: Instant },
    Reconnecting { attempt: u32, retry_at: Instant },
}

impl ConnectionState {
    pub fn is_connected(self) -> bool {
        matches!(self, Self::Connected { .. })
    }

    pub fn is_busy(self) -> bool {
        matches!(self, Self::Connecting { .. } | Self::Reconnecting { .. })
    }

    pub fn label(self, now: Instant) -> String {
        match self {
            Self::Disconnected => "disconnected".to_string(),
            Self::Connecting { attempt } => format!("connecting (attempt {attempt})"),
            Self::Connected { since } => {
                let uptime = now.saturating_duration_since(since).as_secs();
                format!("connected ({uptime}s)")
            }
            Self::Reconnecting { attempt, retry_at } => {
                let remaining = retry_at.saturating_duration_since(now).as_secs();
                format!("reconnecting in {remaining}s (attempt {attempt})")
            }
        }
    }
}

pub enum Popup {
    Quit {
        selected: ConfirmChoice,
    },
    SafeModeOff {
        selected: ConfirmChoice,
    },
    ConfirmCommand {
        pending: PendingCommand,
        selected: ConfirmChoice,
    },
    Help {
        scroll: u16,
    },
    TrafficDetail {
        entry: TrafficEntry,
    },
    ColorPicker {
        picker: ColorPicker,
    },
    ThemeReset {
        selected: ConfirmChoice,
    },
}

#[derive(Default)]
pub struct OverviewState {
    pub focus: usize,
}

pub struct LightingState {
    pub group: usize,
    pub animation: usize,
    pub filter: String,
}

pub struct WhiteLightState {
    pub warm: u8,
    pub cold: u8,
    pub selected: [bool; 15],
    pub focus: usize,
    pub filter: String,
    pub status: [Option<bool>; 15],
}

pub struct OutletsState {
    pub row: usize,
    pub action: usize,
    pub last_sent: [Option<bool>; Outlet::ALL.len()],
}

#[derive(Default)]
pub struct AlertsState {
    pub focus: usize,
}

#[derive(Default)]
pub struct SystemState {
    pub focus: usize,
}

pub struct AnimationState {
    pub focus: usize,
    pub values: [Option<f64>; 29],
    pub committed: [Option<f64>; 29],
    pub dirty: [bool; 29],
    pub filter: String,
}

pub struct SmoothieState {
    pub focus: usize,
    pub relay_choice: [bool; 7],
    pub laser: u8,
    pub status: SmoothieStatus,
    pub last_http: Option<String>,
    pub http_error: Option<String>,
}

#[derive(Default)]
pub struct ThemeViewState {
    pub focus: usize,
}

pub enum TextInputMode {
    Pizza {
        operation: Option<crate::pizzamat::Operation>,
        fields: Vec<String>,
        input: Input,
    },
    Search {
        section: Section,
        input: Input,
    },
    ExactKnob {
        knob: Knob,
        input: Input,
    },
    ExactWarm {
        input: Input,
    },
    ExactCold {
        input: Input,
    },
    ExactLaser {
        input: Input,
    },
}

pub struct TextInputState {
    pub mode: TextInputMode,
}

pub struct App {
    pub options: TuiOptions,
    pub mqtt_tx: UnboundedSender<MqttCommand>,
    pub http_tx: UnboundedSender<HttpCommand>,
    pub pizza_tx: UnboundedSender<crate::pizzamat::Request>,
    pub pizzamat: super::pizzamat::State,
    pub should_quit: bool,
    pub section: Section,
    pub zone: FocusZone,
    pub popup: Option<Popup>,
    pub connection: ConnectionState,
    pub alert: AlertState,
    pub safe_mode: bool,
    pub theme: Theme,
    pub theme_preset: ThemePreset,
    pub show_log: bool,
    pub log_height_pct: u16,
    pub log_state: TuiWidgetState,
    pub throbber: ThrobberState,
    pub traffic: TrafficLog,
    pub traffic_selected: Option<usize>,
    pub status: Option<StatusMessage>,
    pub mqtt_generation: u64,
    pub overview: OverviewState,
    pub lighting: LightingState,
    pub white_light: WhiteLightState,
    pub outlets: OutletsState,
    pub alerts: AlertsState,
    pub system: SystemState,
    pub animation: AnimationState,
    pub smoothie: SmoothieState,
    pub theme_view: ThemeViewState,
    pub text_input: Option<TextInputState>,
}

impl App {
    pub fn new(
        options: TuiOptions,
        mqtt_tx: UnboundedSender<MqttCommand>,
        http_tx: UnboundedSender<HttpCommand>,
        pizza_tx: UnboundedSender<crate::pizzamat::Request>,
    ) -> Self {
        let section = Section::from_tui_view(options.view);
        let connect = options.connect;
        let safe_mode = options.safe_mode;
        let theme = options.theme.clone();
        let theme_preset = options.theme_preset;

        let mut app = Self {
            options,
            mqtt_tx,
            http_tx,
            pizza_tx,
            pizzamat: super::pizzamat::State::default(),
            should_quit: false,
            section,
            zone: FocusZone::Content,
            popup: None,
            connection: ConnectionState::Disconnected,
            alert: AlertState {
                condition: AlertCondition::Unknown,
                dim: false,
            },
            safe_mode,
            theme,
            theme_preset,
            show_log: false,
            log_height_pct: 35,
            log_state: TuiWidgetState::new(),
            throbber: ThrobberState::default(),
            traffic: TrafficLog::default(),
            traffic_selected: None,
            status: Some(StatusMessage {
                text: "starting…".to_string(),
                kind: StatusKind::Info,
                at: Instant::now(),
            }),
            mqtt_generation: 0,
            overview: OverviewState { focus: 0 },
            lighting: LightingState {
                group: 0,
                animation: 0,
                filter: String::new(),
            },
            white_light: WhiteLightState {
                warm: 0,
                cold: 0,
                selected: [false; 15],
                focus: 0,
                filter: String::new(),
                status: [None; 15],
            },
            outlets: OutletsState {
                row: 0,
                action: 0,
                last_sent: [None; Outlet::ALL.len()],
            },
            alerts: AlertsState::default(),
            system: SystemState::default(),
            animation: AnimationState {
                focus: 0,
                values: [None; 29],
                committed: [None; 29],
                dirty: [false; 29],
                filter: String::new(),
            },
            smoothie: SmoothieState {
                focus: 0,
                relay_choice: [true; 7],
                laser: 0,
                status: SmoothieStatus::default(),
                last_http: None,
                http_error: None,
            },
            theme_view: ThemeViewState::default(),
            text_input: None,
        };

        if connect {
            if app.mqtt_tx.send(MqttCommand::Connect).is_err() {
                app.set_status("MQTT supervisor is not running", StatusKind::Error);
            } else {
                app.connection = ConnectionState::Connecting { attempt: 0 };
                app.set_status("connecting…", StatusKind::Info);
            }
        } else {
            app.set_status(
                "disconnected — press c to connect, ? for help",
                StatusKind::Info,
            );
        }

        app
    }

    pub fn set_status(&mut self, text: impl Into<String>, kind: StatusKind) {
        self.status = Some(StatusMessage {
            text: text.into(),
            kind,
            at: Instant::now(),
        });
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        // Free-form food and names must accept q/Q and all global shortcuts.
        // Ctrl+C still exits; Escape cancels the current form.
        if !ctrl
            && matches!(
                self.text_input.as_ref().map(|state| &state.mode),
                Some(TextInputMode::Pizza { .. })
            )
        {
            self.handle_text_input_key(key);
            return;
        }
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let upper_q = matches!(key.code, KeyCode::Char('Q'))
            || (matches!(key.code, KeyCode::Char('q')) && shift);
        if upper_q || (ctrl && matches!(key.code, KeyCode::Char('c'))) {
            self.should_quit = true;
            return;
        }

        if self.popup.is_some() {
            self.handle_popup_key(key);
            return;
        }

        if ctrl {
            match key.code {
                KeyCode::Char('l') => {
                    self.text_input = None;
                    self.toggle_log();
                    return;
                }
                KeyCode::Char('t') => {
                    self.text_input = None;
                    self.zone = if self.zone == FocusZone::Traffic {
                        FocusZone::Content
                    } else {
                        FocusZone::Traffic
                    };
                    return;
                }
                KeyCode::Char('g') => {
                    self.text_input = None;
                    if self.zone == FocusZone::AppLog {
                        self.zone = FocusZone::Content;
                    } else {
                        self.show_log = true;
                        self.zone = FocusZone::AppLog;
                    }
                    return;
                }
                KeyCode::Up if self.show_log && self.zone == FocusZone::AppLog => {
                    self.text_input = None;
                    self.log_height_pct = (self.log_height_pct + 5).min(50);
                    return;
                }
                KeyCode::Down if self.show_log && self.zone == FocusZone::AppLog => {
                    self.text_input = None;
                    self.log_height_pct = self.log_height_pct.saturating_sub(5).max(20);
                    return;
                }
                _ => {}
            }
        }

        if self.text_input.is_some() {
            self.handle_text_input_key(key);
            return;
        }

        // In the white-light view, w/c focus the warm/cold sliders. `c` would
        // otherwise be captured by the global connect shortcut.
        if self.zone == FocusZone::Content && self.section == Section::WhiteLight {
            match key.code {
                KeyCode::Char('w') => {
                    self.white_light.focus = 0;
                    return;
                }
                KeyCode::Char('c') => {
                    self.white_light.focus = 1;
                    return;
                }
                _ => {}
            }
        }

        // These actions are available from every focus zone.
        match key.code {
            KeyCode::Char('?') => {
                self.popup = Some(Popup::Help { scroll: 0 });
                return;
            }
            KeyCode::Char('S') => {
                self.toggle_safe_mode();
                return;
            }
            KeyCode::Char('c') => {
                self.request_connect();
                return;
            }
            KeyCode::Char('d') => {
                self.request_disconnect();
                return;
            }
            KeyCode::Char('r') => {
                self.request_reconnect();
                return;
            }
            KeyCode::Char('[') => {
                self.change_section(self.section.prev());
                return;
            }
            KeyCode::Char(']') => {
                self.change_section(self.section.next());
                return;
            }
            KeyCode::Tab => {
                self.cycle_zone(true);
                return;
            }
            KeyCode::BackTab => {
                self.cycle_zone(false);
                return;
            }
            _ => {}
        }

        if self.zone == FocusZone::AppLog {
            self.handle_log_key(key);
            return;
        }
        if self.zone == FocusZone::Traffic {
            self.handle_traffic_key(key);
            return;
        }

        match key.code {
            KeyCode::Char('q') => {
                self.go_back();
                return;
            }
            KeyCode::Esc => {
                self.escape();
                return;
            }
            _ => {}
        }

        match self.zone {
            FocusZone::Sidebar => self.handle_sidebar_key(key),
            FocusZone::Content => self.handle_content_key(key),
            FocusZone::Traffic | FocusZone::AppLog => {}
        }
    }

    fn handle_popup_key(&mut self, key: KeyEvent) {
        let Some(popup) = self.popup.take() else {
            return;
        };

        if matches!(key.code, KeyCode::Char('q')) {
            if let Popup::ColorPicker { picker } = &popup {
                self.restore_picker_color(picker);
            }
            self.set_status("cancelled", StatusKind::Info);
            return;
        }

        let mut next = None;
        match popup {
            Popup::Quit { mut selected } => {
                if matches!(key.code, KeyCode::Tab | KeyCode::Left | KeyCode::Right) {
                    selected = selected.toggle();
                    next = Some(Popup::Quit { selected });
                } else if matches!(key.code, KeyCode::Char('y'))
                    || (matches!(key.code, KeyCode::Enter) && selected.is_confirm())
                {
                    self.should_quit = true;
                } else if matches!(key.code, KeyCode::Char('n') | KeyCode::Esc)
                    || (matches!(key.code, KeyCode::Enter) && !selected.is_confirm())
                {
                    self.set_status("quit cancelled", StatusKind::Info);
                } else {
                    next = Some(Popup::Quit { selected });
                }
            }
            Popup::SafeModeOff { mut selected } => {
                if matches!(key.code, KeyCode::Tab | KeyCode::Left | KeyCode::Right) {
                    selected = selected.toggle();
                    next = Some(Popup::SafeModeOff { selected });
                } else if matches!(key.code, KeyCode::Char('y'))
                    || (matches!(key.code, KeyCode::Enter) && selected.is_confirm())
                {
                    self.safe_mode = false;
                    self.set_status("safe mode disabled", StatusKind::Error);
                } else if matches!(key.code, KeyCode::Char('n') | KeyCode::Esc)
                    || (matches!(key.code, KeyCode::Enter) && !selected.is_confirm())
                {
                    self.set_status("safe mode remains enabled", StatusKind::Info);
                } else {
                    next = Some(Popup::SafeModeOff { selected });
                }
            }
            Popup::ConfirmCommand {
                pending,
                mut selected,
            } => {
                if matches!(key.code, KeyCode::Tab | KeyCode::Left | KeyCode::Right) {
                    selected = selected.toggle();
                    next = Some(Popup::ConfirmCommand { pending, selected });
                } else if matches!(key.code, KeyCode::Char('y'))
                    || (matches!(key.code, KeyCode::Enter) && selected.is_confirm())
                {
                    self.send_actions(pending.actions);
                } else if matches!(key.code, KeyCode::Char('n') | KeyCode::Esc)
                    || (matches!(key.code, KeyCode::Enter) && !selected.is_confirm())
                {
                    self.set_status("command cancelled", StatusKind::Info);
                } else {
                    next = Some(Popup::ConfirmCommand { pending, selected });
                }
            }
            Popup::Help { mut scroll } => match key.code {
                KeyCode::Esc | KeyCode::Char('?') => {}
                KeyCode::Up => {
                    scroll = scroll.saturating_sub(1);
                    next = Some(Popup::Help { scroll });
                }
                KeyCode::Down => {
                    scroll = scroll.saturating_add(1);
                    next = Some(Popup::Help { scroll });
                }
                KeyCode::PageUp => {
                    scroll = scroll.saturating_sub(10);
                    next = Some(Popup::Help { scroll });
                }
                KeyCode::PageDown => {
                    scroll = scroll.saturating_add(10);
                    next = Some(Popup::Help { scroll });
                }
                _ => next = Some(Popup::Help { scroll }),
            },
            Popup::TrafficDetail { entry } => match key.code {
                KeyCode::Esc | KeyCode::Enter => {}
                _ => next = Some(Popup::TrafficDetail { entry }),
            },
            Popup::ColorPicker { mut picker } => match picker.handle_key(key) {
                PickerOutcome::Changed => {
                    self.theme.set(picker.role, picker.current_color());
                    next = Some(Popup::ColorPicker { picker });
                }
                PickerOutcome::Reset => {
                    self.theme.set(picker.role, picker.default_color);
                    self.set_status(
                        format!("theme {} reset", picker.role.title()),
                        StatusKind::Info,
                    );
                }
                PickerOutcome::Apply => {
                    self.theme.set(picker.role, picker.current_color());
                    self.set_status(
                        format!("theme {} updated", picker.role.title()),
                        StatusKind::Success,
                    );
                }
                PickerOutcome::Cancel => {
                    self.restore_picker_color(&picker);
                    self.set_status("color change cancelled", StatusKind::Info);
                }
                PickerOutcome::None => {
                    next = Some(Popup::ColorPicker { picker });
                }
            },
            Popup::ThemeReset { mut selected } => {
                if matches!(key.code, KeyCode::Tab | KeyCode::Left | KeyCode::Right) {
                    selected = selected.toggle();
                    next = Some(Popup::ThemeReset { selected });
                } else if matches!(key.code, KeyCode::Char('y'))
                    || (matches!(key.code, KeyCode::Enter) && selected.is_confirm())
                {
                    self.theme = Theme::preset(self.theme_preset);
                    self.set_status("theme reset", StatusKind::Success);
                } else if matches!(key.code, KeyCode::Char('n') | KeyCode::Esc)
                    || (matches!(key.code, KeyCode::Enter) && !selected.is_confirm())
                {
                    self.set_status("theme reset cancelled", StatusKind::Info);
                } else {
                    next = Some(Popup::ThemeReset { selected });
                }
            }
        }

        self.popup = next;
    }

    fn restore_picker_color(&mut self, picker: &ColorPicker) {
        self.theme.set(picker.role, picker.original_color);
    }

    fn handle_text_input_key(&mut self, key: KeyEvent) {
        let Some(mut state) = self.text_input.take() else {
            return;
        };

        // Pizzamat accepts free-form names and food, including q/Q.
        // Other inputs follow the global "q closes the current view" rule.
        if matches!(key.code, KeyCode::Char('q'))
            && !matches!(state.mode, TextInputMode::Pizza { .. })
        {
            if let TextInputMode::Search { section, .. } = &state.mode {
                self.clear_filter(*section);
            }
            return;
        }

        match &mut state.mode {
            TextInputMode::Pizza {
                operation,
                fields,
                input,
            } => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    let value = input.value().trim().to_string();
                    if value.is_empty() || value.chars().any(char::is_control) || value.len() > 4096
                    {
                        self.set_status(
                            "Enter a nonempty value (maximum 4096 bytes, no control characters)",
                            StatusKind::Error,
                        );
                        self.text_input = Some(state);
                    } else if let Some(operation) = *operation {
                        if operation == crate::pizzamat::Operation::SetDefault
                            && fields.is_empty()
                            && value.contains(char::is_whitespace)
                        {
                            self.set_status(
                                "Default-order service must be a single word",
                                StatusKind::Error,
                            );
                            self.text_input = Some(state);
                            return;
                        }
                        fields.push(value);
                        if fields.len() < operation.fields().len() {
                            *input = Input::default();
                            self.text_input = Some(state);
                        } else {
                            match crate::pizzamat::Request::new(
                                operation,
                                &self.options.pizzamat.user,
                                fields,
                            ) {
                                Ok(request) => self.send_pizzamat(request),
                                Err(error) => {
                                    self.set_status(error.to_string(), StatusKind::Error);
                                    fields.pop();
                                    self.text_input = Some(state);
                                }
                            }
                        }
                    } else {
                        self.options.pizzamat.user = value;
                        self.set_status("Pizzamat identity updated", StatusKind::Info);
                    }
                }
                _ => {
                    input.handle_event(&Event::Key(key));
                    self.text_input = Some(state);
                }
            },
            TextInputMode::Search { section, input } => match key.code {
                KeyCode::Esc => {
                    self.clear_filter(*section);
                }
                KeyCode::Enter => {
                    self.apply_search_filter(*section, input.value());
                }
                _ => {
                    input.handle_event(&Event::Key(key));
                    self.apply_search_filter(*section, input.value());
                    self.text_input = Some(state);
                }
            },
            TextInputMode::ExactKnob { knob, input } => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    if let Ok(value) = input.value().trim().parse::<f64>() {
                        self.set_knob_value(*knob, value);
                    }
                }
                _ => {
                    input.handle_event(&Event::Key(key));
                    self.text_input = Some(state);
                }
            },
            TextInputMode::ExactWarm { input } => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    if let Ok(value) = input.value().trim().parse::<f64>() {
                        self.white_light.warm = value.round().clamp(0.0, 255.0) as u8;
                    }
                }
                _ => {
                    input.handle_event(&Event::Key(key));
                    self.text_input = Some(state);
                }
            },
            TextInputMode::ExactCold { input } => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    if let Ok(value) = input.value().trim().parse::<f64>() {
                        self.white_light.cold = value.round().clamp(0.0, 255.0) as u8;
                    }
                }
                _ => {
                    input.handle_event(&Event::Key(key));
                    self.text_input = Some(state);
                }
            },
            TextInputMode::ExactLaser { input } => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    if let Ok(value) = input.value().trim().parse::<f64>() {
                        self.smoothie.laser = value.round().clamp(0.0, 255.0) as u8;
                    }
                }
                _ => {
                    input.handle_event(&Event::Key(key));
                    self.text_input = Some(state);
                }
            },
        }
    }

    fn handle_log_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.show_log = false;
                self.zone = FocusZone::Content;
            }
            KeyCode::Char('t') => {
                tracing::warn!("test log line");
                self.set_status("test log line emitted", StatusKind::Info);
            }
            _ => {}
        }
    }

    fn handle_traffic_key(&mut self, key: KeyEvent) {
        let len = self.traffic.filtered(self.section).len();
        if len == 0 {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                self.zone = FocusZone::Content;
            }
            return;
        }

        match key.code {
            KeyCode::Up => {
                self.traffic_selected = Some(match self.traffic_selected {
                    Some(index) => index.saturating_sub(1),
                    None => len - 1,
                });
            }
            KeyCode::Down => {
                self.traffic_selected = Some(match self.traffic_selected {
                    Some(index) => (index + 1).min(len - 1),
                    None => 0,
                });
            }
            KeyCode::PageUp => {
                self.traffic_selected = Some(
                    self.traffic_selected
                        .unwrap_or(len)
                        .saturating_sub(5)
                        .min(len - 1),
                );
            }
            KeyCode::PageDown => {
                self.traffic_selected = Some((self.traffic_selected.unwrap_or(0) + 5).min(len - 1));
            }
            KeyCode::Char('g') => {
                self.traffic_selected = Some(0);
            }
            KeyCode::Char('G') => {
                self.traffic_selected = Some(len - 1);
            }
            KeyCode::Enter => {
                if let Some(index) = self.traffic_selected {
                    let entry = self
                        .traffic
                        .filtered(self.section)
                        .get(index)
                        .copied()
                        .cloned();
                    if let Some(entry) = entry {
                        self.popup = Some(Popup::TrafficDetail { entry });
                    }
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.zone = FocusZone::Content;
                self.traffic_selected = None;
            }
            _ => {}
        }
    }

    fn handle_sidebar_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => self.change_section(self.section.prev()),
            KeyCode::Down => self.change_section(self.section.next()),
            KeyCode::Home => self.change_section(Section::ALL[0]),
            KeyCode::End => self.change_section(Section::ALL[Section::ALL.len() - 1]),
            KeyCode::Enter => self.zone = FocusZone::Content,
            _ => {}
        }
    }

    fn handle_content_key(&mut self, key: KeyEvent) {
        match self.section {
            Section::Overview => self.handle_overview_key(key),
            Section::Lighting => self.handle_lighting_key(key),
            Section::WhiteLight => self.handle_white_light_key(key),
            Section::Outlets => self.handle_outlets_key(key),
            Section::Alerts => self.handle_alerts_key(key),
            Section::System => self.handle_system_key(key),
            Section::Animation => self.handle_animation_key(key),
            Section::Smoothie => self.handle_smoothie_key(key),
            Section::Theme => self.handle_theme_key(key),
            Section::Pizzamat => self.handle_pizzamat_key(key),
        }
    }

    fn handle_overview_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => {
                self.overview.focus = self
                    .overview
                    .focus
                    .checked_sub(1)
                    .unwrap_or(Section::ALL.len() - 1);
            }
            KeyCode::Down => {
                self.overview.focus = (self.overview.focus + 1) % Section::ALL.len();
            }
            KeyCode::Enter => {
                self.change_section(Section::ALL[self.overview.focus]);
            }
            _ => {}
        }
    }

    fn handle_lighting_key(&mut self, key: KeyEvent) {
        let visible = self.visible_lighting_groups();
        match key.code {
            KeyCode::Up => {
                self.lighting.group = next_in(&visible, self.lighting.group, false);
            }
            KeyCode::Down => {
                self.lighting.group = next_in(&visible, self.lighting.group, true);
            }
            KeyCode::Home => {
                if let Some(first) = visible.first() {
                    self.lighting.group = *first;
                }
            }
            KeyCode::End => {
                if let Some(last) = visible.last() {
                    self.lighting.group = *last;
                }
            }
            KeyCode::Left => {
                self.lighting.animation = (self.lighting.animation + DmxAnimation::ALL.len() - 1)
                    % DmxAnimation::ALL.len();
            }
            KeyCode::Right => {
                self.lighting.animation = (self.lighting.animation + 1) % DmxAnimation::ALL.len();
            }
            KeyCode::Enter => {
                if visible.is_empty() {
                    self.set_status("no matching groups", StatusKind::Info);
                } else {
                    self.send_dmx();
                }
            }
            KeyCode::Char('/') => self.start_search(),
            _ => {}
        }
    }

    fn handle_white_light_key(&mut self, key: KeyEvent) {
        let items = self.white_light_focus_items();
        match key.code {
            KeyCode::Up => {
                self.white_light.focus = next_in(&items, self.white_light.focus, false);
            }
            KeyCode::Down => {
                self.white_light.focus = next_in(&items, self.white_light.focus, true);
            }
            KeyCode::Left => {
                self.adjust_white_light(if key.modifiers.contains(KeyModifiers::SHIFT) {
                    -16
                } else {
                    -1
                })
            }
            KeyCode::Right => {
                self.adjust_white_light(if key.modifiers.contains(KeyModifiers::SHIFT) {
                    16
                } else {
                    1
                })
            }
            KeyCode::Home => self.set_white_light_bound(false),
            KeyCode::End => self.set_white_light_bound(true),
            KeyCode::Char('a') => self.white_light.selected = [true; 15],
            KeyCode::Char('n') => self.white_light.selected = [false; 15],
            KeyCode::Char('/') => self.start_search(),
            KeyCode::Char(' ') => self.toggle_white_light_focus(),
            KeyCode::Enter => match self.white_light.focus {
                0 => {
                    self.text_input = Some(TextInputState {
                        mode: TextInputMode::ExactWarm {
                            input: Input::new(self.white_light.warm.to_string()),
                        },
                    });
                }
                1 => {
                    self.text_input = Some(TextInputState {
                        mode: TextInputMode::ExactCold {
                            input: Input::new(self.white_light.cold.to_string()),
                        },
                    });
                }
                2 => self.apply_white_light(),
                focus => {
                    if let Some(index) = focus.checked_sub(3) {
                        if index < 15 {
                            self.white_light.selected[index] = !self.white_light.selected[index];
                        }
                    }
                }
            },
            KeyCode::Char(ch) if ch.is_ascii_digit() => match self.white_light.focus {
                0 => self.start_exact_warm(key),
                1 => self.start_exact_cold(key),
                _ => {}
            },
            _ => {}
        }
    }

    fn toggle_white_light_focus(&mut self) {
        match self.white_light.focus {
            2 => self.apply_white_light(),
            focus => {
                if let Some(index) = focus.checked_sub(3) {
                    if index < 15 {
                        self.white_light.selected[index] = !self.white_light.selected[index];
                    }
                }
            }
        }
    }

    fn adjust_white_light(&mut self, delta: i16) {
        match self.white_light.focus {
            0 => self.white_light.warm = adjust_u8(self.white_light.warm, delta),
            1 => self.white_light.cold = adjust_u8(self.white_light.cold, delta),
            _ => {}
        }
    }

    fn set_white_light_bound(&mut self, maximum: bool) {
        match self.white_light.focus {
            0 => self.white_light.warm = if maximum { 255 } else { 0 },
            1 => self.white_light.cold = if maximum { 255 } else { 0 },
            _ => {}
        }
    }

    fn apply_white_light(&mut self) {
        let cells: Vec<String> = white_light::CELLS
            .iter()
            .enumerate()
            .filter(|(index, _)| self.white_light.selected[*index])
            .map(|(_, cell)| cell.id.to_string())
            .collect();
        if cells.is_empty() {
            self.set_status("no white-light cells selected", StatusKind::Error);
            return;
        }
        self.request_command(ControlCommand::WhiteLight {
            warm: self.white_light.warm,
            cold: self.white_light.cold,
            cells,
        });
    }

    fn handle_outlets_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => {
                self.outlets.row = self
                    .outlets
                    .row
                    .checked_sub(1)
                    .unwrap_or(Outlet::ALL.len() - 1);
            }
            KeyCode::Down => {
                self.outlets.row = (self.outlets.row + 1) % Outlet::ALL.len();
            }
            KeyCode::Left | KeyCode::Right => {
                self.outlets.action = 1 - self.outlets.action;
            }
            KeyCode::Enter | KeyCode::Char(' ') => self.send_outlet(),
            _ => {}
        }
    }

    fn send_dmx(&mut self) {
        self.request_command(ControlCommand::Dmx {
            group: DmxGroup::ALL[self.lighting.group],
            animation: DmxAnimation::ALL[self.lighting.animation],
        });
    }

    fn send_outlet(&mut self) {
        let outlet = Outlet::ALL[self.outlets.row];
        let on = self.outlets.action == 0;
        self.request_command(ControlCommand::Outlet { outlet, on });
    }

    fn handle_alerts_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => {
                self.alerts.focus = self.alerts.focus.checked_sub(1).unwrap_or(5);
            }
            KeyCode::Down => {
                self.alerts.focus = (self.alerts.focus + 1) % 6;
            }
            KeyCode::Enter => self.send_alert(),
            _ => {}
        }
    }

    fn send_alert(&mut self) {
        let command = match self.alerts.focus {
            0 => ControlCommand::Radiation {
                target: RadiationTarget::Make,
            },
            1 => ControlCommand::Radiation {
                target: RadiationTarget::Hack,
            },
            2 => ControlCommand::Sob {
                sound: SobSound::Hackbuzzer,
            },
            3 => ControlCommand::Sob {
                sound: SobSound::Dingdong,
            },
            4 => ControlCommand::Sob {
                sound: SobSound::Chewbacca,
            },
            _ => ControlCommand::Sob {
                sound: SobSound::NootNoot,
            },
        };
        self.request_command(command);
    }

    fn handle_system_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Left => self.system.focus = 0,
            KeyCode::Down | KeyCode::Right => self.system.focus = 1,
            KeyCode::Enter => {
                let on = self.system.focus == 0;
                self.request_command(ControlCommand::Telinit { on });
            }
            _ => {}
        }
    }

    fn handle_animation_key(&mut self, key: KeyEvent) {
        let visible = self.visible_knobs();
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Up => {
                self.animation.focus = next_in(&visible, self.animation.focus, false);
            }
            KeyCode::Down => {
                self.animation.focus = next_in(&visible, self.animation.focus, true);
            }
            KeyCode::PageUp => {
                self.animation.focus = move_in_list(&visible, self.animation.focus, -5);
            }
            KeyCode::PageDown => {
                self.animation.focus = move_in_list(&visible, self.animation.focus, 5);
            }
            KeyCode::Home => {
                let spec = &KNOB_SPECS[self.animation.focus];
                self.animation.values[self.animation.focus] = Some(spec.min);
                self.animation.dirty[self.animation.focus] = true;
            }
            KeyCode::End => {
                let spec = &KNOB_SPECS[self.animation.focus];
                self.animation.values[self.animation.focus] = Some(spec.max);
                self.animation.dirty[self.animation.focus] = true;
            }
            KeyCode::Left => {
                let spec = &KNOB_SPECS[self.animation.focus];
                let step = if shift {
                    spec.step_big()
                } else {
                    spec.step_small()
                };
                self.adjust_knob(-step);
            }
            KeyCode::Right => {
                let spec = &KNOB_SPECS[self.animation.focus];
                let step = if shift {
                    spec.step_big()
                } else {
                    spec.step_small()
                };
                self.adjust_knob(step);
            }
            KeyCode::Char('H') => {
                let step = KNOB_SPECS[self.animation.focus].step_big();
                self.adjust_knob(-step);
            }
            KeyCode::Char('L') => {
                let step = KNOB_SPECS[self.animation.focus].step_big();
                self.adjust_knob(step);
            }
            KeyCode::Enter => {
                if visible.is_empty() {
                    self.set_status("no matching knobs", StatusKind::Info);
                } else {
                    self.send_knob();
                }
            }
            KeyCode::Esc => {
                let index = self.animation.focus;
                self.animation.values[index] = self.animation.committed[index];
                self.animation.dirty[index] = false;
            }
            KeyCode::Char('/') => self.start_search(),
            KeyCode::Char(ch) if ch.is_ascii_digit() || ch == '-' || ch == '.' => {
                self.start_exact_knob(self.animation.focus, key);
            }
            _ => {}
        }
    }

    fn adjust_knob(&mut self, delta: f64) {
        let index = self.animation.focus;
        let spec = &KNOB_SPECS[index];
        let current = self.animation.values[index].unwrap_or(spec.min);
        let value = (current + delta).clamp(spec.min, spec.max);
        self.animation.values[index] = Some(value);
        self.animation.dirty[index] = true;
    }

    fn send_knob(&mut self) {
        let index = self.animation.focus;
        let spec = KNOB_SPECS[index];
        let value = self.animation.values[index].unwrap_or(spec.min);
        self.animation.values[index] = Some(value);
        self.request_command(ControlCommand::KnobSet {
            knob: spec.knob,
            value,
        });
    }

    fn handle_smoothie_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => {
                self.smoothie.focus = self.smoothie.focus.checked_sub(1).unwrap_or(8);
            }
            KeyCode::Down => {
                self.smoothie.focus = (self.smoothie.focus + 1) % 9;
            }
            KeyCode::Left => match self.smoothie.focus {
                0..=6 => {
                    let index = self.smoothie.focus;
                    self.smoothie.relay_choice[index] = !self.smoothie.relay_choice[index];
                }
                7 => self.smoothie.laser = self.smoothie.laser.saturating_sub(1),
                _ => {}
            },
            KeyCode::Right => match self.smoothie.focus {
                0..=6 => {
                    let index = self.smoothie.focus;
                    self.smoothie.relay_choice[index] = !self.smoothie.relay_choice[index];
                }
                7 => self.smoothie.laser = self.smoothie.laser.saturating_add(1),
                _ => {}
            },
            KeyCode::Home if self.smoothie.focus == 7 => {
                self.smoothie.laser = 0;
            }
            KeyCode::End if self.smoothie.focus == 7 => {
                self.smoothie.laser = 255;
            }
            KeyCode::Enter => self.send_smoothie(),
            KeyCode::Char('z') => self.request_command(ControlCommand::SmoothieLaserZero),
            KeyCode::Char(ch) if ch.is_ascii_digit() && self.smoothie.focus == 7 => {
                self.start_exact_laser(key);
            }
            _ => {}
        }
    }

    fn send_smoothie(&mut self) {
        match self.smoothie.focus {
            0..=6 => {
                let index = self.smoothie.focus;
                let relay = SmoothieRelay::ALL[index];
                let on = self.smoothie.relay_choice[index];
                self.request_command(ControlCommand::SmoothieRelay { relay, on });
            }
            7 => {
                let power = self.smoothie.laser;
                self.request_command(ControlCommand::SmoothieLaser { power });
            }
            _ => self.request_command(ControlCommand::SmoothieLaserZero),
        }
    }

    fn handle_theme_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => {
                self.theme_view.focus = self
                    .theme_view
                    .focus
                    .checked_sub(1)
                    .unwrap_or(ThemeRole::ALL.len() - 1);
            }
            KeyCode::Down => {
                self.theme_view.focus = (self.theme_view.focus + 1) % ThemeRole::ALL.len();
            }
            KeyCode::Enter => {
                let role = ThemeRole::ALL[self.theme_view.focus];
                let current = self.theme.get(role);
                let default = Theme::preset(self.theme_preset).get(role);
                self.popup = Some(Popup::ColorPicker {
                    picker: ColorPicker::new(role, current, default),
                });
            }
            KeyCode::Char('x') => {
                let role = ThemeRole::ALL[self.theme_view.focus];
                self.theme.reset_role(role, self.theme_preset);
                self.set_status(format!("{} reset", role.title()), StatusKind::Info);
            }
            KeyCode::Char('X') => {
                self.popup = Some(Popup::ThemeReset {
                    selected: ConfirmChoice::Cancel,
                });
            }
            _ => {}
        }
    }

    fn start_search(&mut self) {
        if matches!(
            self.section,
            Section::Lighting | Section::WhiteLight | Section::Animation
        ) {
            self.text_input = Some(TextInputState {
                mode: TextInputMode::Search {
                    section: self.section,
                    input: Input::default(),
                },
            });
        }
    }

    fn start_exact_knob(&mut self, index: usize, key: KeyEvent) {
        let mut input = Input::default();
        input.handle_event(&Event::Key(key));
        self.text_input = Some(TextInputState {
            mode: TextInputMode::ExactKnob {
                knob: KNOB_SPECS[index].knob,
                input,
            },
        });
    }

    fn start_exact_warm(&mut self, key: KeyEvent) {
        let mut input = Input::default();
        input.handle_event(&Event::Key(key));
        self.text_input = Some(TextInputState {
            mode: TextInputMode::ExactWarm { input },
        });
    }

    fn start_exact_cold(&mut self, key: KeyEvent) {
        let mut input = Input::default();
        input.handle_event(&Event::Key(key));
        self.text_input = Some(TextInputState {
            mode: TextInputMode::ExactCold { input },
        });
    }

    fn start_exact_laser(&mut self, key: KeyEvent) {
        let mut input = Input::default();
        input.handle_event(&Event::Key(key));
        self.text_input = Some(TextInputState {
            mode: TextInputMode::ExactLaser { input },
        });
    }

    fn apply_search_filter(&mut self, section: Section, value: &str) {
        match section {
            Section::Lighting => {
                self.lighting.filter = value.to_string();
                self.ensure_lighting_focus();
            }
            Section::WhiteLight => {
                self.white_light.filter = value.to_string();
                self.ensure_white_light_focus();
            }
            Section::Animation => {
                self.animation.filter = value.to_string();
                self.ensure_animation_focus();
            }
            _ => {}
        }
    }

    fn clear_filter(&mut self, section: Section) {
        match section {
            Section::Lighting => {
                self.lighting.filter.clear();
                self.ensure_lighting_focus();
            }
            Section::WhiteLight => {
                self.white_light.filter.clear();
                self.ensure_white_light_focus();
            }
            Section::Animation => {
                self.animation.filter.clear();
                self.ensure_animation_focus();
            }
            _ => {}
        }
    }

    fn ensure_lighting_focus(&mut self) {
        let visible = self.visible_lighting_groups();
        if visible.is_empty() {
            self.lighting.group = 0;
        } else if !visible.contains(&self.lighting.group) {
            self.lighting.group = visible[0];
        }
    }

    fn ensure_white_light_focus(&mut self) {
        if self.white_light.focus < 3 {
            return;
        }
        let index = self.white_light.focus - 3;
        if !self.visible_white_light_cells().contains(&index) {
            self.white_light.focus = 2;
        }
    }

    fn ensure_animation_focus(&mut self) {
        let visible = self.visible_knobs();
        if visible.is_empty() {
            self.animation.focus = 0;
        } else if !visible.contains(&self.animation.focus) {
            self.animation.focus = visible[0];
        }
    }

    fn set_knob_value(&mut self, knob: Knob, value: f64) {
        let index = knob.index();
        let value = knob.spec().clamp(value);
        self.animation.values[index] = Some(value);
        self.animation.dirty[index] = true;
        self.set_status(
            format!(
                "knob {} = {} (Enter to send)",
                knob.name(),
                format_value(value)
            ),
            StatusKind::Info,
        );
    }

    pub fn visible_lighting_groups(&self) -> Vec<usize> {
        let needle = self.lighting.filter.to_ascii_lowercase();
        DmxGroup::ALL
            .iter()
            .enumerate()
            .filter(|(_, group)| needle.is_empty() || group.as_str().contains(needle.as_str()))
            .map(|(index, _)| index)
            .collect()
    }

    pub fn visible_white_light_cells(&self) -> Vec<usize> {
        let needle = self.white_light.filter.to_ascii_lowercase();
        white_light::CELLS
            .iter()
            .enumerate()
            .filter(|(_, cell)| {
                needle.is_empty()
                    || cell.id.to_ascii_lowercase().contains(needle.as_str())
                    || cell.group.label().contains(needle.as_str())
            })
            .map(|(index, _)| index)
            .collect()
    }

    pub fn visible_knobs(&self) -> Vec<usize> {
        let needle = self.animation.filter.to_ascii_lowercase();
        KNOB_SPECS
            .iter()
            .enumerate()
            .filter(|(_, spec)| {
                needle.is_empty() || spec.name.to_ascii_lowercase().contains(needle.as_str())
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn white_light_focus_items(&self) -> Vec<usize> {
        let mut items = vec![0, 1, 2];
        items.extend(
            self.visible_white_light_cells()
                .into_iter()
                .map(|index| 3 + index),
        );
        items
    }

    fn toggle_log(&mut self) {
        self.show_log = !self.show_log;
        if !self.show_log && self.zone == FocusZone::AppLog {
            self.zone = FocusZone::Content;
        }
    }

    fn cycle_zone(&mut self, forward: bool) {
        let mut zones = vec![FocusZone::Sidebar, FocusZone::Content, FocusZone::Traffic];
        if self.show_log {
            zones.push(FocusZone::AppLog);
        }
        let current = zones
            .iter()
            .position(|zone| *zone == self.zone)
            .unwrap_or(1);
        let next = if forward {
            (current + 1) % zones.len()
        } else {
            (current + zones.len() - 1) % zones.len()
        };
        self.zone = zones[next];
    }

    fn change_section(&mut self, section: Section) {
        self.section = section;
        self.text_input = None;
        self.traffic_selected = None;
        match section {
            Section::Lighting => self.ensure_lighting_focus(),
            Section::WhiteLight => self.ensure_white_light_focus(),
            Section::Animation => self.ensure_animation_focus(),
            _ => {}
        }
    }

    fn go_back(&mut self) {
        if self.section != Section::Overview {
            self.change_section(Section::Overview);
        } else {
            self.popup = Some(Popup::Quit {
                selected: ConfirmChoice::Cancel,
            });
        }
    }

    fn escape(&mut self) {
        let filter_active = match self.section {
            Section::Lighting => !self.lighting.filter.is_empty(),
            Section::WhiteLight => !self.white_light.filter.is_empty(),
            Section::Animation => !self.animation.filter.is_empty(),
            _ => false,
        };
        if filter_active {
            self.clear_filter(self.section);
        } else if self.section != Section::Overview {
            self.change_section(Section::Overview);
        }
    }

    fn toggle_safe_mode(&mut self) {
        if self.safe_mode {
            self.popup = Some(Popup::SafeModeOff {
                selected: ConfirmChoice::Cancel,
            });
        } else {
            self.safe_mode = true;
            self.set_status("safe mode enabled", StatusKind::Info);
        }
    }

    fn request_connect(&mut self) {
        if self.mqtt_tx.send(MqttCommand::Connect).is_err() {
            self.set_status("MQTT supervisor is not running", StatusKind::Error);
            return;
        }
        self.connection = ConnectionState::Connecting { attempt: 0 };
        self.set_status("connecting…", StatusKind::Info);
    }

    fn request_disconnect(&mut self) {
        if self.mqtt_tx.send(MqttCommand::Disconnect).is_err() {
            self.set_status("MQTT supervisor is not running", StatusKind::Error);
            return;
        }
        self.connection = ConnectionState::Disconnected;
        self.set_status("disconnected", StatusKind::Info);
    }

    fn request_reconnect(&mut self) {
        if self.mqtt_tx.send(MqttCommand::Reconnect).is_err() {
            self.set_status("MQTT supervisor is not running", StatusKind::Error);
            return;
        }
        self.connection = ConnectionState::Connecting { attempt: 0 };
        self.set_status("reconnecting…", StatusKind::Info);
    }

    fn request_command(&mut self, command: ControlCommand) {
        let actions = match command.expand(&self.options.smoothie_url) {
            Ok(actions) => actions,
            Err(err) => {
                self.set_status(format!("error: {err}"), StatusKind::Error);
                return;
            }
        };

        let has_mqtt = actions
            .iter()
            .any(|action| action.transport() == Transport::Mqtt);
        if has_mqtt && !self.connection.is_connected() {
            self.set_status("Not connected — command not sent.", StatusKind::Error);
            for action in actions
                .iter()
                .filter(|action| action.transport() == Transport::Mqtt)
            {
                self.traffic.push(TrafficEntry::error(
                    action.target(),
                    action.payload(),
                    "not connected",
                    sections_for_action(action),
                ));
            }
            return;
        }

        if self.safe_mode {
            self.popup = Some(Popup::ConfirmCommand {
                pending: PendingCommand {
                    description: command.description(),
                    actions,
                    destructive: command.destructive(),
                },
                selected: ConfirmChoice::Cancel,
            });
            return;
        }

        self.send_actions(actions);
    }

    fn send_actions(&mut self, actions: Vec<Action>) {
        if actions.is_empty() {
            return;
        }
        let count = actions.len();
        if self.options.dry_run {
            for action in &actions {
                self.traffic.push(TrafficEntry::from_action(
                    action,
                    TrafficDirection::Dry,
                    None,
                ));
            }
            self.set_status(format!("dry-run: {count} action(s)"), StatusKind::Info);
            return;
        }

        for action in actions {
            match action {
                Action::Publish {
                    topic,
                    payload,
                    retain,
                } => {
                    if self
                        .mqtt_tx
                        .send(MqttCommand::Publish {
                            topic,
                            payload,
                            retain,
                        })
                        .is_err()
                    {
                        self.set_status("MQTT supervisor stopped", StatusKind::Error);
                    }
                }
                Action::Http { url, body } => {
                    let description = body.trim_end().to_string();
                    if self
                        .http_tx
                        .send(HttpCommand {
                            url,
                            body,
                            description,
                        })
                        .is_err()
                    {
                        self.set_status("HTTP worker stopped", StatusKind::Error);
                    }
                }
            }
        }
        self.set_status(format!("sent {count} command(s)"), StatusKind::Info);
    }

    pub fn handle_mqtt_event(&mut self, event: MqttEvent) {
        match event {
            MqttEvent::Connecting { attempt } => {
                self.connection = ConnectionState::Connecting { attempt };
                self.set_status(format!("connecting (attempt {attempt})"), StatusKind::Info);
            }
            MqttEvent::Connected { generation } => {
                self.mqtt_generation = generation;
                self.connection = ConnectionState::Connected {
                    since: Instant::now(),
                };
                self.set_status("connected", StatusKind::Success);
                tracing::info!("mqtt connected (generation {generation})");
            }
            MqttEvent::ConnectionLost { generation, reason } => {
                if generation == self.mqtt_generation {
                    self.connection = ConnectionState::Disconnected;
                    self.set_status(format!("connection lost: {reason}"), StatusKind::Error);
                    tracing::warn!("mqtt connection lost: {reason}");
                }
            }
            MqttEvent::Disconnected { reason } => {
                self.connection = ConnectionState::Disconnected;
                self.set_status(format!("disconnected: {reason}"), StatusKind::Info);
            }
            MqttEvent::ConnectFailed { attempt, reason } => {
                self.connection = ConnectionState::Disconnected;
                self.set_status(
                    format!("connect attempt {attempt} failed: {reason}"),
                    StatusKind::Error,
                );
            }
            MqttEvent::Reconnecting { attempt, delay } => {
                self.connection = ConnectionState::Reconnecting {
                    attempt,
                    retry_at: Instant::now() + delay,
                };
                self.set_status(
                    format!("reconnecting in {}s (attempt {attempt})", delay.as_secs()),
                    StatusKind::Info,
                );
            }
            MqttEvent::Message {
                generation,
                topic,
                payload,
            } => {
                if generation == self.mqtt_generation {
                    self.handle_message(&topic, &payload);
                }
            }
            MqttEvent::Published {
                topic,
                payload,
                retain,
            } => {
                self.traffic
                    .push(TrafficEntry::published(&topic, &payload, retain));
                if topic == crate::action::TOPIC_OUTLET {
                    if let Some((index, on)) = outlet_from_payload(&payload) {
                        self.outlets.last_sent[index] = Some(on);
                    }
                }
                if let Some(name) = topic.strip_prefix(TOPIC_KNOB_SET_PREFIX) {
                    if let Some(knob) = Knob::from_name(name) {
                        if let Ok(value) = payload.parse::<f64>() {
                            let index = knob.index();
                            self.animation.committed[index] = Some(value);
                            if self.animation.values[index]
                                .map(|current| (current - value).abs() < 1e-9)
                                .unwrap_or(true)
                            {
                                self.animation.dirty[index] = false;
                            }
                        }
                    }
                }
                self.set_status(format!("published {topic}"), StatusKind::Success);
                tracing::debug!("published {topic}: {payload}");
            }
            MqttEvent::PublishFailed {
                topic,
                payload,
                reason,
            } => {
                self.traffic.push(TrafficEntry::error(
                    &topic,
                    &payload,
                    &reason,
                    crate::tui::traffic::sections_for_topic(&topic),
                ));
                self.set_status(format!("publish failed: {reason}"), StatusKind::Error);
                tracing::warn!("publish failed for {topic}: {reason}");
            }
            MqttEvent::PublishRejected {
                topic,
                payload,
                reason,
            } => {
                self.traffic.push(TrafficEntry::error(
                    &topic,
                    &payload,
                    &reason,
                    crate::tui::traffic::sections_for_topic(&topic),
                ));
                self.set_status(format!("publish rejected: {reason}"), StatusKind::Error);
                tracing::warn!("publish rejected for {topic}: {reason}");
            }
        }
    }

    fn handle_message(&mut self, topic: &str, payload: &str) {
        self.traffic.push(TrafficEntry::received(topic, payload));

        if topic == TOPIC_ALERT_CONDITION {
            self.alert = parse_alert(payload);
        }

        if let Some(id) = topic.strip_prefix(TOPIC_WHITE_LIGHT_STATUS_PREFIX) {
            if let Some(index) = white_light::index_of(id) {
                self.white_light.status[index] = Some(parse_bool(payload));
            }
        }

        if let Some(name) = topic.strip_prefix(TOPIC_KNOB_GET_PREFIX) {
            if let Some(knob) = Knob::from_name(name) {
                if let Ok(value) = payload.trim().parse::<f64>() {
                    let index = knob.index();
                    self.animation.committed[index] = Some(value);
                    if !self.animation.dirty[index] {
                        self.animation.values[index] = Some(value);
                    }
                }
            }
        }

        if topic.starts_with(TOPIC_SMOOTHIE_PREFIX) {
            self.smoothie.status.apply(topic, payload, Instant::now());
        }

        tracing::debug!("mqtt message {topic}: {payload}");
    }

    pub fn handle_http_event(&mut self, event: HttpEvent) {
        let url = self.options.smoothie_url.clone();
        match event.result {
            Ok(()) => {
                self.smoothie.http_error = None;
                self.smoothie.last_http = Some(event.description);
                self.traffic.push(TrafficEntry::http(&url, &event.body));
                self.set_status("smoothiesaur command sent", StatusKind::Success);
                tracing::info!("smoothiesaur command sent: {}", event.body.trim_end());
            }
            Err(reason) => {
                self.smoothie.http_error = Some(reason.clone());
                self.traffic
                    .push(TrafficEntry::http_error(&url, &event.body, &reason));
                self.set_status(format!("HTTP error: {reason}"), StatusKind::Error);
                tracing::warn!("smoothiesaur command failed: {reason}");
            }
        }
    }

    pub fn on_tick(&mut self) {
        self.throbber.calc_next();
        let clear_status = self
            .status
            .as_ref()
            .map(|status| {
                status.kind != StatusKind::Error && status.at.elapsed() > Duration::from_secs(12)
            })
            .unwrap_or(false);
        if clear_status {
            self.status = None;
        }
    }

    pub fn connection_color(&self) -> Color {
        match self.connection {
            ConnectionState::Connected { .. } => self.theme.success,
            ConnectionState::Connecting { .. } | ConnectionState::Reconnecting { .. } => {
                self.theme.warning
            }
            ConnectionState::Disconnected => self.theme.error,
        }
    }

    pub fn alert_color(&self) -> Color {
        if self.alert.dim {
            return self.theme.muted;
        }
        match self.alert.condition {
            AlertCondition::Green => self.theme.success,
            AlertCondition::Blue => Color::Blue,
            AlertCondition::Contact => self.theme.warning,
            AlertCondition::Offworld => self.theme.alert,
            AlertCondition::Intruder | AlertCondition::Red | AlertCondition::SelfDestruct => {
                self.theme.error
            }
            AlertCondition::Biohazard => self.theme.alert,
            AlertCondition::Unknown => self.theme.muted,
        }
    }

    pub fn alert_label(&self) -> &'static str {
        self.alert.condition.label()
    }
}

fn next_in(list: &[usize], current: usize, forward: bool) -> usize {
    if list.is_empty() {
        return current;
    }
    match list.iter().position(|item| *item == current) {
        Some(index) => {
            if forward {
                list[(index + 1) % list.len()]
            } else {
                list[(index + list.len() - 1) % list.len()]
            }
        }
        None => {
            if forward {
                list[0]
            } else {
                list[list.len() - 1]
            }
        }
    }
}

fn move_in_list(list: &[usize], current: usize, delta: isize) -> usize {
    if list.is_empty() {
        return current;
    }
    let position = list.iter().position(|item| *item == current).unwrap_or(0) as isize;
    let next = (position + delta).clamp(0, list.len() as isize - 1) as usize;
    list[next]
}

fn adjust_u8(value: u8, delta: i16) -> u8 {
    (value as i16 + delta).clamp(0, 255) as u8
}

fn outlet_from_payload(payload: &str) -> Option<(usize, bool)> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    let args = value.get("args")?.as_str()?;
    let mut parts = args.split_whitespace();
    let name = parts.next()?;
    let state = parts.next()?;
    let index = Outlet::ALL
        .iter()
        .position(|outlet| outlet.topic_name() == name)?;
    let on = match state {
        "on" => true,
        "off" => false,
        _ => return None,
    };
    Some((index, on))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_bits_are_unique() {
        let mut seen = 0u16;
        for section in Section::ALL {
            assert_eq!(seen & section.bit(), 0);
            seen |= section.bit();
        }
    }

    #[test]
    fn next_in_wraps_and_handles_missing_items() {
        let list = [2, 4, 6];
        assert_eq!(next_in(&list, 2, true), 4);
        assert_eq!(next_in(&list, 6, true), 2);
        assert_eq!(next_in(&list, 2, false), 6);
        assert_eq!(next_in(&list, 5, true), 2);
        assert_eq!(next_in(&[], 3, true), 3);
    }

    #[test]
    fn connection_labels_include_the_backoff() {
        let now = Instant::now();
        let state = ConnectionState::Reconnecting {
            attempt: 2,
            retry_at: now + Duration::from_secs(4),
        };
        assert!(state.label(now).contains('4'));
        assert!(ConnectionState::Connected { since: now }.is_connected());
        assert!(ConnectionState::Connecting { attempt: 1 }.is_busy());
    }
}
