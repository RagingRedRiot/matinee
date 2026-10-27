//! Windows: the terminal pane runs PowerShell on a pseudo console, and the
//! boundary gives commands a fresh profile but no isolation. Apps run on a
//! private desktop of the session's own ([`wm`]), each window captured and
//! composited into the frame, with input posted to them ([`input`]).
//! Accessibility is not read yet.

mod a11y;
pub mod boundary;
mod display;
mod input;
mod proc;
mod shell;
pub mod viewport;
mod wm;

pub use a11y::{A11y, App};
pub use boundary::Boundary;
pub use display::Display;
pub use proc::Child;
pub use shell::Shell;
pub use wm::parent_of;

/// The shell `matinee shell` runs by default, and the one seeds use.
pub const SHELL: &str = "powershell.exe";
/// What the terminal pane's prompt shows before each command.
pub const PROMPT_CHAR: &str = ">";
