//! A story's desktop session: the display, the apps running on it, the
//! terminal pane, and the recording, all inside the isolation boundary.
//! What the display and the boundary are made of depends on the platform
//! (see [`crate::platform`]); the session, the runner, and everything
//! downstream of a frame do not.

use crate::{
    camera::{Camera, Rect},
    frame::Frame,
    platform::{A11y, Boundary, Button, Child, Display, Window},
    record::Recording,
    story::{CameraMode, CaptionPosition, HighlightStyle, Look, PanePosition, Story, Theme},
    terminal::{Pane, Terminal},
};
use std::{
    io,
    path::Path,
    time::{Duration, Instant},
};

pub struct Session<'a> {
    boundary: &'a Boundary,
    display: Display,
    /// Launched programs, by launch name.
    children: Vec<(String, Child)>,
    period: Duration,
    /// When the session started, for pacing frames in real time.
    started: Instant,
    /// Frames rendered so far; frame `n` is shown at `n * period`.
    frames: u32,
    size: [u32; 2],
    scale: u32,
    recording: Option<Recording>,
    /// The same frames with the camera held on the whole screen, when a
    /// fixed-camera recording was asked for too.
    fixed: Option<Recording>,
    /// The terminal pane, when the story has one.
    terminal: Option<(Terminal, Pane)>,
    /// Whether recordings draw the pointer: only once a step has used it.
    pointer_shown: bool,
    /// Windows already reported as too big for the window area.
    oversized: Vec<String>,
    /// How far recordings' lossy compression may move a color.
    lossy: u8,
    /// The theme being recorded, how overlays look in it, and whether
    /// captions need a band above the frame and below it.
    theme: Theme,
    look: Look,
    bands: (bool, bool),
    artifacts: bool,
    /// What recordings show of the screen.
    pub camera: Camera,
}

