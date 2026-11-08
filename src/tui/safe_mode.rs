//! Safe-mode confirmation data.

use crate::action::Action;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ConfirmChoice {
    Cancel,
    Confirm,
}

impl ConfirmChoice {
    pub fn toggle(self) -> Self {
        match self {
            Self::Cancel => Self::Confirm,
            Self::Confirm => Self::Cancel,
        }
    }

    pub fn is_confirm(self) -> bool {
        matches!(self, Self::Confirm)
    }
}

#[derive(Clone, Debug)]
pub enum PendingCommand {
    Device {
        description: String,
        actions: Vec<Action>,
        destructive: bool,
    },
    Heizung(crate::heizung::Request),
}

impl PendingCommand {
    pub fn preview_lines(&self, max_actions: usize) -> Vec<String> {
        let (description, actions, destructive) = match self {
            Self::Device {
                description,
                actions,
                destructive,
            } => (description, actions, destructive),
            Self::Heizung(request) => {
                return vec![
                    "Set heating targets".into(),
                    format!("AMQP cmd/heizung {}", request.args()),
                ];
            }
        };
        let mut lines = vec![description.clone()];
        for action in actions.iter().take(max_actions) {
            lines.push(action.describe());
        }
        if actions.len() > max_actions {
            lines.push(format!("… {} more action(s)", actions.len() - max_actions));
        }
        if *destructive {
            lines.push("WARNING: destructive command".to_string());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_toggles() {
        assert_eq!(ConfirmChoice::Cancel.toggle(), ConfirmChoice::Confirm);
        assert_eq!(ConfirmChoice::Confirm.toggle(), ConfirmChoice::Cancel);
    }

    #[test]
    fn preview_lists_all_actions_and_a_destructive_warning() {
        let pending = PendingCommand::Device {
            description: "test".to_string(),
            actions: vec![
                Action::Publish {
                    topic: "a".to_string(),
                    payload: "1".to_string(),
                    retain: false,
                },
                Action::Publish {
                    topic: "b".to_string(),
                    payload: "2".to_string(),
                    retain: false,
                },
            ],
            destructive: true,
        };
        let lines = pending.preview_lines(1);
        assert_eq!(lines[0], "test");
        assert!(lines[1].contains("MQTT a"));
        assert!(lines.iter().any(|line| line.contains("more action")));
        assert!(lines.iter().any(|line| line.contains("WARNING")));
    }
}
