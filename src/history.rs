//! Bounded, bucketed sensor history shared by the TUI and MCP.

use std::collections::{BTreeMap, BTreeSet};

use eyre::{Result, ensure, eyre};
use serde_json::{Value, json};

use crate::influx::{self, Options};

const BUCKETS: u64 = 600;
pub const DEFAULT_MINUTES: u64 = 60;
pub const MAX_MINUTES: u64 = 43200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    Climate,
    Power,
}

impl Domain {
    pub fn database(self) -> &'static str {
        match self {
            Self::Climate => crate::climate::DATABASE,
            Self::Power => crate::power::DATABASE,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Climate => "climate",
            Self::Power => "power",
        }
    }
}

pub fn validate_minutes(minutes: u64) -> Result<()> {
    ensure!(
        (1..=MAX_MINUTES).contains(&minutes),
        "minutes must be between 1 and {MAX_MINUTES}"
    );
    Ok(())
}

#[derive(Debug)]
pub struct Sample {
    pub time_ms: u64,
    pub value: Option<f64>,
}

#[derive(Debug)]
pub struct Series {
    pub name: String,
    pub humidity: bool,
    pub samples: Vec<Sample>,
}

#[derive(Debug)]
pub struct History {
    pub domain: Domain,
    pub minutes: u64,
    pub end_ms: u64,
    pub bucket_ms: u64,
    pub series: Vec<Series>,
}

impl History {
    pub fn json(&self) -> Value {
        json!({
            "domain": self.domain.name(), "window_minutes": self.minutes,
            "start_ms": self.end_ms.saturating_sub(self.minutes * 60_000),
            "end_ms": self.end_ms, "bucket_ms": self.bucket_ms, "aggregation": "mean",
            "series": self.series.iter().map(|series| {
                let (metric, unit) = match self.domain {
                    Domain::Power => ("power", "W"),
                    Domain::Climate if series.humidity => ("humidity", "%"),
                    Domain::Climate => ("temperature", "°C"),
                };
                json!({"name": series.name, "metric": metric, "unit": unit,
                    "samples": series.samples.iter().map(|sample| json!({
                        "time_ms": sample.time_ms, "value": sample.value
                    })).collect::<Vec<_>>()})
            }).collect::<Vec<_>>()
        })
    }
}

pub fn preview(domain: Domain, minutes: u64, now: u64) -> Result<Value> {
    Ok(
        json!({"dry_run": true, "database": domain.database(), "domain": domain.name(),
        "window_minutes": minutes, "query": query(domain, minutes, now)?}),
    )
}

pub fn query(domain: Domain, minutes: u64, now: u64) -> Result<String> {
    validate_minutes(minutes)?;
    let window = minutes * 60_000;
    let step = window.div_ceil(BUCKETS);
    let start = now.saturating_sub(window);
    let range = format!("time >= {start}ms AND time <= {now}ms");
    let group = format!("time({step}ms, {}ms)", start % step);
    Ok(match domain {
        Domain::Climate => ["temp", "hum"].map(|measurement| {
            format!("SELECT mean(\"value\") AS \"value\" FROM \"{measurement}\" WHERE {range} GROUP BY {group}, \"location\" fill(null)")
        }).join(";"),
        Domain::Power => format!(
            "SELECT mean(\"P1\") AS \"l1\", mean(\"P2\") AS \"l2\", mean(\"P3\") AS \"l3\", mean(\"P∑\") AS \"total\" FROM \"modbus\" WHERE (\"name\" = 'stromzähler') AND {range} GROUP BY {group} fill(null)"
        ),
    })
}

pub async fn read(options: &Options, domain: Domain, minutes: u64) -> Result<History> {
    let now = influx::now_ms()?;
    let response = influx::query(options, domain.database(), &query(domain, minutes, now)?).await?;
    decode(&response, domain, minutes, now)
}

pub fn decode(value: &Value, domain: Domain, minutes: u64, now: u64) -> Result<History> {
    validate_minutes(minutes)?;
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
                    samples: vec![],
                };
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
                    previous = Some(time);
                    if row[index].is_null() {
                        line.samples.push(Sample {
                            time_ms: time,
                            value: None,
                        });
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
                    line.samples.push(Sample {
                        time_ms: time,
                        value: Some(value),
                    });
                }
                output.insert((id, name.to_owned()), line);
            }
        }
    }
    Ok(History {
        domain,
        minutes,
        end_ms: now,
        bucket_ms,
        series: output.into_values().collect(),
    })
}