impl<'a> Session<'a> {
    /// Start the session for `story` in `theme`. Call this inside the
    /// boundary.
    pub fn start(boundary: &'a Boundary, story: &Story, theme: Theme) -> io::Result<Session<'a>> {
        let mut display = Display::start(boundary, story, theme)?;
        let [w, h] = story.demo.size.map(|n| n as f32);
        let terminal = if story.has_terminal() {
            let pane = &story.terminal;
            // A terminal-only story gets the whole screen unless its pane
            // was placed, leaving room for windows its commands open.
            let whole = story.launches.is_empty() && !pane.leaves_room;
            let rect = match (whole, pane.position) {
                (true, _) => (0.0, 0.0, w, h),
                (false, PanePosition::Bottom) => {
                    let size = pane.size.map_or((h * 0.35).round(), |n| n as f32);
                    display.set_window_area(0, 0, w as i32, (h - size) as i32);
                    (0.0, h - size, w, size)
                }
                (false, PanePosition::Right) => {
                    let size = pane.size.map_or((w * 0.45).round(), |n| n as f32);
                    display.set_window_area(0, 0, (w - size) as i32, h as i32);
                    (w - size, 0.0, size, h)
                }
            };
            let pane = Pane::new(rect, story.terminal.font_size, theme);
            let (rows, cols) = pane.grid();
            Some((
                Terminal::start(boundary, rows, cols, &display.env(false)?)?,
                pane,
            ))
        } else {
            None
        };
        Ok(Session {
            boundary,
            display,
            children: vec![],
            period: Duration::from_secs(1) / story.demo.fps,
            started: Instant::now(),
            frames: 0,
            size: story.demo.size,
            scale: story.demo.scale,
            recording: None,
            fixed: None,
            terminal,
            pointer_shown: false,
            oversized: vec![],
            lossy: story.demo.lossy,
            theme,
            look: story.style.look(theme).clone(),
            bands: story.caption_bands(),
            artifacts: false,
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
        let env = self.display.env(false)?;
        self.spawn_with(name, program, args, env)
    }

    /// Start a program under X11, known to steps as `name`.
    pub fn spawn_x11(&mut self, name: &str, program: &str, args: &[String]) -> io::Result<()> {
        let env = self.display.env(true)?;
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
        let child = self.boundary.spawn_app(&program, args, &env)?;
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
                .shell(&seed.run)
                .envs(self.display.env(false)?)
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
            let windows = self.display.windows();
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
        let (x, y, w, h) = self.display.window_area();
        for window in self.display.windows() {
            let fits = window.x >= x
                && window.y >= y
                && window.x + window.width <= x + w
                && window.y + window.height <= y + h;
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
                window.title, window.width, window.height,
            );
            self.oversized.push(window.title);
        }
    }

    /// Device pixels per point.
    pub fn scale(&self) -> u32 {
        self.scale
    }

    /// Top-level windows on the display, bottom to top.
    pub fn windows(&self) -> Vec<Window> {
        self.display.windows()
    }

    /// The area windows are laid out in: x, y, width, height in points.
    pub fn window_area(&self) -> (i32, i32, i32, i32) {
        self.display.window_area()
    }

    /// Move the pointer to `(x, y)` in logical points, at the current time
    /// on the timeline.
    pub fn pointer_move(&mut self, x: f64, y: f64) {
        let now = self.now();
        self.display.pointer_move(x, y, now);
    }

    /// Press or release a pointer button where the pointer is.
    pub fn pointer_button(&mut self, button: Button, pressed: bool) {
        let now = self.now();
        self.display.pointer_button(button, pressed, now);
    }

    /// Press or release a key by its code (see [`crate::keys`]).
    pub fn key(&mut self, keycode: u32, pressed: bool) {
        let now = self.now();
        self.display.key(keycode, pressed, now);
    }

    /// Connect to the accessibility trees apps publish.
    pub fn accessibility(&self) -> io::Result<A11y> {
        self.display.accessibility()
    }

    /// The time on the session's frame timeline.
    pub fn now(&self) -> Duration {
        self.period * self.frames
    }

    /// Handle apps' requests until the next frame is due, then render it.
    /// Frames are paced in real time, so apps have time to draw, while the
    /// time apps are told comes from the frame count.
    pub fn tick(&mut self) -> io::Result<Frame> {
        let mut due = self.started + self.period * (self.frames + 1);
        let now = Instant::now();
        if due < now {
            // Seeding, compilation, or a slow render can leave the clock
            // behind. Rebase its wall-time anchor instead of rushing input
            // through overdue frames before the app can process it. The
            // frame index, and therefore camera and pointer paths, stay put.
            self.started = now - self.period * self.frames;
            due = now + self.period;
        }
        loop {
            let left = due.saturating_duration_since(Instant::now());
            self.display.dispatch(left)?;
            if left.is_zero() {
                break;
            }
        }
        self.check_children()?;
        self.warn_oversized();
        self.frames += 1;
        let frame = self.current_frame()?;
        self.camera.step();
        let pointer = self.pointer_shown.then(|| self.display.pointer_location());
        crate::platform::viewport::publish(&frame, self.camera.view(), pointer, self.scale);
        let size = (self.size[0] as f64, self.size[1] as f64);
        if let Some(recording) = &mut self.recording {
            let pointer = pointer.map(|at| self.camera.to_output(at, size));
            recording.add(&frame, self.camera.view(), pointer);
        }
        if let Some(fixed) = &mut self.fixed {
            let whole = Rect::new(0.0, 0.0, size.0, size.1);
            let pointer = pointer.map(|(x, y)| (x * size.0 / whole.w, y * size.1 / whole.h));
            fixed.add(&frame, whole, pointer);
        }
        Ok(frame)
    }

    /// Draw the current session without advancing input, the camera, or time.
    pub fn current_frame(&mut self) -> io::Result<Frame> {
        let mut frame = self.display.render(self.now())?;
        if let Some((terminal, pane)) = &self.terminal {
            pane.draw(terminal, &mut frame, self.scale);
        }
        Ok(frame)
    }

    pub fn enable_artifacts(&mut self, enabled: bool) {
        self.artifacts = enabled;
    }

    /// Save a full-resolution still of an assertion or failure in this
    /// session's writable work area. The host collects it after the run.
    pub fn save_artifact(&mut self, name: &str) -> io::Result<()> {
        if self.artifacts {
            let directory = self.boundary.work().join("artifacts");
            std::fs::create_dir_all(&directory)?;
            self.current_frame()?.write_png(&directory.join(name))?;
        }
        Ok(())
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

    /// Keep every frame from now on, and with `fixed_too`, keep the same
    /// frames with the camera held on the whole screen as a second
    /// recording.
    pub fn start_recording(&mut self, fixed_too: bool) {
        let (size, scale, pane, lossy) = (self.size, self.scale, self.pane_rect(), self.lossy);
        let (theme, (above, below)) = (self.theme, self.bands);
        let look = &self.look;
        let new = || {
            let (w, h) = (size[0] as usize, size[1] as usize);
            let mut recording = Recording::new(w, h, scale as usize, theme, look);
            recording.set_pane(pane);
            recording.set_lossy(lossy);
            if above || below {
                recording.set_bands(above, below);
            }
            recording
        };
        self.recording = Some(new());
        self.fixed = fixed_too.then(new);
    }

    /// The recordings being kept: the camera's, and the fixed one.
    fn recordings(&mut self) -> impl Iterator<Item = &mut Recording> {
        self.recording.iter_mut().chain(self.fixed.iter_mut())
    }

    /// Start a new part of the recording, for size reporting.
    pub fn mark_segment(&mut self, label: &str) {
        self.recordings().for_each(|r| r.segment(label));
    }

    /// Caption what is recorded from now on at `position`, or stop with
    /// `None`.
    pub fn caption(&mut self, text: Option<&str>, position: CaptionPosition) {
        self.recordings().for_each(|r| r.caption(text, position));
    }

    /// Mark `rect` on the screen, in points, in what is recorded from now
    /// on.
    pub fn highlight(
        &mut self,
        rect: Rect,
        style: HighlightStyle,
        label: Option<&str>,
        number: Option<u32>,
        color: Option<u32>,
    ) {
        self.recordings()
            .for_each(|r| r.highlight(rect, style, label, number, color));
    }

    /// Take down every highlight in what is recorded from now on.
    pub fn clear_highlights(&mut self) {
        self.recordings().for_each(|r| r.clear_highlights());
    }

    /// Keep what was just recorded as a still picture.
    pub fn mark_still(&mut self) {
        self.recordings().for_each(|r| r.still());
    }

    /// Where the latest `text` in the terminal pane is on the screen, in
    /// points.
    pub fn terminal_text(&self, text: &str) -> Option<Rect> {
        let (terminal, pane) = self.terminal.as_ref()?;
        let (row, col, len) = terminal.find(text)?;
        Some(pane.cells(row, col, 1, len))
    }

    /// Where the lines of the terminal pane from the latest holding `from`
    /// through the next holding `to` are on the screen, in points, as wide
    /// as the pane.
    pub fn terminal_lines(&self, from: &str, to: &str) -> Option<Rect> {
        let (terminal, pane) = self.terminal.as_ref()?;
        let (first, last) = terminal.find_lines(from, to)?;
        let (_, cols) = pane.grid();
        Some(pane.cells(first, 0, last - first + 1, cols))
    }

    /// Mark a click in the recording, where the pointer is.
    pub fn mark_click(&mut self) {
        self.recordings().for_each(|r| r.click());
    }

    /// Mark a key chord in the recording, to show on a badge.
    pub fn mark_key(&mut self, chord: &str) {
        self.recordings().for_each(|r| r.key(chord));
    }

    /// Stop keeping frames and return what was kept: the recording, and
    /// the fixed-camera one if it was asked for.
    pub fn stop_recording(&mut self) -> Option<(Recording, Option<Recording>)> {
        let recording = self.recording.take()?;
        Some((recording, self.fixed.take()))
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
        self.display.check()?;
        for (name, child) in &mut self.children {
            if let Some(status) = child.exited()? {
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
            child.kill();
        }
    }
}
