#![allow(clippy::uninlined_format_args)]
#![allow(clippy::large_enum_variant)]

//! Control CLI and TUI for the subraum MQTT system.
//!
//! The crate is split into a transport/domain layer (`action`, `mqtt`,
//! `smoothiesaur`, `heizung`, `decode`) and a presentation layer (`cli`, `commands`,
//! `tui`). The CLI and TUI build the same [`action::ControlCommand`] values so
//! both front ends send byte-identical wire messages.

pub mod action;
pub mod cli;
pub mod color;
pub mod commands;
pub mod decode;
pub mod heizung;
pub mod knob;
pub mod logging;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod mqtt;
pub mod output;
pub mod pizzamat;
pub mod smoothiesaur;
pub mod tui;
pub mod white_light;

pub const APP_NAME: &str = "subraumctl";
