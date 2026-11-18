//! The deployed 0.11.0 API: balances and transaction values are floating-point euros.
//! Only selected account fields escape this module; transaction responses are ignored.

pub mod cli;
#[cfg(test)]
#[path = "../tests/common/strichliste.rs"]
pub(crate) mod test_fixture;

use std::time::Duration;

use eyre::{Result, ensure, eyre};
use serde_json::{Value, json};

use crate::cli::GlobalArgs;

pub const MCP_COMMENT: &str = "triggered by mcp";
pub const DEFAULT_URL: &str = "https://mate.wahrhe.it/strich-api";

#[derive(Clone, Debug)]
pub struct Options {
    pub url: Option<String>,
    pub user_id: Option<u64>,
    pub timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            url: Some(DEFAULT_URL.into()),
            user_id: None,
            timeout: Duration::from_secs(5),
        }
    }
}

impl Options {
    pub fn from_global(global: &GlobalArgs) -> Self {
        Self {
            url: global.strichliste_url.clone(),
            user_id: global.strichliste_user_id,
            timeout: Duration::from_secs(global.strichliste_timeout),
        }
    }
}

/// This intentionally has no transfer, user lookup, or user selector variant.
#[derive(Clone, Debug)]
pub enum Request {
    Balance,
    Rank,
    Pay(f64),
    Add(f64),
}

impl Request {
    pub fn value(&self) -> Result<Option<f64>> {
        match self {
            Self::Pay(euros) => validate_amount(*euros).map(|value| Some(-value)),
            Self::Add(euros) => validate_amount(*euros).map(Some),
            _ => Ok(None),
        }
    }

    pub fn preview(&self) -> Result<Value> {
        Ok(match self.value()? {
            Some(value) => json!({"dry_run": true, "submitted": false, "value_eur": value}),
            None => json!({"dry_run": true}),
        })
    }
}

pub fn parse_amount(value: &str) -> std::result::Result<f64, String> {
    let amount = value
        .parse::<f64>()
        .map_err(|_| "expected euros as a number".to_string())?;
    validate_amount(amount).map_err(|error| error.to_string())?;
    Ok(amount)
}

pub fn validate_amount(euros: f64) -> Result<f64> {
    ensure!(
        euros.is_finite() && euros > 0.0,
        "amount must be a positive finite number of euros"
    );
    Ok(euros)
}

#[derive(Clone, Debug)]
pub struct User {
    pub id: u64,
    pub name: String,
    balance: Option<f64>,
    weighted_count: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct Transaction {
    pub id: u64,
    pub created_at: String,
    pub value_eur: f64,
    pub comment: Option<String>,
}

impl Transaction {
    pub fn kind(&self) -> &'static str {
        match (self.value_eur, self.comment.as_deref().unwrap_or("")) {
            (value, comment) if value < 0.0 && comment.starts_with("to ") => "Transfer out",
            (value, comment) if value > 0.0 && comment.starts_with("from ") => "Transfer in",
            (value, _) if value < 0.0 => "Pay",
            (value, _) if value > 0.0 => "Add credit",
            _ => "Adjustment",
        }
    }
}

pub struct AccountSnapshot {
    pub balance: Result<f64>,
    pub transactions: Result<TransactionPage>,
}

pub const TRANSACTION_PAGE_SIZE: usize = 50;

#[derive(Clone, Debug)]
pub struct TransactionPage {
    pub entries: Vec<Transaction>,
    pub offset: usize,
    pub total: Option<usize>,
    pub next_offset: Option<usize>,
}

