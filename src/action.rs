//! The single source of truth for MQTT topics and payloads.
//!
//! Both the CLI and the TUI construct [`ControlCommand`] values and expand
//! them here. This keeps the wire format in one place and gives the safe-mode
//! dialog and the traffic pane an exact preview of what will be sent.

use clap::ValueEnum;
use serde_json::json;
use thiserror::Error;

use crate::knob::{Knob, format_value};
use crate::white_light;

pub const TOPIC_DMX: &str = "edi/cmd/dmx";
pub const TOPIC_OUTLET: &str = "edi/cmd/act";
pub const TOPIC_TELINIT: &str = "edi/cmd/telinit";
pub const TOPIC_SOB: &str = "edi.cmd.sob";
pub const TOPIC_RADIATION_MAKE: &str = "Rundumleuchte/Gelblicht";
pub const TOPIC_RADIATION_HACK: &str = "Rundumleuchte/Blaulicht";
pub const TOPIC_ALERT_CONDITION: &str = "heimdallr/alert/condition";
pub const TOPIC_SMOOTHIE_PREFIX: &str = "smoothie/";
pub const TOPIC_KNOB_GET_PREFIX: &str = "knob/get/";
pub const TOPIC_KNOB_SET_PREFIX: &str = "knob/set/";
pub const TOPIC_WHITE_LIGHT_REQUEST_PREFIX: &str = "light/esp/request/";
pub const TOPIC_WHITE_LIGHT_STATUS_PREFIX: &str = "light/esp/status/";
pub const TOPIC_AUDIO_MUTE: &str = "edi/cmd/mute";
pub const TOPIC_AUDIO_UNMUTE: &str = "edi/cmd/unmute";
pub const TOPIC_AUDIO_DETACH: &str = "edi/cmd/amd";
pub const TOPIC_AUDIO_ATTACH: &str = "edi/cmd/ama";
pub const TOPIC_AUDIO_VOLUME: &str = "edi/cmd/volume";

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum DmxGroup {
    #[value(name = "subraum.all", alias = "subraum-all")]
    SubraumAll,
    #[value(name = "hack.all", alias = "hack-all")]
    HackAll,
    #[value(name = "hack.afterglow", alias = "hack-afterglow")]
    HackAfterglow,
    #[value(name = "hack.spots", alias = "hack-spots")]
    HackSpots,
    #[value(name = "hack.floor", alias = "hack-floor")]
    HackFloor,
    #[value(name = "hack.decke", alias = "hack-decke")]
    HackDecke,
    #[value(name = "hack.regal", alias = "hack-regal")]
    HackRegal,
    #[value(name = "make.all", alias = "make-all")]
    MakeAll,
    #[value(name = "make.decke", alias = "make-decke")]
    MakeDecke,
    #[value(name = "make.spots", alias = "make-spots")]
    MakeSpots,
    #[value(name = "flur")]
    Flur,
}

