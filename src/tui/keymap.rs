//! Key binding descriptions for the footer and the help overlay.

use crate::tui::app::{FocusZone, Section};

#[derive(Copy, Clone, Debug)]
pub struct KeyBinding {
    pub keys: &'static str,
    pub action: &'static str,
}

const GLOBAL: &[KeyBinding] = &[
    KeyBinding {
        keys: "Q",
        action: "quit now",
    },
    KeyBinding {
        keys: "q",
        action: "back/quit",
    },
    KeyBinding {
        keys: "Tab",
        action: "focus",
    },
    KeyBinding {
        keys: "[ ]",
        action: "section",
    },
    KeyBinding {
        keys: "Enter",
        action: "activate",
    },
    KeyBinding {
        keys: "?",
        action: "help",
    },
    KeyBinding {
        keys: "Ctrl+L",
        action: "log",
    },
    KeyBinding {
        keys: "Ctrl+T",
        action: "traffic",
    },
    KeyBinding {
        keys: "Ctrl+G",
        action: "focus log",
    },
    KeyBinding {
        keys: "S",
        action: "safe",
    },
    KeyBinding {
        keys: "c d r",
        action: "connect/disconnect/reconnect",
    },
];

const FOOTER_GLOBAL: &[KeyBinding] = &[
    KeyBinding {
        keys: "q",
        action: "back",
    },
    KeyBinding {
        keys: "Q",
        action: "quit",
    },
    KeyBinding {
        keys: "Tab",
        action: "focus",
    },
    KeyBinding {
        keys: "[ ]",
        action: "section",
    },
    KeyBinding {
        keys: "?",
        action: "help",
    },
    KeyBinding {
        keys: "Ctrl+L",
        action: "log",
    },
    KeyBinding {
        keys: "S",
        action: "safe",
    },
    KeyBinding {
        keys: "c/d/r",
        action: "conn",
    },
];

const SIDEBAR: &[KeyBinding] = &[
    KeyBinding {
        keys: "↑ ↓",
        action: "section",
    },
    KeyBinding {
        keys: "Enter",
        action: "open content",
    },
    KeyBinding {
        keys: "[ ]",
        action: "section",
    },
];

const CONTENT_OVERVIEW: &[KeyBinding] = &[
    KeyBinding {
        keys: "↑ ↓",
        action: "select section",
    },
    KeyBinding {
        keys: "Enter",
        action: "open",
    },
];

const CONTENT_LIGHTING: &[KeyBinding] = &[
    KeyBinding {
        keys: "↑ ↓",
        action: "group",
    },
    KeyBinding {
        keys: "← →",
        action: "animation",
    },
    KeyBinding {
        keys: "Enter",
        action: "send",
    },
    KeyBinding {
        keys: "/",
        action: "filter",
    },
];

const CONTENT_WHITE_LIGHT: &[KeyBinding] = &[
    KeyBinding {
        keys: "↑ ↓",
        action: "focus",
    },
    KeyBinding {
        keys: "← →",
        action: "adjust",
    },
    KeyBinding {
        keys: "Space",
        action: "toggle cell",
    },
    KeyBinding {
        keys: "a n",
        action: "all/none",
    },
    KeyBinding {
        keys: "w c",
        action: "warm/cold",
    },
    KeyBinding {
        keys: "Enter",
        action: "apply/send",
    },
    KeyBinding {
        keys: "/",
        action: "filter",
    },
];

const CONTENT_OUTLETS: &[KeyBinding] = &[
    KeyBinding {
        keys: "↑ ↓",
        action: "outlet",
    },
    KeyBinding {
        keys: "← →",
        action: "on/off",
    },
    KeyBinding {
        keys: "Enter Space",
        action: "send",
    },
];

const CONTENT_ALERTS: &[KeyBinding] = &[
    KeyBinding {
        keys: "↑ ↓",
        action: "action",
    },
    KeyBinding {
        keys: "Enter",
        action: "send",
    },
];

const CONTENT_SYSTEM: &[KeyBinding] = &[
    KeyBinding {
        keys: "↑ ↓ ← →",
        action: "choose",
    },
    KeyBinding {
        keys: "Enter",
        action: "send",
    },
];

const CONTENT_ANIMATION: &[KeyBinding] = &[
    KeyBinding {
        keys: "↑ ↓",
        action: "knob",
    },
    KeyBinding {
        keys: "← →",
        action: "adjust",
    },
    KeyBinding {
        keys: "H L",
        action: "big step",
    },
    KeyBinding {
        keys: "Enter",
        action: "send",
    },
    KeyBinding {
        keys: "Esc",
        action: "revert",
    },
    KeyBinding {
        keys: "/",
        action: "filter",
    },
    KeyBinding {
        keys: "digits",
        action: "exact",
    },
];