fn parse_transactions(entries: &[Value], user_id: u64) -> Result<Vec<Transaction>> {
    let mut ids = std::collections::HashSet::new();
    let mut transactions = entries
        .iter()
        .map(|entry| {
            ensure!(
                entry["userId"].as_u64() == Some(user_id),
                "Strichliste transaction belongs to an unexpected user"
            );
            let id = entry["id"]
                .as_u64()
                .ok_or_else(|| eyre!("invalid transaction id"))?;
            ensure!(ids.insert(id), "duplicate Strichliste transaction ids");
            Ok(Transaction {
                id,
                created_at: entry["createDate"]
                    .as_str()
                    .filter(|date| !date.is_empty())
                    .ok_or_else(|| eyre!("invalid transaction date"))?
                    .to_owned(),
                value_eur: number(&entry["value"])
                    .ok_or_else(|| eyre!("invalid transaction value"))?,
                comment: match entry.get("comment") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(comment)) => Some(comment.clone()),
                    _ => return Err(eyre!("invalid transaction comment")),
                },
            })
        })
        .collect::<Result<Vec<_>>>()?;
    // Legacy transaction ids increase with each booking.
    transactions.sort_by_key(|transaction| std::cmp::Reverse(transaction.id));
    Ok(transactions)
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|n| n.is_finite())
}

impl User {
    fn parse(value: &Value) -> Result<Self> {
        Ok(Self {
            id: value["id"]
                .as_u64()
                .ok_or_else(|| eyre!("invalid Strichliste user id"))?,
            name: value["name"]
                .as_str()
                .ok_or_else(|| eyre!("invalid Strichliste user name"))?
                .to_owned(),
            balance: number(&value["balance"]),
            // The legacy aggregate is null for accounts without purchases.
            weighted_count: match value.get("weightedCountOfPurchases") {
                Some(Value::Null) => Some(0.0),
                Some(value) => number(value),
                None => None,
            },
        })
    }

    fn balance_json(&self) -> Result<Value> {
        let balance = self
            .balance
            .ok_or_else(|| eyre!("Strichliste balance missing or invalid"))?;
        Ok(json!({"balance_eur": balance}))
    }
}

pub struct Client {
    http: reqwest::Client,
    base: reqwest::Url,
    options: Options,
    user_id: u64,
}

impl Client {
    pub fn new(options: Options) -> Result<Self> {
        let user_id = options
            .user_id
            .ok_or_else(|| eyre!("configure --strichliste-user-id"))?;
        let base = options.url.as_deref().unwrap_or(DEFAULT_URL);
        let mut base =
            reqwest::Url::parse(base).map_err(|_| eyre!("invalid Strichliste API URL"))?;
        ensure!(
            matches!(base.scheme(), "http" | "https")
                && base.host_str().is_some()
                && base.query().is_none()
                && base.fragment().is_none(),
            "Strichliste API URL must be HTTP(S), without query or fragment"
        );
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        let http = reqwest::Client::builder()
            .timeout(options.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()?;
        Ok(Self {
            http,
            base,
            options,
            user_id,
        })
    }

    fn endpoint(&self, path: &str) -> Result<reqwest::Url> {
        self.base
            .join(path)
            .map_err(|_| eyre!("invalid Strichliste endpoint"))
    }

    async fn get(&self, path: &str) -> Result<Value> {
        let response = self
            .http
            .get(self.endpoint(path)?)
            .send()
            .await
            .map_err(|_| eyre!("Strichliste read failed"))?;
        ensure!(
            response.status().is_success(),
            "Strichliste GET /{path} rejected: HTTP {}",
            response.status()
        );
        // Bound list responses as well as individual accounts.
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| eyre!("Strichliste read failed"))?
        {
            ensure!(
                bytes.len() + chunk.len() <= 8 * 1024 * 1024,
                "Strichliste response too large"
            );
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| eyre!("invalid Strichliste JSON"))
    }

    pub async fn users(&self) -> Result<Vec<User>> {
        tokio::time::timeout(self.options.timeout, self.read_users())
            .await
            .map_err(|_| eyre!("Strichliste user list timed out"))?
    }

