//! Performing a story's steps against a running session.
//!
//! Targets are found through the accessibility tree: an element showing on
//! screen whose name matches exactly (or by prefix, for a name ending in
//! `*`), narrowed by role, a named ancestor
//! (`within`), its window, and its app (`in`). An element nested inside
//! another match, such as a button's own label, counts as the same target.
//! Anything else that leaves zero or several matches fails the step, with a
//! diagnostic listing what is there instead. A target outside its window's
//! visible area is refused rather than clicked where it is not.
//!
//! `wait_for` and `expect` also accept a text field whose contents are
//! exactly the expected text, and, with no context besides the name, that
//! text showing in the terminal pane. Steps that act, such as clicks,
//! match names only.
//!
//! A `run` step types its command line into the terminal pane, runs the
//! command, and fails unless it exits with the expected status. A staged
//! step types the command it shows and runs the one it names. A `run` with
//! `wait = false` leaves its command running, for `type` and `key` steps to
//! send it keys, and checks its status at `wait_exit` or the story's end.

use crate::{
    camera::Rect as CameraRect,
    keys,
    platform::{A11y, App, Button, Node, in_points},
    session::Session,
    story::{Action, CameraMove, Mark, Spot, Step, Story, Target},
};
use std::{
    collections::HashSet,
    fmt, io,
    time::{Duration, Instant},
};

/// How long a target may take to appear before a step fails.
const FIND_TIMEOUT: Duration = Duration::from_secs(5);
/// How long `wait_for` waits by default.
const WAIT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long `expect` gives the app to catch up.
const EXPECT_TIMEOUT: Duration = Duration::from_secs(3);
/// How long the pointer takes to reach a target.
const TRAVEL: Duration = Duration::from_millis(500);
/// How long a click holds the button down.
const CLICK: Duration = Duration::from_millis(80);
/// How long a highlight shows before its step's hold, so it has faded in.
const MARK_IN: Duration = Duration::from_millis(300);
/// How long a drag takes from press to drop.
const DRAG: Duration = Duration::from_millis(900);
/// The pause on the drop target before letting go, so the app sees it.
const DROP_HOVER: Duration = Duration::from_millis(150);
/// The time between typed characters.
const PER_KEY: Duration = Duration::from_millis(70);
/// How long an app may take to show its first window.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a terminal command may run by default.
const RUN_TIMEOUT: Duration = Duration::from_secs(30);
/// The pause between typing a command line and pressing Enter.
const BEFORE_ENTER: Duration = Duration::from_millis(250);

/// Why a story could not be performed.
#[derive(Debug)]
pub struct Failure {
    /// Where it happened, such as "scene 2, step 1".
    pub at: String,
    pub message: String,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.at, self.message)
    }
}

impl From<io::Error> for Failure {
    fn from(e: io::Error) -> Failure {
        Failure {
            at: "session".into(),
            message: e.to_string(),
        }
    }
}

/// A target found on screen.
#[derive(Debug, Clone)]
struct Found {
    /// The middle of the target, in logical points on the output.
    point: (f64, f64),
    /// Its rectangle on the output: x, y, width, height.
    rect: (f64, f64, f64, f64),
    /// A slider's current, minimum, and maximum value.
    value: Option<(f64, f64, f64)>,
}

pub struct Runner<'r, 'b> {
    session: &'r mut Session<'b>,
    a11y: A11y,
    /// Device pixels per point, for rectangles the platform reports in
    /// pixels.
    scale: f64,
    pointer: (f64, f64),
    /// A command left running in the terminal.
    running: Option<Running>,
    step_at: (usize, usize),
}

/// A command a `run` step started without waiting for it.
struct Running {
    command: String,
    exit: i32,
    /// Its status, once it has been seen to finish.
    status: Option<i32>,
}

impl<'r, 'b> Runner<'r, 'b> {
    pub fn new(session: &'r mut Session<'b>, story: &Story) -> io::Result<Runner<'r, 'b>> {
        let a11y = session.accessibility()?;
        let [w, h] = story.demo.size;
        Ok(Runner {
            session,
            a11y,
            scale: story.demo.scale as f64,
            pointer: (w as f64 / 2.0, h as f64 / 2.0),
            running: None,
            step_at: (0, 0),
        })
    }

