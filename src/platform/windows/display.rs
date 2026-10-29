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
use std::{collections::HashSet, io, sync::Once, time::Duration};
use windows_sys::Win32::{
    Foundation::POINT,
    Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint},
    UI::{
        HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
            SetProcessDpiAwarenessContext,
        },
        WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN},
    },
};

/// The scale mismatch is reported once a run, not once a theme.
static SCALE_WARNING: Once = Once::new();

/// The primary display's scaling, in percent, as Windows applies it now.
fn display_scale() -> Option<u32> {
    let (mut x, mut y) = (0u32, 0u32);
    let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    let ok = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) } >= 0;
    (ok && x > 0).then_some(x * 100 / 96)
}

/// What to say when the display is scaled to `display` percent but the
/// story records at `scale`. Apps draw at the display's scaling, while the
/// story lays their windows out for its own, so they show larger or
/// smaller than it intends.
fn scale_warning(display: u32, scale: u32) -> String {
    let wanted = scale * 100;
    let size = display * 100 / wanted;
    format!(
        "matinee: warning: the display is scaled to {display}%, but the story records at \
         scale {scale}, for {wanted}%: apps will draw at {size}% of the size the story lays \
         them out for; set the display's scaling to {wanted}% to match"
    )
}

/// The desktop background behind windows, the same gray as on Linux.
const BACKGROUND: u32 = 0xff80_8c8c;

pub struct Display {
    /// Where the apps run: the private desktop, or the screen. Held for its
    /// lifetime: dropping it ends the session's apps, and returns the
    /// thread from a private desktop to its old one and closes it.
    _desktop: wm::Desktop,
    /// Whether apps run on the user's screen rather than a private desktop:
    /// input then goes through the real input queue.
    screen: bool,
    /// On a private desktop, UI Automation, through which clicks and typed
    /// characters act on controls directly: many apps, such as WinUI and
    /// WPF ones, ignore posted input. What it cannot act on is posted.
    uia_input: Option<A11y>,
    /// Whether the left button's press went through UI Automation, so its
    /// release does too.
    uia_pressed: bool,
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
    /// their windows there, or, with `--screen`, use the user's screen.
    pub fn start(boundary: &Boundary, story: &Story, theme: Theme) -> io::Result<Display> {
        // Without this, Windows scales what a DPI-unaware process sees on a
        // scaled display, and only some of it: window rectangles, captures,
        // and UI Automation would disagree about where a window is.
        unsafe {
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        if let Some(display) = display_scale() {
            let wanted = story.demo.scale * 100;
            if display != wanted {
                SCALE_WARNING
                    .call_once(|| eprintln!("{}", scale_warning(display, story.demo.scale)));
            }
        }
        let [w, h] = story.demo.size;
        let screen = boundary.is_on_screen();
        if screen {
            // Windows are laid out from the screen's top-left corner, so the
            // whole frame must be on it.
            let (sw, sh) =
                unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
            let (fw, fh) = ((w * story.demo.scale) as i32, (h * story.demo.scale) as i32);
            if fw > sw || fh > sh {
                return Err(io::Error::other(format!(
                    "the story's {fw}x{fh} pixel screen does not fit on your {sw}x{sh} screen; \
                     make demo.size or demo.scale smaller"
                )));
            }
        }
        let desktop = wm::Desktop::create(screen)?;
        let uia_input = if screen { None } else { A11y::connect().ok() };
        Ok(Display {
            _desktop: desktop,
            screen,
            uia_input,
            uia_pressed: false,
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
        // Only windows the app has shown. Apps keep hidden windows of their
        // own, such as Chromium's full-size one and Task Manager's tray
        // window, which shown would cover the app.
        for hwnd in wm::app_windows_here() {
            let id = hwnd as isize;
            if self.placed.contains(&id) {
                continue;
            }
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
            if self.screen {
                // Above the user's own windows, so what is drawn and what
                // takes input is the app.
                input::bring_forward(hwnd);
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
            let app_id = wm::program_name(wm::app_of(hwnd));
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
        let (px, py) = (x * self.scale as f64, y * self.scale as f64);
        if self.screen {
            input::send_pointer_move(px, py);
        } else {
            input::pointer_move(px, py, self.buttons);
        }
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
        if let (Some(a11y), Button::Left) = (&self.uia_input, button) {
            let s = self.scale as f64;
            if pressed && a11y.press_at((px * s).round() as i32, (py * s).round() as i32) {
                self.uia_pressed = true;
                return;
            }
            if !pressed && self.uia_pressed {
                self.uia_pressed = false;
                return;
            }
        }
        if self.screen {
            input::send_pointer_button(
                px * self.scale as f64,
                py * self.scale as f64,
                button,
                pressed,
            );
            return;
        }
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
        if self.screen {
            input::send_pointer_scroll(px * self.scale as f64, py * self.scale as f64, dy);
            return;
        }
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
        if let (Some(a11y), true) = (&self.uia_input, pressed) {
            if let Some(c) = input::char_for(keycode, self.modifiers) {
                if a11y.type_char(c) {
                    return;
                }
            }
        }
        if self.screen {
            input::send_key(wm::app_windows_here().first().copied(), keycode, pressed);
            return;
        }
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

#[cfg(test)]
mod tests {
    use super::scale_warning;

    #[test]
    fn a_scale_mismatch_says_how_far_apps_are_off_and_what_to_set() {
        let w = scale_warning(150, 2);
        assert!(w.contains("scaled to 150%"), "{w}");
        assert!(w.contains("75% of the size"), "{w}");
        assert!(w.contains("scaling to 200%"), "{w}");
        assert!(scale_warning(150, 1).contains("150% of the size"));
    }
}
