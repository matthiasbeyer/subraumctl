use std::sync::Arc;

use eyre::{Result, ensure};
use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;

use super::super::backend::Backend;
use super::{Group, McpServer, add_write, choice::Choice};
use crate::action::{ControlCommand, RadiationTarget, SobSound};
use crate::cli::OnOff;
use crate::knob::Knob;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Radiation {
    target: Choice<RadiationTarget>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Sound {
    sound: Choice<SobSound>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Telinit {
    state: Choice<OnOff>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct KnobSet {
    knob: Choice<Knob>,
    value: f64,
}

pub(super) fn register<B: Backend>(server: &mut McpServer, backend: Arc<B>) -> Result<()> {
    add_write(
        server,
        "alert_radiation",
        "Trigger the rotating beacon in make or hack.",
        Group::Alert,
        backend.clone(),
        |args: Radiation| {
            Ok(ControlCommand::Radiation {
                target: args.target.0,
            })
        },
    )?;
    add_write(
        server,
        "alert_sob",
        "Play a predefined sound.",
        Group::Alert,
        backend.clone(),
        |args: Sound| {
            Ok(ControlCommand::Sob {
                sound: args.sound.0,
            })
        },
    )?;
    add_write(
        server,
        "system_telinit",
        "Send system on/off (telinit 4/0). Off shuts down room systems.",
        Group::System,
        backend.clone(),
        |args: Telinit| {
            Ok(ControlCommand::Telinit {
                on: args.state.0.is_on(),
            })
        },
    )?;
    add_write(
        server,
        "knob_set",
        "Set a named animation knob to a finite numeric value.",
        Group::Knob,
        backend,
        |args: KnobSet| {
            ensure!(args.value.is_finite(), "knob value must be finite");
            Ok(ControlCommand::KnobSet {
                knob: args.knob.0,
                value: args.value,
            })
        },
    )
}
