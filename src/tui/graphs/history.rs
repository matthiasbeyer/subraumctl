//! Bounded, bucketed history queries used only by the TUI.

use std::collections::{BTreeMap, BTreeSet};

use eyre::{Result, ensure, eyre};
use serde_json::Value;

use super::Domain;
use crate::influx::{self, Options};

const BUCKETS: u64 = 600;

#[derive(Debug)]
pub struct Series {
    pub name: String,
    pub humidity: bool,
    // Separate segments prevent lines across missing buckets.
    pub segments: Vec<Vec<(f64, f64)>>,
}

#[derive(Debug)]
pub struct History {
    pub minutes: u64,
    pub end_ms: u64,
    pub bucket_ms: u64,
    pub series: Vec<Series>,
}

pub fn query(domain: Domain, minutes: u64, now: u64) -> String {
    let window = minutes * 60_000;
    let step = window.div_ceil(BUCKETS);
    let start = now.saturating_sub(window);
    let range = format!("time >= {start}ms AND time <= {now}ms");
    let group = format!("time({step}ms, {}ms)", start % step);
    match domain {
        Domain::Climate => ["temp", "hum"].map(|measurement| {
            format!("SELECT mean(\"value\") AS \"value\" FROM \"{measurement}\" WHERE {range} GROUP BY {group}, \"location\" fill(null)")
        }).join(";"),
        Domain::Power => format!(
            "SELECT mean(\"P1\") AS \"l1\", mean(\"P2\") AS \"l2\", mean(\"P3\") AS \"l3\", mean(\"P∑\") AS \"total\" FROM \"modbus\" WHERE (\"name\" = 'stromzähler') AND {range} GROUP BY {group} fill(null)"
        ),
    }
}

pub async fn read(options: &Options, domain: Domain, minutes: u64) -> Result<History> {
    let now = influx::now_ms()?;
    let response = influx::query(options, domain.database(), &query(domain, minutes, now)).await?;
    decode(&response, domain, minutes, now)
}

pub fn decode(value: &Value, domain: Domain, minutes: u64, now: u64) -> Result<History> {
    let results = influx::validate_response(value)?;
    let count = if domain == Domain::Climate { 2 } else { 1 };
    ensure!(results.len() == count, "incomplete graph results");
    let bucket_ms = (minutes * 60_000).div_ceil(BUCKETS);
    let start = now.saturating_sub(minutes * 60_000);
    let mut statements = BTreeSet::new();
    let mut output = BTreeMap::new();
    for result in results {
        let id = result["statement_id"]
            .as_u64()
            .ok_or_else(|| eyre!("missing graph statement ID"))?;
        ensure!(
            id < count as u64 && statements.insert(id),
            "unexpected graph statement ID"
        );
        let Some(series) = result.get("series") else {
            continue;
        };
        let series = series
            .as_array()
            .ok_or_else(|| eyre!("invalid graph series"))?;
        if domain == Domain::Power {
            ensure!(series.len() <= 1, "unexpected power graph series count");
        }
        let mut locations = BTreeSet::new();
        for series in series {
            ensure!(series["partial"] != true, "incomplete graph series");
            let measurement = match domain {
                Domain::Power => "modbus",
                Domain::Climate if id == 0 => "temp",
                Domain::Climate => "hum",
            };
            ensure!(
                series["name"] == measurement,
                "unexpected graph measurement"
            );
            let location = if domain == Domain::Climate {
                let location = series["tags"]["location"]
                    .as_str()
                    .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
                    .ok_or_else(|| eyre!("invalid graph location"))?;
                ensure!(locations.insert(location), "duplicate graph location");
                location
            } else {
                ""
            };
            let columns = series["columns"]
                .as_array()
                .ok_or_else(|| eyre!("invalid graph columns"))?;
            let column = |name: &str| -> Result<usize> {
                ensure!(
                    columns.iter().filter(|c| *c == name).count() == 1,
                    "invalid graph column {name}"
                );
                Ok(columns
                    .iter()
                    .position(|c| c == name)
                    .expect("validated column"))
            };
            let time_index = column("time")?;
            let fields = if domain == Domain::Power {
                vec![("l1", "L1"), ("l2", "L2"), ("l3", "L3"), ("total", "Total")]
            } else {
                vec![("value", location)]
            };
            let rows = series["values"]
                .as_array()
                .ok_or_else(|| eyre!("invalid graph values"))?;
            ensure!(rows.len() <= BUCKETS as usize + 1, "too many graph samples");
            for (field, name) in fields {
                let index = column(field)?;
                let mut line = Series {
                    name: name.into(),
                    humidity: id == 1,
                    segments: vec![],
                };
                let mut segment = Vec::new();
                let mut previous = None;
                for row in rows {
                    let row = row.as_array().ok_or_else(|| eyre!("invalid graph row"))?;
                    ensure!(row.len() == columns.len(), "invalid graph row length");
                    let time = row[time_index]
                        .as_u64()
                        .filter(|t| *t >= start && *t <= now)
                        .ok_or_else(|| eyre!("invalid graph timestamp"))?;
                    ensure!(
                        previous.is_none_or(|t| time > t),
                        "unordered graph timestamps"
                    );
                    if (row[index].is_null() || previous.is_some_and(|t| time - t > bucket_ms))
                        && !segment.is_empty()
                    {
                        line.segments.push(std::mem::take(&mut segment));
                    }
                    previous = Some(time);
                    if row[index].is_null() {
                        continue;
                    }
                    let value = row[index]
                        .as_f64()
                        .filter(|v| v.is_finite())
                        .ok_or_else(|| eyre!("invalid graph sample"))?;
                    let value = if domain == Domain::Power {
                        value / 1000.0
                    } else {
                        value
                    };
                    segment.push((-((now - time) as f64) / 60_000.0, value));
                }
                if !segment.is_empty() {
                    line.segments.push(segment);
                }
                output.insert((id, name.to_owned()), line);
            }
        }
    }
    Ok(History {
        minutes,
        end_ms: now,
        bucket_ms,
        series: output.into_values().collect(),
    })
}
