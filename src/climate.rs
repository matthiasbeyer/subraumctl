//! Measured room climate, independent of heating setpoints.

use std::collections::{BTreeMap, BTreeSet};

use eyre::{Result, ensure, eyre};
use serde_json::{Value, json};

use crate::influx::{self, Mode, Options, WINDOW_MS};

pub const DATABASE: &str = "climate";

#[derive(Clone, Debug, Default)]
pub struct Reading {
    pub location: String,
    pub temperature_celsius: Option<f64>,
    pub humidity_percent: Option<f64>,
    pub temperature_time_ms: Option<u64>,
    pub humidity_time_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub mode: Mode,
    pub queried_at_ms: u64,
    pub rooms: Vec<Reading>,
}

impl Snapshot {
    pub fn json(&self) -> Value {
        json!({
            "mode": self.mode.name(), "queried_at_ms": self.queried_at_ms,
            "window_minutes": if self.mode == Mode::Average { Some(15) } else { None },
            "rooms": self.rooms.iter().map(|room| json!({
                "location": room.location, "temperature_celsius": room.temperature_celsius,
                "humidity_percent": room.humidity_percent,
                "temperature_time_ms": room.temperature_time_ms, "humidity_time_ms": room.humidity_time_ms,
            })).collect::<Vec<_>>()
        })
    }
}

pub fn query(mode: Mode, now: u64) -> String {
    ["temp", "hum"].into_iter().map(|measurement| {
        let (aggregate, range) = match mode {
            Mode::Average => ("mean", format!("time >= {}ms AND time <= {now}ms", now.saturating_sub(WINDOW_MS))),
            Mode::Latest => ("last", format!("time <= {now}ms")),
        };
        format!("SELECT {aggregate}(\"value\") AS \"value\" FROM \"{measurement}\" WHERE {range} GROUP BY \"location\" fill(null)")
    }).collect::<Vec<_>>().join(";")
}

pub fn preview(mode: Mode, now: u64) -> Value {
    json!({"dry_run": true, "database": DATABASE, "mode": mode.name(), "query": query(mode, now)})
}

pub async fn read(options: &Options, mode: Mode) -> Result<Snapshot> {
    let now = influx::now_ms()?;
    let response = influx::query(options, DATABASE, &query(mode, now)).await?;
    decode(&response, mode, now)
}

/// Match statement IDs and column names, not response/column order. Missing
/// series and null samples are valid; malformed data is never silently zero.
pub fn decode(value: &Value, mode: Mode, now: u64) -> Result<Snapshot> {
    let results = influx::validate_response(value)?;
    ensure!(results.len() == 2, "incomplete climate results");
    let mut statements = BTreeSet::new();
    let mut rooms = BTreeMap::<String, Reading>::new();
    for result in results {
        let id = result["statement_id"]
            .as_u64()
            .ok_or_else(|| eyre!("missing climate statement ID"))?;
        ensure!(
            id < 2 && statements.insert(id),
            "unexpected climate statement ID"
        );
        let Some(series) = result.get("series") else {
            continue;
        };
        let series = series
            .as_array()
            .ok_or_else(|| eyre!("invalid climate series"))?;
        let mut locations = BTreeSet::new();
        for series in series {
            ensure!(series["partial"] != true, "incomplete climate series");
            ensure!(
                series["name"] == if id == 0 { "temp" } else { "hum" },
                "unexpected climate measurement"
            );
            let location = series["tags"]["location"]
                .as_str()
                .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
                .ok_or_else(|| eyre!("invalid climate location"))?;
            ensure!(locations.insert(location), "duplicate climate location");
            let columns = series["columns"]
                .as_array()
                .ok_or_else(|| eyre!("invalid climate columns"))?;
            let column = |name: &str| -> Result<usize> {
                ensure!(
                    columns.iter().filter(|c| *c == name).count() == 1,
                    "invalid climate column {name}"
                );
                Ok(columns
                    .iter()
                    .position(|c| c == name)
                    .expect("validated column"))
            };
            let value_index = column("value")?;
            let time_index = column("time")?;
            let values = series["values"]
                .as_array()
                .ok_or_else(|| eyre!("invalid climate values"))?;
            ensure!(values.len() <= 1, "unexpected climate sample count");
            let mut sample = None;
            let mut time = None;
            if let Some(row) = values.first() {
                let row = row.as_array().ok_or_else(|| eyre!("invalid climate row"))?;
                ensure!(row.len() == columns.len(), "invalid climate row length");
                if !row[value_index].is_null() {
                    sample = Some(
                        row[value_index]
                            .as_f64()
                            .filter(|v| v.is_finite())
                            .ok_or_else(|| eyre!("invalid climate sample"))?,
                    );
                    if mode == Mode::Latest {
                        time = Some(
                            row[time_index]
                                .as_u64()
                                .filter(|t| *t <= now)
                                .ok_or_else(|| eyre!("invalid climate timestamp"))?,
                        );
                    }
                }
            }
            let room = rooms.entry(location.into()).or_insert_with(|| Reading {
                location: location.into(),
                ..Reading::default()
            });
            if id == 0 {
                room.temperature_celsius = sample;
                room.temperature_time_ms = time;
            } else {
                room.humidity_percent = sample;
                room.humidity_time_ms = time;
            }
        }
    }
    Ok(Snapshot {
        mode,
        queried_at_ms: now,
        rooms: rooms.into_values().collect(),
    })
}
