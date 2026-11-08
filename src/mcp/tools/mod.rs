//! Reviewed tool registrations. Each tool declares its own permission boundary.

mod choice;
mod controls;
mod heizung;
mod lighting;
mod opt_in;
mod pizzamat;
mod reads;

use std::future::Future;
use std::sync::Arc;

use crate::{knob::KNOB_SPECS, white_light};
use eyre::Result;
use rmcp::RoleServer;
use rmcp::model::{CallToolResult, ContentBlock, Tool, ToolAnnotations};
use rmcp::schemars::{self, JsonSchema};
use rmcp::service::RequestContext;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{
    McpServer,
    config::{Access, Group},
};

fn add_write<A, B, F>(
    server: &mut McpServer,
    name: &'static str,
    description: &'static str,
    group: Group,
    backend: Arc<B>,
    command: F,
) -> Result<()>
where
    A: DeserializeOwned + JsonSchema + Send + 'static,
    B: super::backend::Backend,
    F: Fn(A) -> Result<crate::action::ControlCommand> + Send + Sync + 'static,
{
    add(
        server,
        name,
        description,
        group,
        Access::Write,
        move |args: A, context| {
            let command = command(args);
            let backend = backend.clone();
            async move { backend.send(command?, context).await }
        },
    )
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Empty {}

/// The schema and argument validation come from the same Rust type.
fn add<A, F, Fut>(
    server: &mut McpServer,
    name: &'static str,
    description: &'static str,
    group: Group,
    access: Access,
    handler: F,
) -> Result<()>
where
    A: DeserializeOwned + JsonSchema + Send + 'static,
    F: Fn(A, RequestContext<RoleServer>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Value>> + Send + 'static,
{
    let tool = Tool::new(name, description, serde_json::Map::new())
        .with_input_schema::<A>()
        .with_annotations(ToolAnnotations::new().read_only(access == Access::Read));
    let handler = Arc::new(handler);
    server.register(tool, group, access, move |request, context| {
        let handler = handler.clone();
        async move {
            let arguments = Value::Object(request.arguments.unwrap_or_default());
            let result = match serde_json::from_value::<A>(arguments) {
                Ok(arguments) => handler(arguments, context).await,
                Err(error) => Err(eyre::eyre!("invalid arguments: {error}")),
            };
            Ok(match result {
                Ok(value) => CallToolResult::structured(value),
                Err(error) => CallToolResult::error(vec![ContentBlock::text(error.to_string())]),
            }
            .into())
        }
    })
}

pub(super) fn register<B: super::backend::Backend>(
    server: &mut McpServer,
    backend: Arc<B>,
) -> Result<()> {
    lighting::register(server, backend.clone())?;
    controls::register(server, backend.clone())?;
    opt_in::register(server, backend.clone())?;
    heizung::register(server, backend.clone())?;
    pizzamat::register(server, backend.clone())?;
    reads::register(server, backend)?;
    add(
        server,
        "white_light_cells",
        "List known white-light cells and groups.",
        Group::WhiteLight,
        Access::Read,
        |_: Empty, _| async {
            Ok(json!({"cells": white_light::CELLS.iter().map(|cell|
                json!({"id": cell.id, "group": cell.group.label()})
            ).collect::<Vec<_>>()}))
        },
    )?;
    add(
        server,
        "knob_list",
        "List animation knobs, ranges and open range boundaries.",
        Group::Knob,
        Access::Read,
        |_: Empty, _| async {
            Ok(json!({"knobs": KNOB_SPECS.iter().map(|spec| json!({
                "name": spec.name, "min": spec.min, "max": spec.max,
                "open_min": spec.open_min, "open_max": spec.open_max
            })).collect::<Vec<_>>()}))
        },
    )
}
