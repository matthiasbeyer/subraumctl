//! Interactive TUI entry point and terminal lifecycle.

mod app;
mod climate;
mod events;
mod heizung;
mod keymap;
mod pizzamat;
mod power;
mod safe_mode;
mod theme;
mod traffic;
mod ui;
mod views;

use std::io;
use std::time::Duration;

use crossterm::cursor::{Hide, Show};
use crossterm::event::KeyEvent;
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use eyre::Result;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;
use tokio::time::interval;

use crate::cli::TuiView;
use crate::color::{Theme, ThemePreset};
use crate::mqtt::{self, MqttCommand, MqttEvent, MqttOptions};
use crate::smoothiesaur::{self, HttpCommand, HttpEvent};

pub struct TuiOptions {
    pub mqtt: MqttOptions,
    pub view: TuiView,
    pub connect: bool,
    pub safe_mode: bool,
    pub theme: Theme,
    pub theme_preset: ThemePreset,
    pub smoothie_url: String,
    pub pizzamat: crate::pizzamat::Options,
    pub heizung: crate::heizung::Options,
    pub influx: crate::influx::Options,
    pub verbose: u8,
    pub dry_run: bool,
}

pub async fn run(options: TuiOptions) -> Result<()> {
    crate::logging::init_tui(options.verbose)?;
    let mut terminal = TerminalGuard::enter()?;

    let (ui_tx, mut ui_rx) = mpsc::unbounded_channel::<KeyEvent>();
    events::spawn_input_thread(ui_tx);

    let (mqtt_tx, mqtt_rx) = mpsc::unbounded_channel::<MqttCommand>();
    let (mqtt_event_tx, mut mqtt_event_rx) = mpsc::unbounded_channel::<MqttEvent>();
    let mqtt_options = options.mqtt.clone();
    mqtt::spawn_supervisor(mqtt_options, mqtt_rx, mqtt_event_tx);

    let (http_tx, http_rx) = mpsc::unbounded_channel::<HttpCommand>();
    let (http_event_tx, mut http_event_rx) = mpsc::unbounded_channel::<HttpEvent>();
    smoothiesaur::spawn_http_worker(http_rx, http_event_tx, Duration::from_secs(5));

    let (pizza_tx, pizza_rx) = mpsc::unbounded_channel();
    let (pizza_event_tx, mut pizza_event_rx) = mpsc::unbounded_channel();
    pizzamat::spawn_worker(
        options.pizzamat.clone(),
        options.dry_run,
        pizza_rx,
        pizza_event_tx,
    );
    let (heizung_tx, heizung_rx) = mpsc::unbounded_channel();
    let (heizung_event_tx, mut heizung_event_rx) = mpsc::unbounded_channel();
    heizung::spawn_worker(
        options.heizung.clone(),
        options.dry_run,
        heizung_rx,
        heizung_event_tx,
    );
    let (climate_tx, climate_rx) = mpsc::unbounded_channel();
    let (climate_event_tx, mut climate_event_rx) = mpsc::unbounded_channel();
    climate::spawn_worker(
        options.influx.clone(),
        options.dry_run,
        climate_rx,
        climate_event_tx,
    );
    let (power_tx, power_rx) = mpsc::unbounded_channel();
    let (power_event_tx, mut power_event_rx) = mpsc::unbounded_channel();
    power::spawn_worker(
        options.influx.clone(),
        options.dry_run,
        power_rx,
        power_event_tx,
    );
    let mut app = app::App::new(
        options, mqtt_tx, http_tx, pizza_tx, heizung_tx, climate_tx, power_tx,
    );
    let mut ticker = interval(Duration::from_millis(100));

    loop {
        terminal
            .terminal_mut()
            .draw(|frame| ui::draw(frame, &mut app))?;

        tokio::select! {
            Some(key) = ui_rx.recv() => app.handle_key(key),
            Some(event) = mqtt_event_rx.recv() => app.handle_mqtt_event(event),
            Some(event) = http_event_rx.recv() => app.handle_http_event(event),
            Some(event) = pizza_event_rx.recv() => app.handle_pizzamat_event(event),
            Some(event) = heizung_event_rx.recv() => app.handle_heizung_event(event),
            Some(event) = climate_event_rx.recv() => app.handle_climate_event(event),
            Some(event) = power_event_rx.recv() => app.handle_power_event(event),
            _ = ticker.tick() => app.on_tick(),
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let mut stdout = io::stdout();
            let _ = execute!(stdout, LeaveAlternateScreen, Show);
            previous_hook(info);
        }));

        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(err) = execute!(stdout, EnterAlternateScreen, Hide) {
            let _ = disable_raw_mode();
            return Err(err.into());
        }
        let backend = CrosstermBackend::new(stdout);
        let terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(err) => {
                let _ = disable_raw_mode();
                let mut stdout = io::stdout();
                let _ = execute!(stdout, LeaveAlternateScreen, Show);
                return Err(err.into());
            }
        };
        Ok(Self { terminal })
    }

    fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<io::Stdout>> {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen, Show);
        let _ = self.terminal.show_cursor();
    }
}
