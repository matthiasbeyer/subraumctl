//! Clap command tree.
//!
//! Every subcommand documents what it controls, the exact topic and payload it
//! uses, and at least one example. `clap_mangen` renders the same text into the
//! generated man pages.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::action::{DmxAnimation, DmxGroup, Outlet, RadiationTarget, SmoothieRelay, SobSound};
use crate::color::ThemePreset;
use crate::knob::Knob;
use crate::mqtt::DEFAULT_BROKER;
use crate::smoothiesaur::DEFAULT_SMOOTHIE_URL;

#[derive(Parser, Debug)]
#[command(
    name = "subraumctl",
    version,
    about = "Control the subraum over MQTT",
    long_about = "subraumctl controls the subraum lights, outlets, alerts, animation knobs and \
smoothiesaur over MQTT (TCP) and HTTP, and Pizzamat food orders over EDI AMQP. \
It also provides a keyboard-driven ratatui TUI \
with safe mode, automatic reconnects, a live traffic pane and an application log pane.",
    after_help = "Broker: normal MQTT over TCP (tcp:// or ssl://), no WebSockets.\n\
Default: tcp://mqtt.subraum.c3pb.de:1883 (override with --broker or SUBRAUMCTL_BROKER).\n\
\n\
Examples:\n\
  subraumctl dmx subraum.all rainbow\n\
  subraumctl white-light apply --warm 128 --cold 200 --all\n\
  subraumctl knob set cx 42\n\
  subraumctl tui --view animation --accent-color teal\n\
  subraumctl tui --theme high-contrast --error-color 'rgb(255,80,80)'\n\
  subraumctl man --output-dir ./man"
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: CliCommand,
}

#[derive(Args, Clone, Debug)]
pub struct GlobalArgs {
    /// MQTT broker URI (tcp:// or ssl://; WebSockets are not supported)
    #[arg(
        short = 'b',
        long,
        global = true,
        env = "SUBRAUMCTL_BROKER",
        default_value = DEFAULT_BROKER
    )]
    pub broker: String,

    /// MQTT client id (default: subraumctl-<8 hex>)
    #[arg(long, global = true)]
    pub client_id: Option<String>,

    /// MQTT username
    #[arg(long, global = true, env = "SUBRAUMCTL_USERNAME")]
    pub username: Option<String>,

    /// MQTT password (prefer the environment variable)
    #[arg(long, global = true, env = "SUBRAUMCTL_PASSWORD")]
    pub password: Option<String>,

    /// MQTT keep-alive interval in seconds
    #[arg(long, global = true, default_value_t = 60)]
    pub keep_alive: u64,

    /// MQTT connect timeout in seconds
    #[arg(long, global = true, default_value_t = 5)]
    pub connect_timeout: u64,

    /// Use a non-clean MQTT session
    #[arg(long, global = true)]
    pub no_clean_session: bool,

    /// Timeout in seconds for get/subscribe/status commands
    #[arg(long, global = true, default_value_t = 3)]
    pub wait: u64,

    /// Smoothiesaur command endpoint
    #[arg(
        long,
        global = true,
        env = "SUBRAUMCTL_SMOOTHIESAUR_URL",
        default_value = DEFAULT_SMOOTHIE_URL
    )]
    pub smoothiesaur_url: String,

    /// Pizzamat EDI AMQP endpoint
    #[arg(long, global = true, env = "SUBRAUMCTL_PIZZAMAT_AMQP_URL", default_value = crate::pizzamat::DEFAULT_AMQP_URL)]
    pub pizzamat_amqp_url: String,

    /// Pizzamat order identity; use the same identity as your existing bot orders
    #[arg(long, global = true, env = "SUBRAUMCTL_PIZZAMAT_USER")]
    pub pizzamat_user: Option<String>,

    /// Use puzza-* commands for a debug-build Pizzamat server
    #[arg(long, global = true)]
    pub pizzamat_debug: bool,

    /// Pizzamat request timeout in seconds (includes order-start announcements)
    #[arg(
        long,
        global = true,
        default_value_t = 15,
        value_parser = clap::value_parser!(u64).range(1..=120)
    )]
    pub pizzamat_timeout: u64,

    /// Preview wire actions without publishing or sending them
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// Emit one JSON object per line
    #[arg(long, global = true)]
    pub json: bool,

    /// Suppress non-error human output
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,

    /// Increase log verbosity (-v info, -vv debug, -vvv trace)
    #[arg(short = 'v', long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

