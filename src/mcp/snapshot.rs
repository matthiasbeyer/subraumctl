//! Fixed-topic reads and explicit output projections. Never forward raw payloads.

use std::collections::HashMap;
use std::time::Instant;

use serde_json::{Value, json};

use crate::action::{
    SmoothieRelay, TOPIC_ALERT_CONDITION, TOPIC_KNOB_GET_PREFIX, TOPIC_WHITE_LIGHT_STATUS_PREFIX,
};
use crate::decode::parse_alert;
use crate::knob::Knob;
use crate::smoothiesaur::{
    FLOW_TOPIC, LASER_TOPIC, RELAY_TOPIC_PREFIX, SAFETY_TOPIC, SmoothieStatus, TEMP_TOPICS,
};
use crate::white_light;

#[derive(Clone, Copy, Debug)]
pub(super) enum Snapshot {
    Alert,
    WhiteLight,
    Knob(Knob),
    Smoothiesaur,
    Status,
}

impl Snapshot {
    pub fn topics(self) -> Vec<String> {
        match self {
            Self::Alert | Self::Status => vec![TOPIC_ALERT_CONDITION.into()],
            Self::WhiteLight => white_light::ids()
                .map(|id| format!("{TOPIC_WHITE_LIGHT_STATUS_PREFIX}{id}"))
                .collect(),
            Self::Knob(knob) => vec![format!("{TOPIC_KNOB_GET_PREFIX}{}", knob.name())],
            Self::Smoothiesaur => SmoothieRelay::ALL
                .into_iter()
                .map(|relay| format!("{RELAY_TOPIC_PREFIX}{}", relay.topic_name()))
                .chain(TEMP_TOPICS.iter().map(|(topic, _)| (*topic).to_string()))
                .chain([FLOW_TOPIC, SAFETY_TOPIC, LASER_TOPIC].map(str::to_string))
                .collect(),
        }
    }

    pub fn decode(self, messages: &HashMap<String, String>) -> Value {
        let alert = || {
            messages.get(TOPIC_ALERT_CONDITION).map(|payload| {
                let alert = parse_alert(payload);
                json!({"condition": alert.condition.label(), "dim": alert.dim})
            })
        };
        match self {
            Self::Alert => json!({"alert": alert()}),
            Self::Status => json!({"connected": true, "alert": alert()}),
            Self::Knob(knob) => json!({"knob": knob.name(), "value": messages
                .get(&format!("{TOPIC_KNOB_GET_PREFIX}{}", knob.name()))
                .and_then(|value| value.trim().parse::<f64>().ok()).filter(|value| value.is_finite())}),
            Self::WhiteLight => json!({"cells": white_light::CELLS.iter().map(|cell| {
                let on = messages.get(&format!("{TOPIC_WHITE_LIGHT_STATUS_PREFIX}{}", cell.id))
                    .and_then(|value| boolean(value));
                json!({"id": cell.id, "group": cell.group.label(), "on": on})
            }).collect::<Vec<_>>()}),
            Self::Smoothiesaur => {
                let mut state = SmoothieStatus::default();
                // Only the fixed topic catalogue is allowed to influence the result.
                for topic in self.topics() {
                    if let Some(payload) = messages.get(&topic) {
                        state.apply(&topic, payload, Instant::now());
                    }
                }
                json!({
                    "relays": SmoothieRelay::ALL.iter().map(|relay| json!({
                        "relay": relay.topic_name(),
                        "on": messages.get(&format!("{RELAY_TOPIC_PREFIX}{}", relay.topic_name())).and_then(|value| boolean(value))
                    })).collect::<Vec<_>>(),
                    "temperatures": TEMP_TOPICS.iter().enumerate().map(|(i, (_, label))|
                        json!({"sensor": label, "celsius": finite(state.temps[i])})
                    ).collect::<Vec<_>>(),
                    "flow_counter": state.flow_raw,
                    "safety": state.safety_bits().iter().map(|(_, label, on)|
                        json!({"switch": label, "on": state.safety.map(|_| *on)})
                    ).collect::<Vec<_>>(),
                    "laser": {"control_8_percent": finite(state.laser.control_8),
                        "control_16_percent": finite(state.laser.control_16), "volts": finite(state.laser.volts)}
                })
            }
        }
    }
}

fn finite(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite())
}

fn boolean(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "y" => Some(true),
        "false" | "off" | "no" | "n" => Some(false),
        value => value
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .map(|v| v != 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_cannot_request_wildcards_or_forward_unreviewed_data() {
        let secret = "private-user-data";
        let mut messages = HashMap::from([
            ("smoothie/private".into(), secret.into()),
            ("light/esp/status/private-device".into(), secret.into()),
            (TOPIC_ALERT_CONDITION.into(), format!("green {secret}")),
            (format!("{TOPIC_KNOB_GET_PREFIX}cx"), "NaN".into()),
            (TEMP_TOPICS[0].0.into(), "inf".into()),
        ]);
        for snapshot in [
            Snapshot::Alert,
            Snapshot::WhiteLight,
            Snapshot::Knob(Knob::Cx),
            Snapshot::Smoothiesaur,
            Snapshot::Status,
        ] {
            assert!(
                snapshot
                    .topics()
                    .iter()
                    .all(|topic| !topic.contains(['#', '+']))
            );
            let value = snapshot.decode(&messages).to_string();
            assert!(!value.contains(secret));
            assert!(!value.contains("private-device"));
            assert!(!value.contains("NaN"));
            assert!(!value.contains("inf"));
        }
        assert!(Snapshot::Knob(Knob::Cx).decode(&messages)["value"].is_null());
        messages.insert(format!("{TOPIC_KNOB_GET_PREFIX}cx"), "42".into());
        assert_eq!(Snapshot::Knob(Knob::Cx).decode(&messages)["value"], 42.0);
        assert_eq!(
            Snapshot::Status.decode(&messages)["alert"]["condition"],
            "Code: Green"
        );
    }
}