    async fn read_users(&self) -> Result<Vec<User>> {
        let mut users = Vec::new();
        let mut ids = std::collections::HashSet::new();
        let mut path = "user".to_owned();
        let mut overall_count = None;
        // Limit both total work and memory even if pagination metadata is inconsistent.
        for page in 0..1000 {
            let value = self.get(&path).await?;
            let (entries, total, limit) = if let Some(entries) = value.as_array() {
                ensure!(
                    page == 0,
                    "Strichliste pagination response lost its metadata"
                );
                (entries, entries.len(), None)
            } else {
                let entries = value["entries"]
                    .as_array()
                    .ok_or_else(|| eyre!("expected Strichliste user list with an entries array"))?;
                let total = page_number(&value, "overallCount")?
                    .ok_or_else(|| eyre!("Strichliste user list is missing overallCount"))?;
                let offset = page_number(&value, "offset")?.unwrap_or(0);
                let limit = page_number(&value, "limit")?;
                ensure!(
                    offset == users.len(),
                    "Strichliste user page has an unexpected offset"
                );
                ensure!(
                    limit.is_none_or(|limit| limit > 0 && entries.len() <= limit),
                    "invalid Strichliste user page limit"
                );
                (entries, total, limit)
            };
            ensure!(total <= 100_000, "Strichliste user list too large");
            ensure!(
                overall_count.is_none_or(|count| count == total),
                "Strichliste user count changed during pagination; refresh again"
            );
            overall_count = Some(total);
            ensure!(
                users.len() + entries.len() <= total,
                "Strichliste entries exceed overallCount"
            );
            for value in entries {
                let user = User::parse(value)?;
                ensure!(ids.insert(user.id), "duplicate Strichliste user ids");
                users.push(user);
            }
            if users.len() == total {
                return Ok(users);
            }
            ensure!(
                !entries.is_empty(),
                "Strichliste user pagination made no progress"
            );
            let limit = limit.unwrap_or(entries.len()).min(1000);
            path = format!("user?limit={limit}&offset={}", users.len());
        }
        Err(eyre!("Strichliste user pagination exceeded the page limit"))
    }

