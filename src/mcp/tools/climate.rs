use super::*;
use crate::influx::Mode;
use choice::Choice;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadClimate {
    /// average (default): last 15 minutes; latest: last sample with timestamp per metric.
    mode: Option<Choice<Mode>>,
}

pub(super) fn register<B: super::super::backend::Backend>(
    server: &mut McpServer,
    backend: Arc<B>,
) -> Result<()> {
    add(
        server,
        "climate_status",
        "Read measured room temperatures (Celsius) and humidity (percent), grouped by location. Optional mode: average (15 minutes, default) or latest (includes sample timestamps, possibly old). Missing readings are null.",
        Group::Climate,
        Access::Read,
        move |args: ReadClimate, context| {
            let backend = backend.clone();
            async move {
                backend
                    .climate(args.mode.map_or_else(Mode::default, |mode| mode.0), context)
                    .await
            }
        },
    )
}
