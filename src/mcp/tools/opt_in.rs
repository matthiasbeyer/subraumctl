//! Explicit opt-ins for device power, Smoothiesaur, and raw MQTT writes.

use std::sync::Arc;

use eyre::{Result, ensure};
use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;

use super::super::backend::Backend;
use super::{Empty, Group, McpServer, add_write, choice::Choice};
use crate::action::{ControlCommand, Outlet, SmoothieRelay};
use crate::cli::OnOff;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OutletSet {
    outlet: Choice<Outlet>,
    state: Choice<OnOff>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Relay {
    relay: Choice<SmoothieRelay>,
    state: Choice<OnOff>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Laser {
    power: u8,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Publish {
    topic: String,
    /// Literal payload text. Files are never read.
    payload: String,
    #[serde(default)]
    retain: bool,
}

pub(super) fn register<B: Backend>(server: &mut McpServer, backend: Arc<B>) -> Result<()> {
    add_write(
        server,
        "outlet",
        "Switch a named outlet. Dangerous: may interrupt connected equipment.",
        Group::Outlet,
        backend.clone(),
        |args: OutletSet| {
            Ok(ControlCommand::Outlet {
                outlet: args.outlet.0,
                on: args.state.0.is_on(),
            })
        },
    )?;
    add_write(
        server,
        "smoothiesaur_relay",
        "Switch a Smoothiesaur relay. Dangerous: controls physical machinery.",
        Group::Smoothiesaur,
        backend.clone(),
        |args: Relay| {
            Ok(ControlCommand::SmoothieRelay {
                relay: args.relay.0,
                on: args.state.0.is_on(),
            })
        },
    )?;
    add_write(
        server,
        "smoothiesaur_laser",
        "Set Smoothiesaur laser power (0–255). Dangerous: controls a physical laser.",
        Group::Smoothiesaur,
        backend.clone(),
        |args: Laser| Ok(ControlCommand::SmoothieLaser { power: args.power }),
    )?;
    add_write(
        server,
        "smoothiesaur_zero",
        "Set Smoothiesaur laser power to zero.",
        Group::Smoothiesaur,
        backend.clone(),
        |_: Empty| Ok(ControlCommand::SmoothieLaserZero),
    )?;
    add_write(
        server,
        "publish",
        "Publish literal text to ANY MQTT topic, optionally retained. Bypasses other tool-group restrictions; no topic/payload filtering.",
        Group::Publish,
        backend,
        |args: Publish| {
            ensure!(!args.topic.is_empty(), "MQTT topic must not be empty");
            Ok(ControlCommand::RawPublish {
                topic: args.topic,
                payload: args.payload,
                retain: args.retain,
            })
        },
    )
}
