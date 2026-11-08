use std::sync::Arc;

use eyre::{Result, ensure};
use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;

use super::super::backend::Backend;
use super::{Access, Group, McpServer, add, choice::Choice};
use crate::action::{ControlCommand, DmxAnimation, DmxGroup};
use crate::white_light;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Dmx {
    group: Choice<DmxGroup>,
    animation: Choice<DmxAnimation>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WhiteLight {
    warm: u8,
    cold: u8,
    /// Known cell IDs; select these or set all=true.
    #[serde(default)]
    cells: Vec<String>,
    #[serde(default)]
    all: bool,
}

pub(super) fn register<B: Backend>(server: &mut McpServer, backend: Arc<B>) -> Result<()> {
    let dmx = backend.clone();
    add(
        server,
        "dmx",
        "Select a DMX animation for a known lighting group.",
        Group::Dmx,
        Access::Write,
        move |args: Dmx, context| {
            let backend = dmx.clone();
            async move {
                backend
                    .send(
                        ControlCommand::Dmx {
                            group: args.group.0,
                            animation: args.animation.0,
                        },
                        context,
                    )
                    .await
            }
        },
    )?;
    add(
        server,
        "white_light_apply",
        "Set warm/cold levels (0–255) on selected known cells, or all cells.",
        Group::WhiteLight,
        Access::Write,
        move |args: WhiteLight, context| {
            let backend = backend.clone();
            async move {
                ensure!(
                    args.all == args.cells.is_empty(),
                    "select either cells or all=true"
                );
                let mut cells = if args.all {
                    white_light::ids().map(str::to_string).collect()
                } else {
                    args.cells
                };
                ensure!(
                    cells.iter().all(|id| white_light::find(id).is_some()),
                    "unknown white-light cell"
                );
                cells.sort();
                cells.dedup();
                backend
                    .send(
                        ControlCommand::WhiteLight {
                            warm: args.warm,
                            cold: args.cold,
                            cells,
                        },
                        context,
                    )
                    .await
            }
        },
    )
}
