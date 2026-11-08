use super::*;
use crate::pizzamat::{Operation, Request};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Service {
    service: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Food {
    food: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DefaultOrder {
    service: String,
    food: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeliveryService {
    name: String,
    realname: Option<String>,
    description: Option<String>,
    order_url: Option<String>,
    menu_url: Option<String>,
}

fn register_operation<A, B>(
    server: &mut McpServer,
    backend: Arc<B>,
    name: &'static str,
    description: &'static str,
    operation: Operation,
    fields: fn(A) -> Vec<String>,
) -> Result<()>
where
    A: DeserializeOwned + JsonSchema + Send + 'static,
    B: super::super::backend::Backend,
{
    add(
        server,
        name,
        description,
        Group::Pizzamat,
        if operation.read_only() {
            Access::Read
        } else {
            Access::Write
        },
        move |args: A, context| {
            let backend = backend.clone();
            let request = Request::new(operation, backend.pizzamat_user(), &fields(args));
            async move { backend.pizzamat(request?, context).await }
        },
    )
}

pub(super) fn register<B: super::super::backend::Backend>(
    server: &mut McpServer,
    backend: Arc<B>,
) -> Result<()> {
    for (name, description, operation) in [
        (
            "pizzamat_list",
            "Show everyone's current orders, commitment status and delivery-service information.",
            Operation::List,
        ),
        (
            "pizzamat_services",
            "List available delivery services.",
            Operation::Services,
        ),
        (
            "pizzamat_lock",
            "End ordering by marking the list as ordered; does not place an order with the restaurant.",
            Operation::Lock,
        ),
        (
            "pizzamat_unlock",
            "Reopen ordering by removing the ordered marker.",
            Operation::Unlock,
        ),
        (
            "pizzamat_reset",
            "Clear the entire ordering session, including everyone's orders and selected service.",
            Operation::Reset,
        ),
        (
            "pizzamat_join",
            "Join the order or mark your existing order uncommitted, using the configured Pizzamat user.",
            Operation::Join,
        ),
        (
            "pizzamat_clear",
            "Remove your entire personal order, using the configured Pizzamat user.",
            Operation::Clear,
        ),
        (
            "pizzamat_commit",
            "Mark your personal order finished, using the configured Pizzamat user.",
            Operation::Commit,
        ),
    ] {
        register_operation(
            server,
            backend.clone(),
            name,
            description,
            operation,
            |_: Empty| vec![],
        )?;
    }
    register_operation(
        server,
        backend.clone(),
        "pizzamat_start",
        "Start an order at a known delivery service; Pizzamat announces it in the room.",
        Operation::Init,
        |args: Service| vec![args.service],
    )?;
    register_operation(
        server,
        backend.clone(),
        "pizzamat_add",
        "Append food to your order. Use food 'default' for your saved usual order. Marks your order uncommitted.",
        Operation::Add,
        |args: Food| vec![args.food],
    )?;
    register_operation(
        server,
        backend.clone(),
        "pizzamat_remove",
        "Remove all foods containing the case-sensitive food substring from your order. Marks your order uncommitted.",
        Operation::Remove,
        |args: Food| vec![args.food],
    )?;
    register_operation(
        server,
        backend.clone(),
        "pizzamat_set_default",
        "Save your usual food for a delivery service, using the configured Pizzamat user.",
        Operation::SetDefault,
        |args: DefaultOrder| vec![args.service, args.food],
    )?;
    register_operation(
        server,
        backend.clone(),
        "pizzamat_service_add",
        "Add or update a delivery service. Omitted optional fields are cleared.",
        Operation::ServiceAdd,
        |args: DeliveryService| {
            vec![
                args.name,
                optional(args.realname),
                optional(args.description),
                optional(args.order_url),
                optional(args.menu_url),
            ]
        },
    )?;
    register_operation(
        server,
        backend,
        "pizzamat_service_remove",
        "Remove a delivery service from the shared catalogue.",
        Operation::ServiceRemove,
        |args: Service| vec![args.service],
    )
}

fn optional(value: Option<String>) -> String {
    value
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "-".into())
}
