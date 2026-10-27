//! The display apps run on: a private desktop of the session's own, with no
//! user watching. Each window is captured on its own through `PrintWindow`
//! ([`wm`]) and composited into the frame by its position, and input is
//! posted to the windows under the pointer and in focus ([`input`]).
//!
//! matinee works in logical points; Windows works in device pixels. This
//! module is the boundary: the public API is in points, and everything it
//! hands to [`wm`] and [`input`] is multiplied by [`Display::scale`] into
//! device pixels, and divided coming back.

use super::{a11y::A11y, boundary::Boundary, input, wm};
use crate::{
    frame::Frame,
    platform::{Button, Window},
    story::{Story, Theme},
};
use std::{collections::HashSet, io, time::Duration};

/// The desktop background behind windows, the same gray as on Linux.
const BACKGROUND: u32 = 0xff80_8c8c;

pub struct Display {
    /// The private desktop. Held for its lifetime: dropping it returns the
    /// thread to its old desktop and closes this one.
    _desktop: wm::Desktop,
    /// The screen size in logical points.
    size: (u32, u32),
    /// Device pixels per point.
    scale: u32,
    /// The area windows are laid out in: x, y, width, height in points.
    area: (i32, i32, i32, i32),
    theme: Theme,
    /// Where the pointer is, in logical points.
    pointer: (f64, f64),
    /// The pointer buttons currently held, as a mouse-message flag set (see
    /// [`input::button_flag`]).
    buttons: usize,
    /// Which modifier keys are held.
    modifiers: input::Modifiers,
    /// The windows matinee has already given their place, by `HWND`, so a
    /// window is placed once and then left where the app may move it.
    placed: HashSet<isize>,
}