    /// Launch the story's apps and wait until each is ready: its `wait_for`
    /// target is showing, or else it has shown a window.
    pub fn launch(&mut self, story: &Story) -> Result<(), Failure> {
        self.session.seed(story).map_err(|e| Failure {
            at: "seed".into(),
            message: e.to_string(),
        })?;
        self.session.launch(story)?;
        for launch in &story.launches {
            let at = format!("launch {}", launch.name);
            let timeout = launch.timeout.unwrap_or(LAUNCH_TIMEOUT);
            match &launch.wait_for {
                Some(name) => {
                    let target = Target {
                        name: name.clone(),
                        app: Some(launch.name.clone()),
                        ..Target::default()
                    };
                    self.wait_until(&target, timeout, false)
                        .map_err(|message| Failure {
                            at: at.clone(),
                            message,
                        })?;
                }
                None => {
                    let start = Instant::now();
                    while self.session.windows().is_empty() {
                        if start.elapsed() > timeout {
                            return Err(Failure {
                                at,
                                message: format!("no window appeared within {timeout:?}"),
                            });
                        }
                        self.session.tick()?;
                    }
                }
            }
        }
        // Windows first draw at their own size and are then resized to
        // their place; steps start once every window has settled there.
        let start = Instant::now();
        while !self.session.windows().iter().all(|w| w.placed) {
            if start.elapsed() > LAUNCH_TIMEOUT {
                return Err(Failure {
                    at: "launch".into(),
                    message: "a window never took its place on screen".into(),
                });
            }
            self.session.tick()?;
        }
        // One more frame, drawn at the final size.
        self.session.tick()?;
        Ok(())
    }

    /// Perform every scene in order.
    pub fn perform(&mut self, story: &Story) -> Result<usize, Failure> {
        let mut done = 0;
        for (si, scene) in story.scenes.iter().enumerate() {
            let label = match &scene.caption {
                Some(caption) => format!("scene {}: {caption}", si + 1),
                None => format!("scene {}", si + 1),
            };
            self.session.mark_segment(&label);
            let position = story.caption_position(scene);
            self.session.caption(scene.caption.as_deref(), position);
            for (ti, step) in scene.steps.iter().enumerate() {
                self.step_at = (si + 1, ti + 1);
                self.step(step).map_err(|message| Failure {
                    at: format!("scene {}, step {}", si + 1, ti + 1),
                    message,
                })?;
                done += 1;
            }
            self.session.caption(None, position);
            self.session.clear_highlights();
        }
        if self.running.is_some() {
            self.wait_exit(RUN_TIMEOUT).map_err(|message| Failure {
                at: "end of story".into(),
                message,
            })?;
        }
        Ok(done)
    }

