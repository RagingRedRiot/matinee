//! Linux: a headless Wayland compositor for the display, namespaces for the
//! boundary, D-Bus and AT-SPI for accessibility, bash on a pseudo-terminal
//! behind the terminal pane, and a Wayland window for the viewport.

pub mod a11y;
pub mod boundary;
pub mod bus;
pub mod compositor;
mod display;
pub mod portal;
mod proc;
mod shell;
pub mod viewport;

pub use a11y::{A11y, App};
pub use boundary::Boundary;
pub use display::Display;
pub use proc::Child;
pub use shell::Shell;

/// The parent of process `pid`, if it is running.
pub fn parent_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The parent is the second field after the parenthesized name.
    stat.rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().nth(1))
        .and_then(|p| p.parse().ok())
}

/// Whether accessibility trees always report device pixels. AT-SPI
/// toolkits report points or device pixels, some both, so each tree's
/// space is worked out from its sizes.
pub const ACCESSIBILITY_IN_PIXELS: bool = false;

/// The shell `matinee shell` runs by default, and the one seeds use.
pub const SHELL: &str = "bash";
/// What the terminal pane's prompt shows before each command.
pub const PROMPT_CHAR: &str = "$";
