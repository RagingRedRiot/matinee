//! Linux: a headless Wayland compositor for the display, namespaces for the
//! boundary, D-Bus and AT-SPI for accessibility, bash on a pseudo-terminal
//! behind the terminal pane, and a Wayland window for the viewport.

pub mod a11y;
pub mod boundary;
pub mod bus;
pub mod compositor;
mod display;
pub mod portal;
mod shell;
pub mod viewport;

pub use a11y::{A11y, App};
pub use boundary::Boundary;
pub use display::Display;
pub use shell::Shell;

/// The shell `matinee shell` runs by default, and the one seeds use.
pub const SHELL: &str = "bash";
/// What the terminal pane's prompt shows before each command.
pub const PROMPT_CHAR: &str = "$";
