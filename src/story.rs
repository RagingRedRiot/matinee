//! The storyboard: a TOML file that describes one demo.
//!
//! Loading a storyboard parses it and checks it as a whole, so a mistake is
//! reported before anything launches. Errors name the scene and step.
//!
//! Paths in a storyboard are relative to the storyboard's own directory.

use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

/// A checked storyboard.
#[derive(Debug, Clone, PartialEq)]
pub struct Story {
    /// The directory holding the storyboard; relative paths start here.
    pub dir: PathBuf,
    pub demo: Demo,
    /// Commands run before any app launches, off camera.
    pub seeds: Vec<Seed>,
    pub launches: Vec<Launch>,
    pub camera: Camera,
    pub session: Session,
    pub terminal: TerminalPane,
    /// Where captions go, unless a scene says otherwise.
    pub captions: CaptionPosition,
    pub scenes: Vec<Scene>,
}

/// Where a caption goes: over the frame at its bottom or top, over the
/// frame on the half away from the action, or in a band added below or
/// above the frame, where it covers nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptionPosition {
    Bottom,
    Top,
    Auto,
    Below,
    Above,
}

/// matinee's terminal pane, drawn into the frame beside the app windows.
#[derive(Debug, Clone, PartialEq)]
pub struct TerminalPane {
    pub position: PanePosition,
    /// Height for a bottom pane, width for a right one, in points. `None`
    /// picks a size from the screen.
    pub size: Option<u32>,
    /// Text size in points.
    pub font_size: f32,
    /// Whether the pane leaves the rest of the screen to windows even when
    /// the story launches no app, for windows its commands open. Set when
    /// the storyboard gives the pane a position or size.
    pub leaves_room: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanePosition {
    Bottom,
    Right,
}

impl Story {
    /// The GUI mechanisms a checked story declares, rather than a claim
    /// about what its app will publish at runtime.
    pub fn control_usage(&self) -> Vec<&'static str> {
        if self.launches.is_empty() {
            return vec!["terminal only"];
        }
        let mut semantic = self.launches.iter().any(|l| l.wait_for.is_some());
        let terminal = self.has_terminal();
        let mut coordinates = false;
        let mut keyboard = false;
        let mut unqualified = false;
        for step in self.steps() {
            match &step.action {
                Action::Hover(_)
                | Action::Click(_)
                | Action::Drag { .. }
                | Action::DragSlider { .. }
                | Action::Camera(CameraMove::Focus { .. }) => {
                    semantic = true;
                }
                Action::Expect(t) | Action::WaitFor(t) => {
                    if !terminal
                        || t.role.is_some()
                        || t.within.is_some()
                        || t.near.is_some()
                        || t.window.is_some()
                        || t.app.is_some()
                    {
                        semantic = true;
                    } else {
                        unqualified = true;
                    }
                }
                Action::HoverAt(_)
                | Action::ClickAt(_)
                | Action::DragAt { .. }
                | Action::Camera(CameraMove::FocusAt { .. })
                | Action::Highlight(Highlight {
                    mark: Mark::Area { .. },
                    ..
                }) => coordinates = true,
                Action::Highlight(Highlight {
                    mark: Mark::Target(t),
                    ..
                }) if !terminal
                    || t.role.is_some()
                    || t.within.is_some()
                    || t.near.is_some()
                    || t.window.is_some()
                    || t.app.is_some() =>
                {
                    semantic = true;
                }
                Action::Type {
                    terminal: false, ..
                }
                | Action::Key {
                    terminal: false, ..
                } => {
                    keyboard = true;
                }
                _ => {}
            }
        }
        let mut usage = vec![];
        if semantic {
            usage.push("semantic targets required");
        }
        if coordinates {
            usage.push("window coordinates");
        }
        if keyboard {
            usage.push("focused keyboard input");
        }
        if usage.is_empty() {
            usage.push("capture only; GUI result unverified");
        }
        if unqualified {
            usage.push("unqualified assertions may match GUI or terminal");
        }
        usage
    }

    /// Whether the story shows a terminal: it runs commands, or launches
    /// no app and so is a terminal demo.
    pub fn has_terminal(&self) -> bool {
        self.launches.is_empty()
            || self
                .steps()
                .any(|s| matches!(s.action, Action::Run { .. } | Action::Playback { .. }))
    }
}

