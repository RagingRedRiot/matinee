//! What differs from one desktop platform to the next: the isolation
//! boundary, the display that apps draw on and take input from, the
//! accessibility tree, the shell behind the terminal pane, and the viewport
//! window. Everything above this module, from the storyboard to the camera
//! and the encoder, is the same on every platform.
//!
//! Each platform provides the same names with the same shapes, selected at
//! compile time, the way the standard library does. Linux has the full
//! pipeline; Windows runs terminal and GUI stories, with the gaps
//! `docs/windows.md` lists.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

mod tree;
mod view;

/// A rectangle in device pixels on the desktop: x, y, width, height.
pub type TextRect = (f64, f64, f64, f64);
pub use tree::{Node, in_points};

/// A top-level window on the display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub title: String,
    /// The app's identifier, such as a Wayland app ID.
    pub app_id: String,
    /// The window's position and size in logical points.
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Whether it has keyboard focus.
    pub focused: bool,
    /// Whether it has been given its place: sized to the window area, or,
    /// for a dialog, centered at its own size.
    pub placed: bool,
}

/// A pointer button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}