#[derive(Subcommand, Debug)]
pub enum CliCommand {
    /// Start the MCP server on stdio (only explicitly registered tools are exposed)
    #[cfg(feature = "mcp")]
    Mcp(McpArgs),

    /// Send a DMX animation command to edi/cmd/dmx
    ///
    /// Payload: {"args":"<group> <animation>"}
    #[command(
        after_help = "Examples:\n  subraumctl dmx subraum.all rainbow\n  subraumctl dmx hack.floor backgroundb"
    )]
    Dmx(DmxArgs),

    /// Apply warm/cold values to white-light cells
    ///
    /// Publishes `w <warm> c <cold> ` to light/esp/request/<cell> for every
    /// selected cell. Cell status is read from light/esp/status/<cell>.
    #[command(
        after_help = "Examples:\n  subraumctl white-light apply --warm 128 --cold 200 --all\n  subraumctl white-light apply --warm 10 --cold 20 --cell LM3_A020A63498E7\n  subraumctl white-light cells\n  subraumctl white-light status"
    )]
    WhiteLight(WhiteLightArgs),

    /// Switch a simple outlet via edi/cmd/act
    ///
    /// Payload: {"args":"<outlet> on"} or {"args":"<outlet> off"}
    #[command(
        after_help = "Outlets: make.colors, make.bright, ert, stationaere_kuehlungsloesung, compressor, white-power, dodekaeder\n\nExamples:\n  subraumctl outlet make.colors on\n  subraumctl outlet white-power off"
    )]
    Outlet(OutletArgs),

    /// Alert tab: radiation emitters, sob sounds and the alert LED
    #[command(
        after_help = "Examples:\n  subraumctl alert radiation make\n  subraumctl alert sob noot-noot\n  subraumctl alert led --watch"
    )]
    Alert(AlertArgs),

    /// System commands (telinit)
    #[command(
        after_help = "Examples:\n  subraumctl system telinit on\n  subraumctl system telinit off"
    )]
    System(SystemArgs),

    /// Read or set animation knobs on knob/get/<name> and knob/set/<name>
    #[command(
        after_help = "Examples:\n  subraumctl knob list\n  subraumctl knob get phase\n  subraumctl knob set cx 42\n  subraumctl knob watch cx cy cz"
    )]
    Knob(KnobArgs),

    /// Control the smoothiesaur over its HTTP command endpoint
    ///
    /// Relay and laser commands are plain HTTP POSTs to --smoothiesaur-url.
    /// Status values are read from the smoothie/# MQTT topics.
    #[command(
        name = "smoothiesaur",
        visible_alias = "smoothie",
        after_help = "Examples:\n  subraumctl smoothiesaur relay pump-primary on\n  subraumctl smoothiesaur laser 128\n  subraumctl smoothiesaur zero\n  subraumctl smoothiesaur status --watch"
    )]
    Smoothiesaur(SmoothieArgs),

    /// Connect, print broker information and the alert condition
    Status,

    /// Publish an arbitrary MQTT message
    ///
    /// The payload is taken from the positional argument or from
    /// --payload-file. Use --retain to set the retain flag.
    #[command(
        after_help = "Examples:\n  subraumctl publish edi/cmd/dmx '{\"args\":\"subraum.all rainbow\"}'\n  subraumctl publish edi/cmd/dmx --payload-file payload.json --retain"
    )]
    Publish(PublishArgs),

    /// Subscribe to a topic and print messages
    #[command(
        after_help = "Examples:\n  subraumctl subscribe 'knob/get/#' --count 10\n  subraumctl subscribe 'light/esp/status/#' --count 0 --wait 30"
    )]
    Subscribe(SubscribeArgs),

    /// Monitor decoded status topics
    #[command(
        after_help = "Examples:\n  subraumctl monitor --kind all\n  subraumctl monitor --kind smoothiesaur --duration 30"
    )]
    Monitor(MonitorArgs),

    /// Start the interactive keyboard-driven TUI
    #[command(
        after_help = "Examples:\n  subraumctl tui\n  subraumctl tui --view animation --accent-color teal\n  subraumctl tui --theme high-contrast --error-color 'rgb(255,80,80)'\n  subraumctl tui --no-safe-mode"
    )]
    Tui(TuiArgs),

    /// Generate man pages for subraumctl and its subcommands
    #[command(
        after_help = "Examples:\n  subraumctl man\n  subraumctl man dmx\n  subraumctl man --output-dir ./man"
    )]
    Man(ManArgs),

    /// Generate shell completions
    #[command(
        after_help = "Examples:\n  subraumctl completions bash\n  subraumctl completions zsh"
    )]
    Completions(CompletionsArgs),
}