/// What the isolated session may see beyond the project.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Session {
    /// Give the session outside access through pasta. Off by default.
    pub network: bool,
    /// Extra host paths shown read-only at the same path inside, as
    /// written: `~` is the host home, and relative paths start at the
    /// storyboard.
    pub mounts: Vec<String>,
    /// Folders of programs, mounted like `mounts` and put at the front of
    /// `PATH` inside, in order.
    pub path: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Demo {
    /// Output path without extension; each theme adds `-light.gif` or
    /// `-dark.gif`.
    pub output: PathBuf,
    /// Logical screen size, in points.
    pub size: [u32; 2],
    /// Device pixels per point.
    pub scale: u32,
    pub fps: u32,
    pub themes: Vec<Theme>,
    /// How far, in any channel, lossy GIF compression may move a color;
    /// 0 is exact.
    pub lossy: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    pub fn name(self) -> &'static str {
        match self {
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Seed {
    pub run: String,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    /// What steps call this app with `in`; defaults to the program's file
    /// name.
    pub name: String,
    pub app: String,
    pub args: Vec<String>,
    /// An accessible name whose appearance means the app is ready.
    pub wait_for: Option<String>,
    pub timeout: Option<Duration>,
    /// Run the app under Xwayland, for apps that cannot use Wayland.
    pub x11: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    pub mode: CameraMode,
    pub max_zoom: f64,
    pub ease: Duration,
    /// How wide, in pixels, the recording is expected to be shown. Auto
    /// framing zooms until text reads at that width.
    pub reading_width: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CameraMode {
    /// Follow the action.
    Auto,
    /// Never zoom.
    Fixed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub caption: Option<String>,
    /// Where this scene's caption goes, in place of the story's.
    pub caption_position: Option<CaptionPosition>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub action: Action,
    /// Time held after the action, on camera.
    pub hold: Option<Duration>,
    /// How long to wait for a target, an assertion, or a command.
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Run a command in the terminal and check its exit status. When
    /// `shown` is set, the terminal displays it instead of `command`.
    Run {
        command: String,
        shown: Option<String>,
        exit: i32,
        /// Whether the step waits for the command to finish. A command
        /// that does not is left running for `type` and `key` steps, and
        /// its status is checked by a later `wait_exit`, or at the end.
        wait: bool,
    },
    /// Wait for the command still running in the terminal, named as its
    /// `run` or `terminal` text, and check its exit status.
    WaitExit(String),
    /// Display a command and play back canned output; nothing runs.
    /// `output` is relative to the storyboard's folder.
    Playback {
        shown: String,
        output: PathBuf,
    },
    WaitFor(Target),
    /// Fail unless the target is on screen.
    Expect(Target),
    Hover(Target),
    Click(Target),
    Drag {
        from: Target,
        to: Target,
    },
    /// Drag a slider to a fraction of its range, 0 to 1.
    DragSlider {
        target: Target,
        to: f64,
    },
    /// Explicit control, for things with no accessible name: a point in a
    /// window rather than a named target.
    HoverAt(Spot),
    ClickAt(Spot),
    DragAt {
        from: Spot,
        to: Spot,
    },
    /// Type text into the focused window, or into the program running in
    /// the terminal.
    Type {
        text: String,
        terminal: bool,
    },
    /// A key chord such as `ctrl+k` or `enter`, pressed in the focused
    /// window or in the terminal.
    Key {
        chord: String,
        terminal: bool,
    },
    Camera(CameraMove),
    /// Mark part of the screen until the scene ends or highlights are
    /// cleared.
    Highlight(Highlight),
    /// Take down the scene's highlights.
    ClearHighlights,
    /// Only a hold.
    Pause,
}

/// A highlight: what it marks and how.
#[derive(Debug, Clone, PartialEq)]
pub struct Highlight {
    pub mark: Mark,
    pub style: HighlightStyle,
    /// A callout shown beside the mark.
    pub label: Option<String>,
    /// A numbered marker on the mark's corner, for a sequence of steps.
    pub number: Option<u32>,
}

/// What a highlight marks.
#[derive(Debug, Clone, PartialEq)]
pub enum Mark {
    /// A control by name, or, with no context besides the name, text in
    /// the terminal pane or in an app's text.
    Target(Target),
    /// The lines from one holding `from` through one holding `to`, in the
    /// terminal pane or an app's text.
    Lines { from: String, to: String },
    /// A rectangle in a window, in points: x, y, width, height, with
    /// negative x and y counting from the right and bottom edges.
    Area {
        rect: [f64; 4],
        window: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HighlightStyle {
    /// An outline around the mark.
    #[default]
    Box,
    /// The rest of the screen dimmed, with the mark outlined.
    Spotlight,
}

/// A point in a window, in points from its top-left corner; a negative
/// coordinate counts from the right or bottom edge.
#[derive(Debug, Clone, PartialEq)]
pub struct Spot {
    pub x: f64,
    pub y: f64,
    /// The window's exact title; needed when more than one is open.
    pub window: Option<String>,
}

impl Action {
    /// Whether the terminal shows something other than what runs.
    pub fn is_staged(&self) -> bool {
        matches!(
            self,
            Action::Run { shown: Some(_), .. } | Action::Playback { .. }
        )
    }
}

/// An accessible element, found by name and narrowed by context.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Target {
    pub name: String,
    pub role: Option<String>,
    /// The name of an ancestor that contains the target.
    pub within: Option<String>,
    /// Prefer the match closest to this named element.
    pub near: Option<String>,
    /// Exact title of the window holding the target.
    pub window: Option<String>,
    /// The launch name of the app holding the target.
    pub app: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CameraMove {
    /// Ease back out to the whole screen.
    Overview,
    Focus {
        target: Target,
        zoom: Option<f64>,
    },
    /// Frame a window-relative point when the desired area has no stable
    /// accessible name, without moving the pointer there.
    FocusAt {
        spot: Spot,
        zoom: Option<f64>,
    },
}

/// A storyboard that could not be read or does not make sense.
#[derive(Debug)]
pub struct Error {
    pub path: PathBuf,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

impl std::error::Error for Error {}

impl Story {
    pub fn load(path: &Path) -> Result<Story, Error> {
        let fail = |message: String| Error {
            path: path.to_owned(),
            message,
        };
        let text = std::fs::read_to_string(path).map_err(|e| fail(e.to_string()))?;
        let dir = path.parent().unwrap_or(Path::new("")).to_owned();
        Story::parse(&text, dir).map_err(fail)
    }

    /// Parse and check storyboard text whose relative paths start at `dir`.
    pub fn parse(text: &str, dir: PathBuf) -> Result<Story, String> {
        let raw: RawStory = toml::from_str(text).map_err(|e| e.to_string())?;
        raw.check(dir)
    }

    pub fn steps(&self) -> impl Iterator<Item = &Step> {
        self.scenes.iter().flat_map(|s| &s.steps)
    }

    /// Where `scene`'s caption goes.
    pub fn caption_position(&self, scene: &Scene) -> CaptionPosition {
        scene.caption_position.unwrap_or(self.captions)
    }

    /// Whether some caption goes in a band above the frame, and whether
    /// one goes in a band below it.
    pub fn caption_bands(&self) -> (bool, bool) {
        let captioned = || {
            self.scenes
                .iter()
                .filter(|s| s.caption.is_some())
                .map(|s| self.caption_position(s))
        };
        (
            captioned().any(|p| p == CaptionPosition::Above),
            captioned().any(|p| p == CaptionPosition::Below),
        )
    }
}

/// Parse a duration written with a unit: `2.4s` or `600ms`.
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let bad = || format!("`{text}` is not a duration; write it like `2.4s` or `600ms`");
    let (number, scale) = if let Some(n) = text.strip_suffix("ms") {
        (n, 1e-3)
    } else if let Some(n) = text.strip_suffix('s') {
        (n, 1.0)
    } else {
        return Err(bad());
    };
    let value: f64 = number.trim().parse().map_err(|_| bad())?;
    Duration::try_from_secs_f64(value * scale).map_err(|_| bad())
}

// The file as written. Everything is checked when it becomes a `Story`.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStory {
    demo: RawDemo,
    #[serde(default)]
    seed: Vec<RawSeed>,
    launch: Option<OneOrMany<RawLaunch>>,
    camera: Option<RawCamera>,
    session: Option<RawSession>,
    terminal: Option<RawTerminal>,
    captions: Option<RawCaptions>,
    #[serde(default)]
    scene: Vec<RawScene>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCaptions {
    position: Option<CaptionPosition>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTerminal {
    position: Option<PanePosition>,
    size: Option<u32>,
    font_size: Option<f32>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDemo {
    output: PathBuf,
    size: [u32; 2],
    scale: Option<u32>,
    fps: Option<u32>,
    themes: Option<Vec<Theme>>,
    lossy: Option<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSeed {
    run: String,
    timeout: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLaunch {
    name: Option<String>,
    app: String,
    #[serde(default)]
    args: Vec<String>,
    wait_for: Option<String>,
    timeout: Option<String>,
    #[serde(default)]
    x11: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCamera {
    mode: Option<CameraMode>,
    max_zoom: Option<f64>,
    ease: Option<String>,
    reading_width: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSession {
    #[serde(default)]
    network: bool,
    #[serde(default)]
    mounts: Vec<String>,
    #[serde(default)]
    path: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScene {
    caption: Option<String>,
    caption_position: Option<CaptionPosition>,
    steps: Vec<RawStep>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawStep {
    run: Option<String>,
    terminal: Option<String>,
    output: Option<PathBuf>,
    exit: Option<i32>,
    wait: Option<bool>,
    wait_exit: Option<String>,
    wait_for: Option<String>,
    expect: Option<String>,
    hover: Option<String>,
    click: Option<String>,
    drag: Option<String>,
    drag_slider: Option<String>,
    hover_at: Option<[f64; 2]>,
    click_at: Option<[f64; 2]>,
    drag_from: Option<[f64; 2]>,
    drag_to: Option<[f64; 2]>,
    to: Option<RawTo>,
    to_role: Option<String>,
    to_within: Option<String>,
    #[serde(rename = "type")]
    type_text: Option<String>,
    key: Option<String>,
    camera: Option<RawCameraMove>,
    highlight: Option<RawHighlight>,
    highlight_at: Option<[f64; 4]>,
    style: Option<HighlightStyle>,
    label: Option<String>,
    number: Option<u32>,
    clear: Option<String>,
    role: Option<String>,
    within: Option<String>,
    near: Option<String>,
    window: Option<String>,
    #[serde(rename = "in")]
    app: Option<String>,
    hold: Option<String>,
    timeout: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawHighlight {
    Name(String),
    Lines(RawLines),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLines {
    from: String,
    to: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawTo {
    Name(String),
    Fraction(f64),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawCameraMove {
    Named(String),
    Focus(RawFocus),
    FocusAt(RawFocusAt),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFocus {
    focus: String,
    zoom: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFocusAt {
    focus_at: [f64; 2],
    zoom: Option<f64>,
    window: Option<String>,
}

fn duration(field: &str, text: Option<String>) -> Result<Option<Duration>, String> {
    text.map(|t| parse_duration(&t).map_err(|e| format!("{field}: {e}")))
        .transpose()
}

impl RawStory {
    fn check(self, dir: PathBuf) -> Result<Story, String> {
        let d = self.demo;
        if d.size.contains(&0) {
            return Err("demo.size must be two positive numbers".into());
        }
        let scale = d.scale.unwrap_or(2);
        if !(1..=4).contains(&scale) {
            return Err("demo.scale must be from 1 to 4".into());
        }
        let fps = d.fps.unwrap_or(15);
        if !(1..=60).contains(&fps) {
            return Err("demo.fps must be from 1 to 60".into());
        }
        let themes = d.themes.unwrap_or_else(|| vec![Theme::Light, Theme::Dark]);
        if themes.is_empty() {
            return Err("demo.themes must name at least one theme".into());
        }
        if themes.iter().collect::<BTreeSet<_>>().len() != themes.len() {
            return Err("demo.themes lists a theme twice".into());
        }
        let lossy = d.lossy.unwrap_or(crate::record::LOSSY);
        if lossy > 32 {
            return Err("demo.lossy must be from 0 (exact) to 32".into());
        }
        let demo = Demo {
            output: d.output,
            size: d.size,
            scale,
            fps,
            themes,
            lossy,
        };

        let seeds = self
            .seed
            .into_iter()
            .enumerate()
            .map(|(i, s)| {
                Ok(Seed {
                    run: s.run,
                    timeout: duration(&format!("seed {}: timeout", i + 1), s.timeout)?,
                })
            })
            .collect::<Result<_, String>>()?;

        let raw_launches = match self.launch {
            None => vec![],
            Some(OneOrMany::One(l)) => vec![l],
            Some(OneOrMany::Many(v)) => v,
        };
        let mut launches = Vec::new();
        for l in raw_launches {
            let name = match l.name {
                Some(n) => n,
                None => Path::new(&l.app)
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .ok_or_else(|| format!("launch: `{}` has no program name", l.app))?,
            };
            if launches.iter().any(|o: &Launch| o.name == name) {
                return Err(format!(
                    "launch: two apps are named `{name}`; give one a `name`"
                ));
            }
            launches.push(Launch {
                timeout: duration(&format!("launch {name}: timeout"), l.timeout)?,
                name,
                app: l.app,
                args: l.args,
                wait_for: l.wait_for,
                x11: l.x11,
            });
        }

        let c = self.camera.unwrap_or(RawCamera {
            mode: None,
            max_zoom: None,
            ease: None,
            reading_width: None,
        });
        let max_zoom = c.max_zoom.unwrap_or(2.2);
        if !(1.0..=8.0).contains(&max_zoom) {
            return Err("camera.max_zoom must be from 1 to 8".into());
        }
        let reading_width = c.reading_width.unwrap_or(crate::camera::READING_WIDTH);
        if !(200..=4000).contains(&reading_width) {
            return Err("camera.reading_width must be from 200 to 4000 pixels".into());
        }
        let camera = Camera {
            mode: c.mode.unwrap_or(CameraMode::Auto),
            max_zoom,
            reading_width,
            ease: duration("camera.ease", c.ease)?.unwrap_or(Duration::from_millis(400)),
        };

        if self.scene.is_empty() {
            return Err("a storyboard needs at least one [[scene]]".into());
        }
        let mut scenes = Vec::new();
        for (si, raw) in self.scene.into_iter().enumerate() {
            if raw.steps.is_empty() {
                return Err(format!("scene {}: it has no steps", si + 1));
            }
            let mut steps = Vec::new();
            for (ti, step) in raw.steps.into_iter().enumerate() {
                let step = step
                    .check(&launches, &dir, max_zoom)
                    .map_err(|e| format!("scene {}, step {}: {e}", si + 1, ti + 1))?;
                steps.push(step);
            }
            if raw.caption_position.is_some() && raw.caption.is_none() {
                return Err(format!(
                    "scene {}: caption_position is set, but the scene has no caption",
                    si + 1
                ));
            }
            scenes.push(Scene {
                caption: raw.caption,
                caption_position: raw.caption_position,
                steps,
            });
        }
        check_running(&scenes)?;

        let session = Session {
            network: self.session.as_ref().is_some_and(|s| s.network),
            mounts: self
                .session
                .as_ref()
                .map(|s| s.mounts.clone())
                .unwrap_or_default(),
            path: self.session.map(|s| s.path).unwrap_or_default(),
        };
        let t = self.terminal.unwrap_or(RawTerminal {
            position: None,
            size: None,
            font_size: None,
        });
        let terminal = TerminalPane {
            position: t.position.unwrap_or(PanePosition::Bottom),
            size: t.size,
            font_size: t.font_size.unwrap_or(15.0),
            leaves_room: t.position.is_some() || t.size.is_some(),
        };
        if !(6.0..=72.0).contains(&terminal.font_size) {
            return Err("terminal.font_size must be from 6 to 72".into());
        }
        let limit = match terminal.position {
            PanePosition::Bottom => demo.size[1],
            PanePosition::Right => demo.size[0],
        };
        if terminal.size.is_some_and(|n| n == 0 || n >= limit) {
            return Err(format!(
                "terminal.size must leave room for windows: less than {limit}"
            ));
        }
        if let Some(m) = session.mounts.iter().find(|m| m.trim().is_empty()) {
            return Err(format!("session.mounts: `{m}` is not a path"));
        }
        if let Some(m) = session.path.iter().find(|m| m.trim().is_empty()) {
            return Err(format!("session.path: `{m}` is not a path"));
        }

        let captions = self
            .captions
            .and_then(|c| c.position)
            .unwrap_or(CaptionPosition::Bottom);
        Ok(Story {
            dir,
            demo,
            seeds,
            launches,
            camera,
            session,
            terminal,
            captions,
            scenes,
        })
    }
}

/// Check the order of terminal steps: one command runs at a time, keys sent
/// to the terminal need a program running there to read them, and
/// `wait_exit` names the command that is running.
fn check_running(scenes: &[Scene]) -> Result<(), String> {
    let mut running: Option<(&str, Option<&str>)> = None;
    for (si, scene) in scenes.iter().enumerate() {
        for (ti, step) in scene.steps.iter().enumerate() {
            let at = |e: String| format!("scene {}, step {}: {e}", si + 1, ti + 1);
            let busy = |what: &str, (command, _): (&str, Option<&str>)| {
                at(format!(
                    "{what} while `{command}` is still running; add a `wait_exit` step first"
                ))
            };
            match &step.action {
                Action::Run {
                    command,
                    shown,
                    wait,
                    ..
                } => {
                    if let Some(r) = running {
                        return Err(busy("this runs a command", r));
                    }
                    if !wait {
                        running = Some((command, shown.as_deref()));
                    }
                }
                Action::Playback { .. } => {
                    if let Some(r) = running {
                        return Err(busy("this plays back output", r));
                    }
                }
                Action::WaitExit(name) => match running {
                    Some((command, shown)) if name == command || Some(name.as_str()) == shown => {
                        running = None;
                    }
                    Some((command, _)) => {
                        return Err(at(format!(
                            "`wait_exit = \"{name}\"` names no running command; `{command}` is running"
                        )));
                    }
                    None => {
                        return Err(at(format!(
                            "`wait_exit = \"{name}\"`, but no command is running; \
                             start one with `run` and `wait = false`"
                        )));
                    }
                },
                Action::Type { terminal: true, .. } | Action::Key { terminal: true, .. }
                    if running.is_none() =>
                {
                    return Err(at(
                        "this sends keys to the terminal, but nothing is running there to \
                         read them; start a program with `run` and `wait = false`"
                            .into(),
                    ));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

impl RawStep {
    /// A highlight of `mark`, with this step's style, label, and number.
    fn highlight(&mut self, mark: Mark) -> Result<Action, String> {
        if self.label.as_ref().is_some_and(|l| l.trim().is_empty()) {
            return Err("`label` is empty".into());
        }
        if self.number == Some(0) {
            return Err("`number` counts from 1".into());
        }
        Ok(Action::Highlight(Highlight {
            mark,
            style: self.style.take().unwrap_or_default(),
            label: self.label.take(),
            number: self.number.take(),
        }))
    }

    fn check(mut self, launches: &[Launch], dir: &Path, max_zoom: f64) -> Result<Step, String> {
        let hold = duration("hold", self.hold.take())?;
        let timeout = duration("timeout", self.timeout.take())?;

        let actions: Vec<&str> = [
            ("run", self.run.is_some()),
            ("terminal", self.terminal.is_some() && self.run.is_none()),
            ("wait_exit", self.wait_exit.is_some()),
            ("wait_for", self.wait_for.is_some()),
            ("expect", self.expect.is_some()),
            ("hover", self.hover.is_some()),
            ("click", self.click.is_some()),
            ("drag", self.drag.is_some()),
            ("drag_slider", self.drag_slider.is_some()),
            ("hover_at", self.hover_at.is_some()),
            ("click_at", self.click_at.is_some()),
            ("drag_from", self.drag_from.is_some()),
            ("type", self.type_text.is_some()),
            ("key", self.key.is_some()),
            ("camera", self.camera.is_some()),
            ("highlight", self.highlight.is_some()),
            ("highlight_at", self.highlight_at.is_some()),
            ("clear", self.clear.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, set)| set.then_some(name))
        .collect();
        if actions.len() > 1 {
            return Err(format!(
                "a step does one thing, but this one has {}; split it into steps",
                actions.join(", ")
            ));
        }
        let action = actions.first().copied();

        // Keys that only make sense alongside particular actions.
        let only_with = |key: &str, set: bool, allowed: &[&str]| -> Result<(), String> {
            if set && !action.is_some_and(|a| allowed.contains(&a)) {
                return Err(format!("`{key}` only applies to {}", allowed.join(" or ")));
            }
            Ok(())
        };
        only_with("exit", self.exit.is_some(), &["run"])?;
        only_with("wait", self.wait.is_some(), &["run"])?;
        if self.wait == Some(false) && timeout.is_some() {
            return Err(
                "a `run` that does not wait has no timeout; put it on the `wait_exit` step".into(),
            );
        }
        only_with("output", self.output.is_some(), &["terminal", "run"])?;
        only_with("to", self.to.is_some(), &["drag", "drag_slider"])?;
        only_with("to_role", self.to_role.is_some(), &["drag"])?;
        only_with("to_within", self.to_within.is_some(), &["drag"])?;
        only_with("drag_to", self.drag_to.is_some(), &["drag_from"])?;
        let marks = ["highlight", "highlight_at"];
        only_with("style", self.style.is_some(), &marks)?;
        only_with("label", self.label.is_some(), &marks)?;
        only_with("number", self.number.is_some(), &marks)?;
        let at = ["hover_at", "click_at", "drag_from", "highlight_at"];
        let targeted = [
            "wait_for",
            "expect",
            "hover",
            "click",
            "drag",
            "drag_slider",
            "highlight",
        ];
        let context = [
            ("role", self.role.is_some()),
            ("within", self.within.is_some()),
            ("near", self.near.is_some()),
            ("window", self.window.is_some()),
            ("in", self.app.is_some()),
        ];
        // Keys go to the terminal with `in = "terminal"`, or when there is
        // no app window to go to.
        let keys = matches!(action, Some("type" | "key"));
        if keys {
            if let Some(app) = &self.app {
                if app != "terminal" {
                    return Err(format!(
                        "`in = \"{app}\"` on a `{}` step: keys go to the focused window, \
                         or to the terminal with `in = \"terminal\"`",
                        action.unwrap()
                    ));
                }
            }
        }
        let to_terminal = keys && (self.app.take().is_some() || launches.is_empty());
        for (key, set) in context {
            if key == "window" && action.is_some_and(|a| at.contains(&a)) {
                continue;
            }
            if key == "in" && keys {
                continue;
            }
            only_with(key, set, &targeted)?;
        }
        if action.is_some_and(|a| at.contains(&a)) && launches.is_empty() {
            return Err("this step acts on an app window, but the storyboard launches none".into());
        }

        // A plain `wait_for`, `expect`, or `highlight` may be text in the
        // terminal pane.
        let terminal_text = matches!(action, Some("wait_for" | "expect" | "highlight"))
            && context.iter().all(|(_, set)| !set);
        let targets_app = action.is_some_and(|a| targeted.contains(&a)) && !terminal_text
            || matches!(
                self.camera,
                Some(RawCameraMove::Focus(_) | RawCameraMove::FocusAt(_))
            );
        if targets_app && launches.is_empty() {
            return Err("this step targets an app, but the storyboard launches none".into());
        }
        if let Some(app) = &self.app {
            if !launches.iter().any(|l| &l.name == app) {
                let names: Vec<_> = launches.iter().map(|l| l.name.as_str()).collect();
                return Err(format!(
                    "`in = \"{app}\"` names no launched app; launched: {}",
                    names.join(", ")
                ));
            }
        }

        let target = |name: String, s: &mut RawStep| Target {
            name,
            role: s.role.take(),
            within: s.within.take(),
            near: s.near.take(),
            window: s.window.take(),
            app: s.app.take(),
        };

        let action = match action {
            None if hold.is_some() => Action::Pause,
            None if self.output.is_some() => {
                return Err("`output` needs `terminal`, the command to display".into());
            }
            None => return Err("this step does nothing".into()),
            Some("run") => {
                let shown = self.terminal.take();
                if shown.as_ref().is_some_and(|s| s.trim().is_empty()) {
                    return Err("`terminal` is empty".into());
                }
                if self.output.is_some() {
                    return Err(
                        "`output` plays back canned output in place of running a command; \
                         use it without `run`"
                            .into(),
                    );
                }
                Action::Run {
                    command: self.run.take().unwrap(),
                    shown,
                    exit: self.exit.unwrap_or(0),
                    wait: self.wait.unwrap_or(true),
                }
            }
            Some("wait_exit") => Action::WaitExit(self.wait_exit.take().unwrap()),
            Some("terminal") => {
                let shown = self.terminal.take().unwrap();
                let Some(output) = self.output.take() else {
                    return Err(
                        "`terminal` only displays a command; add `run` for the command \
                         that actually runs, or `output` for canned output"
                            .into(),
                    );
                };
                let file = dir.join(&output);
                if !file.is_file() {
                    return Err(format!("output file {} does not exist", file.display()));
                }
                Action::Playback { shown, output }
            }
            Some("wait_for") => Action::WaitFor(target(self.wait_for.take().unwrap(), &mut self)),
            Some("expect") => Action::Expect(target(self.expect.take().unwrap(), &mut self)),
            Some("hover") => Action::Hover(target(self.hover.take().unwrap(), &mut self)),
            Some("click") => Action::Click(target(self.click.take().unwrap(), &mut self)),
            Some("drag") => {
                let to = match self.to.take() {
                    Some(RawTo::Name(n)) => Target {
                        name: n,
                        role: self.to_role.take(),
                        within: self.to_within.take(),
                        near: None,
                        window: self.window.clone(),
                        app: self.app.clone(),
                    },
                    Some(RawTo::Fraction(_)) => {
                        return Err("`to` for a drag names the drop target".into());
                    }
                    None => return Err("a drag needs `to`, the drop target".into()),
                };
                let from = target(self.drag.take().unwrap(), &mut self);
                Action::Drag { from, to }
            }
            Some("drag_slider") => {
                let to = match self.to.take() {
                    Some(RawTo::Fraction(f)) if (0.0..=1.0).contains(&f) => f,
                    Some(_) => return Err("`to` for a slider is a number from 0 to 1".into()),
                    None => return Err("drag_slider needs `to`, a number from 0 to 1".into()),
                };
                Action::DragSlider {
                    target: target(self.drag_slider.take().unwrap(), &mut self),
                    to,
                }
            }
            Some("hover_at") => {
                let [x, y] = self.hover_at.unwrap();
                Action::HoverAt(Spot {
                    x,
                    y,
                    window: self.window.take(),
                })
            }
            Some("click_at") => {
                let [x, y] = self.click_at.unwrap();
                Action::ClickAt(Spot {
                    x,
                    y,
                    window: self.window.take(),
                })
            }
            Some("drag_from") => {
                let [x, y] = self.drag_from.unwrap();
                let Some([tx, ty]) = self.drag_to else {
                    return Err("`drag_from` needs `drag_to`, where to let go".into());
                };
                let window = self.window.take();
                Action::DragAt {
                    from: Spot {
                        x,
                        y,
                        window: window.clone(),
                    },
                    to: Spot {
                        x: tx,
                        y: ty,
                        window,
                    },
                }
            }
            Some("type") => Action::Type {
                text: self.type_text.take().unwrap(),
                terminal: to_terminal,
            },
            Some("key") => {
                let chord = self.key.take().unwrap();
                if to_terminal {
                    crate::keys::terminal_bytes(&chord, false)?;
                }
                Action::Key {
                    chord,
                    terminal: to_terminal,
                }
            }
            Some("camera") => match self.camera.take().unwrap() {
                RawCameraMove::Named(n) if n == "overview" => Action::Camera(CameraMove::Overview),
                RawCameraMove::Named(n) => {
                    return Err(format!(
                        "camera = \"{n}\" is not a camera move; use \"overview\" or \
                         {{ focus = \"name\", zoom = 1.8 }}"
                    ));
                }
                RawCameraMove::Focus(f) => {
                    if let Some(z) = f.zoom {
                        if !(1.0..=max_zoom).contains(&z) {
                            return Err(format!(
                                "camera zoom {z} is outside 1 to camera.max_zoom ({max_zoom})"
                            ));
                        }
                    }
                    Action::Camera(CameraMove::Focus {
                        target: Target {
                            name: f.focus,
                            ..Target::default()
                        },
                        zoom: f.zoom,
                    })
                }
                RawCameraMove::FocusAt(f) => {
                    if let Some(z) = f.zoom {
                        if !(1.0..=max_zoom).contains(&z) {
                            return Err(format!(
                                "camera zoom {z} is outside 1 to camera.max_zoom ({max_zoom})"
                            ));
                        }
                    }
                    Action::Camera(CameraMove::FocusAt {
                        spot: Spot {
                            x: f.focus_at[0],
                            y: f.focus_at[1],
                            window: f.window,
                        },
                        zoom: f.zoom,
                    })
                }
            },
            Some("highlight") => {
                let mark = match self.highlight.take().unwrap() {
                    RawHighlight::Name(name) => Mark::Target(target(name, &mut self)),
                    RawHighlight::Lines(lines) => {
                        if let Some((key, _)) = context.iter().find(|(_, set)| *set) {
                            return Err(format!(
                                "`{key}` does not apply to a highlight of lines, which \
                                 looks in the terminal pane and every app's text"
                            ));
                        }
                        Mark::Lines {
                            from: lines.from,
                            to: lines.to,
                        }
                    }
                };
                self.highlight(mark)?
            }
            Some("highlight_at") => {
                let rect = self.highlight_at.unwrap();
                if rect[2] <= 0.0 || rect[3] <= 0.0 {
                    return Err(
                        "highlight_at is [x, y, width, height], with a positive size".into(),
                    );
                }
                let window = self.window.take();
                self.highlight(Mark::Area { rect, window })?
            }
            Some("clear") => match self.clear.take().as_deref() {
                Some("highlights") => Action::ClearHighlights,
                Some(other) => {
                    return Err(format!(
                        "clear = \"{other}\" clears nothing; use clear = \"highlights\""
                    ));
                }
                None => unreachable!(),
            },
            Some(other) => unreachable!("unhandled action {other}"),
        };

        Ok(Step {
            action,
            hold,
            timeout,
        })
    }
}
