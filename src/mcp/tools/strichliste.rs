use std::sync::Arc;

use eyre::Result;
use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;

use super::super::backend::Backend;
use super::{Access, Empty, Group, McpServer, add};
use crate::strichliste::Request;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Amount {
    /// Positive amount in euros, not cents. Fractional cents are supported.
    amount: f64,
}

pub(super) fn register<B: Backend>(server: &mut McpServer, backend: Arc<B>) -> Result<()> {
    for (name, description, request) in [
        (
            "strichliste_balance",
            "Read only the configured user's balance in euros.",
            Request::Balance,
        ),
        (
            "strichliste_rank",
            "Read only the configured user's rank by weightedCountOfPurchases, highest first; ties share rank.",
            Request::Rank,
        ),
    ] {
        let backend = backend.clone();
        add(
            server,
            name,
            description,
            Group::Strichliste,
            Access::Read,
            move |_: Empty, context| {
                let backend = backend.clone();
                let request = request.clone();
                async move { backend.strichliste(request, context).await }
            },
        )?;
    }
    // Config permission alone cannot register these opt-in financial mutations.
    if backend.strichliste_mcp_writes() {
        for (name, description, pay) in [
            (
                "strichliste_pay",
                "Debit a positive euro amount from the configured user; comment is always 'triggered by mcp'.",
                true,
            ),
            (
                "strichliste_add",
                "Credit a positive euro amount to the configured user; comment is always 'triggered by mcp'.",
                false,
            ),
        ] {
            let backend = backend.clone();
            add(
                server,
                name,
                description,
                Group::Strichliste,
                Access::Write,
                move |args: Amount, context| {
                    let backend = backend.clone();
                    async move {
                        let request = if pay {
                            Request::Pay(args.amount)
                        } else {
                            Request::Add(args.amount)
                        };
                        request.value()?;
                        backend.strichliste(request, context).await
                    }
                },
            )?;
        }
    }
    Ok(())
}