impl Display {
    /// Start the display for `story` in `theme`: make the session's private
    /// desktop and move this thread onto it, so the apps it launches open
    /// their windows there.
    pub fn start(_boundary: &Boundary, story: &Story, theme: Theme) -> io::Result<Display> {
        let desktop = wm::Desktop::create(super::DESKTOP)?;
        let [w, h] = story.demo.size;
        Ok(Display {
            _desktop: desktop,
            size: (w, h),
            scale: story.demo.scale,
            area: (0, 0, w as i32, h as i32),
            theme,
            pointer: (w as f64 / 2.0, h as f64 / 2.0),
            buttons: 0,
            modifiers: input::Modifiers::default(),
            placed: HashSet::new(),
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

    /// Place any newly shown windows, then wait for `timeout`. Apps draw and
    /// answer posted input on their own threads, so there is no event loop
    /// to run here.
    pub fn dispatch(&mut self, timeout: Duration) -> io::Result<()> {
        self.place_new_windows();
        std::thread::sleep(timeout);
        Ok(())
    }

    /// Give every app window not yet placed its place in the window area: a
    /// window at least as large as the area is sized to fill it; a smaller
    /// window or a dialog is centered at its own size. Each window is placed
    /// once and then remembered.
    fn place_new_windows(&mut self) {
        let (ax, ay, aw, ah) = self.area;
        let s = self.scale as i32;
        for hwnd in wm::hidden_app_windows_here() {
            let id = hwnd as isize;
            if self.placed.contains(&id) {
                continue;
            }
            // Apps may create a window hidden on a desktop no one is
            // watching; matinee reveals it so it can be captured.
            wm::show(hwnd);
            if wm::is_owned(hwnd) {
                // A dialog keeps its own size, centered in the area.
                let r = wm::visible_rect(hwnd);
                let (ww, wh) = ((r.right - r.left) / s, (r.bottom - r.top) / s);
                let (x, y) = (ax + (aw - ww) / 2, ay + (ah - wh) / 2);
                wm::place(hwnd, x * s, y * s, None);
            } else {
                // A top-level window fills the area, as the Linux compositor
                // tiles one, so a demo frames the app the same way.
                wm::place(hwnd, ax * s, ay * s, Some((aw * s, ah * s)));
            }
            self.placed.insert(id);
        }
    }

    /// Draw the screen: the gray background with every drawn window
    /// composited over it, bottom of the stack first.
    ///
    /// The frame is `size * scale` device pixels wide and high, row-major,
    /// one `0xAARRGGBB` pixel each, fully opaque. [`wm::drawn_windows`]
    /// gives the windows top of the stack first, so they are drawn in
    /// reverse. Each is captured with [`wm::capture`] (device pixels of its
    /// visible rectangle, already opaque) and copied in at its
    /// [`wm::visible_rect`] origin, clipped to the frame.
    pub fn render(&mut self, _time: Duration) -> io::Result<Frame> {
        let w = (self.size.0 * self.scale) as usize;
        let h = (self.size.1 * self.scale) as usize;
        let mut pixels = vec![BACKGROUND; w * h];

        for hwnd in wm::drawn_windows().into_iter().rev() {
            if let Some((cw, ch, px)) = wm::capture(hwnd) {
                let r = wm::visible_rect(hwnd);
                for cy in 0..ch {
                    for cx in 0..cw {
                        let dx = r.left + cx as i32;
                        let dy = r.top + cy as i32;
                        if dx >= 0 && dy >= 0 {
                            let ux = dx as usize;
                            let uy = dy as usize;
                            if ux < w && uy < h {
                                let src_idx = cy * cw + cx;
                                let dst_idx = uy * w + ux;
                                pixels[dst_idx] = px[src_idx];
                            }
                        }
                    }
                }
            }
        }
        Ok(Frame {
            width: w,
            height: h,
            pixels,
        })
    }

    /// Top-level app windows, bottom to top, their geometry in points.
    ///
    /// [`wm::app_windows_here`] gives them top first, so they are reversed.
    /// Each window's position and size come from [`wm::visible_rect`]
    /// divided by [`Display::scale`]; its title from [`wm::title`]; its
    /// `app_id` from [`wm::program_name`] of [`wm::process_of`]. The window
    /// on top of the stack (first from `app_windows_here`) is the focused
    /// one. A window is `placed` once it is in [`Display::placed`].
    pub fn windows(&self) -> Vec<Window> {
        let apps = wm::app_windows_here();
        let top = apps.first().copied();
        let s = self.scale as i32;
        let mut ws = Vec::new();
        for &hwnd in apps.iter().rev() {
            let r = wm::visible_rect(hwnd);
            let title = wm::title(hwnd);
            let app_id = wm::program_name(wm::process_of(hwnd));
            let x = r.left / s;
            let y = r.top / s;
            let width = (r.right - r.left) / s;
            let height = (r.bottom - r.top) / s;
            let focused = Some(hwnd) == top;
            let placed = self.placed.contains(&(hwnd as isize));
            ws.push(Window {
                title,
                app_id,
                x,
                y,
                width,
                height,
                focused,
                placed,
            });
        }
        ws
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
        self.pointer
    }

    /// Move the pointer to `(x, y)` in logical points, posting the move to
    /// the window under it with the held buttons.
    ///
    /// Record `(x, y)` as the pointer location, then call
    /// [`input::pointer_move`] with the point scaled to device pixels (each
    /// coordinate times [`Display::scale`]) and [`Display::buttons`].
    pub fn pointer_move(&mut self, x: f64, y: f64, _time: Duration) {
        self.pointer = (x, y);
        input::pointer_move(x * self.scale as f64, y * self.scale as f64, self.buttons);
    }

    /// Press or release a pointer button where the pointer is.
    ///
    /// Update [`Display::buttons`] first: set the button's
    /// [`input::button_flag`] when `pressed`, clear it otherwise. Then call
    /// [`input::pointer_button`] with the pointer location scaled to device
    /// pixels, `button`, `pressed`, and the updated buttons.
    pub fn pointer_button(&mut self, button: Button, pressed: bool, _time: Duration) {
        let flag = input::button_flag(button);
        if pressed {
            self.buttons |= flag;
        } else {
            self.buttons &= !flag;
        }
        let (px, py) = self.pointer;
        input::pointer_button(
            px * self.scale as f64,
            py * self.scale as f64,
            button,
            pressed,
            self.buttons,
        );
    }

    /// Scroll by `dy` logical points; positive scrolls down.
    ///
    /// Call [`input::pointer_scroll`] with the pointer location scaled to
    /// device pixels, `dy` unchanged, and [`Display::buttons`].
    pub fn pointer_scroll(&mut self, dy: f64, _time: Duration) {
        let (px, py) = self.pointer;
        input::pointer_scroll(
            px * self.scale as f64,
            py * self.scale as f64,
            dy,
            self.buttons,
        );
    }

    /// Press or release a key by its code (see [`crate::keys`]).
    ///
    /// Record the key on [`Display::modifiers`] with
    /// [`input::Modifiers::apply`]. Then, if there is an app window (the
    /// first from [`wm::app_windows_here`]), call [`input::key`] on it with
    /// `keycode`, `pressed`, and the modifiers. With no app window, do
    /// nothing.
    pub fn key(&mut self, keycode: u32, pressed: bool, _time: Duration) {
        let _ = self.modifiers.apply(keycode, pressed);
        if let Some(top) = wm::app_windows_here().first().cloned() {
            input::key(top, keycode, pressed, self.modifiers);
        }
    }

    /// Nothing runs beside the apps that could exit, so nothing to check.
    pub fn check(&mut self) -> io::Result<()> {
        Ok(())
    }

    /// Connect to the accessibility trees apps publish through UI
    /// Automation.
    pub fn accessibility(&self) -> io::Result<A11y> {
        A11y::connect()
    }
}
