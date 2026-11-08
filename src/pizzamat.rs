//! Pizzamat's EDI command vocabulary, shared by the TUI and MCP.

use eyre::{Result, ensure};
use std::time::Duration;

pub mod transport;

pub const DEFAULT_AMQP_URL: &str = "amqp://amqp.subraum.c3pb.de:5672/%2f";

#[derive(Clone)]
pub struct Options {
    pub amqp_url: String,
    pub user: String,
    pub debug: bool,
    pub timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            amqp_url: DEFAULT_AMQP_URL.into(),
            user: String::new(),
            debug: false,
            timeout: Duration::from_secs(15),
        }
    }
}

impl Options {
    pub fn from_global(global: &crate::cli::GlobalArgs) -> Self {
        Self {
            amqp_url: global.pizzamat_amqp_url.clone(),
            user: global.pizzamat_user.clone().unwrap_or_default(),
            debug: global.pizzamat_debug,
            timeout: Duration::from_secs(global.pizzamat_timeout),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    List,
    Services,
    Init,
    Lock,
    Unlock,
    Reset,
    Join,
    Add,
    Remove,
    Clear,
    Commit,
    SetDefault,
    ServiceAdd,
    ServiceRemove,
}

impl Operation {
    pub const ALL: [Self; 14] = [
        Self::List,
        Self::Services,
        Self::Init,
        Self::Join,
        Self::Add,
        Self::Remove,
        Self::Clear,
        Self::Commit,
        Self::Lock,
        Self::Unlock,
        Self::Reset,
        Self::SetDefault,
        Self::ServiceAdd,
        Self::ServiceRemove,
    ];

    pub fn command(self) -> &'static str {
        match self {
            Self::List => "pizza-list",
            Self::Services => "pizza-services",
            Self::Init => "pizza-init",
            Self::Lock => "pizza-lock",
            Self::Unlock => "pizza-unlock",
            Self::Reset => "pizza-reset",
            Self::Join => "pizza-join",
            Self::Add => "pizza-add",
            Self::Remove => "pizza-rm",
            Self::Clear => "pizza-clear",
            Self::Commit => "pizza-commit",
            Self::SetDefault => "pizza-set-default",
            Self::ServiceAdd => "pizza-service-add",
            Self::ServiceRemove => "pizza-service-rm",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::List => "Show orders",
            Self::Services => "Show delivery services",
            Self::Init => "Start order",
            Self::Lock => "End ordering (mark ordered)",
            Self::Unlock => "Reopen ordering",
            Self::Reset => "Reset everyone's orders",
            Self::Join => "Join / uncommit my order",
            Self::Add => "Add food to my order",
            Self::Remove => "Remove matching food",
            Self::Clear => "Remove my entire order",
            Self::Commit => "Finish my order",
            Self::SetDefault => "Set my usual order",
            Self::ServiceAdd => "Add / update delivery service",
            Self::ServiceRemove => "Remove delivery service",
        }
    }

    pub fn fields(self) -> &'static [&'static str] {
        match self {
            Self::Init | Self::ServiceRemove => &["Delivery service"],
            Self::Add => &["Food (or default for your usual order)"],
            Self::Remove => &["Food substring (case-sensitive)"],
            Self::SetDefault => &["Delivery service", "Default food"],
            Self::ServiceAdd => &[
                "Service name",
                "Display name (- for none)",
                "Description (- for none)",
                "Order URL (- for none)",
                "Menu URL (- for none)",
            ],
            _ => &[],
        }
    }

    pub fn read_only(self) -> bool {
        matches!(self, Self::List | Self::Services)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub operation: Operation,
    pub user: String,
    pub args: String,
}

impl Request {
    pub fn command(&self, debug: bool) -> String {
        let command = self.operation.command();
        if debug {
            command.replacen("pizza-", "puzza-", 1)
        } else {
            command.to_string()
        }
    }

    pub fn payload(&self, debug: bool, source: &str) -> serde_json::Value {
        serde_json::json!({
            "cmd": self.command(debug), "args": self.args, "user": self.user,
            "src": source, "dst": null,
        })
    }

    pub fn new(operation: Operation, user: &str, fields: &[String]) -> Result<Self> {
        let user = user.trim();
        validate_text(user, "Pizzamat user")?;
        ensure!(
            fields.len() == operation.fields().len(),
            "incorrect Pizzamat argument count"
        );
        let fields: Vec<_> = fields.iter().map(|s| s.trim()).collect();
        for (value, label) in fields.iter().zip(operation.fields()) {
            validate_text(value, label)?;
        }
        if operation == Operation::SetDefault {
            ensure!(
                !fields[0].contains(char::is_whitespace),
                "default-order service must be a single word"
            );
        }
        let args = if operation == Operation::ServiceAdd {
            fields
                .iter()
                .map(|field| quote(field))
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            fields.join(" ")
        };
        Ok(Self {
            operation,
            user: user.to_string(),
            args,
        })
    }
}

fn validate_text(value: &str, label: &str) -> Result<()> {
    ensure!(!value.is_empty(), "{label} must not be empty");
    ensure!(
        value.len() <= 4096,
        "{label} is too long (maximum 4096 bytes)"
    );
    ensure!(
        !value.chars().any(char::is_control),
        "{label} must not contain control characters"
    );
    Ok(())
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn food_is_literal_and_identity_is_required() {
        let request = Request::new(
            Operation::Add,
            " alice ",
            &["Pizza \"Quattro\" & soda".into()],
        )
        .unwrap();
        assert_eq!(request.user, "alice");
        assert_eq!(request.args, "Pizza \"Quattro\" & soda");
        assert!(Request::new(Operation::Clear, " ", &[]).is_err());
        assert!(Request::new(Operation::Remove, "alice", &[" ".into()]).is_err());
        assert!(Request::new(Operation::List, "alice", &["extra".into()]).is_err());
    }

    #[test]
    fn service_fields_use_pizzamats_quoting_rules() {
        let request = Request::new(
            Operation::ServiceAdd,
            "alice",
            &[
                "pizza".into(),
                "The \"Pizza\" Place".into(),
                "a\\b".into(),
                "-".into(),
                "https://example/menu".into(),
            ],
        )
        .unwrap();
        assert_eq!(
            request.args,
            r#""pizza" "The \"Pizza\" Place" "a\\b" "-" "https://example/menu""#
        );
        assert!(
            Request::new(
                Operation::SetDefault,
                "alice",
                &["two words".into(), "pizza".into()]
            )
            .is_err()
        );
    }

    #[test]
    fn wire_envelope_uses_edi_identity_and_explicit_debug_names() {
        let request =
            Request::new(Operation::Add, "alice", &["Quattro \"formaggi\"".into()]).unwrap();
        assert_eq!(
            request.payload(false, "subraumctl.recv.test"),
            serde_json::json!({
                "cmd": "pizza-add", "args": "Quattro \"formaggi\"", "user": "alice",
                "src": "subraumctl.recv.test", "dst": null,
            })
        );
        assert_eq!(request.payload(true, "test")["cmd"], "puzza-add");
    }
}