#[cfg(feature = "mcp")]
#[derive(Args, Debug)]
pub struct McpArgs {
    /// TOML tool permissions file (omitting this uses built-in defaults)
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct DmxArgs {
    /// DMX group (for example subraum.all or hack.floor)
    #[arg(value_enum)]
    pub group: DmxGroup,

    /// Animation to activate
    #[arg(value_enum)]
    pub animation: DmxAnimation,
}

#[derive(Args, Debug)]
pub struct WhiteLightArgs {
    #[command(subcommand)]
    pub command: WhiteLightCommand,
}

#[derive(Subcommand, Debug)]
pub enum WhiteLightCommand {
    /// Apply warm/cold values to the selected cells
    Apply(WhiteLightApplyArgs),

    /// List the known white-light cell ids
    Cells,

    /// Print the last known status of the white-light cells
    Status(WhiteLightStatusArgs),
}

#[derive(Args, Debug)]
#[command(after_help = "Topic: light/esp/request/<cell>\nPayload: w <warm> c <cold>")]
pub struct WhiteLightApplyArgs {
    /// Warm-white value (0..=255)
    #[arg(long, value_parser = clap::value_parser!(u8))]
    pub warm: u8,

    /// Cold-white value (0..=255)
    #[arg(long, value_parser = clap::value_parser!(u8))]
    pub cold: u8,

    /// Cell id; repeat the flag for multiple cells
    #[arg(long = "cell", value_name = "ID", conflicts_with = "all")]
    pub cells: Vec<String>,

    /// Select every known cell
    #[arg(long, conflicts_with = "cells")]
    pub all: bool,
}

#[derive(Args, Debug)]
pub struct WhiteLightStatusArgs {
    /// Keep watching until interrupted
    #[arg(long)]
    pub watch: bool,
}

#[derive(Args, Debug)]
pub struct OutletArgs {
    /// Outlet name
    #[arg(value_enum)]
    pub outlet: Outlet,

    /// Desired state
    #[arg(value_enum)]
    pub state: OnOff,
}

#[derive(Args, Debug)]
pub struct AlertArgs {
    #[command(subcommand)]
    pub command: AlertCommand,
}

#[derive(Subcommand, Debug)]
pub enum AlertCommand {
    /// Trigger an electromagnetic radiation emitter
    Radiation {
        /// Which emitter to trigger
        #[arg(value_enum)]
        target: RadiationTarget,
    },

    /// Trigger a sob sound on edi.cmd.sob
    Sob {
        /// Which sound to play
        #[arg(value_enum)]
        sound: SobSound,
    },

    /// Print or watch heimdallr/alert/condition
    Led {
        /// Keep watching until interrupted
        #[arg(long)]
        watch: bool,
    },
}

#[derive(Args, Debug)]
pub struct SystemArgs {
    #[command(subcommand)]
    pub command: SystemCommand,
}

#[derive(Subcommand, Debug)]
pub enum SystemCommand {
    /// Send a telinit command (on -> 4, off -> 0)
    Telinit {
        /// Desired state
        #[arg(value_enum)]
        state: OnOff,
    },
}

#[derive(Args, Debug)]
pub struct KnobArgs {
    #[command(subcommand)]
    pub command: KnobCommand,
}

#[derive(Subcommand, Debug)]
pub enum KnobCommand {
    /// List all knobs with their ranges and open flags
    List,

    /// Read the current value of a knob
    Get {
        /// Knob name
        #[arg(value_enum)]
        knob: Knob,
    },

    /// Set a knob value
    Set {
        /// Knob name
        #[arg(value_enum)]
        knob: Knob,

        /// New value
        value: f64,
    },

    /// Watch knob/get/<name> updates (all knobs when no names are given)
    Watch {
        /// Knob names to watch
        #[arg(value_enum)]
        knobs: Vec<Knob>,
    },
}

#[derive(Args, Debug)]
pub struct SmoothieArgs {
    #[command(subcommand)]
    pub command: SmoothieCommand,
}

#[derive(Subcommand, Debug)]
pub enum SmoothieCommand {
    /// Switch a smoothiesaur relay
    Relay {
        /// Relay name
        #[arg(value_enum)]
        relay: SmoothieRelay,

        /// Desired state
        #[arg(value_enum)]
        state: OnOff,
    },

