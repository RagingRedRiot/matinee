//! Performing a story's steps against a running session.
//!
//! Targets are found through the accessibility tree: an element showing on
//! screen whose name matches exactly, narrowed by role, a named ancestor
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
//! step types the command it shows and runs the one it names.

use crate::{
    a11y::{A11y, App, Node},
    compositor::BUTTON_LEFT,
    keys,
    session::Session,
    story::{Action, Step, Story, Target},
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
    pointer: (f64, f64),
}

impl<'r, 'b> Runner<'r, 'b> {
    pub fn new(session: &'r mut Session<'b>, story: &Story) -> io::Result<Runner<'r, 'b>> {
        let a11y = A11y::connect(&session.buses)?;
        let [w, h] = story.demo.size;
        Ok(Runner {
            session,
            a11y,
            pointer: (w as f64 / 2.0, h as f64 / 2.0),
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
                    while self.session.compositor.windows().is_empty() {
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
        Ok(())
    }

    /// Perform every scene in order.
    pub fn perform(&mut self, story: &Story) -> Result<usize, Failure> {
        let mut done = 0;
        for (si, scene) in story.scenes.iter().enumerate() {
            for (ti, step) in scene.steps.iter().enumerate() {
                self.step(step).map_err(|message| Failure {
                    at: format!("scene {}, step {}", si + 1, ti + 1),
                    message,
                })?;
                done += 1;
            }
        }
        Ok(done)
    }

    fn step(&mut self, step: &Step) -> Result<(), String> {
        let io = |e: io::Error| e.to_string();
        match &step.action {
            Action::WaitFor(target) => {
                self.wait_until(target, step.timeout.unwrap_or(WAIT_TIMEOUT), true)?;
            }
            Action::Expect(target) => {
                self.wait_until(target, step.timeout.unwrap_or(EXPECT_TIMEOUT), true)?;
            }
            Action::Hover(target) => {
                let found = self.wait_until(target, step.timeout.unwrap_or(FIND_TIMEOUT), false)?;
                self.move_to(found.point, TRAVEL).map_err(io)?;
            }
            Action::Click(target) => {
                let found = self.wait_until(target, step.timeout.unwrap_or(FIND_TIMEOUT), false)?;
                self.move_to(found.point, TRAVEL).map_err(io)?;
                self.click().map_err(io)?;
            }
            Action::Type(text) => {
                self.need_window("type")?;
                for c in text.chars() {
                    let press = keys::for_char(c)
                        .ok_or_else(|| format!("`{c}` cannot be typed on the keyboard"))?;
                    self.press(&press).map_err(io)?;
                    self.session.run_for(PER_KEY).map_err(io)?;
                }
            }
            Action::Key(chord) => {
                self.need_window("key")?;
                let press = keys::chord(chord)?;
                self.press(&press).map_err(io)?;
                self.session.run_for(PER_KEY).map_err(io)?;
            }
            Action::Pause => {}
            // The camera only matters once frames are recorded.
            Action::Camera(_) => {}
            Action::Run {
                command,
                shown,
                exit,
            } => {
                self.type_command(shown.as_deref().unwrap_or(command))?;
                let terminal = self.terminal()?;
                terminal.run(command).map_err(io)?;
                let timeout = step.timeout.unwrap_or(RUN_TIMEOUT);
                let start = Instant::now();
                let status = loop {
                    if let Some(status) = self.terminal()?.finished().map_err(io)? {
                        break status;
                    }
                    if start.elapsed() > timeout {
                        return Err(format!(
                            "`{command}` was still running after {timeout:?}; the terminal shows:\n{}",
                            tail(&self.terminal()?.contents())
                        ));
                    }
                    self.session.tick().map_err(io)?;
                };
                self.terminal()?.prompt();
                if status != *exit {
                    return Err(format!(
                        "`{command}` exited with {status}, expected {exit}; the terminal shows:\n{}",
                        tail(&self.terminal()?.contents())
                    ));
                }
            }
            Action::Playback { shown, output } => {
                let text = std::fs::read_to_string(self.session.story_dir().join(output))
                    .map_err(|e| format!("{}: {e}", output.display()))?;
                self.type_command(shown)?;
                let terminal = self.terminal()?;
                terminal.show(&text.replace('\n', "\r\n"));
                terminal.prompt();
                self.session.tick().map_err(io)?;
            }
            Action::Drag { from, to } => {
                let timeout = step.timeout.unwrap_or(FIND_TIMEOUT);
                let source = self.wait_until(from, timeout, false)?;
                self.move_to(source.point, TRAVEL).map_err(io)?;
                let target = self.wait_until(to, timeout, false)?;
                self.drag(target.point).map_err(io)?;
            }
            Action::DragSlider { target, to } => {
                let found = self.wait_until(target, step.timeout.unwrap_or(FIND_TIMEOUT), false)?;
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
        Ok(())
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

    /// Keys go to the focused window; fail clearly when there is none.
    fn need_window(&self, step: &str) -> Result<(), String> {
        if self.session.compositor.windows().is_empty() {
            return Err(format!(
                "a `{step}` step needs an app window to type into; terminal commands are `run` steps"
            ));
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
            let now = self.session.now();
            self.session.compositor.pointer_move(at.0, at.1, now);
            self.pointer = at;
            if t >= 1.0 {
                return Ok(());
            }
            self.session.tick()?;
        }
    }

    /// Press where the pointer is, carry it to `point`, and let go there.
    fn drag(&mut self, point: (f64, f64)) -> io::Result<()> {
        let now = self.session.now();
        self.session
            .compositor
            .pointer_button(BUTTON_LEFT, true, now);
        self.session.run_for(CLICK)?;
        self.move_to(point, DRAG)?;
        self.session.run_for(DROP_HOVER)?;
        let now = self.session.now();
        self.session
            .compositor
            .pointer_button(BUTTON_LEFT, false, now);
        self.session.tick()?;
        Ok(())
    }

    fn click(&mut self) -> io::Result<()> {
        let now = self.session.now();
        self.session
            .compositor
            .pointer_button(BUTTON_LEFT, true, now);
        self.session.run_for(CLICK)?;
        let now = self.session.now();
        self.session
            .compositor
            .pointer_button(BUTTON_LEFT, false, now);
        self.session.tick()?;
        Ok(())
    }

    fn press(&mut self, press: &keys::Press) -> io::Result<()> {
        let now = self.session.now();
        let keyboard = &mut self.session.compositor;
        for &m in &press.modifiers {
            keyboard.key(m, true, now);
        }
        keyboard.key(press.key, true, now);
        keyboard.key(press.key, false, now);
        for &m in press.modifiers.iter().rev() {
            keyboard.key(m, false, now);
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
                        n.name == target.name
                            || (text && n.text.as_deref() == Some(target.name.as_str()))
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
                .is_none_or(|r| node.role.eq_ignore_ascii_case(r))
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
            [(a, i)] => self.on_screen(&trees[*a].1[*i], target),
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
    fn on_screen(&self, node: &Node, target: &Target) -> Result<Found, String> {
        let windows = self.session.compositor.windows();
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
        let (x, y, w, h) = node
            .extents
            .ok_or_else(|| format!("\"{}\" does not report its position", target.name))?;
        let (cx, cy) = (x as f64 + w as f64 / 2.0, y as f64 + h as f64 / 2.0);
        let size = window.rect.size;
        if cx < 0.0 || cy < 0.0 || cx >= size.w as f64 || cy >= size.h as f64 {
            return Err(format!(
                "\"{}\" is outside its window ({x},{y} {w}x{h} in a {}x{} window); scroll it into view first",
                target.name, size.w, size.h
            ));
        }
        let point = (window.rect.loc.x as f64 + cx, window.rect.loc.y as f64 + cy);
        let (ax, ay, aw, ah) = self.session.compositor.window_area();
        let inside = |v: f64, start: i32, len: i32| v >= start as f64 && v < (start + len) as f64;
        if !inside(point.0, ax, aw) || !inside(point.1, ay, ah) {
            return Err(format!(
                "\"{}\" is behind the terminal pane: the window is {}x{} but the space beside the pane is {aw}x{ah}; make the pane smaller or the screen larger",
                target.name, size.w, size.h
            ));
        }
        let (left, top) = (
            window.rect.loc.x as f64 + x as f64,
            window.rect.loc.y as f64 + y as f64,
        );
        Ok(Found {
            point,
            rect: (left, top, w as f64, h as f64),
            value: node.value,
        })
    }
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
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        // The parent is the second field after the parenthesized name.
        let Some(parent) = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().nth(1))
            .and_then(|p| p.parse().ok())
        else {
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
            .take(20)
            .map(|n| format!("\"{}\"", n.name))
            .collect();
        if names.is_empty() {
            out.push_str("; no app has published an accessibility tree");
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
    fn a_process_descends_from_itself_and_its_parent() {
        let me = std::process::id();
        let parent = std::os::unix::process::parent_id();
        assert!(descends_from(me, me));
        assert!(descends_from(me, parent));
        assert!(!descends_from(parent, me));
    }
}
