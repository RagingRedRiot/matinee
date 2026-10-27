//! The viewport window is not available on Windows yet.

use crate::{camera::Rect, frame::Frame};
use std::io;

pub fn open(_capacity: usize, _screen: (u32, u32)) -> io::Result<()> {
    Err(io::Error::other("--view is not available on Windows yet"))
}

pub fn publish(_frame: &Frame, _view: Rect, _pointer: Option<(f64, f64)>, _scale: u32) {}

pub fn close() {}
