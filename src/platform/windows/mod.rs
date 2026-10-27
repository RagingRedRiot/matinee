//! Windows: terminal stories so far. The terminal pane runs PowerShell on
//! a pseudo console, the boundary gives commands a fresh profile but no
//! isolation, and the display shows no windows yet.

mod a11y;
pub mod boundary;
mod display;
mod shell;
pub mod viewport;

pub use a11y::{A11y, App};
pub use boundary::Boundary;
pub use display::Display;
pub use shell::Shell;

/// The shell `matinee shell` runs by default, and the one seeds use.
pub const SHELL: &str = "powershell.exe";
/// What the terminal pane's prompt shows before each command.
pub const PROMPT_CHAR: &str = ">";
