//! Crossterm input thread.
//!
//! Reading terminal events blocks, so it lives on a dedicated thread and
//! forwards events into the async TUI loop.

use std::time::Duration;

#[cfg(feature = "tui-mouse")]
use crossterm::event::MouseEventKind;
use crossterm::event::{self, Event, KeyEventKind};
use tokio::sync::mpsc::UnboundedSender;

#[cfg(feature = "tui-mouse")]
pub fn spawn_input_thread(tx: UnboundedSender<Event>) {
    std::thread::Builder::new()
        .name("subraumctl-input".to_string())
        .spawn(move || {
            loop {
                if tx.is_closed() {
                    break;
                }
                match event::poll(Duration::from_millis(50)) {
                    Ok(true) => match event::read() {
                        Ok(Event::Key(key))
                            if key.kind == KeyEventKind::Press
                                || key.kind == KeyEventKind::Repeat =>
                        {
                            let _ = tx.send(Event::Key(key));
                        }
                        Ok(Event::Mouse(mouse))
                            if matches!(
                                mouse.kind,
                                MouseEventKind::Down(_)
                                    | MouseEventKind::Drag(_)
                                    | MouseEventKind::ScrollUp
                                    | MouseEventKind::ScrollDown
                            ) =>
                        {
                            let _ = tx.send(Event::Mouse(mouse));
                        }
                        // Resizes need no handling: the TUI redraws on its
                        // tick and ratatui re-reads the terminal size then.
                        Ok(_) => {}
                        Err(_) => break,
                    },
                    Ok(false) => {}
                    Err(_) => break,
                }
            }
        })
        .expect("failed to spawn TUI input thread");
}

#[cfg(not(feature = "tui-mouse"))]
pub fn spawn_input_thread(tx: UnboundedSender<crossterm::event::KeyEvent>) {
    std::thread::Builder::new()
        .name("subraumctl-input".to_string())
        .spawn(move || {
            loop {
                if tx.is_closed() {
                    break;
                }
                match event::poll(Duration::from_millis(50)) {
                    Ok(true) => match event::read() {
                        Ok(Event::Key(key))
                            if key.kind == KeyEventKind::Press
                                || key.kind == KeyEventKind::Repeat =>
                        {
                            let _ = tx.send(key);
                        }
                        // Resizes need no handling: the TUI redraws on its
                        // tick and ratatui re-reads the terminal size then.
                        Ok(_) => {}
                        Err(_) => break,
                    },
                    Ok(false) => {}
                    Err(_) => break,
                }
            }
        })
        .expect("failed to spawn TUI input thread");
}