    fn step(&mut self, step: &Step) -> Result<(), String> {
        let io = |e: io::Error| e.to_string();
        match &step.action {
            Action::WaitFor(target) | Action::Expect(target) => {
                let timeout = if matches!(&step.action, Action::WaitFor(_)) {
                    WAIT_TIMEOUT
                } else {
                    EXPECT_TIMEOUT
                };
                self.wait_until(target, step.timeout.unwrap_or(timeout), true)?;
                self.session
                    .save_artifact(&format!(
                        "scene-{}-step-{}.png",
                        self.step_at.0, self.step_at.1,
                    ))
                    .map_err(io)?;
            }
            Action::Hover(target) => {
                let found = self.wait_until(target, step.timeout.unwrap_or(FIND_TIMEOUT), false)?;
                self.frame(rect(found.rect), None).map_err(io)?;
                self.move_to(found.point, TRAVEL).map_err(io)?;
            }
            Action::Click(target) => {
                let found = self.wait_until(target, step.timeout.unwrap_or(FIND_TIMEOUT), false)?;
                self.frame(rect(found.rect), None).map_err(io)?;
                self.move_to(found.point, TRAVEL).map_err(io)?;
                self.click().map_err(io)?;
            }
            Action::HoverAt(spot) => {
                let point = self.spot(spot)?;
                self.frame(around(point), None).map_err(io)?;
                self.move_to(point, TRAVEL).map_err(io)?;
            }
            Action::ClickAt(spot) => {
                let point = self.spot(spot)?;
                self.frame(around(point), None).map_err(io)?;
                self.move_to(point, TRAVEL).map_err(io)?;
                self.click().map_err(io)?;
            }
            Action::DragAt { from, to } => {
                let (start, end) = (self.spot(from)?, self.spot(to)?);
                self.frame(around(start).union(around(end)), None)
                    .map_err(io)?;
                self.move_to(start, TRAVEL).map_err(io)?;
                self.drag(end).map_err(io)?;
            }
            Action::Type {
                text,
                terminal: true,
            } => {
                self.terminal_input()?;
                let mut buf = [0; 4];
                for c in text.chars() {
                    let bytes = if c == '\n' {
                        "\r"
                    } else {
                        c.encode_utf8(&mut buf)
                    };
                    self.terminal()?.send(bytes.as_bytes()).map_err(io)?;
                    self.session.run_for(PER_KEY).map_err(io)?;
                }
            }
            Action::Key {
                chord,
                terminal: true,
            } => {
                self.terminal_input()?;
                let app_cursor = self.terminal()?.application_cursor();
                let bytes = keys::terminal_bytes(chord, app_cursor)?;
                self.session.mark_key(chord);
                self.terminal()?.send(&bytes).map_err(io)?;
                self.session.run_for(PER_KEY).map_err(io)?;
            }
            Action::Type { text, .. } => {
                self.need_window("type")?;
                for c in text.chars() {
                    let press = keys::for_char(c)
                        .ok_or_else(|| format!("`{c}` cannot be typed on the keyboard"))?;
                    self.press(&press).map_err(io)?;
                    self.session.run_for(PER_KEY).map_err(io)?;
                }
            }
            Action::Key { chord, .. } => {
                self.need_window("key")?;
                let press = keys::chord(chord)?;
                self.session.mark_key(chord);
                self.press(&press).map_err(io)?;
                self.session.run_for(PER_KEY).map_err(io)?;
            }
            Action::Pause => {}
            Action::Highlight(highlight) => {
                let timeout = step.timeout.unwrap_or(FIND_TIMEOUT);
                let at = self.mark_rect(&highlight.mark, timeout)?;
                self.frame(at, None).map_err(io)?;
                self.session.highlight(
                    at,
                    highlight.style,
                    highlight.label.as_deref(),
                    highlight.number,
                    highlight.color,
                );
                self.session.run_for(MARK_IN).map_err(io)?;
            }
            Action::ClearHighlights => self.session.clear_highlights(),
            Action::Camera(CameraMove::Overview) => {
                let moved = self.session.camera.overview();
                self.settle_camera(moved).map_err(io)?;
            }
            Action::Camera(CameraMove::Focus { target, zoom }) => {
                let found = self.wait_until(target, step.timeout.unwrap_or(FIND_TIMEOUT), false)?;
                self.frame(rect(found.rect), *zoom).map_err(io)?;
            }
            Action::Camera(CameraMove::FocusAt { spot, zoom }) => {
                let point = self.spot(spot)?;
                self.frame(around(point), *zoom).map_err(io)?;
            }
            Action::Run {
                command,
                shown,
                exit,
                wait,
            } => {
                if let Some(running) = &self.running {
                    return Err(format!("`{}` is still running", running.command));
                }
                // What a command causes is usually elsewhere on screen.
                let moved = self.session.camera.overview();
                self.settle_camera(moved).map_err(io)?;
                self.type_command(shown.as_deref().unwrap_or(command))?;
                let terminal = self.terminal()?;
                terminal.run(command).map_err(io)?;
                self.running = Some(Running {
                    command: command.clone(),
                    exit: *exit,
                    status: None,
                });
                if *wait {
                    self.wait_exit(step.timeout.unwrap_or(RUN_TIMEOUT))?;
                } else {
                    self.session.tick().map_err(io)?;
                }
            }
            Action::WaitExit(_) => {
                if self.running.is_none() {
                    return Err("no command is running".into());
                }
                self.wait_exit(step.timeout.unwrap_or(RUN_TIMEOUT))?;
            }
            Action::Playback { shown, output } => {
                let text = std::fs::read_to_string(self.session.story_dir().join(output))
                    .map_err(|e| format!("{}: {e}", output.display()))?;
                let moved = self.session.camera.overview();
                self.settle_camera(moved).map_err(io)?;
                self.type_command(shown)?;
                let terminal = self.terminal()?;
                terminal.show(&text.replace('\n', "\r\n"));
                terminal.prompt();
                self.session.tick().map_err(io)?;
            }
            Action::Drag { from, to } => {
                let timeout = step.timeout.unwrap_or(FIND_TIMEOUT);
                let source = self.wait_until(from, timeout, false)?;
                let target = self.wait_until(to, timeout, false)?;
                self.frame(rect(source.rect).union(rect(target.rect)), None)
                    .map_err(io)?;
                self.move_to(source.point, TRAVEL).map_err(io)?;
                let target = self.wait_until(to, timeout, false)?;
                self.drag(target.point).map_err(io)?;
            }
            Action::DragSlider { target, to } => {
                let found = self.wait_until(target, step.timeout.unwrap_or(FIND_TIMEOUT), false)?;
                self.frame(rect(found.rect), None).map_err(io)?;
                let (current, min, max) = found.value.ok_or_else(|| {
                    format!("\"{}\" is not a slider that reports its value", target.name)
                })?;
                if max <= min {
                    return Err(format!("\"{}\" has no range to drag along", target.name));
                }
                // The thumb sits at the current value's fraction of the
                // slider's length; the drag ends at the requested one.
                let (x, y, w, h) = found.rect;
                let at = |fraction: f64| {
                    if w >= h {
                        (x + w * fraction, y + h / 2.0)
                    } else {
                        (x + w / 2.0, y + h * (1.0 - fraction))
                    }
                };
                self.move_to(at((current - min) / (max - min)), TRAVEL)
                    .map_err(io)?;
                self.drag(at(*to)).map_err(io)?;
            }
        }
        if let Some(hold) = step.hold {
            self.session.run_for(hold).map_err(io)?;
        }
        if matches!(step.action, Action::Highlight(_)) {
            self.session.mark_still();
        }
        Ok(())
    }

