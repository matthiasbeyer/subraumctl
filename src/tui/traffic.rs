//! Wire-traffic ring buffer shown below the content area.
//!
//! This is deliberately separate from the application log: the traffic pane
//! answers "what did we send and what did the current section receive?".

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::style::Color;

use crate::action::{
    Action, TOPIC_ALERT_CONDITION, TOPIC_AUDIO_ATTACH, TOPIC_AUDIO_DETACH, TOPIC_AUDIO_MUTE,
    TOPIC_AUDIO_UNMUTE, TOPIC_AUDIO_VOLUME, TOPIC_DMX, TOPIC_KNOB_GET_PREFIX,
    TOPIC_KNOB_SET_PREFIX, TOPIC_OUTLET, TOPIC_RADIATION_HACK, TOPIC_RADIATION_MAKE,
    TOPIC_SMOOTHIE_PREFIX, TOPIC_SOB, TOPIC_TELINIT, TOPIC_WHITE_LIGHT_REQUEST_PREFIX,
    TOPIC_WHITE_LIGHT_STATUS_PREFIX,
};
use crate::color::Theme;
use crate::tui::app::Section;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TrafficDirection {
    Tx,
    Rx,
    Http,
    Error,
    Dry,
}

impl TrafficDirection {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Tx => "↑",
            Self::Rx => "↓",
            Self::Http => "→",
            Self::Error => "!",
            Self::Dry => "◇",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Tx => "TX",
            Self::Rx => "RX",
            Self::Http => "HTTP",
            Self::Error => "ERR",
            Self::Dry => "DRY",
        }
    }

    pub fn color(self, theme: &Theme) -> Color {
        match self {
            Self::Tx | Self::Http => theme.accent,
            Self::Rx => theme.success,
            Self::Error => theme.error,
            Self::Dry => theme.muted,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TrafficEntry {
    pub at: String,
    pub direction: TrafficDirection,
    pub transport: &'static str,
    pub target: String,
    pub payload: String,
    pub note: Option<String>,
    pub sections: u16,
}

impl TrafficEntry {
    pub fn from_action(action: &Action, direction: TrafficDirection, note: Option<String>) -> Self {
        Self {
            at: now_hms(),
            direction,
            transport: action.transport().as_str(),
            target: action.target().to_string(),
            payload: action.payload().to_string(),
            note,
            sections: sections_for_action(action),
        }
    }

    pub fn received(topic: &str, payload: &str) -> Self {
        Self {
            at: now_hms(),
            direction: TrafficDirection::Rx,
            transport: "mqtt",
            target: topic.to_string(),
            payload: payload.to_string(),
            note: None,
            sections: sections_for_topic(topic),
        }
    }

    pub fn published(topic: &str, payload: &str, retain: bool) -> Self {
        Self {
            at: now_hms(),
            direction: TrafficDirection::Tx,
            transport: "mqtt",
            target: topic.to_string(),
            payload: payload.to_string(),
            note: retain.then(|| "retained".to_string()),
            sections: sections_for_topic(topic),
        }
    }

    pub fn http(url: &str, body: &str) -> Self {
        Self {
            at: now_hms(),
            direction: TrafficDirection::Http,
            transport: "http",
            target: url.to_string(),
            payload: body.to_string(),
            note: None,
            sections: Section::Overview.bit() | Section::Smoothie.bit(),
        }
    }

    pub fn error(target: &str, payload: &str, reason: &str, sections: u16) -> Self {
        Self {
            at: now_hms(),
            direction: TrafficDirection::Error,
            transport: "mqtt",
            target: target.to_string(),
            payload: payload.to_string(),
            note: Some(reason.to_string()),
            sections,
        }
    }

    pub fn http_error(url: &str, body: &str, reason: &str) -> Self {
        Self {
            at: now_hms(),
            direction: TrafficDirection::Error,
            transport: "http",
            target: url.to_string(),
            payload: body.to_string(),
            note: Some(reason.to_string()),
            sections: Section::Overview.bit() | Section::Smoothie.bit(),
        }
    }

    pub fn detail(&self) -> String {
        let mut lines = vec![
            format!(
                "{}  {}  {}",
                self.at,
                self.direction.label(),
                self.transport
            ),
            format!("target: {}", self.target),
            format!("payload: {}", self.payload),
        ];
        if let Some(note) = &self.note {
            lines.push(format!("note: {note}"));
        }
        lines.push(format!("sections mask: 0x{:03x}", self.sections));
        lines.join("\n")
    }
}

#[derive(Clone, Debug)]
pub struct TrafficLog {
    entries: VecDeque<TrafficEntry>,
    capacity: usize,
}

impl Default for TrafficLog {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            capacity: 1000,
        }
    }
}

