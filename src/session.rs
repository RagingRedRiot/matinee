//! A story's desktop session: the built-in compositor, the session and
//! accessibility buses, and the apps running under them, all inside the
//! isolation boundary.

use crate::{
    boundary::Boundary,
    bus::Buses,
    camera::{Camera, Rect},
    compositor::{Compositor, Frame},
    portal,
    record::Recording,
    story::{CameraMode, PanePosition, Story, Theme},
    terminal::{Pane, Terminal},
};
use std::{
    io,
    path::Path,
    process::Child,
    time::{Duration, Instant},
};

/// The compositor's socket name in the session's runtime directory.
const DISPLAY: &str = "wayland-1";

pub struct Session<'a> {
    boundary: &'a Boundary,
    pub compositor: Compositor,
    pub buses: Buses,
    /// Launched programs, by launch name.
    children: Vec<(String, Child)>,
    /// Xwayland's display, for `DISPLAY`, when a launch needs X11.
    x11_display: Option<String>,
    period: Duration,
    /// When the session started, for pacing frames in real time.
    started: Instant,
    /// Frames rendered so far; frame `n` is shown at `n * period`.
    frames: u32,
    size: [u32; 2],
    scale: u32,
    recording: Option<Recording>,
    /// The terminal pane, when the story has one.
    terminal: Option<(Terminal, Pane)>,
    /// Whether recordings draw the pointer: only once a step has used it.
    pointer_shown: bool,
    /// Windows already reported as too big for the window area.
    oversized: Vec<String>,
    /// How far recordings' lossy compression may move a color.
    lossy: u8,
    /// What recordings show of the screen.
    pub camera: Camera,
}