    /// Where a highlight's mark is on the screen, in points, waiting up to
    /// `timeout` for it to show.
    fn mark_rect(&mut self, mark: &Mark, timeout: Duration) -> Result<CameraRect, String> {
        match mark {
            Mark::Area {
                rect: [x, y, w, h],
                window,
            } => {
                let spot = Spot {
                    x: *x,
                    y: *y,
                    window: window.clone(),
                };
                let (px, py) = self.spot(&spot)?;
                Ok(CameraRect::new(px, py, *w, *h))
            }
            Mark::Target(target)
                if target.role.is_none()
                    && target.within.is_none()
                    && target.near.is_none()
                    && target.window.is_none()
                    && target.app.is_none() =>
            {
                // Text in the terminal or an app's text, or else a control
                // with that name.
                let found = self.poll(timeout, |r| {
                    r.text_rect(&target.name)
                        .or_else(|| r.find(target, false).ok().map(|f| rect(f.rect)))
                })?;
                found.ok_or_else(|| {
                    format!(
                        "nothing showing matches \"{}\": no text in the terminal or an app, \
                         and no control with that name",
                        target.name
                    )
                })
            }
            Mark::Target(target) => {
                let found = self.wait_until(target, timeout, false)?;
                Ok(rect(found.rect))
            }
            Mark::Lines { from, to } => {
                let found = self.poll(timeout, |r| r.lines_rect(from, to))?;
                found.ok_or_else(|| {
                    format!(
                        "no lines run from one holding \"{from}\" to one holding \"{to}\" \
                         in the terminal or an app's text"
                    )
                })
            }
        }
    }

    /// Where the latest `text` shows: in the terminal pane, or in an app's
    /// text.
    fn text_rect(&self, text: &str) -> Option<CameraRect> {
        self.session.terminal_text(text).or_else(|| {
            let ((x, y, w, h), _) = self.a11y.text_bounds(text)?;
            Some(self.in_points(x, y, w, h))
        })
    }

    /// Where the lines from the latest holding `from` through one holding
    /// `to` show, as wide as the terminal pane or the text holding them.
    fn lines_rect(&self, from: &str, to: &str) -> Option<CameraRect> {
        self.session.terminal_lines(from, to).or_else(|| {
            let ((_, top, _, _), (hx, _, hw, _)) = self.a11y.text_bounds(from)?;
            let ((_, ty, _, th), _) = self.a11y.text_bounds(to)?;
            (ty >= top).then(|| self.in_points(hx, top, hw, ty + th - top))
        })
    }

    /// A rectangle in device pixels on the screen, in points.
    fn in_points(&self, x: f64, y: f64, w: f64, h: f64) -> CameraRect {
        let s = self.scale;
        CameraRect::new(x / s, y / s, w / s, h / s)
    }

    /// `found` once it gives something, letting the apps draw between
    /// tries, or `None` after `timeout`.
    fn poll<T>(
        &mut self,
        timeout: Duration,
        found: impl Fn(&Self) -> Option<T>,
    ) -> Result<Option<T>, String> {
        let start = Instant::now();
        loop {
            if let Some(value) = found(self) {
                return Ok(Some(value));
            }
            if start.elapsed() >= timeout {
                return Ok(None);
            }
            for _ in 0..3 {
                self.session.tick().map_err(|e| e.to_string())?;
            }
        }
    }

