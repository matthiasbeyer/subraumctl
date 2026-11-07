//! Shared InfluxDB v1 read transport and sampling modes.
//! Database names, queries and decoding belong to each domain (climate, etc.).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::ValueEnum;
use eyre::{Result, ensure, eyre};
use serde_json::Value;

pub const DEFAULT_URL: &str = "http://fritteuse.subraum.c3pb.de:8086/query";
pub const WINDOW_MS: u64 = 15 * 60 * 1000;

#[derive(Clone, Debug)]
pub struct Options {
    pub url: String,
    pub timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            url: DEFAULT_URL.into(),
            timeout: Duration::from_secs(3),
        }
    }
}

impl Options {
    pub fn from_global(global: &crate::cli::GlobalArgs) -> Self {
        Self {
            url: global.influx_url.clone(),
            timeout: Duration::from_secs(global.influx_timeout),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    /// Mean over the last 15 minutes.
    #[default]
    Average,
    /// Most recent sample per metric and location, including older samples.
    Latest,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Average => "average",
            Self::Latest => "latest",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Average => "15-minute average",
            Self::Latest => "Latest samples",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Average => Self::Latest,
            Self::Latest => Self::Average,
        }
    }
}

pub fn now_ms() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}

/// GET only; request parameters are URL encoded, including Unicode databases.
pub async fn query(options: &Options, database: &str, query: &str) -> Result<Value> {
    let mut url = reqwest::Url::parse(&options.url).map_err(|_| eyre!("invalid InfluxDB URL"))?;
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "InfluxDB requires HTTP or HTTPS"
    );
    url.query_pairs_mut()
        .clear()
        .append_pair("db", database)
        .append_pair("q", query)
        .append_pair("epoch", "ms");
    let client = reqwest::Client::builder()
        .timeout(options.timeout)
        .build()
        .map_err(|_| eyre!("could not create InfluxDB HTTP client"))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| eyre!("InfluxDB request failed or timed out"))?;
    ensure!(
        response.status().is_success(),
        "InfluxDB HTTP status {}",
        response.status()
    );
    let bytes = response
        .bytes()
        .await
        .map_err(|_| eyre!("could not read InfluxDB response"))?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| eyre!("invalid InfluxDB JSON"))?;
    validate_response(&value)?;
    Ok(value)
}

pub fn validate_response(value: &Value) -> Result<&Vec<Value>> {
    ensure!(value.get("error").is_none(), "InfluxDB query failed");
    let results = value["results"]
        .as_array()
        .ok_or_else(|| eyre!("missing InfluxDB results"))?;
    ensure!(!results.is_empty(), "missing InfluxDB results");
    for result in results {
        ensure!(
            result.is_object() && result.get("error").is_none(),
            "InfluxDB query failed"
        );
        ensure!(result["partial"] != true, "incomplete InfluxDB results");
    }
    Ok(results)
}
