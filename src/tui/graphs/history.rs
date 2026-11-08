//! Convert timestamped sensor history into gap-preserving chart segments.

use super::Domain;
use crate::{history, influx::Options};
use eyre::Result;

#[cfg(test)]
pub use history::query;

#[derive(Debug)]
pub struct Series {
    pub name: String,
    pub humidity: bool,
    pub segments: Vec<Vec<(f64, f64)>>,
}

#[derive(Debug)]
pub struct History {
    pub minutes: u64,
    pub end_ms: u64,
    pub bucket_ms: u64,
    pub series: Vec<Series>,
}

impl From<history::History> for History {
    fn from(history: history::History) -> Self {
        Self {
            minutes: history.minutes,
            end_ms: history.end_ms,
            bucket_ms: history.bucket_ms,
            series: history
                .series
                .into_iter()
                .map(|series| {
                    let mut segments = Vec::new();
                    let mut segment = Vec::new();
                    let mut previous = None;
                    for sample in series.samples {
                        if (sample.value.is_none()
                            || previous.is_some_and(|t| sample.time_ms - t > history.bucket_ms))
                            && !segment.is_empty()
                        {
                            segments.push(std::mem::take(&mut segment));
                        }
                        previous = Some(sample.time_ms);
                        if let Some(value) = sample.value {
                            segment.push((
                                -((history.end_ms - sample.time_ms) as f64) / 60_000.0,
                                value,
                            ));
                        }
                    }
                    if !segment.is_empty() {
                        segments.push(segment);
                    }
                    Series {
                        name: series.name,
                        humidity: series.humidity,
                        segments,
                    }
                })
                .collect(),
        }
    }
}

pub async fn read(options: &Options, domain: Domain, minutes: u64) -> Result<History> {
    history::read(options, domain, minutes)
        .await
        .map(Into::into)
}

#[cfg(test)]
pub fn decode(
    value: &serde_json::Value,
    domain: Domain,
    minutes: u64,
    now: u64,
) -> Result<History> {
    history::decode(value, domain, minutes, now).map(Into::into)
}
