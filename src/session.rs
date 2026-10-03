//! A story's desktop session: the built-in compositor, the session and
//! accessibility buses, and the apps running under them, all inside the
//! isolation boundary.

use crate::{
    boundary::{Boundary, HOME},
    bus::Buses,
    compositor::{Compositor, Frame},
    portal,
    record::Recording,
    story::{PanePosition, Story, Theme},
    terminal::{Pane, Terminal},
};
use std::{
    io,
    path::{Path, PathBuf},
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
}

impl<'a> Session<'a> {
    /// Start the session for `story` in `theme`. Call this inside the
    /// boundary.
    pub fn start(boundary: &'a Boundary, story: &Story, theme: Theme) -> io::Result<Session<'a>> {
        let runtime = PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() }));
        let buses = Buses::start(boundary, &runtime, theme)?;
        let keyfile = Path::new(HOME).join(".config/glib-2.0/settings/keyfile");
        std::fs::create_dir_all(keyfile.parent().unwrap())?;
        std::fs::write(&keyfile, portal::Settings::new(theme).keyfile())?;
        let mut compositor = Compositor::new(
            &runtime.join(DISPLAY),
            (story.demo.size[0], story.demo.size[1]),
            story.demo.scale,
        )?;
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
            period: Duration::from_secs(1) / story.demo.fps,
            started: Instant::now(),
            frames: 0,
            size: story.demo.size,
            scale: story.demo.scale,
            recording: None,
            terminal,
            pointer_shown: false,
        })
    }

    /// Start a program that connects to the session's display, known to
    /// steps as `name`.
    /// A leading `~/` in `program` means the session's home folder.
    pub fn spawn(&mut self, name: &str, program: &str, args: &[String]) -> io::Result<()> {
        let program = match program.strip_prefix("~/") {
            Some(rest) => format!("{HOME}/{rest}"),
            None => program.to_owned(),
        };
        let program = program.as_str();
        let child = self
            .boundary
            .command(program)
            .args(args)
            .envs(app_env(&self.buses))
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
            if seed.network {
                return Err(io::Error::other(format!(
                    "seed {}: network access is not supported yet",
                    i + 1
                )));
            }
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
            self.spawn(&launch.name, &launch.app, &launch.args)?;
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
            let shown = !self.compositor.windows().is_empty();
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
        self.frames += 1;
        let mut frame = self.compositor.render(self.now())?;
        if let Some((terminal, pane)) = &self.terminal {
            pane.draw(terminal, &mut frame, self.scale);
        }
        if let Some(recording) = &mut self.recording {
            let pointer = self
                .pointer_shown
                .then(|| self.compositor.pointer_location());
            recording.add(&frame, pointer);
        }
        Ok(frame)
    }

    /// Draw the pointer in recordings from now on.
    pub fn show_pointer(&mut self) {
        self.pointer_shown = true;
    }

    /// Keep every frame from now on.
    pub fn start_recording(&mut self) {
        self.recording = Some(Recording::new(
            self.size[0] as usize,
            self.size[1] as usize,
            self.scale as usize,
        ));
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