    /// Where a window-relative point is on the output. The window is the
    /// one with the spot's exact title, or the only one open.
    fn spot(&self, spot: &crate::story::Spot) -> Result<(f64, f64), String> {
        let windows = self.session.windows();
        let titles = || {
            windows
                .iter()
                .map(|w| format!("\"{}\"", w.title))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let window = match &spot.window {
            Some(title) => windows
                .iter()
                .find(|w| &w.title == title)
                .ok_or_else(|| format!("no window is titled \"{title}\"; windows: {}", titles()))?,
            None => match windows.as_slice() {
                [only] => only,
                [] => return Err("no window is open".into()),
                _ => {
                    return Err(format!(
                        "{} windows are open; name one with `window`: {}",
                        windows.len(),
                        titles()
                    ));
                }
            },
        };
        let (w, h) = (window.width as f64, window.height as f64);
        let x = if spot.x < 0.0 { w + spot.x } else { spot.x };
        let y = if spot.y < 0.0 { h + spot.y } else { spot.y };
        if !(0.0..w).contains(&x) || !(0.0..h).contains(&y) {
            return Err(format!(
                "[{}, {}] is outside \"{}\", which is {w}x{h}",
                spot.x, spot.y, window.title
            ));
        }
        Ok((window.x as f64 + x, window.y as f64 + y))
    }

    fn terminal(&mut self) -> Result<&mut crate::terminal::Terminal, String> {
        self.session
            .terminal()
            .ok_or_else(|| "this story has no terminal pane".to_string())
    }

    /// Type a command line into the terminal pane, then press Enter.
    fn type_command(&mut self, line: &str) -> Result<(), String> {
        let io = |e: io::Error| e.to_string();
        for c in line.chars() {
            let text = if c == '\n' {
                "\r\n> ".to_string()
            } else {
                c.to_string()
            };
            self.terminal()?.show(&text);
            self.session.run_for(PER_KEY).map_err(io)?;
        }
        self.session.run_for(BEFORE_ENTER).map_err(io)?;
        self.terminal()?.show("\r\n");
        Ok(())
    }

    /// Wait for the running command to finish, then show the prompt and
    /// check its status.
    fn wait_exit(&mut self, timeout: Duration) -> Result<(), String> {
        let io = |e: io::Error| e.to_string();
        let start = Instant::now();
        let status = loop {
            if let Some(status) = self.poll_running()? {
                break status;
            }
            if start.elapsed() > timeout {
                let command = self.running.as_ref().unwrap().command.clone();
                return Err(format!(
                    "`{command}` was still running after {timeout:?}; the terminal shows:\n{}",
                    tail(&self.terminal()?.contents())
                ));
            }
            self.session.tick().map_err(io)?;
        };
        let Running { command, exit, .. } = self.running.take().unwrap();
        self.terminal()?.prompt();
        if status != exit {
            return Err(format!(
                "`{command}` exited with {status}, expected {exit}; the terminal shows:\n{}",
                tail(&self.terminal()?.contents())
            ));
        }
        Ok(())
    }

    /// The running command's status, once it has finished.
    fn poll_running(&mut self) -> Result<Option<i32>, String> {
        if let Some(status) = self.running.as_ref().and_then(|r| r.status) {
            return Ok(Some(status));
        }
        let status = self.terminal()?.finished().map_err(|e| e.to_string())?;
        if let Some(running) = &mut self.running {
            running.status = status;
        }
        Ok(status)
    }

    /// Keys sent to the terminal need a program there to read them.
    fn terminal_input(&mut self) -> Result<(), String> {
        let Some(command) = self.running.as_ref().map(|r| r.command.clone()) else {
            return Err("nothing is running in the terminal to read the keys".into());
        };
        if let Some(status) = self.poll_running()? {
            return Err(format!(
                "`{command}` already exited with {status}, so nothing is reading the keys"
            ));
        }
        let moved = self.session.camera.overview();
        self.settle_camera(moved).map_err(|e| e.to_string())
    }

    /// Keys go to the focused window; fail clearly when there is none.
    fn need_window(&self, step: &str) -> Result<(), String> {
        if self.session.windows().is_empty() {
            return Err(format!(
                "a `{step}` step needs an app window to type into; terminal commands are `run` steps"
            ));
        }
        Ok(())
    }

    /// Point the camera at `focus`, and let it get there before anything
    /// else moves.
    fn frame(&mut self, focus: CameraRect, zoom: Option<f64>) -> io::Result<()> {
        let moved = self.session.camera.frame(focus, zoom);
        self.settle_camera(moved)
    }

    /// Run frames until the camera has finished a move it just started.
    fn settle_camera(&mut self, moved: bool) -> io::Result<()> {
        while moved && !self.session.camera.settled() {
            self.session.tick()?;
        }
        Ok(())
    }

    /// Ease the pointer to `point` over `duration`.
    fn move_to(&mut self, point: (f64, f64), duration: Duration) -> io::Result<()> {
        let from = self.pointer;
        self.session.show_pointer();
        let start = self.session.now();
        loop {
            let t = ((self.session.now() - start).as_secs_f64() / duration.as_secs_f64()).min(1.0);
            // Smoothstep: slow at both ends, like a hand.
            let e = t * t * (3.0 - 2.0 * t);
            let at = (
                from.0 + (point.0 - from.0) * e,
                from.1 + (point.1 - from.1) * e,
            );
            self.session.pointer_move(at.0, at.1);
            self.pointer = at;
            if t >= 1.0 {
                return Ok(());
            }
            self.session.tick()?;
        }
    }

    /// Press where the pointer is, carry it to `point`, and let go there.
    fn drag(&mut self, point: (f64, f64)) -> io::Result<()> {
        self.session.pointer_button(Button::Left, true);
        self.session.run_for(CLICK)?;
        self.move_to(point, DRAG)?;
        self.session.run_for(DROP_HOVER)?;
        self.session.pointer_button(Button::Left, false);
        self.session.tick()?;
        Ok(())
    }

    fn click(&mut self) -> io::Result<()> {
        self.session.mark_click();
        self.session.pointer_button(Button::Left, true);
        self.session.run_for(CLICK)?;
        self.session.pointer_button(Button::Left, false);
        self.session.tick()?;
        Ok(())
    }

    fn press(&mut self, press: &keys::Press) -> io::Result<()> {
        for &m in &press.modifiers {
            self.session.key(m, true);
        }
        self.session.key(press.key, true);
        self.session.key(press.key, false);
        for &m in press.modifiers.iter().rev() {
            self.session.key(m, false);
        }
        Ok(())
    }

    /// Keep the session running until `target` can be found, and return it.
    /// After `timeout`, fail with what was found instead. With `text`, a
    /// target with no context besides its name may also be text in the
    /// terminal pane; only steps that check, not act, allow that.
    fn wait_until(
        &mut self,
        target: &Target,
        timeout: Duration,
        text: bool,
    ) -> Result<Found, String> {
        let start = Instant::now();
        let plain = text
            && target.role.is_none()
            && target.within.is_none()
            && target.near.is_none()
            && target.window.is_none()
            && target.app.is_none();
        loop {
            if plain
                && self
                    .session
                    .terminal()
                    .is_some_and(|t| t.contains(&target.name))
            {
                return Ok(Found {
                    point: self.pointer,
                    rect: (self.pointer.0, self.pointer.1, 0.0, 0.0),
                    value: None,
                });
            }
            let attempt = self.find(target, text);
            match attempt {
                Ok(found) => return Ok(found),
                Err(e) if start.elapsed() >= timeout => return Err(e),
                Err(_) => {
                    // Let the app draw a few frames before looking again.
                    for _ in 0..3 {
                        self.session.tick().map_err(|e| e.to_string())?;
                    }
                }
            }
        }
    }

    /// Find `target` once. With `text`, a text field whose contents equal
    /// the target's name matches too.
    fn find(&self, target: &Target, text: bool) -> Result<Found, String> {
        let apps = self.apps_for(target)?;
        let mut trees: Vec<(App, Vec<Node>)> = apps
            .into_iter()
            .map(|app| {
                let nodes = self.a11y.nodes(&app, false);
                (app, nodes)
            })
            .collect();
        trees.retain(|(_, nodes)| !nodes.is_empty());

        // Every showing element with the name, then narrowed by context.
        let named: Vec<(usize, usize)> = trees
            .iter()
            .enumerate()
            .flat_map(|(a, (_, nodes))| {
                nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| {
                        names(&target.name, &n.name)
                            || (text && n.text.as_deref().is_some_and(|t| names(&target.name, t)))
                    })
                    .map(move |(i, _)| (a, i))
            })
            .collect();
        let fits = |&(a, i): &(usize, usize)| {
            let nodes = &trees[a].1;
            let node = &nodes[i];
            target
                .role
                .as_ref()
                .is_none_or(|r| same_role(&node.role, r))
                && target.window.as_ref().is_none_or(|w| &node.window == w)
                && target
                    .within
                    .as_ref()
                    .is_none_or(|w| ancestors(nodes, i).any(|p| &nodes[p].name == w))
        };
        let matches: Vec<(usize, usize)> = named.iter().copied().filter(fits).collect();
        // A match inside another match is the same control.
        let set: HashSet<(usize, usize)> = matches.iter().copied().collect();
        let mut outer: Vec<(usize, usize)> = matches
            .iter()
            .copied()
            .filter(|&(a, i)| !ancestors(&trees[a].1, i).any(|p| set.contains(&(a, p))))
            .collect();

        if outer.len() > 1 {
            if let Some(near) = &target.near {
                outer = self.nearest(&trees, &outer, near)?;
            }
        }
        match outer.as_slice() {
            [] => Err(not_found(target, &trees, &named)),
            [(a, i)] => self.on_screen(&trees[*a].1, *i, target),
            several => Err(ambiguous(target, &trees, several)),
        }
    }