impl<'a> Session<'a> {
    /// Start the session for `story` in `theme`. Call this inside the
    /// boundary.
    pub fn start(boundary: &'a Boundary, story: &Story, theme: Theme) -> io::Result<Session<'a>> {
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
        let [w, h] = story.demo.size.map(|n| n as f32);
        let terminal = if story.has_terminal() {
            let pane = &story.terminal;
            let rect = match (story.launches.is_empty(), pane.position) {
                (true, _) => (0.0, 0.0, w, h),
                (false, PanePosition::Bottom) => {
                    let size = pane.size.map_or((h * 0.35).round(), |n| n as f32);
                    compositor.set_window_area(0, 0, w as i32, (h - size) as i32);
                    (0.0, h - size, w, size)
                }
                (false, PanePosition::Right) => {
                    let size = pane.size.map_or((w * 0.45).round(), |n| n as f32);
                    compositor.set_window_area(0, 0, (w - size) as i32, h as i32);
                    (w - size, 0.0, size, h)
                }
            };
            let pane = Pane::new(rect, story.terminal.font_size, theme);
            let (rows, cols) = pane.grid();
            Some((
                Terminal::start(boundary, rows, cols, &app_env(&buses))?,
                pane,
            ))
        } else {
            None
        };
        Ok(Session {
            boundary,
            compositor,
            buses,
            children: vec![],
            x11_display,
            period: Duration::from_secs(1) / story.demo.fps,
            started: Instant::now(),
            frames: 0,
            size: story.demo.size,
            scale: story.demo.scale,
            recording: None,
            terminal,
            pointer_shown: false,
            oversized: vec![],
            lossy: story.demo.lossy,
            camera: Camera::new(
                w as f64,
                h as f64,
                story.camera.max_zoom.min(story.demo.scale as f64),
                (story.camera.ease.as_secs_f64() * story.demo.fps as f64).round() as u32,
                story.camera.mode == CameraMode::Fixed,
                story.camera.reading_width,
            ),
        })
    }

    /// Start a program that connects to the session's display, known to
    /// steps as `name`.
    /// A leading `~/` in `program` means the session's home folder.
    pub fn spawn(&mut self, name: &str, program: &str, args: &[String]) -> io::Result<()> {
        let env = owned(app_env(&self.buses));
        self.spawn_with(name, program, args, env)
    }

    /// Start a program under Xwayland, known to steps as `name`.
    pub fn spawn_x11(&mut self, name: &str, program: &str, args: &[String]) -> io::Result<()> {
        let display = self
            .x11_display
            .clone()
            .ok_or_else(|| io::Error::other("Xwayland is not running"))?;
        let mut env = owned(x11_env(&self.buses));
        env.push(("DISPLAY".into(), display));
        self.spawn_with(name, program, args, env)
    }

    fn spawn_with(
        &mut self,
        name: &str,
        program: &str,
        args: &[String],
        env: Vec<(String, String)>,
    ) -> io::Result<()> {
        let program = match program.strip_prefix("~/") {
            Some(rest) => format!("{}/{rest}", self.boundary.home().display()),
            None => program.to_owned(),
        };
        let program = program.as_str();
        let child = self
            .boundary
            .command(program)
            .args(args)
            .envs(env)
            .spawn()
            .map_err(|e| io::Error::new(e.kind(), format!("{program}: {e}")))?;
        self.children.push((name.to_owned(), child));
        Ok(())
    }

    /// The process ID of the program launched as `name`.
    pub fn pid(&self, name: &str) -> Option<u32> {
        self.children
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, c)| c.id())
    }

    /// Launch names of the running programs.
    pub fn launched(&self) -> impl Iterator<Item = &str> {
        self.children.iter().map(|(n, _)| n.as_str())
    }

    /// Run the storyboard's seed commands, in order, before any app starts.
    pub fn seed(&mut self, story: &Story) -> io::Result<()> {
        for (i, seed) in story.seeds.iter().enumerate() {
            let mut child = self
                .boundary
                .command("bash")
                .args(["-c", &seed.run])
                .envs(app_env(&self.buses))
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()?;
            let timeout = seed.timeout.unwrap_or(Duration::from_secs(60));
            let start = Instant::now();
            let status = loop {
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    return Err(io::Error::other(format!(
                        "seed {}: `{}` did not finish within {timeout:?}",
                        i + 1,
                        seed.run
                    )));
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            if !status.success() {
                let mut stderr = String::new();
                if let Some(mut e) = child.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut e, &mut stderr);
                }
                return Err(io::Error::other(format!(
                    "seed {}: `{}` failed ({status}){}",
                    i + 1,
                    seed.run,
                    if stderr.trim().is_empty() {
                        String::new()
                    } else {
                        format!(":\n{}", stderr.trim_end())
                    }
                )));
            }
        }
        Ok(())
    }

    /// The storyboard's folder as seen inside the session.
    pub fn story_dir(&self) -> &Path {
        self.boundary.story_dir()
    }

    /// The terminal, when the story has one.
    pub fn terminal(&mut self) -> Option<&mut Terminal> {
        self.terminal.as_mut().map(|(t, _)| t)
    }

    /// Start every app the storyboard launches.
    pub fn launch(&mut self, story: &Story) -> io::Result<()> {
        for launch in &story.launches {
            if launch.x11 {
                self.spawn_x11(&launch.name, &launch.app, &launch.args)?;
            } else {
                self.spawn(&launch.name, &launch.app, &launch.args)?;
            }
        }
        Ok(())
    }

    /// Run the session in real time until a window is shown and the screen
    /// has stopped changing for `quiet`, then return the last frame. If a
    /// window is shown but the screen never stops changing, the frame at
    /// `timeout` is returned instead.
    pub fn settle(&mut self, quiet: Duration, timeout: Duration) -> io::Result<Frame> {
        let start = Instant::now();
        let mut last: Option<Frame> = None;
        let mut stable_since = Instant::now();
        loop {
            let frame = self.tick()?;
            // A window counts once it has taken its final size: before
            // that, a slow app can look settled at its first size.
            let windows = self.compositor.windows();
            let shown = !windows.is_empty() && windows.iter().all(|w| w.placed);
            if !shown || last.as_ref().is_none_or(|l| l.pixels != frame.pixels) {
                stable_since = Instant::now();
            }
            if shown && stable_since.elapsed() >= quiet {
                return Ok(frame);
            }
            if start.elapsed() > timeout {
                if shown {
                    // Something keeps changing, such as a clock; the
                    // latest frame is as settled as the screen gets.
                    eprintln!(
                        "matinee: the screen was still changing after {timeout:?}; using the latest frame"
                    );
                    return Ok(frame);
                }
                return Err(io::Error::other(format!(
                    "no window appeared within {timeout:?}"
                )));
            }
            last = Some(frame);
        }
    }

    /// Warn, once per window, about a window that does not fit the window
    /// area: an app with a larger minimum size keeps it, and the rest is
    /// hidden behind the terminal pane or past the screen's edge.
    fn warn_oversized(&mut self) {
        let (x, y, w, h) = self.compositor.window_area();
        for window in self.compositor.windows() {
            let r = window.rect;
            let fits = r.loc.x >= x
                && r.loc.y >= y
                && r.loc.x + r.size.w <= x + w
                && r.loc.y + r.size.h <= y + h;
            if !window.placed || fits || self.oversized.contains(&window.title) {
                continue;
            }
            let hidden = if self.terminal.is_some() {
                "behind the terminal pane; make the story's size larger or the pane smaller"
            } else {
                "past the screen's edge; make the story's size larger"
            };
            eprintln!(
                "matinee: warning: window \"{}\" is {}x{} points, larger than its {w}x{h} area, so part of it is hidden {hidden}",
                window.title, window.rect.size.w, window.rect.size.h,
            );
            self.oversized.push(window.title);
        }
    }

    /// Device pixels per point.
    pub fn scale(&self) -> u32 {
        self.scale
    }

    /// The time on the session's frame timeline.
    pub fn now(&self) -> Duration {
        self.period * self.frames
    }

    /// Handle apps' requests until the next frame is due, then render it.
    /// Frames are paced in real time, so apps have time to draw, while the
    /// time apps are told comes from the frame count.
    pub fn tick(&mut self) -> io::Result<Frame> {
        let due = self.started + self.period * (self.frames + 1);
        loop {
            let left = due.saturating_duration_since(Instant::now());
            self.compositor.dispatch(left)?;
            if left.is_zero() {
                break;
            }
        }
        self.check_children()?;
        self.warn_oversized();
        self.frames += 1;
        let mut frame = self.compositor.render(self.now())?;
        if let Some((terminal, pane)) = &self.terminal {
            pane.draw(terminal, &mut frame, self.scale);
        }
        self.camera.step();
        if let Some(recording) = &mut self.recording {
            let size = (self.size[0] as f64, self.size[1] as f64);
            let pointer = self.pointer_shown.then(|| {
                self.camera
                    .to_output(self.compositor.pointer_location(), size)
            });
            recording.add(&frame, self.camera.view(), pointer);
        }
        Ok(frame)
    }

    /// The terminal pane's rectangle, when the story has one.
    pub fn pane_rect(&self) -> Option<Rect> {
        self.terminal.as_ref().map(|(_, pane)| {
            let (x, y, w, h) = pane.rect;
            Rect::new(x as f64, y as f64, w as f64, h as f64)
        })
    }

    /// Draw the pointer in recordings from now on.
    pub fn show_pointer(&mut self) {
        self.pointer_shown = true;
    }

    /// Keep every frame from now on.
    pub fn start_recording(&mut self) {
        let mut recording = Recording::new(
            self.size[0] as usize,
            self.size[1] as usize,
            self.scale as usize,
        );
        recording.set_pane(self.pane_rect());
        recording.set_lossy(self.lossy);
        self.recording = Some(recording);
    }

    /// Start a new part of the recording, for size reporting.
    pub fn mark_segment(&mut self, label: &str) {
        if let Some(recording) = &mut self.recording {
            recording.segment(label);
        }
    }

    /// Caption what is recorded from now on, or stop with `None`.
    pub fn caption(&mut self, text: Option<&str>) {
        if let Some(recording) = &mut self.recording {
            recording.caption(text);
        }
    }

    /// Mark a click in the recording, where the pointer is.
    pub fn mark_click(&mut self) {
        if let Some(recording) = &mut self.recording {
            recording.click();
        }
    }

    /// Mark a key chord in the recording, to show on a badge.
    pub fn mark_key(&mut self, chord: &str) {
        if let Some(recording) = &mut self.recording {
            recording.key(chord);
        }
    }

    /// Stop keeping frames and return what was kept.
    pub fn stop_recording(&mut self) -> Option<Recording> {
        self.recording.take()
    }

    /// Run frames for `duration` of the timeline.
    pub fn run_for(&mut self, duration: Duration) -> io::Result<()> {
        let end = self.now() + duration;
        while self.now() < end {
            self.tick()?;
        }
        Ok(())
    }

    /// Fail if a launched program, a bus, or the registry has exited.
    fn check_children(&mut self) -> io::Result<()> {
        self.buses.check()?;
        for (name, child) in &mut self.children {
            if let Some(status) = child.try_wait()? {
                return Err(io::Error::other(format!("{name} exited early ({status})")));
            }
        }
        Ok(())
    }
}