impl TrafficLog {
    pub fn push(&mut self, entry: TrafficEntry) {
        self.entries.push_back(entry);
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
    }

    pub fn filtered(&self, section: Section) -> Vec<&TrafficEntry> {
        if section == Section::Overview {
            self.entries.iter().collect()
        } else {
            let bit = section.bit();
            self.entries
                .iter()
                .filter(|entry| entry.sections & bit != 0)
                .collect()
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

pub fn sections_for_topic(topic: &str) -> u16 {
    let mut mask = Section::Overview.bit();
    if topic == TOPIC_DMX {
        mask |= Section::Lighting.bit();
    } else if topic.starts_with(TOPIC_WHITE_LIGHT_REQUEST_PREFIX)
        || topic.starts_with(TOPIC_WHITE_LIGHT_STATUS_PREFIX)
    {
        mask |= Section::WhiteLight.bit();
    } else if topic == TOPIC_OUTLET {
        mask |= Section::Outlets.bit();
    } else if topic == TOPIC_RADIATION_MAKE
        || topic == TOPIC_RADIATION_HACK
        || topic == TOPIC_SOB
        || topic == TOPIC_ALERT_CONDITION
    {
        mask |= Section::Alerts.bit();
    } else if topic == TOPIC_TELINIT {
        mask |= Section::System.bit();
    } else if topic == TOPIC_AUDIO_MUTE
        || topic == TOPIC_AUDIO_UNMUTE
        || topic == TOPIC_AUDIO_DETACH
        || topic == TOPIC_AUDIO_ATTACH
        || topic == TOPIC_AUDIO_VOLUME
    {
        mask |= Section::Audio.bit();
    } else if topic.starts_with(TOPIC_KNOB_GET_PREFIX) || topic.starts_with(TOPIC_KNOB_SET_PREFIX) {
        mask |= Section::Animation.bit();
    } else if topic.starts_with(TOPIC_SMOOTHIE_PREFIX) {
        mask |= Section::Smoothie.bit();
    }
    mask
}

pub fn sections_for_action(action: &Action) -> u16 {
    match action {
        Action::Publish { topic, .. } => sections_for_topic(topic),
        Action::Http { .. } => Section::Overview.bit() | Section::Smoothie.bit(),
    }
}

pub fn now_hms() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let hour = (seconds / 3600) % 24;
    let minute = (seconds / 60) % 60;
    let second = seconds % 60;
    format!("{hour:02}:{minute:02}:{second:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_are_tagged_with_the_expected_sections() {
        assert_ne!(sections_for_topic(TOPIC_DMX) & Section::Lighting.bit(), 0);
        assert_ne!(
            sections_for_topic("light/esp/status/LM3_A020A63498E7") & Section::WhiteLight.bit(),
            0
        );
        assert_ne!(
            sections_for_topic("knob/get/cx") & Section::Animation.bit(),
            0
        );
        assert_ne!(
            sections_for_topic("smoothie/coolant/flow") & Section::Smoothie.bit(),
            0
        );
    }

    #[test]
    fn ring_buffer_evicts_old_entries() {
        let mut log = TrafficLog::default();
        for index in 0..1100 {
            log.push(TrafficEntry::received(&format!("test/{index}"), "x"));
        }
        assert_eq!(log.len(), 1000);
    }

    #[test]
    fn overview_sees_everything() {
        let mut log = TrafficLog::default();
        log.push(TrafficEntry::received(TOPIC_DMX, "x"));
        log.push(TrafficEntry::received("smoothie/coolant/flow", "1"));
        assert_eq!(log.filtered(Section::Overview).len(), 2);
        assert_eq!(log.filtered(Section::Lighting).len(), 1);
    }
}