    /// Set the laser power (0..=255)
    Laser {
        /// Laser power
        #[arg(value_parser = clap::value_parser!(u8))]
        power: u8,
    },

    /// Set the laser power to zero
    Zero,

    /// Print or watch smoothie/# status topics
    Status {
        /// Keep watching until interrupted
        #[arg(long)]
        watch: bool,
    },
}

#[derive(Args, Debug)]
pub struct PublishArgs {
    /// MQTT topic
    pub topic: String,

    /// MQTT payload (required unless --payload-file is used)
    #[arg(required_unless_present = "payload_file")]
    pub payload: Option<String>,

    /// Read the payload from a file
    #[arg(long, value_name = "PATH", conflicts_with = "payload")]
    pub payload_file: Option<PathBuf>,

    /// Set the retain flag
    #[arg(long)]
    pub retain: bool,
}

#[derive(Args, Debug)]
pub struct SubscribeArgs {
    /// MQTT topic filter
    pub topic: String,

    /// Number of messages to print; 0 means until the timeout/interrupt
    #[arg(long, default_value_t = 1)]
    pub count: usize,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum MonitorKind {
    #[value(name = "alert")]
    Alert,
    #[value(name = "white-light", alias = "white_light")]
    WhiteLight,
    #[value(name = "knobs")]
    Knobs,
    #[value(name = "smoothiesaur", alias = "smoothie")]
    Smoothie,
    #[value(name = "all")]
    All,
}

#[derive(Args, Debug)]
pub struct MonitorArgs {
    /// Which group of status topics to decode
    #[arg(long, value_enum, default_value = "all")]
    pub kind: MonitorKind,

    /// Seconds to stream before exiting; 0 or omitted means until interrupted
    #[arg(long, value_name = "SECS")]
    pub duration: Option<u64>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum TuiView {
    #[value(name = "overview")]
    Overview,
    #[value(name = "lighting")]
    Lighting,
    #[value(name = "white-light", alias = "white_light")]
    WhiteLight,
    #[value(name = "outlets")]
    Outlets,
    #[value(name = "alerts")]
    Alerts,
    #[value(name = "system")]
    System,
    #[value(name = "animation")]
    Animation,
    #[value(name = "smoothiesaur", alias = "smoothie")]
    Smoothie,
    #[value(name = "theme")]
    Theme,
    #[value(name = "pizzamat", alias = "pizza")]
    Pizzamat,
}

#[derive(Args, Debug)]
#[command(
    after_help = "Colors accept CSS names (teal, orange, rebeccapurple), #rgb/#rrggbb, \
rgb(...), hsl(...) and bare r,g,b or hex values. Alpha is ignored."
)]
pub struct TuiArgs {
    /// Section to open first
    #[arg(long, value_enum, default_value = "overview")]
    pub view: TuiView,

    /// Start disconnected instead of connecting automatically
    #[arg(long)]
    pub no_connect: bool,

    /// Start with safe mode disabled (commands send without confirmation)
    #[arg(long)]
    pub no_safe_mode: bool,

    /// Theme preset
    #[arg(long, value_enum, default_value = "dark")]
    pub theme: ThemePreset,

    /// Background color
    #[arg(long, value_name = "COLOR")]
    pub background_color: Option<String>,

    /// Foreground color
    #[arg(long, value_name = "COLOR")]
    pub foreground_color: Option<String>,

    /// Accent color
    #[arg(long, value_name = "COLOR")]
    pub accent_color: Option<String>,

    /// Selection background color
    #[arg(long, value_name = "COLOR")]
    pub selection_color: Option<String>,

    /// Success color
    #[arg(long, value_name = "COLOR")]
    pub success_color: Option<String>,

    /// Warning color
    #[arg(long, value_name = "COLOR")]
    pub warning_color: Option<String>,

    /// Error color
    #[arg(long, value_name = "COLOR")]
    pub error_color: Option<String>,

    /// Muted color
    #[arg(long, value_name = "COLOR")]
    pub muted_color: Option<String>,

    /// Border color
    #[arg(long, value_name = "COLOR")]
    pub border_color: Option<String>,

