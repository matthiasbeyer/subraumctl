use super::*;
use crate::history::{self, Domain};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadHistory {
    /// History window in whole minutes, 1..=43200 (30 days). Default: 60.
    #[serde(default = "default_minutes")]
    #[schemars(range(min = 1, max = 43200))]
    minutes: u64,
}

fn default_minutes() -> u64 {
    history::DEFAULT_MINUTES
}

pub(super) fn register<B: super::super::backend::Backend>(
    server: &mut McpServer,
    backend: Arc<B>,
) -> Result<()> {
    for (name, description, group, domain) in [
        (
            "climate_history",
            "Read room temperature (Celsius) and humidity (percent) history by location. Optional minutes: 1..43200, default 60. Returns up to 601 timestamped mean buckets per series, with units and nulls for missing readings. Timestamps are bucket starts, not sensor sample times. Indoor climate, not an outdoor weather forecast.",
            Group::Climate,
            Domain::Climate,
        ),
        (
            "power_history",
            "Read meter power history for L1, L2, L3 and Total in watts. Optional minutes: 1..43200, default 60. Returns up to 601 timestamped mean buckets per series; missing readings are null and negatives are preserved. Timestamps are bucket starts. Total comes from the meter, not the sum of phases. This is power, not energy consumption in kWh.",
            Group::Power,
            Domain::Power,
        ),
    ] {
        let backend = backend.clone();
        add(
            server,
            name,
            description,
            group,
            Access::Read,
            move |args: ReadHistory, context| {
                let backend = backend.clone();
                async move {
                    history::validate_minutes(args.minutes)?;
                    backend.history(domain, args.minutes, context).await
                }
            },
        )?;
    }
    Ok(())
}
