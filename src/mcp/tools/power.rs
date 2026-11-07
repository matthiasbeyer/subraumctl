use super::*;
use crate::influx::Mode;
use choice::Choice;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadPower {
    /// average (default): last 15 minutes; latest: newest meter record with timestamp.
    mode: Option<Choice<Mode>>,
}

pub(super) fn register<B: super::super::backend::Backend>(
    server: &mut McpServer,
    backend: Arc<B>,
) -> Result<()> {
    add(
        server,
        "power_status",
        "Read meter power in watts for L1, L2, L3 and total. Optional mode: average (15 minutes, default) or latest (newest meter record with timestamp, possibly old). Missing readings are null. Stored values are divided by 1000 as in the original display.",
        Group::Power,
        Access::Read,
        move |args: ReadPower, context| {
            let backend = backend.clone();
            async move {
                backend
                    .power(args.mode.map_or_else(Mode::default, |mode| mode.0), context)
                    .await
            }
        },
    )
}
