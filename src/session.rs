//! A story's desktop session: the built-in compositor, the session and
//! accessibility buses, and the apps running under them, all inside the
//! isolation boundary.

use crate::{
    boundary::{Boundary, HOME},
    bus::Buses,
    compositor::{Compositor, Frame},
    portal,
    record::Recording,
    story::{Story, Theme},
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
        let compositor = Compositor::new(
            &runtime.join(DISPLAY),
            (story.demo.size[0], story.demo.size[1]),
            story.demo.scale,
        )?;
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
        })
    }

    /// Start a program that connects to the session's display, known to
    /// steps as `name`.
    pub fn spawn(&mut self, name: &str, program: &str, args: &[String]) -> io::Result<()> {
        let child = self
            .boundary
            .command(program)
            .args(args)
            .env("WAYLAND_DISPLAY", DISPLAY)
            .env("XDG_SESSION_TYPE", "wayland")
            .env("GDK_BACKEND", "wayland")
            .env("QT_QPA_PLATFORM", "wayland")
            .env("SDL_VIDEODRIVER", "wayland")
            .env("MOZ_ENABLE_WAYLAND", "1")
            .env("ELECTRON_OZONE_PLATFORM_HINT", "wayland")
            .envs(self.buses.env())
            // GTK's own settings come from the keyfile written at start.
            .env("GSETTINGS_BACKEND", "keyfile")
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

    /// Start every app the storyboard launches.
    pub fn launch(&mut self, story: &Story) -> io::Result<()> {
        for launch in &story.launches {
            self.spawn(&launch.name, &launch.app, &launch.args)?;
        }
        Ok(())
    }

    /// Run the session in real time until a window is shown and the screen
    /// has stopped changing for `quiet`, then return the last frame.
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
                return Err(io::Error::other(if shown {
                    format!("the screen was still changing after {timeout:?}")
                } else {
                    format!("no window appeared within {timeout:?}")
                }));
            }
            last = Some(frame);
        }
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
        let frame = self.compositor.render(self.now())?;
        if let Some(recording) = &mut self.recording {
            recording.add(&frame, self.compositor.pointer_location());
        }
        Ok(frame)
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