impl Drop for Session<'_> {
    /// Stop launched apps before the display and buses they use go away,
    /// so they do not report a lost connection on the way out.
    fn drop(&mut self) {
        for (_, child) in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Environment for programs in the session: the display, the buses, and
/// the settings that make toolkits use them.
fn app_env(buses: &Buses) -> Vec<(&str, &str)> {
    let mut env = vec![
        ("WAYLAND_DISPLAY", DISPLAY),
        ("XDG_SESSION_TYPE", "wayland"),
        ("GDK_BACKEND", "wayland"),
        ("QT_QPA_PLATFORM", "wayland"),
        ("SDL_VIDEODRIVER", "wayland"),
        ("MOZ_ENABLE_WAYLAND", "1"),
        ("ELECTRON_OZONE_PLATFORM_HINT", "wayland"),
        // GTK's own settings come from the keyfile written at start.
        ("GSETTINGS_BACKEND", "keyfile"),
    ];
    env.extend(buses.env());
    env
}

fn owned(env: Vec<(&str, &str)>) -> Vec<(String, String)> {
    env.into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

/// Environment for X11 apps: the buses and settings, without the Wayland
/// display, so toolkits that support both use X11. `DISPLAY` is added by
/// the caller.
fn x11_env(buses: &Buses) -> Vec<(&str, &str)> {
    let mut env = vec![
        ("XDG_SESSION_TYPE", "x11"),
        ("GDK_BACKEND", "x11"),
        ("QT_QPA_PLATFORM", "xcb"),
        ("SDL_VIDEODRIVER", "x11"),
        ("GSETTINGS_BACKEND", "keyfile"),
    ];
    env.extend(buses.env());
    env
}
