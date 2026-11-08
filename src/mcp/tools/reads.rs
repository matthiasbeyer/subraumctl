use std::sync::Arc;

use eyre::Result;
use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;

use super::super::{backend::Backend, snapshot::Snapshot};
use super::{Access, Empty, Group, McpServer, add, choice::Choice};
use crate::knob::Knob;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct KnobGet {
    knob: Choice<Knob>,
}

pub(super) fn register<B: Backend>(server: &mut McpServer, backend: Arc<B>) -> Result<()> {
    for (name, description, group, snapshot) in [
        (
            "alert_led",
            "Read the decoded alert condition once.",
            Group::Alert,
            Snapshot::Alert,
        ),
        (
            "white_light_status",
            "Read known white-light cell states once; missing or invalid values are null.",
            Group::WhiteLight,
            Snapshot::WhiteLight,
        ),
        (
            "smoothiesaur_status",
            "Read selected Smoothiesaur telemetry once; missing or invalid values are null.",
            Group::Smoothiesaur,
            Snapshot::Smoothiesaur,
        ),
        (
            "status",
            "Check MQTT connectivity and read the alert condition. No broker URI or credentials are returned.",
            Group::Status,
            Snapshot::Status,
        ),
    ] {
        let backend = backend.clone();
        add(
            server,
            name,
            description,
            group,
            Access::Read,
            move |_: Empty, context| {
                let backend = backend.clone();
                async move { backend.snapshot(snapshot, context).await }
            },
        )?;
    }
    add(
        server,
        "knob_get",
        "Read one known animation knob; missing or invalid values are null.",
        Group::Knob,
        Access::Read,
        move |args: KnobGet, context| {
            let backend = backend.clone();
            async move { backend.snapshot(Snapshot::Knob(args.knob.0), context).await }
        },
    )
}