    /// The apps a target may be in: the one named by `in`, or all of them.
    fn apps_for(&self, target: &Target) -> Result<Vec<App>, String> {
        let apps = self.a11y.apps();
        let Some(name) = &target.app else {
            return Ok(apps);
        };
        let pid = self
            .session
            .pid(name)
            .ok_or_else(|| format!("`in = \"{name}\"` names no running app"))?;
        Ok(apps
            .into_iter()
            .filter(|app| descends_from(app.pid, pid))
            .collect())
    }

    /// Keep the matches closest to the element named `near`.
    fn nearest(
        &self,
        trees: &[(App, Vec<Node>)],
        matches: &[(usize, usize)],
        near: &str,
    ) -> Result<Vec<(usize, usize)>, String> {
        let anchors: Vec<(usize, &Node)> = trees
            .iter()
            .enumerate()
            .flat_map(|(a, (_, nodes))| {
                nodes.iter().filter(|n| n.name == near).map(move |n| (a, n))
            })
            .collect();
        if anchors.is_empty() {
            return Err(format!("`near = \"{near}\"` names nothing showing"));
        }
        let center = |n: &Node| {
            n.extents
                .map(|(x, y, w, h)| (x as f64 + w as f64 / 2.0, y as f64 + h as f64 / 2.0))
        };
        let distance = |&(a, i): &(usize, usize)| {
            let node = &trees[a].1[i];
            anchors
                .iter()
                .filter(|(aa, n)| *aa == a && n.window == node.window)
                .filter_map(|(_, n)| Some((center(n)?, center(node)?)))
                .map(|((x1, y1), (x2, y2))| (x1 - x2).hypot(y1 - y2))
                .fold(f64::INFINITY, f64::min)
        };
        let best = matches.iter().map(distance).fold(f64::INFINITY, f64::min);
        Ok(matches
            .iter()
            .copied()
            .filter(|m| distance(m) == best)
            .collect())
    }

