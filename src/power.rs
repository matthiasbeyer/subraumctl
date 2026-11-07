//! Meter power readings. Stored values are divided by 1000, as in the original display.

use eyre::{Result, ensure, eyre};
use serde_json::{Value, json};

use crate::influx::{self, Mode, Options, WINDOW_MS};

pub const DATABASE: &str = "subraum-stromzähler";
const FIELDS: [(&str, &str); 4] = [("P1", "l1"), ("P2", "l2"), ("P3", "l3"), ("P∑", "total")];

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub mode: Mode,
    pub queried_at_ms: u64,
    pub sample_time_ms: Option<u64>,
    pub l1_watts: Option<f64>,
    pub l2_watts: Option<f64>,
    pub l3_watts: Option<f64>,
    pub total_watts: Option<f64>,
}

impl Snapshot {
    pub fn json(&self) -> Value {
        json!({
            "mode": self.mode.name(), "queried_at_ms": self.queried_at_ms,
            "window_minutes": if self.mode == Mode::Average { Some(15) } else { None },
            "sample_time_ms": self.sample_time_ms,
            "l1_watts": self.l1_watts, "l2_watts": self.l2_watts,
            "l3_watts": self.l3_watts, "total_watts": self.total_watts,
        })
    }
}

pub fn query(mode: Mode, now: u64) -> String {
    let fields = FIELDS
        .iter()
        .map(|(field, alias)| match mode {
            Mode::Average => format!("mean(\"{field}\") AS \"{alias}\""),
            Mode::Latest => format!("\"{field}\" AS \"{alias}\""),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let range = match mode {
        Mode::Average => format!(
            "time >= {}ms AND time <= {now}ms",
            now.saturating_sub(WINDOW_MS)
        ),
        Mode::Latest => format!("time <= {now}ms"),
    };
    // Latest uses one meter record so the phase values and total have the same
    // timestamp. Multiple last() selectors would lose this timestamp in InfluxDB.
    let suffix = if mode == Mode::Latest {
        " ORDER BY time DESC LIMIT 1"
    } else {
        ""
    };
    format!("SELECT {fields} FROM \"modbus\" WHERE (\"name\" = 'stromzähler') AND {range}{suffix}")
}

pub fn preview(mode: Mode, now: u64) -> Value {
    json!({"dry_run": true, "database": DATABASE, "mode": mode.name(), "query": query(mode, now)})
}

pub async fn read(options: &Options, mode: Mode) -> Result<Snapshot> {
    let now = influx::now_ms()?;
    let response = influx::query(options, DATABASE, &query(mode, now)).await?;
    decode(&response, mode, now)
}

pub fn decode(value: &Value, mode: Mode, now: u64) -> Result<Snapshot> {
    let results = influx::validate_response(value)?;
    ensure!(
        results.len() == 1 && results[0]["statement_id"] == 0,
        "unexpected power statement ID"
    );
    let mut snapshot = Snapshot {
        mode,
        queried_at_ms: now,
        sample_time_ms: None,
        l1_watts: None,
        l2_watts: None,
        l3_watts: None,
        total_watts: None,
    };
    let Some(series) = results[0].get("series") else {
        return Ok(snapshot);
    };
    let series = series
        .as_array()
        .ok_or_else(|| eyre!("invalid power series"))?;
    ensure!(series.len() <= 1, "unexpected power series count");
    let Some(series) = series.first() else {
        return Ok(snapshot);
    };
    ensure!(series["name"] == "modbus", "unexpected power measurement");
    ensure!(series["partial"] != true, "incomplete power series");
    let columns = series["columns"]
        .as_array()
        .ok_or_else(|| eyre!("invalid power columns"))?;
    let column = |name: &str| -> Result<usize> {
        ensure!(
            columns.iter().filter(|c| *c == name).count() == 1,
            "invalid power column {name}"
        );
        Ok(columns
            .iter()
            .position(|c| c == name)
            .expect("validated column"))
    };
    let time_index = column("time")?;
    let indices = [
        column("l1")?,
        column("l2")?,
        column("l3")?,
        column("total")?,
    ];
    let values = series["values"]
        .as_array()
        .ok_or_else(|| eyre!("invalid power values"))?;
    ensure!(values.len() <= 1, "unexpected power sample count");
    let Some(row) = values.first() else {
        return Ok(snapshot);
    };
    let row = row.as_array().ok_or_else(|| eyre!("invalid power row"))?;
    ensure!(row.len() == columns.len(), "invalid power row length");
    let watts = |index: usize| -> Result<Option<f64>> {
        if row[index].is_null() {
            return Ok(None);
        }
        let value = row[index]
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or_else(|| eyre!("invalid power sample"))?;
        Ok(Some(value / 1000.0))
    };
    snapshot.l1_watts = watts(indices[0])?;
    snapshot.l2_watts = watts(indices[1])?;
    snapshot.l3_watts = watts(indices[2])?;
    // P∑ is authoritative; never invent a total from potentially missing phases.
    snapshot.total_watts = watts(indices[3])?;
    if mode == Mode::Latest {
        snapshot.sample_time_ms = Some(
            row[time_index]
                .as_u64()
                .filter(|time| *time <= now)
                .ok_or_else(|| eyre!("invalid power timestamp"))?,
        );
    }
    Ok(snapshot)
}

pub fn format_watts(value: Option<f64>) -> String {
    value.map_or_else(|| "? W".into(), |value| format!("{value:.2} W"))
}