    fn select<'a>(&self, users: &'a [User]) -> Result<&'a User> {
        let mut matching = users.iter().filter(|user| user.id == self.user_id);
        let user = matching
            .next()
            .ok_or_else(|| eyre!("configured Strichliste user not found"))?;
        ensure!(matching.next().is_none(), "duplicate Strichliste user id");
        Ok(user)
    }

    pub async fn current_user(&self) -> Result<User> {
        let id = self.user_id;
        let user = User::parse(&self.get(&format!("user/{id}")).await?)?;
        ensure!(user.id == id, "Strichliste returned an unexpected user");
        Ok(user)
    }

    /// Account balance and the first page of transactions.
    /// Kept separate from Request so history is not exposed through MCP.
    pub async fn account_snapshot(&self) -> Result<AccountSnapshot> {
        self.account_snapshot_at(0).await
    }

    pub async fn account_snapshot_at(&self, offset: usize) -> Result<AccountSnapshot> {
        let id = self.user_id;
        let value = self.get(&format!("user/{id}")).await?;
        let user = User::parse(&value)?;
        ensure!(user.id == id, "Strichliste returned an unexpected user");
        Ok(AccountSnapshot {
            balance: user
                .balance
                .ok_or_else(|| eyre!("Strichliste balance missing or invalid")),
            transactions: self
                .transaction_page(id, TRANSACTION_PAGE_SIZE, offset)
                .await,
        })
    }

    /// History remains separate from Request and is not available through MCP.
    pub async fn transactions(&self, limit: usize, offset: usize) -> Result<TransactionPage> {
        ensure!(
            (1..=1000).contains(&limit),
            "transaction limit must be between 1 and 1000"
        );
        let id = self.user_id;
        self.transaction_page(id, limit, offset).await
    }

    async fn transaction_page(
        &self,
        id: u64,
        limit: usize,
        offset: usize,
    ) -> Result<TransactionPage> {
        let value = self
            .get(&format!(
                "user/{id}/transaction?limit={limit}&offset={offset}"
            ))
            .await?;
        let (entries, total) = if let Some(entries) = value.as_array() {
            (entries, None)
        } else {
            let entries = value["entries"]
                .as_array()
                .ok_or_else(|| eyre!("expected Strichliste transaction entries array"))?;
            if let Some(actual) = page_number(&value, "offset")? {
                ensure!(
                    actual == offset,
                    "unexpected Strichliste transaction page offset"
                );
            }
            if let Some(actual) = page_number(&value, "limit")? {
                ensure!(
                    actual > 0 && actual <= limit && entries.len() <= actual,
                    "invalid Strichliste transaction page limit"
                );
            }
            (entries, page_number(&value, "overallCount")?)
        };
        ensure!(
            entries.len() <= limit,
            "Strichliste transaction page exceeds requested limit"
        );
        let end = offset
            .checked_add(entries.len())
            .ok_or_else(|| eyre!("Strichliste transaction offset overflow"))?;
        if let Some(total) = total {
            ensure!(
                entries.is_empty() || end <= total,
                "Strichliste transactions exceed overallCount"
            );
            ensure!(
                !entries.is_empty() || offset >= total,
                "Strichliste transaction pagination made no progress"
            );
        }
        let next_offset = (!entries.is_empty()
            && total.map_or(entries.len() == limit, |total| end < total))
        .then_some(end);
        Ok(TransactionPage {
            entries: parse_transactions(entries, id)?,
            offset,
            total,
            next_offset,
        })
    }

    pub async fn execute(&self, request: &Request, comment: &str) -> Result<Value> {
        // Validate before any IO.
        let value = request.value()?;
        if matches!(request, Request::Rank) {
            let users = self.users().await?;
            let current = self.select(&users)?;
            let score = current
                .weighted_count
                .ok_or_else(|| eyre!("weightedCountOfPurchases missing or invalid"))?;
            let mut rank = 1;
            for user in &users {
                let other = user
                    .weighted_count
                    .ok_or_else(|| eyre!("weightedCountOfPurchases missing or invalid"))?;
                if other > score {
                    rank += 1;
                }
            }
            return Ok(json!({"rank": rank, "weighted_count_of_purchases": score}));
        }
        let user = self.current_user().await?;
        match value {
            Some(value) => {
                self.post(user.id, json!({"value": value, "comment": comment}))
                    .await
            }
            None => user.balance_json(),
        }
    }

    async fn post(&self, user_id: u64, body: Value) -> Result<Value> {
        let response = self
            .http
            .post(self.endpoint(&format!("user/{user_id}/transaction"))?)
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(&body)?)
            .send()
            .await
            .map_err(|_| {
                eyre!("Strichliste transaction outcome unknown; check your balance before retrying")
            })?;
        ensure!(
            response.status().is_success(),
            "Strichliste POST /user/{user_id}/transaction returned HTTP {}; transaction was not confirmed; check your balance before retrying",
            response.status()
        );
        // Never expose the raw response: it may include other users or transaction history.
        Ok(json!({"submitted": true, "value_eur": body["value"]}))
    }

    pub async fn transfer(&self, recipient: &User, euros: f64, comment: &str) -> Result<Value> {
        let value = -validate_amount(euros)?;
        let sender = self.current_user().await?;
        ensure!(sender.id != recipient.id, "cannot send to yourself");
        self.post(
            sender.id,
            json!({
                "value": value,
                "comment": format!("to {}: {}", recipient.name, comment),
                "toUserId": recipient.id,
                "toUserComment": format!("from {}: {}", sender.name, comment),
            }),
        )
        .await
    }
}

fn page_number(value: &Value, field: &str) -> Result<Option<usize>> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|number| usize::try_from(number).ok())
            .map(Some)
            .ok_or_else(|| eyre!("invalid Strichliste pagination field: {field}")),
    }
}

/// MCP's boundary fixes the identity and audit comment independently of tool arguments.
pub async fn execute_mcp(options: Options, request: &Request, dry_run: bool) -> Result<Value> {
    if dry_run {
        let mut preview = request.preview()?;
        if request.value()?.is_some() {
            preview["comment"] = json!(MCP_COMMENT);
        }
        return Ok(preview);
    }
    Client::new(options)?.execute(request, MCP_COMMENT).await
}