impl DmxGroup {
    pub const ALL: [Self; 11] = [
        Self::SubraumAll,
        Self::HackAll,
        Self::HackAfterglow,
        Self::HackSpots,
        Self::HackFloor,
        Self::HackDecke,
        Self::HackRegal,
        Self::MakeAll,
        Self::MakeDecke,
        Self::MakeSpots,
        Self::Flur,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SubraumAll => "subraum.all",
            Self::HackAll => "hack.all",
            Self::HackAfterglow => "hack.afterglow",
            Self::HackSpots => "hack.spots",
            Self::HackFloor => "hack.floor",
            Self::HackDecke => "hack.decke",
            Self::HackRegal => "hack.regal",
            Self::MakeAll => "make.all",
            Self::MakeDecke => "make.decke",
            Self::MakeSpots => "make.spots",
            Self::Flur => "flur",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum DmxAnimation {
    #[value(name = "alert")]
    Alert,
    #[value(name = "backgrounda")]
    Backgrounda,
    #[value(name = "backgroundb")]
    Backgroundb,
    #[value(name = "backgroundc")]
    Backgroundc,
    #[value(name = "backgroundd")]
    Backgroundd,
    #[value(name = "rainbow")]
    Rainbow,
    #[value(name = "white")]
    White,
    #[value(name = "black")]
    Black,
    #[value(name = "fullhour")]
    Fullhour,
}

impl DmxAnimation {
    pub const ALL: [Self; 9] = [
        Self::Alert,
        Self::Backgrounda,
        Self::Backgroundb,
        Self::Backgroundc,
        Self::Backgroundd,
        Self::Rainbow,
        Self::White,
        Self::Black,
        Self::Fullhour,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Alert => "alert",
            Self::Backgrounda => "backgrounda",
            Self::Backgroundb => "backgroundb",
            Self::Backgroundc => "backgroundc",
            Self::Backgroundd => "backgroundd",
            Self::Rainbow => "rainbow",
            Self::White => "white",
            Self::Black => "black",
            Self::Fullhour => "fullhour",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Backgrounda => "background a",
            Self::Backgroundb => "background b",
            Self::Backgroundc => "background c",
            Self::Backgroundd => "background d",
            other => other.as_str(),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Outlet {
    #[value(name = "make.colors", alias = "make-colors")]
    MakeColors,
    #[value(name = "make.bright", alias = "make-bright")]
    MakeBright,
    #[value(name = "ert")]
    Ert,
    #[value(name = "stationaere_kuehlungsloesung")]
    StationaereKuehlungsloesung,
    #[value(name = "compressor")]
    Compressor,
    #[value(name = "white-power")]
    WhitePower,
    #[value(name = "dodekaeder")]
    Dodekaeder,
}

impl Outlet {
    pub const ALL: [Self; 7] = [
        Self::MakeColors,
        Self::MakeBright,
        Self::Ert,
        Self::StationaereKuehlungsloesung,
        Self::Compressor,
        Self::WhitePower,
        Self::Dodekaeder,
    ];

    pub fn topic_name(self) -> &'static str {
        match self {
            Self::MakeColors => "make.colors",
            Self::MakeBright => "make.bright",
            Self::Ert => "ert",
            Self::StationaereKuehlungsloesung => "stationaere_kuehlungsloesung",
            Self::Compressor => "compressor",
            Self::WhitePower => "white-power",
            Self::Dodekaeder => "dodekaeder",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::MakeColors => "make.colors",
            Self::MakeBright => "make.bright",
            Self::Ert => "ert",
            Self::StationaereKuehlungsloesung => "hackpad",
            Self::Compressor => "compressor",
            Self::WhitePower => "white power",
            Self::Dodekaeder => "dodekaeder",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum RadiationTarget {
    #[value(name = "make")]
    Make,
    #[value(name = "hack")]
    Hack,
}

impl RadiationTarget {
    pub fn topic(self) -> &'static str {
        match self {
            Self::Make => TOPIC_RADIATION_MAKE,
            Self::Hack => TOPIC_RADIATION_HACK,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Make => ".make",
            Self::Hack => ".hack",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum SobSound {
    #[value(name = "hackbuzzer")]
    Hackbuzzer,
    #[value(name = "dingdong")]
    Dingdong,
    #[value(name = "chewbacca")]
    Chewbacca,
    #[value(name = "noot-noot", alias = "noot")]
    NootNoot,
}

impl SobSound {
    pub const ALL: [Self; 4] = [
        Self::Hackbuzzer,
        Self::Dingdong,
        Self::Chewbacca,
        Self::NootNoot,
    ];

    pub fn payload_arg(self) -> &'static str {
        match self {
            Self::Hackbuzzer => "hackbuzzer",
            Self::Dingdong => "dingdong",
            Self::Chewbacca => "chewbacca",
            Self::NootNoot => "noot noot",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::NootNoot => "noot noot",
            other => other.payload_arg(),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum SmoothieRelay {
    #[value(name = "fan-port", alias = "fanPort")]
    FanPort,
    #[value(name = "fan-starboard", alias = "fanStarboard")]
    FanStarboard,
    #[value(name = "pump-primary", alias = "pumpPrimary")]
    PumpPrimary,
    #[value(name = "pump-secondary", alias = "pumpSecondary")]
    PumpSecondary,
    #[value(name = "target-laser", alias = "targetLaser")]
    TargetLaser,
    #[value(name = "air-valve", alias = "airValve")]
    AirValve,
    #[value(name = "sterilizer")]
    Sterilizer,
}

impl SmoothieRelay {
    pub const ALL: [Self; 7] = [
        Self::FanPort,
        Self::FanStarboard,
        Self::PumpPrimary,
        Self::PumpSecondary,
        Self::TargetLaser,
        Self::AirValve,
        Self::Sterilizer,
    ];

    pub fn topic_name(self) -> &'static str {
        match self {
            Self::FanPort => "fanPort",
            Self::FanStarboard => "fanStarboard",
            Self::PumpPrimary => "pumpPrimary",
            Self::PumpSecondary => "pumpSecondary",
            Self::TargetLaser => "targetLaser",
            Self::AirValve => "airValve",
            Self::Sterilizer => "sterilizer",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::FanPort => "port fan",
            Self::FanStarboard => "starboard fan",
            Self::PumpPrimary => "primary coolant pump",
            Self::PumpSecondary => "secondary coolant pump",
            Self::TargetLaser => "targeting laser",
            Self::AirValve => "air valve",
            Self::Sterilizer => "sterilizer",
        }
    }

    pub fn on_command(self) -> &'static str {
        match self {
            Self::FanPort => "M771I15P010100",
            Self::FanStarboard => "M771I15P010101",
            Self::PumpPrimary => "M771I15P010104",
            Self::PumpSecondary => "M771I15P010103",
            Self::TargetLaser => "M771I15P01010f",
            Self::AirValve => "M771I15P010107",
            Self::Sterilizer => "M771I15P010105",
        }
    }

    pub fn off_command(self) -> &'static str {
        match self {
            Self::FanPort => "M771I15P010000",
            Self::FanStarboard => "M771I15P010001",
            Self::PumpPrimary => "M771I15P010004",
            Self::PumpSecondary => "M771I15P010003",
            Self::TargetLaser => "M771I15P01000f",
            Self::AirValve => "M771I15P010007",
            Self::Sterilizer => "M771I15P010005",
        }
    }

    pub fn from_topic_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|relay| relay.topic_name() == name)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum AudioRoom {
    #[value(name = "hack")]
    Hack,
    #[value(name = "make")]
    Make,
    #[value(name = "cook")]
    Cook,
    #[value(name = "elab")]
    Elab,
}

impl AudioRoom {
    pub const ALL: [Self; 4] = [Self::Hack, Self::Make, Self::Cook, Self::Elab];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hack => "hack",
            Self::Make => "make",
            Self::Cook => "cook",
            Self::Elab => "elab",
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Hack => 0,
            Self::Make => 1,
            Self::Cook => 2,
            Self::Elab => 3,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Transport {
    Mqtt,
    Http,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mqtt => "mqtt",
            Self::Http => "http",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Publish {
        topic: String,
        payload: String,
        retain: bool,
    },
    Http {
        url: String,
        body: String,
    },
}

impl Action {
    pub fn transport(&self) -> Transport {
        match self {
            Self::Publish { .. } => Transport::Mqtt,
            Self::Http { .. } => Transport::Http,
        }
    }

    pub fn target(&self) -> &str {
        match self {
            Self::Publish { topic, .. } => topic,
            Self::Http { url, .. } => url,
        }
    }

    pub fn payload(&self) -> &str {
        match self {
            Self::Publish { payload, .. } => payload,
            Self::Http { body, .. } => body,
        }
    }

    pub fn retain(&self) -> bool {
        match self {
            Self::Publish { retain, .. } => *retain,
            Self::Http { .. } => false,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Publish {
                topic,
                payload,
                retain,
            } => {
                if *retain {
                    format!("MQTT {topic} (retain) <- {payload}")
                } else {
                    format!("MQTT {topic} <- {payload}")
                }
            }
            Self::Http { url, body } => {
                format!("HTTP {url} <- {}", body.trim_end())
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum ControlCommand {
    Dmx {
        group: DmxGroup,
        animation: DmxAnimation,
    },
    WhiteLight {
        warm: u8,
        cold: u8,
        cells: Vec<String>,
    },
    Outlet {
        outlet: Outlet,
        on: bool,
    },
    Radiation {
        target: RadiationTarget,
    },
    Sob {
        sound: SobSound,
    },
    Telinit {
        on: bool,
    },
    KnobSet {
        knob: Knob,
        value: f64,
    },
    SmoothieRelay {
        relay: SmoothieRelay,
        on: bool,
    },
    SmoothieLaser {
        power: u8,
    },
    SmoothieLaserZero,
    AudioMute {
        room: AudioRoom,
        muted: bool,
    },
    AudioDetach {
        room: AudioRoom,
        detached: bool,
    },
    AudioVolume {
        room: AudioRoom,
        percent: u8,
    },
    RawPublish {
        topic: String,
        payload: String,
        retain: bool,
    },
}

#[derive(Debug, Error)]
pub enum ActionError {
    #[error("no white-light cells selected")]
    NoCellsSelected,
    #[error("unknown white-light cell '{0}'")]
    UnknownCell(String),
    #[error("knob value must be finite")]
    NonFiniteValue,
    #[error("MQTT topic must not be empty")]
    EmptyTopic,
}

impl ControlCommand {
    pub fn expand(&self, smoothie_url: &str) -> Result<Vec<Action>, ActionError> {
        match self {
            Self::Dmx { group, animation } => Ok(vec![Action::Publish {
                topic: TOPIC_DMX.to_string(),
                payload: json!({
                    "args": format!("{} {}", group.as_str(), animation.as_str())
                })
                .to_string(),
                retain: false,
            }]),
            Self::WhiteLight { warm, cold, cells } => {
                if cells.is_empty() {
                    return Err(ActionError::NoCellsSelected);
                }
                let mut actions = Vec::with_capacity(cells.len());
                for cell in cells {
                    let known = white_light::find(cell)
                        .ok_or_else(|| ActionError::UnknownCell(cell.clone()))?;
                    actions.push(Action::Publish {
                        topic: format!("{TOPIC_WHITE_LIGHT_REQUEST_PREFIX}{}", known.id),
                        payload: format!("w {warm} c {cold} "),
                        retain: false,
                    });
                }
                Ok(actions)
            }
            Self::Outlet { outlet, on } => Ok(vec![Action::Publish {
                topic: TOPIC_OUTLET.to_string(),
                payload: json!({
                    "args": format!("{} {}", outlet.topic_name(), if *on { "on" } else { "off" })
                })
                .to_string(),
                retain: false,
            }]),
            Self::Radiation { target } => Ok(vec![Action::Publish {
                topic: target.topic().to_string(),
                payload: json!({
                    "name": "Rundumleuchte",
                    "time": "2"
                })
                .to_string(),
                retain: false,
            }]),
            Self::Sob { sound } => Ok(vec![Action::Publish {
                topic: TOPIC_SOB.to_string(),
                payload: json!({
                    "cmd": "sob",
                    "args": sound.payload_arg(),
                    "user": "subraumctl",
                    "src": "na"
                })
                .to_string(),
                retain: false,
            }]),
            Self::Telinit { on } => Ok(vec![Action::Publish {
                topic: TOPIC_TELINIT.to_string(),
                payload: json!({
                    "args": if *on { "4" } else { "0" }
                })
                .to_string(),
                retain: false,
            }]),
            Self::KnobSet { knob, value } => {
                if !value.is_finite() {
                    return Err(ActionError::NonFiniteValue);
                }
                Ok(vec![Action::Publish {
                    topic: format!("{TOPIC_KNOB_SET_PREFIX}{}", knob.name()),
                    payload: value.to_string(),
                    retain: false,
                }])
            }
            Self::SmoothieRelay { relay, on } => Ok(vec![Action::Http {
                url: smoothie_url.to_string(),
                body: format!(
                    "{}\n",
                    if *on {
                        relay.on_command()
                    } else {
                        relay.off_command()
                    }
                ),
            }]),
            Self::SmoothieLaser { power } => {
                let power = *power;
                Ok(vec![Action::Http {
                    url: smoothie_url.to_string(),
                    body: format!("M771I19P{power:02x}{power:02x}\n"),
                }])
            }
            Self::SmoothieLaserZero => Ok(vec![Action::Http {
                url: smoothie_url.to_string(),
                body: "M771I19P0000\n".to_string(),
            }]),
            Self::AudioMute { room, muted } => Ok(vec![Action::Publish {
                topic: if *muted {
                    TOPIC_AUDIO_MUTE
                } else {
                    TOPIC_AUDIO_UNMUTE
                }
                .to_string(),
                payload: json!({
                    "args": room.as_str(),
                    "user": "subraumctl"
                })
                .to_string(),
                retain: false,
            }]),
            Self::AudioDetach { room, detached } => Ok(vec![Action::Publish {
                topic: if *detached {
                    TOPIC_AUDIO_DETACH
                } else {
                    TOPIC_AUDIO_ATTACH
                }
                .to_string(),
                payload: json!({
                    "args": room.as_str(),
                    "user": "subraumctl"
                })
                .to_string(),
                retain: false,
            }]),
            Self::AudioVolume { room, percent } => Ok(vec![Action::Publish {
                topic: TOPIC_AUDIO_VOLUME.to_string(),
                payload: json!({
                    "args": format!("{} {}%", room.as_str(), percent),
                    "user": "subraumctl"
                })
                .to_string(),
                retain: false,
            }]),
            Self::RawPublish {
                topic,
                payload,
                retain,
            } => {
                if topic.is_empty() {
                    return Err(ActionError::EmptyTopic);
                }
                Ok(vec![Action::Publish {
                    topic: topic.clone(),
                    payload: payload.clone(),
                    retain: *retain,
                }])
            }
        }
    }

    pub fn description(&self) -> String {
        match self {
            Self::Dmx { group, animation } => {
                format!("DMX {} {}", group.as_str(), animation.as_str())
            }
            Self::WhiteLight { warm, cold, cells } => {
                format!("white light w={warm} c={cold} for {} cell(s)", cells.len())
            }
            Self::Outlet { outlet, on } => {
                format!(
                    "outlet {} {}",
                    outlet.label(),
                    if *on { "on" } else { "off" }
                )
            }
            Self::Radiation { target } => format!("radiation emitter {}", target.label()),
            Self::Sob { sound } => format!("sob sound {}", sound.label()),
            Self::Telinit { on } => format!("telinit {}", if *on { "on (4)" } else { "off (0)" }),
            Self::KnobSet { knob, value } => {
                format!("set knob {} = {}", knob.name(), format_value(*value))
            }
            Self::SmoothieRelay { relay, on } => format!(
                "smoothiesaur relay {} {}",
                relay.label(),
                if *on { "on" } else { "off" }
            ),
            Self::SmoothieLaser { power } => format!("smoothiesaur laser {power}"),
            Self::SmoothieLaserZero => "smoothiesaur laser zero".to_string(),
            Self::AudioMute { room, muted } => {
                format!(
                    "audio {} {}",
                    if *muted { "mute" } else { "unmute" },
                    room.as_str()
                )
            }
            Self::AudioDetach { room, detached } => format!(
                "audio {} {}",
                if *detached { "detach" } else { "attach" },
                room.as_str()
            ),
            Self::AudioVolume { room, percent } => {
                format!("audio volume {} {percent}%", room.as_str())
            }
            Self::RawPublish { topic, .. } => format!("publish {topic}"),
        }
    }

    pub fn destructive(&self) -> bool {
        match self {
            Self::Telinit { on } => !*on,
            Self::SmoothieLaserZero => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dmx_payload_matches_the_web_ui() {
        let actions = ControlCommand::Dmx {
            group: DmxGroup::SubraumAll,
            animation: DmxAnimation::Rainbow,
        }
        .expand("http://example/command")
        .unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0],
            Action::Publish {
                topic: TOPIC_DMX.to_string(),
                payload: r#"{"args":"subraum.all rainbow"}"#.to_string(),
                retain: false,
            }
        );
    }

    #[test]
    fn outlet_payload_uses_the_topic_name() {
        let actions = ControlCommand::Outlet {
            outlet: Outlet::Dodekaeder,
            on: true,
        }
        .expand("http://example/command")
        .unwrap();
        assert_eq!(
            actions[0],
            Action::Publish {
                topic: TOPIC_OUTLET.to_string(),
                payload: r#"{"args":"dodekaeder on"}"#.to_string(),
                retain: false,
            }
        );
    }

    #[test]
    fn white_light_expands_to_one_message_per_cell() {
        let actions = ControlCommand::WhiteLight {
            warm: 12,
            cold: 34,
            cells: vec![
                "LM3_A020A63498E7".to_string(),
                "LM3_18FE34FE4C87".to_string(),
            ],
        }
        .expand("http://example/command")
        .unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(
            actions[0],
            Action::Publish {
                topic: "light/esp/request/LM3_A020A63498E7".to_string(),
                payload: "w 12 c 34 ".to_string(),
                retain: false,
            }
        );
    }

    #[test]
    fn white_light_rejects_empty_or_unknown_cells() {
        assert!(matches!(
            ControlCommand::WhiteLight {
                warm: 1,
                cold: 2,
                cells: vec![],
            }
            .expand("http://example/command"),
            Err(ActionError::NoCellsSelected)
        ));
        assert!(matches!(
            ControlCommand::WhiteLight {
                warm: 1,
                cold: 2,
                cells: vec!["nope".to_string()],
            }
            .expand("http://example/command"),
            Err(ActionError::UnknownCell(_))
        ));
    }

    #[test]
    fn radiation_targets_are_correct() {
        let make = ControlCommand::Radiation {
            target: RadiationTarget::Make,
        }
        .expand("http://example/command")
        .unwrap();
        assert_eq!(make[0].target(), TOPIC_RADIATION_MAKE);
        let hack = ControlCommand::Radiation {
            target: RadiationTarget::Hack,
        }
        .expand("http://example/command")
        .unwrap();
        assert_eq!(hack[0].target(), TOPIC_RADIATION_HACK);
    }

    #[test]
    fn sob_noot_keeps_the_space() {
        let actions = ControlCommand::Sob {
            sound: SobSound::NootNoot,
        }
        .expand("http://example/command")
        .unwrap();
        assert_eq!(actions[0].target(), TOPIC_SOB);
        let value: serde_json::Value =
            serde_json::from_str(actions[0].payload()).expect("valid JSON payload");
        assert_eq!(value["args"].as_str(), Some("noot noot"));
        assert_eq!(value["cmd"].as_str(), Some("sob"));
        assert_eq!(value["user"].as_str(), Some("subraumctl"));
        assert_eq!(value["src"].as_str(), Some("na"));
    }

    #[test]
    fn telinit_uses_the_expected_values() {
        let on = ControlCommand::Telinit { on: true }
            .expand("http://example/command")
            .unwrap();
        assert_eq!(on[0].payload(), r#"{"args":"4"}"#);
        let off = ControlCommand::Telinit { on: false }
            .expand("http://example/command")
            .unwrap();
        assert_eq!(off[0].payload(), r#"{"args":"0"}"#);
        assert!(ControlCommand::Telinit { on: false }.destructive());
    }

    #[test]
    fn knob_set_uses_the_topic_name_and_decimal_string() {
        let actions = ControlCommand::KnobSet {
            knob: Knob::PulseLead,
            value: 0.25,
        }
        .expand("http://example/command")
        .unwrap();
        assert_eq!(
            actions[0],
            Action::Publish {
                topic: "knob/set/pulseLead".to_string(),
                payload: "0.25".to_string(),
                retain: false,
            }
        );
    }

    #[test]
    fn smoothie_commands_match_the_web_ui() {
        let relay = ControlCommand::SmoothieRelay {
            relay: SmoothieRelay::PumpPrimary,
            on: true,
        }
        .expand("http://device/command")
        .unwrap();
        assert_eq!(
            relay[0],
            Action::Http {
                url: "http://device/command".to_string(),
                body: "M771I15P010104\n".to_string(),
            }
        );
        let laser = ControlCommand::SmoothieLaser { power: 10 }
            .expand("http://device/command")
            .unwrap();
        assert_eq!(laser[0].payload(), "M771I19P0a0a\n");
        let zero = ControlCommand::SmoothieLaserZero
            .expand("http://device/command")
            .unwrap();
        assert_eq!(zero[0].payload(), "M771I19P0000\n");
    }

    #[test]
    fn all_smoothie_commands_are_known() {
        for relay in SmoothieRelay::ALL {
            assert_eq!(
                SmoothieRelay::from_topic_name(relay.topic_name()),
                Some(relay)
            );
            assert!(relay.on_command().starts_with("M771I15P"));
            assert!(relay.off_command().starts_with("M771I15P"));
        }
    }

    #[test]
    fn audio_commands_use_expected_topics_and_payloads() {
        let mute = ControlCommand::AudioMute {
            room: AudioRoom::Hack,
            muted: true,
        }
        .expand("")
        .unwrap();
        assert_eq!(mute[0].target(), TOPIC_AUDIO_MUTE);
        assert_eq!(mute[0].payload(), r#"{"args":"hack","user":"subraumctl"}"#);

        let unmute = ControlCommand::AudioMute {
            room: AudioRoom::Make,
            muted: false,
        }
        .expand("")
        .unwrap();
        assert_eq!(unmute[0].target(), TOPIC_AUDIO_UNMUTE);
        assert_eq!(
            unmute[0].payload(),
            r#"{"args":"make","user":"subraumctl"}"#
        );

        let detach = ControlCommand::AudioDetach {
            room: AudioRoom::Cook,
            detached: true,
        }
        .expand("")
        .unwrap();
        assert_eq!(detach[0].target(), TOPIC_AUDIO_DETACH);
        assert_eq!(
            detach[0].payload(),
            r#"{"args":"cook","user":"subraumctl"}"#
        );
        let attach = ControlCommand::AudioDetach {
            room: AudioRoom::Elab,
            detached: false,
        }
        .expand("")
        .unwrap();
        assert_eq!(attach[0].target(), TOPIC_AUDIO_ATTACH);
        assert_eq!(
            attach[0].payload(),
            r#"{"args":"elab","user":"subraumctl"}"#
        );

        let volume = ControlCommand::AudioVolume {
            room: AudioRoom::Cook,
            percent: 65,
        }
        .expand("")
        .unwrap();
        assert_eq!(volume[0].target(), TOPIC_AUDIO_VOLUME);
        assert_eq!(
            volume[0].payload(),
            r#"{"args":"cook 65%","user":"subraumctl"}"#
        );
    }
}
