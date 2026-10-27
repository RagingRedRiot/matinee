//! The display on Windows shows no windows yet: frames are the bare
//! background, which the terminal pane covers in a terminal story, and
//! input goes nowhere. Launching an app is refused.

use super::{a11y::A11y, boundary::Boundary};
use crate::{
    frame::Frame,
    platform::{Button, Window},
    story::{Story, Theme},
};
use std::{io, time::Duration};

/// The desktop background behind windows, the same gray as on Linux.
const BACKGROUND: u32 = 0xff80_8c8c;

pub struct Display {
    size: (u32, u32),
    scale: u32,
    area: (i32, i32, i32, i32),
    theme: Theme,
}

impl Display {
    /// Start the display for `story` in `theme`.
    pub fn start(_boundary: &Boundary, story: &Story, theme: Theme) -> io::Result<Display> {
        if let Some(launch) = story.launches.first() {
            return Err(io::Error::other(format!(
                "launch {}: apps cannot be shown on Windows yet; only terminal stories run",
                launch.name
            )));
        }
        let [w, h] = story.demo.size;
        Ok(Display {
            size: (w, h),
            scale: story.demo.scale,
            area: (0, 0, w as i32, h as i32),
            theme,
        })
    }

    /// Environment for programs on the display: the theme being recorded,
    /// so a matinee run inside follows it. There is no X11 here.
    pub fn env(&self, x11: bool) -> io::Result<Vec<(String, String)>> {
        if x11 {
            return Err(io::Error::other("there is no X11 on Windows"));
        }
        Ok(vec![("MATINEE_THEME".into(), self.theme.name().into())])
    }

    /// Wait for `timeout`; there are no apps to serve.
    pub fn dispatch(&mut self, timeout: Duration) -> io::Result<()> {
        std::thread::sleep(timeout);
        Ok(())
    }

    /// Draw the screen: the background alone.
    pub fn render(&mut self, _time: Duration) -> io::Result<Frame> {
        let (w, h) = (
            (self.size.0 * self.scale) as usize,
            (self.size.1 * self.scale) as usize,
        );
        Ok(Frame {
            width: w,
            height: h,
            pixels: vec![BACKGROUND; w * h],
        })
    }

    /// Top-level windows: none.
    pub fn windows(&self) -> Vec<Window> {
        vec![]
    }

    /// The area windows are laid out in: x, y, width, height in points.
    pub fn window_area(&self) -> (i32, i32, i32, i32) {
        self.area
    }

    pub fn set_window_area(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.area = (x, y, w, h);
    }

    /// Where the pointer is, in logical points.
    pub fn pointer_location(&self) -> (f64, f64) {
        (self.size.0 as f64 / 2.0, self.size.1 as f64 / 2.0)
    }

    pub fn pointer_move(&mut self, _x: f64, _y: f64, _time: Duration) {}

    pub fn pointer_button(&mut self, _button: Button, _pressed: bool, _time: Duration) {}

    pub fn pointer_scroll(&mut self, _dy: f64, _time: Duration) {}

    pub fn key(&mut self, _keycode: u32, _pressed: bool, _time: Duration) {}

    /// Nothing runs beside the display, so nothing can have exited.
    pub fn check(&mut self) -> io::Result<()> {
        Ok(())
    }

    pub fn accessibility(&self) -> io::Result<A11y> {
        Ok(A11y)
    }
}