    /// Alert LED color
    #[arg(long, value_name = "COLOR")]
    pub alert_color: Option<String>,
}

#[derive(Args, Debug)]
pub struct ManArgs {
    /// Directory to write .1 files into (default: print the selected page)
    #[arg(long, value_name = "DIR")]
    pub output_dir: Option<PathBuf>,

    /// Subcommand name to render (default: the root page)
    #[arg(value_name = "NAME")]
    pub name: Option<String>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum CompletionShell {
    #[value(name = "bash")]
    Bash,
    #[value(name = "zsh")]
    Zsh,
    #[value(name = "fish")]
    Fish,
    #[value(name = "powershell")]
    PowerShell,
    #[value(name = "elvish")]
    Elvish,
}

impl CompletionShell {
    pub fn into_shell(self) -> clap_complete::Shell {
        match self {
            Self::Bash => clap_complete::Shell::Bash,
            Self::Zsh => clap_complete::Shell::Zsh,
            Self::Fish => clap_complete::Shell::Fish,
            Self::PowerShell => clap_complete::Shell::PowerShell,
            Self::Elvish => clap_complete::Shell::Elvish,
        }
    }
}

#[derive(Args, Debug)]
pub struct CompletionsArgs {
    /// Shell to generate completions for
    #[arg(value_enum)]
    pub shell: CompletionShell,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum OnOff {
    #[value(name = "on")]
    On,
    #[value(name = "off")]
    Off,
}

impl OnOff {
    pub fn is_on(self) -> bool {
        matches!(self, Self::On)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[test]
    fn command_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn mcp_requires_the_build_feature_and_explicit_subcommand() {
        let result = Cli::try_parse_from(["subraumctl", "mcp", "--config", "permissions.toml"]);
        #[cfg(feature = "mcp")]
        assert!(matches!(result.unwrap().command, CliCommand::Mcp(_)));
        #[cfg(not(feature = "mcp"))]
        assert!(result.is_err());
        assert!(Cli::try_parse_from(["subraumctl"]).is_err());
        assert!(matches!(
            Cli::try_parse_from(["subraumctl", "status"])
                .unwrap()
                .command,
            CliCommand::Status
        ));
    }

    #[test]
    fn parses_a_dmx_command() {
        let cli = Cli::try_parse_from(["subraumctl", "dmx", "subraum.all", "rainbow"]).unwrap();
        match cli.command {
            CliCommand::Dmx(args) => {
                assert_eq!(args.group, DmxGroup::SubraumAll);
                assert_eq!(args.animation, DmxAnimation::Rainbow);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_dodekaeder_outlet() {
        let cli = Cli::try_parse_from(["subraumctl", "outlet", "dodekaeder", "on"]).unwrap();
        match cli.command {
            CliCommand::Outlet(args) => {
                assert_eq!(args.outlet, Outlet::Dodekaeder);
                assert!(args.state.is_on());
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_white_light_with_cells() {
        let cli = Cli::try_parse_from([
            "subraumctl",
            "white-light",
            "apply",
            "--warm",
            "1",
            "--cold",
            "2",
            "--cell",
            "LM3_A020A63498E7",
        ])
        .unwrap();
        match cli.command {
            CliCommand::WhiteLight(args) => match args.command {
                WhiteLightCommand::Apply(apply) => {
                    assert_eq!(apply.warm, 1);
                    assert_eq!(apply.cold, 2);
                    assert_eq!(apply.cells, vec!["LM3_A020A63498E7".to_string()]);
                }
                other => panic!("unexpected white-light command: {other:?}"),
            },
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn rejects_out_of_range_laser_power() {
        assert!(Cli::try_parse_from(["subraumctl", "smoothiesaur", "laser", "256"]).is_err());
    }

    #[test]
    fn parses_tui_theme_colors() {
        let cli = Cli::try_parse_from([
            "subraumctl",
            "tui",
            "--theme",
            "high-contrast",
            "--accent-color",
            "teal",
            "--error-color",
            "rgb(255,80,80)",
        ])
        .unwrap();
        match cli.command {
            CliCommand::Tui(args) => {
                assert_eq!(args.theme, ThemePreset::HighContrast);
                assert_eq!(args.accent_color.as_deref(), Some("teal"));
                assert_eq!(args.error_color.as_deref(), Some("rgb(255,80,80)"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
