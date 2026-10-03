//! matinee performs a scripted demo against the real program and records it.
//!
//! The pipeline is built into matinee rather than borrowed from external
//! tools (see BUILD-DECISIONS.md). Each stage gets its own module as it is
//! built: the storyboard, the isolation boundary, the headless compositor,
//! accessibility targets and input, the terminal, the camera on a
//! frame-indexed timeline, and the GIF encoder.

pub mod a11y;
pub mod boundary;
pub mod bus;
pub mod compositor;
pub mod keys;
pub mod portal;
pub mod record;
pub mod runner;
pub mod session;
pub mod story;
pub mod terminal;