    /// Where `node` is on the output, if it is inside its window.
    fn on_screen(&self, nodes: &[Node], i: usize, target: &Target) -> Result<Found, String> {
        let node = &nodes[i];
        let windows = self.session.windows();
        let window = windows
            .iter()
            .find(|w| w.title == node.window)
            .or_else(|| (windows.len() == 1).then(|| &windows[0]))
            .ok_or_else(|| {
                let titles: Vec<_> = windows.iter().map(|w| format!("\"{}\"", w.title)).collect();
                format!(
                    "\"{}\" is in window \"{}\", which is not on screen; windows: {}",
                    target.name,
                    node.window,
                    titles.join(", ")
                )
            })?;
        let (x, y, w, h) = in_points(
            nodes,
            i,
            (window.width, window.height),
            self.session.scale() as i32,
        )
        .ok_or_else(|| format!("\"{}\" does not report its position", target.name))?;
        let (cx, cy) = (x as f64 + w as f64 / 2.0, y as f64 + h as f64 / 2.0);
        if cx < 0.0 || cy < 0.0 || cx >= window.width as f64 || cy >= window.height as f64 {
            return Err(format!(
                "\"{}\" is outside its window ({x},{y} {w}x{h} in a {}x{} window); scroll it into view first",
                target.name, window.width, window.height
            ));
        }
        let point = (window.x as f64 + cx, window.y as f64 + cy);
        let (ax, ay, aw, ah) = self.session.window_area();
        let inside = |v: f64, start: i32, len: i32| v >= start as f64 && v < (start + len) as f64;
        if !inside(point.0, ax, aw) || !inside(point.1, ay, ah) {
            return Err(format!(
                "\"{}\" is behind the terminal pane: the window is {}x{} but the space beside the pane is {aw}x{ah}; make the pane smaller or the screen larger",
                target.name, window.width, window.height
            ));
        }
        let (left, top) = (window.x as f64 + x as f64, window.y as f64 + y as f64);
        Ok(Found {
            point,
            rect: (left, top, w as f64, h as f64),
            value: node.value,
        })
    }
}

/// Whether `name`, as a storyboard writes it, matches `actual`: exactly;
/// when it ends in `*`, as a prefix, for labels that change as an app runs
/// (`j1 release*` matches `j1 release, running, Now`); and when it also
/// starts with `*`, anywhere, for a console's whole buffer in one field.
fn names(name: &str, actual: &str) -> bool {
    match name.strip_suffix('*') {
        Some(rest) => match rest.strip_prefix('*') {
            Some(inner) => actual.contains(inner),
            None => actual.starts_with(rest),
        },
        None => actual == name,
    }
}

/// Whether two role names mean the same role. Toolkits name some roles
/// differently: GTK says "button" where AT-SPI says "push button", and
/// "text box" where it says "entry".
fn same_role(a: &str, b: &str) -> bool {
    const ALIASES: &[&[&str]] = &[&["button", "push button"], &["text box", "entry"]];
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    a == b
        || ALIASES
            .iter()
            .any(|group| group.contains(&a.as_str()) && group.contains(&b.as_str()))
}

/// A found target's rectangle, for the camera.
fn rect((x, y, w, h): (f64, f64, f64, f64)) -> CameraRect {
    CameraRect::new(x, y, w, h)
}

/// A small area around a point given by coordinates, for the camera.
fn around((x, y): (f64, f64)) -> CameraRect {
    CameraRect::new(x - 40.0, y - 25.0, 80.0, 50.0)
}