const CONTENT_SMOOTHIE: &[KeyBinding] = &[
    KeyBinding {
        keys: "↑ ↓",
        action: "item",
    },
    KeyBinding {
        keys: "← →",
        action: "choose",
    },
    KeyBinding {
        keys: "Enter",
        action: "send",
    },
    KeyBinding {
        keys: "z",
        action: "laser zero",
    },
    KeyBinding {
        keys: "digits",
        action: "laser power",
    },
];

const CONTENT_THEME: &[KeyBinding] = &[
    KeyBinding {
        keys: "↑ ↓",
        action: "role",
    },
    KeyBinding {
        keys: "Enter",
        action: "color picker",
    },
    KeyBinding {
        keys: "x",
        action: "reset role",
    },
    KeyBinding {
        keys: "X",
        action: "reset theme",
    },
];

const TRAFFIC: &[KeyBinding] = &[
    KeyBinding {
        keys: "↑ ↓",
        action: "scroll",
    },
    KeyBinding {
        keys: "PgUp PgDn",
        action: "page",
    },
    KeyBinding {
        keys: "Enter",
        action: "detail",
    },
    KeyBinding {
        keys: "Esc q",
        action: "back",
    },
];

const APP_LOG: &[KeyBinding] = &[
    KeyBinding {
        keys: "Esc q",
        action: "close log",
    },
    KeyBinding {
        keys: "Ctrl+L",
        action: "toggle log",
    },
    KeyBinding {
        keys: "Ctrl+↑↓",
        action: "resize",
    },
    KeyBinding {
        keys: "t",
        action: "test line",
    },
];

pub fn global_bindings() -> &'static [KeyBinding] {
    FOOTER_GLOBAL
}

pub fn zone_bindings(zone: FocusZone) -> &'static [KeyBinding] {
    match zone {
        FocusZone::Sidebar => SIDEBAR,
        FocusZone::Content => CONTENT_OVERVIEW,
        FocusZone::Traffic => TRAFFIC,
        FocusZone::AppLog => APP_LOG,
    }
}

pub fn section_bindings(section: Section) -> &'static [KeyBinding] {
    match section {
        Section::Overview => CONTENT_OVERVIEW,
        Section::Lighting => CONTENT_LIGHTING,
        Section::WhiteLight => CONTENT_WHITE_LIGHT,
        Section::Outlets => CONTENT_OUTLETS,
        Section::Alerts => CONTENT_ALERTS,
        Section::System => CONTENT_SYSTEM,
        Section::Animation => CONTENT_ANIMATION,
        Section::Smoothie => CONTENT_SMOOTHIE,
        Section::Theme => CONTENT_THEME,
        Section::Pizzamat => &[
            KeyBinding {
                keys: "↑ ↓",
                action: "action",
            },
            KeyBinding {
                keys: "Enter",
                action: "run / edit",
            },
            KeyBinding {
                keys: "PgUp PgDn",
                action: "scroll reply",
            },
            KeyBinding {
                keys: "Home",
                action: "reply top",
            },
        ],
    }
}

pub fn help_text() -> String {
    let mut text = String::new();
    push_bindings(&mut text, "Global", GLOBAL);
    push_bindings(&mut text, "Sidebar", SIDEBAR);
    push_bindings(&mut text, "Traffic pane", TRAFFIC);
    push_bindings(&mut text, "Application log", APP_LOG);
    for section in Section::ALL {
        push_bindings(&mut text, section.title(), section_bindings(section));
    }
    text.push_str("Color picker\n");
    for (keys, action) in [
        ("m", "switch RGB/HSV/HSL"),
        ("Tab", "next component"),
        ("← → / h l", "adjust component"),
        ("Shift+← → / H L", "big step"),
        ("Home End", "component min/max"),
        ("digits - .", "type an exact value"),
        ("p", "named-color palette"),
        ("/", "filter palette"),
        ("Space", "preview original"),
        ("r", "reset role"),
        ("Enter", "apply"),
        ("Esc", "cancel"),
    ] {
        text.push_str(&format!("  {keys:<18} {action}\n"));
    }
    text.push_str("\nSafe mode\n");
    text.push_str("  Safe mode is on by default and confirms device MQTT publishes and\n");
    text.push_str("  smoothiesaur HTTP commands. Pizzamat actions run immediately.\n");
    text.push_str("  Press S to toggle safe mode; turning\n");
    text.push_str("  it off requires a second confirmation.\n");
    text.push_str("\nQuit semantics\n");
    text.push_str("  Q always quits immediately. q closes the current popup, input,\n");
    text.push_str("  log pane or traffic focus; otherwise it returns to Overview.\n");
    text.push_str("  In Overview, q opens the quit confirmation popup.\n");
    text.push_str("  Pizzamat text fields accept q/Q as text; Esc cancels, Ctrl+C quits.\n");
    text
}

fn push_bindings(text: &mut String, title: &str, bindings: &[KeyBinding]) {
    text.push_str(title);
    text.push('\n');
    for binding in bindings {
        text.push_str(&format!("  {:<18} {}\n", binding.keys, binding.action));
    }
    text.push('\n');
}
