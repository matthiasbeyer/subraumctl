//! MCP-only permissions. These never change CLI or TUI behavior.

use std::collections::BTreeMap;
use std::path::Path;

use eyre::{Result, WrapErr};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub enum Permission {
    #[serde(rename = "r")]
    Read,
    #[serde(rename = "w")]
    Write,
    #[serde(rename = "x")]
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

/// Config groups may cover several separately registered MCP tools.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    Dmx,
    WhiteLight,
    Outlet,
    Alert,
    System,
    Knob,
    Smoothiesaur,
    Status,
    Publish,
    Heizung,
    Climate,
    Power,
    Pizzamat,
}

impl Group {
    pub const ALL: [Self; 13] = [
        Self::Dmx,
        Self::WhiteLight,
        Self::Outlet,
        Self::Alert,
        Self::System,
        Self::Knob,
        Self::Smoothiesaur,
        Self::Status,
        Self::Publish,
        Self::Heizung,
        Self::Climate,
        Self::Power,
        Self::Pizzamat,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Dmx => "dmx",
            Self::WhiteLight => "white_light",
            Self::Outlet => "outlet",
            Self::Alert => "alert",
            Self::System => "system",
            Self::Knob => "knob",
            Self::Smoothiesaur => "smoothiesaur",
            Self::Status => "status",
            Self::Publish => "publish",
            Self::Heizung => "heizung",
            Self::Climate => "climate",
            Self::Power => "power",
            Self::Pizzamat => "pizzamat",
        }
    }

    fn supports(self, access: Access) -> bool {
        match access {
            Access::Read => matches!(
                self,
                Self::WhiteLight
                    | Self::Alert
                    | Self::Knob
                    | Self::Smoothiesaur
                    | Self::Status
                    | Self::Heizung
                    | Self::Climate
                    | Self::Power
                    | Self::Pizzamat
            ),
            Access::Write => !matches!(self, Self::Status | Self::Climate | Self::Power),
        }
    }

    fn default_permission(self) -> Permission {
        match self {
            Self::Outlet | Self::Smoothiesaur | Self::Publish => Permission::Disabled,
            Self::Status | Self::Climate | Self::Power => Permission::Read,
            _ => Permission::Write,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    tools: BTreeMap<Group, Permission>,
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let Some(path) = path else {
            return Ok(Self::default());
        };
        let text = std::fs::read_to_string(path)
            .wrap_err_with(|| format!("failed to read MCP config {}", path.display()))?;
        toml::from_str(&text).wrap_err_with(|| format!("invalid MCP config {}", path.display()))
    }

    pub fn permission(&self, group: Group) -> Permission {
        self.tools
            .get(&group)
            .copied()
            .unwrap_or_else(|| group.default_permission())
    }

    pub fn allows(&self, group: Group, access: Access) -> bool {
        group.supports(access)
            && match self.permission(group) {
                Permission::Read => access == Access::Read,
                Permission::Write => true,
                Permission::Disabled => false,
            }
    }

    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        for group in Group::ALL {
            if self.permission(group) == Permission::Read && !group.supports(Access::Read) {
                warnings.push(format!(
                    "MCP group '{}' is write-only; permission 'r' disables it",
                    group.name()
                ));
            }
            if matches!(group, Group::Outlet | Group::Smoothiesaur)
                && (self.allows(group, Access::Read) || self.allows(group, Access::Write))
            {
                warnings.push(format!(
                    "MCP group '{}' is enabled: this is dangerous and might break things",
                    group.name()
                ));
            }
        }
        warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_allow_supported_operations_except_explicit_opt_ins() {
        let config: Config = toml::from_str("").unwrap();
        for group in Group::ALL {
            let enabled = !matches!(group, Group::Outlet | Group::Smoothiesaur | Group::Publish);
            for access in [Access::Read, Access::Write] {
                assert_eq!(
                    config.allows(group, access),
                    enabled && group.supports(access)
                );
            }
        }
        assert!(config.warnings().is_empty());
    }

    #[test]
    fn permissions_cover_reads_writes_and_disabled_groups() {
        for group in Group::ALL {
            for permission in ["r", "w", "x"] {
                let config: Config =
                    toml::from_str(&format!("[tools]\n{} = '{permission}'", group.name())).unwrap();
                assert_eq!(
                    config.allows(group, Access::Read),
                    permission != "x" && group.supports(Access::Read)
                );
                assert_eq!(
                    config.allows(group, Access::Write),
                    permission == "w" && group.supports(Access::Write)
                );
                let read_warning = config.warnings().iter().any(|w| w.contains("write-only"));
                assert_eq!(
                    read_warning,
                    permission == "r" && !group.supports(Access::Read)
                );
            }
        }
    }

    #[test]
    fn dangerous_groups_warn_only_when_effectively_enabled() {
        let config: Config = toml::from_str("[tools]\noutlet = 'w'\nsmoothiesaur = 'r'").unwrap();
        let warnings = config.warnings();
        assert_eq!(warnings.len(), 2);
        assert!(
            warnings
                .iter()
                .all(|w| w.contains("dangerous and might break things"))
        );
        let config: Config = toml::from_str("[tools]\noutlet = 'r'").unwrap();
        assert_eq!(config.warnings().len(), 1);
        assert!(config.warnings()[0].contains("write-only"));
    }

    #[test]
    fn rejects_unknown_keys_permissions_and_duplicates() {
        for input in [
            "tool = {}",
            "[tools]\nknbo = 'r'",
            "[tools]\nsubscribe = 'r'",
            "[tools]\nknob = 'rw'",
            "[tools]\nknob = true",
            "[tools]\nknob = 'r'\nknob = 'w'",
        ] {
            assert!(toml::from_str::<Config>(input).is_err(), "{input}");
        }
    }
}