/// The last lines of a terminal screen, for a diagnostic.
fn tail(contents: &str) -> String {
    let lines: Vec<&str> = contents.trim_end().lines().collect();
    let start = lines.len().saturating_sub(8);
    lines[start..]
        .iter()
        .map(|l| format!("  | {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Indices of `i`'s ancestors, nearest first.
fn ancestors(nodes: &[Node], i: usize) -> impl Iterator<Item = usize> + '_ {
    std::iter::successors(nodes[i].parent, move |&p| nodes[p].parent)
}

/// Whether process `pid` is `root` or one of its descendants.
fn descends_from(mut pid: u32, root: u32) -> bool {
    for _ in 0..64 {
        if pid == root {
            return true;
        }
        let Some(parent) = crate::platform::parent_of(pid) else {
            return false;
        };
        if parent == 0 || parent == pid {
            return false;
        }
        pid = parent;
    }
    false
}

fn describe(nodes: &[Node], i: usize) -> String {
    let node = &nodes[i];
    let path: Vec<&str> = ancestors(nodes, i)
        .map(|p| nodes[p].name.as_str())
        .filter(|n| !n.is_empty())
        .collect();
    let at = node
        .extents
        .map(|(x, y, w, h)| format!(" at {x},{y} {w}x{h}"))
        .unwrap_or_default();
    let within = if path.len() > 1 {
        format!(" within \"{}\"", path[0])
    } else {
        String::new()
    };
    format!(
        "{} \"{}\"{within} in window \"{}\"{at}",
        node.role, node.name, node.window
    )
}

fn not_found(target: &Target, trees: &[(App, Vec<Node>)], named: &[(usize, usize)]) -> String {
    let mut out = format!("nothing showing matches \"{}\"", target.name);
    let fields: Vec<String> = trees
        .iter()
        .flat_map(|(_, nodes)| nodes.iter())
        .filter_map(|n| Some(format!("{} {:?}", n.role, n.text.as_ref()?)))
        .take(8)
        .collect();
    if !fields.is_empty() {
        out.push_str(&format!("; text fields contain {}", fields.join(", ")));
    }
    if !named.is_empty() {
        out.push_str("; with that name but other context:");
        for &(a, i) in named.iter().take(8) {
            out.push_str(&format!("\n  {}", describe(&trees[a].1, i)));
        }
        return out;
    }
    let lower = target.name.to_lowercase();
    let similar: Vec<String> = trees
        .iter()
        .flat_map(|(_, nodes)| nodes.iter())
        .filter(|n| !n.name.is_empty() && n.name.to_lowercase().contains(&lower))
        .map(|n| format!("{} \"{}\"", n.role, n.name))
        .collect::<Vec<_>>();
    if !similar.is_empty() {
        out.push_str(&format!("; similar names: {}", similar.join(", ")));
    } else {
        let mut seen = HashSet::new();
        let names: Vec<String> = trees
            .iter()
            .flat_map(|(_, nodes)| nodes.iter())
            .filter(|n| n.depth > 0 && !n.name.is_empty() && seen.insert(n.name.clone()))
            .map(|n| format!("\"{}\"", n.name))
            .collect();
        if names.is_empty() {
            out.push_str("; no app has published an accessibility tree");
        } else if names.len() > 20 {
            out.push_str(&format!(
                "; showing names include {}, and {} more; `matinee probe` lists them all",
                names[..20].join(", "),
                names.len() - 20
            ));
        } else {
            out.push_str(&format!("; showing names include {}", names.join(", ")));
        }
    }
    out
}

fn ambiguous(target: &Target, trees: &[(App, Vec<Node>)], several: &[(usize, usize)]) -> String {
    let mut out = format!(
        "{} elements match \"{}\"; add `role`, `within`, `near`, or `window` to pick one:",
        several.len(),
        target.name
    );
    for &(a, i) in several.iter().take(8) {
        out.push_str(&format!("\n  {}", describe(&trees[a].1, i)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_star_matches_by_prefix() {
        assert!(names("j1 release*", "j1 release, running, Now"));
        assert!(names("j1 release*", "j1 release"));
        assert!(!names("j1 release*", "j2 release"));
        assert!(names("Drop here", "Drop here"));
        assert!(!names("Drop", "Drop here"));
        assert!(names("*3 passed*", "> make test\r\n3 passed, 0 failed\r\n"));
        assert!(!names(
            "*3 passed*",
            "> make test\r\n0 passed, 3 failed\r\n"
        ));
    }

    #[test]
    fn role_names_from_different_toolkits_match() {
        assert!(same_role("push button", "button"));
        assert!(same_role("Button", "push button"));
        assert!(same_role("text box", "entry"));
        assert!(!same_role("toggle button", "button"));
        assert!(!same_role("label", "button"));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_long_list_of_names_points_at_probe() {
        let app = App {
            bus: ":1.1".into(),
            path: zbus::zvariant::OwnedObjectPath::try_from("/app").unwrap(),
            name: "app".into(),
            pid: 1,
        };
        let node = |name: String| Node {
            name,
            role: "label".into(),
            depth: 1,
            parent: Some(0),
            window: "Window".into(),
            extents: None,
            showing: true,
            focused: false,
            text: None,
            value: None,
        };
        let target = Target {
            name: "Missing".into(),
            role: None,
            within: None,
            near: None,
            window: None,
            app: None,
        };
        let few: Vec<Node> = (0..20).map(|i| node(format!("item {i}"))).collect();
        let text = not_found(&target, &[(app.clone(), few)], &[]);
        assert!(text.ends_with("\"item 19\""), "{text}");

        let many: Vec<Node> = (0..25).map(|i| node(format!("item {i}"))).collect();
        let text = not_found(&target, &[(app, many)], &[]);
        assert!(!text.contains("item 20"), "{text}");
        assert!(
            text.contains("and 5 more; `matinee probe` lists them all"),
            "{text}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_process_descends_from_itself_and_its_parent() {
        let me = std::process::id();
        let parent = std::os::unix::process::parent_id();
        assert!(descends_from(me, me));
        assert!(descends_from(me, parent));
        assert!(!descends_from(parent, me));
    }
}
