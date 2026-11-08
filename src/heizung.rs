//! Heating commands for the EDI AMQP service. Readings are setpoints, not sensors.

use std::time::Duration;

use clap::ValueEnum;
use eyre::{Result, ensure, eyre};
use serde_json::{Value, json};

pub mod transport;

pub const DEFAULT_AMQP_URL: &str = "amqp://amqp.subraum.c3pb.de:5672/%2f";
pub const DEFAULT_DURATION: u32 = 180;
// The firmware interprets the two timer registers as signed 32-bit milliseconds.
pub const MAX_DURATION: u32 = i32::MAX as u32 / 60_000;

#[derive(Clone)]
pub struct Options {
    pub amqp_url: String,
    pub timeout: Duration,
}

impl Options {
    pub fn from_global(global: &crate::cli::GlobalArgs) -> Self {
        Self {
            amqp_url: global.heizung_amqp_url.clone(),
            timeout: Duration::from_secs(global.heizung_timeout),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Room {
    Make,
    Hack,
    Cook,
    Elab,
    Shit,
    Storage,
}

impl Room {
    pub const ALL: [Self; 6] = [
        Self::Make,
        Self::Hack,
        Self::Cook,
        Self::Elab,
        Self::Shit,
        Self::Storage,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Make => "make",
            Self::Hack => "hack",
            Self::Cook => "cook",
            Self::Elab => "elab",
            Self::Shit => "shit",
            Self::Storage => "storage",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    rooms: Vec<Room>,
    setting: Option<(f64, u32)>,
}

impl Request {
    pub fn read_only(&self) -> bool {
        self.setting.is_none()
    }

    /// An omitted selection reads every known room.
    pub fn get(rooms: Vec<Room>) -> Result<Self> {
        let rooms = if rooms.is_empty() {
            Room::ALL.to_vec()
        } else {
            rooms
        };
        validate_rooms(&rooms)?;
        Ok(Self {
            rooms,
            setting: None,
        })
    }

    pub fn set(rooms: Vec<Room>, temperature: f64, duration: u32) -> Result<Self> {
        validate_rooms(&rooms)?;
        validate_setting(temperature, duration)?;
        Ok(Self {
            rooms,
            setting: Some(((temperature * 10.0).round() / 10.0, duration)),
        })
    }

    pub fn args(&self) -> String {
        let rooms = self
            .rooms
            .iter()
            .map(|room| room.name())
            .collect::<Vec<_>>()
            .join(" ");
        match self.setting {
            Some((temperature, duration)) => format!("set {rooms} {temperature:.1} -d {duration}"),
            None => format!("get {rooms}"),
        }
    }

    pub fn payload(&self, source: &str) -> Value {
        json!({"cmd": "heizung", "args": self.args(), "user": "subraumctl", "src": source, "dst": null})
    }

    /// Decode only the documented service reply, never arbitrary backend text.
    pub fn decode(&self, message: &str) -> Result<Value> {
        if let Some((temperature, duration)) = self.setting {
            let rooms = self
                .rooms
                .iter()
                .map(|room| room.name())
                .collect::<Vec<_>>()
                .join(",");
            let prefix = format!("Set temperature in {rooms} to ");
            let suffix = format!("°C for {duration} minutes");
            let actual = message
                .strip_prefix(&prefix)
                .and_then(|text| text.strip_suffix(&suffix))
                .and_then(|text| text.parse::<f64>().ok());
            ensure!(
                actual == Some(temperature),
                "heizung did not acknowledge the requested setting; check status before retrying"
            );
            return Ok(
                json!({"rooms": self.rooms.iter().map(|room| room.name()).collect::<Vec<_>>(),
                "target_celsius": temperature, "duration_minutes": duration, "acknowledged": true}),
            );
        }

        let mut readings = Vec::new();
        for line in message.lines() {
            let (name, value) = line.split_once(": ").ok_or_else(invalid_status)?;
            let room = Room::from_str(name, false).map_err(|_| invalid_status())?;
            ensure!(
                self.rooms.contains(&room) && !readings.iter().any(|(seen, _, _)| *seen == room),
                "unexpected or duplicate heizung room"
            );
            let (target, remaining) = value.split_once(", ").ok_or_else(invalid_status)?;
            let target: f64 = target.parse().map_err(|_| invalid_status())?;
            ensure!(
                target.is_finite() && (0.0..=6553.5).contains(&target),
                "invalid heizung target temperature"
            );
            let remaining = remaining
                .strip_suffix(" remaining")
                .ok_or_else(invalid_status)?;
            let parts = remaining
                .split(':')
                .map(str::parse::<u32>)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| invalid_status())?;
            ensure!(
                parts.len() == 3 && parts[1] < 60 && parts[2] < 60,
                "invalid heizung remaining time"
            );
            let seconds = parts[0]
                .checked_mul(3600)
                .and_then(|hours| hours.checked_add(parts[1] * 60 + parts[2]))
                .filter(|seconds| *seconds <= i32::MAX as u32 / 1000)
                .ok_or_else(invalid_status)?;
            readings.push((room, target, seconds));
        }
        ensure!(
            readings.len() == self.rooms.len(),
            "incomplete heizung status reply"
        );
        let rooms = self.rooms.iter().map(|room| {
            let (_, target, remaining) = readings.iter().find(|(seen, _, _)| seen == room).expect("validated room set");
            json!({"room": room.name(), "target_celsius": target, "remaining_seconds": remaining})
        }).collect::<Vec<_>>();
        Ok(json!({"rooms": rooms}))
    }
}

pub fn validate_setting(temperature: f64, duration: u32) -> Result<()> {
    ensure!(
        temperature.is_finite() && (15.0..=25.0).contains(&temperature),
        "temperature must be between 15 and 25 degrees Celsius"
    );
    ensure!(
        (temperature * 10.0 - (temperature * 10.0).round()).abs() < 1e-9,
        "temperature must use steps of 0.1 degrees Celsius"
    );
    ensure!(
        duration <= MAX_DURATION,
        "duration must be 0..={MAX_DURATION} minutes"
    );
    Ok(())
}

fn validate_rooms(rooms: &[Room]) -> Result<()> {
    ensure!(
        !rooms.is_empty() && rooms.len() <= Room::ALL.len(),
        "select one to six rooms"
    );
    for (index, room) in rooms.iter().enumerate() {
        ensure!(
            !rooms[..index].contains(room),
            "duplicate room: {}",
            room.name()
        );
    }
    Ok(())
}

fn invalid_status() -> eyre::Report {
    eyre!("invalid heizung status reply")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_vocabulary_and_wire_envelope() {
        let request = Request::set(vec![Room::Hack, Room::Make], 21.5, DEFAULT_DURATION).unwrap();
        assert_eq!(
            request.payload("subraumctl.recv.test"),
            json!({
                "cmd": "heizung", "args": "set hack make 21.5 -d 180", "user": "subraumctl",
                "src": "subraumctl.recv.test", "dst": null
            })
        );
        assert_eq!(
            Request::get(vec![]).unwrap().args(),
            "get make hack cook elab shit storage"
        );
    }

    #[test]
    fn settings_respect_firmware_limits_before_io() {
        for temperature in [f64::NAN, f64::INFINITY, 14.9, 25.1, 20.55] {
            assert!(Request::set(vec![Room::Make], temperature, 180).is_err());
        }
        for temperature in [15.0, 20.1, 25.0] {
            for duration in [0, DEFAULT_DURATION, MAX_DURATION] {
                assert!(Request::set(vec![Room::Make], temperature, duration).is_ok());
            }
        }
        assert!(Request::set(vec![Room::Make], 20.0, MAX_DURATION + 1).is_err());
        assert!(Request::set(vec![], 20.0, 180).is_err());
        assert!(Request::get(vec![Room::Make, Room::Make]).is_err());
    }

    #[test]
    fn status_is_scoped_complete_and_structured() {
        let request = Request::get(vec![Room::Hack, Room::Make]).unwrap();
        assert_eq!(
            request
                .decode("make: 18.0, 00:00:00 remaining\nhack: 21.5, 02:59:58 remaining")
                .unwrap(),
            json!({"rooms": [{"room": "hack", "target_celsius": 21.5, "remaining_seconds": 10798},
                {"room": "make", "target_celsius": 18.0, "remaining_seconds": 0}]})
        );
        for reply in [
            "",
            "Command failed",
            "hack: 21.0, 00:00:00 remaining",
            "hack: NaN, 00:00:00 remaining\nmake: 18, 00:00:00 remaining",
            "hack: 21, 00:60:00 remaining\nmake: 18, 00:00:00 remaining",
            "hack: 21, 999999999:00:00 remaining\nmake: 18, 00:00:00 remaining",
            "hack: 21, 00:00:00 remaining\nhack: 21, 00:00:00 remaining",
            "hack: 21, 00:00:00 remaining\nstorage: 18, 00:00:00 remaining",
        ] {
            assert!(request.decode(reply).is_err(), "{reply}");
        }
    }

    #[test]
    fn writes_require_matching_service_acknowledgement() {
        let request = Request::set(vec![Room::Hack], 21.0, 180).unwrap();
        assert_eq!(
            request
                .decode("Set temperature in hack to 21.0°C for 180 minutes")
                .unwrap()["acknowledged"],
            true
        );
        for reply in [
            "Command failed",
            "Set temperature in make to 21.0°C for 180 minutes",
            "Set temperature in hack to 22.0°C for 180 minutes",
        ] {
            assert!(request.decode(reply).is_err());
        }
        let request = Request::set(vec![Room::Hack], 20.1 + f64::EPSILON * 16.0, 180).unwrap();
        assert!(
            request
                .decode("Set temperature in hack to 20.1°C for 180 minutes")
                .is_ok()
        );
    }
}
