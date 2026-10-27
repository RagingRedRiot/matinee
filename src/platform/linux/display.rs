//! The display apps run on: the built-in compositor, with the session and
//! accessibility buses beside it and the settings that put toolkits in the
//! session's theme.

use super::{a11y::A11y, boundary::Boundary, bus::Buses, compositor::Compositor, portal};
use crate::{
    frame::Frame,
    platform::{Button, Window},
    story::{Story, Theme},
};
use std::{io, time::Duration};

/// The compositor's socket name in the session's runtime directory.
const DISPLAY: &str = "wayland-1";

pub struct Display {
    compositor: Compositor,
    buses: Buses,
    /// Xwayland's display, for `DISPLAY`, when a launch needs X11.
    x11_display: Option<String>,
    theme: Theme,
}

impl Display {
    /// Start the display for `story` in `theme`. Call this inside the
    /// boundary.
    pub fn start(boundary: &Boundary, story: &Story, theme: Theme) -> io::Result<Display> {
        let runtime = boundary.runtime_dir().to_path_buf();
        let buses = Buses::start(boundary, &runtime, theme)?;
        let keyfile = boundary.home().join(".config/glib-2.0/settings/keyfile");
        std::fs::create_dir_all(keyfile.parent().unwrap())?;
        std::fs::write(&keyfile, portal::Settings::new(theme).keyfile())?;
        let mut compositor = Compositor::new(
            &runtime.join(DISPLAY),
            (story.demo.size[0], story.demo.size[1]),
            story.demo.scale,
        )?;
        let x11_display = if story.launches.iter().any(|l| l.x11) {
            let display = compositor.start_xwayland(&runtime)?;
            // Qt 5 finds the accessibility bus here on X11, as desktops
            // publish it; asking the session bus loses a startup race.
            compositor.set_x11_root_property("AT_SPI_BUS", &buses.a11y)?;
            Some(format!(":{display}"))
        } else {
            None
        };
        Ok(Display {
            compositor,
            buses,
            x11_display,
            theme,
        })
    }

    /// Environment for programs on the display: where it is, the buses,
    /// and the settings that make toolkits use them. With `x11`, the X11
    /// display instead of the Wayland one, so toolkits that support both
    /// use X11; it exists for apps that require X11.
    pub fn env(&self, x11: bool) -> io::Result<Vec<(String, String)>> {
        let theme = self.theme.name();
        let mut env: Vec<(&str, &str)> = if x11 {
            vec![
                ("XDG_SESSION_TYPE", "x11"),
                ("GDK_BACKEND", "x11"),
                ("QT_QPA_PLATFORM", "xcb"),
                ("SDL_VIDEODRIVER", "x11"),
                ("GSETTINGS_BACKEND", "keyfile"),
                ("LIBGL_ALWAYS_SOFTWARE", "1"),
                ("MATINEE_THEME", theme),
            ]
        } else {
            vec![
                ("WAYLAND_DISPLAY", DISPLAY),
                ("XDG_SESSION_TYPE", "wayland"),
                ("GDK_BACKEND", "wayland"),
                ("QT_QPA_PLATFORM", "wayland"),
                ("SDL_VIDEODRIVER", "wayland"),
                ("MOZ_ENABLE_WAYLAND", "1"),
                ("ELECTRON_OZONE_PLATFORM_HINT", "wayland"),
                // GTK's own settings come from the keyfile written at start.
                ("GSETTINGS_BACKEND", "keyfile"),
                // The session has no GPU, so OpenGL ends up in Mesa's
                // software renderer anyway; going straight there skips the
                // failed hardware probes and the warnings they print.
                ("LIBGL_ALWAYS_SOFTWARE", "1"),
                // The theme being recorded, so a matinee run inside follows it.
                ("MATINEE_THEME", theme),
            ]
        };
        env.extend(self.buses.env());
        let mut env: Vec<(String, String)> = env
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        if x11 {
            let display = self
                .x11_display
                .clone()
                .ok_or_else(|| io::Error::other("Xwayland is not running"))?;
            env.push(("DISPLAY".into(), display));
        }
        Ok(env)
    }

    /// Handle apps' requests for up to `timeout`.
    pub fn dispatch(&mut self, timeout: Duration) -> io::Result<()> {
        self.compositor.dispatch(timeout)
    }

    /// Draw the screen, telling apps that a frame was shown at `time`.
    pub fn render(&mut self, time: Duration) -> io::Result<Frame> {
        self.compositor.render(time)
    }

    /// Top-level windows, bottom to top.
    pub fn windows(&self) -> Vec<Window> {
        self.compositor.windows()
    }

    /// The area windows are laid out in: x, y, width, height in points.
    pub fn window_area(&self) -> (i32, i32, i32, i32) {
        self.compositor.window_area()
    }

    pub fn set_window_area(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.compositor.set_window_area(x, y, w, h);
    }

    /// Where the pointer is, in logical points.
    pub fn pointer_location(&self) -> (f64, f64) {
        self.compositor.pointer_location()
    }

    /// Move the pointer to `(x, y)` in logical points.
    pub fn pointer_move(&mut self, x: f64, y: f64, time: Duration) {
        self.compositor.pointer_move(x, y, time);
    }

    /// Press or release a pointer button. Pressing focuses and raises the
    /// window under the pointer.
    pub fn pointer_button(&mut self, button: Button, pressed: bool, time: Duration) {
        self.compositor.pointer_button(button, pressed, time);
    }

    /// Scroll by `dy` logical points; positive scrolls down.
    pub fn pointer_scroll(&mut self, dy: f64, time: Duration) {
        self.compositor.pointer_scroll(dy, time);
    }

    /// Press or release a key by its code (see [`crate::keys`]).
    pub fn key(&mut self, keycode: u32, pressed: bool, time: Duration) {
        self.compositor.key(keycode, pressed, time);
    }

    /// Fail if a bus or the registry has exited.
    pub fn check(&mut self) -> io::Result<()> {
        self.buses.check()
    }

    /// Connect to the accessibility trees apps publish.
    pub fn accessibility(&self) -> io::Result<A11y> {
        A11y::connect(&self.buses)
    }
}
