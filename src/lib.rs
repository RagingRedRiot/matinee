//! matinee performs a scripted demo against the real program and records it.
//!
//! The pipeline is built into matinee rather than borrowed from external
//! tools (see BUILD-DECISIONS.md). Each stage gets its own module as it is
//! built: the storyboard, the session and the runner that performs steps
//! in it, the terminal pane, the camera on a frame-indexed timeline, and
//! the GIF encoder. What depends on the desktop platform, such as the
//! isolation boundary, the headless compositor, and the accessibility
//! tree, lives under [`platform`].

pub mod camera;
pub mod discover;
pub mod frame;
pub mod keys;
pub mod lzw;
pub mod overlay;
pub mod palette;
pub mod platform;
pub mod record;
pub mod runner;
pub mod session;
pub mod story;
pub mod terminal;
