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
    pub scenes: Vec<Scene>,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanePosition {
    Bottom,
    Right,
}

impl Story {
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
    /// The pinned wall clock, as written (RFC 3339).
    pub clock: Option<String>,
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
    /// Outside network access, which is off unless asked for.
    pub network: bool,
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
    pub network: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    pub mode: CameraMode,
    pub max_zoom: f64,
    pub ease: Duration,
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
    },
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
    Type(String),
    /// A key chord such as `ctrl+k` or `enter`.
    Key(String),
    Camera(CameraMove),
    /// Only a hold.
    Pause,
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
    #[serde(default)]
    scene: Vec<RawScene>,
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
    clock: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSeed {
    run: String,
    timeout: Option<String>,
    #[serde(default)]
    network: bool,
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
    network: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCamera {
    mode: Option<CameraMode>,
    max_zoom: Option<f64>,
    ease: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSession {
    #[serde(default)]
    mounts: Vec<String>,
    #[serde(default)]
    path: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScene {
    caption: Option<String>,
    steps: Vec<RawStep>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawStep {
    run: Option<String>,
    terminal: Option<String>,
    output: Option<PathBuf>,
    exit: Option<i32>,
    wait_for: Option<String>,
    expect: Option<String>,
    hover: Option<String>,
    click: Option<String>,
    drag: Option<String>,
    drag_slider: Option<String>,
    to: Option<RawTo>,
    to_role: Option<String>,
    to_within: Option<String>,
    #[serde(rename = "type")]
    type_text: Option<String>,
    key: Option<String>,
    camera: Option<RawCameraMove>,
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
enum RawTo {
    Name(String),
    Fraction(f64),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawCameraMove {
    Named(String),
    Focus(RawFocus),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFocus {
    focus: String,
    zoom: Option<f64>,
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
        let demo = Demo {
            output: d.output,
            size: d.size,
            scale,
            fps,
            themes,
            clock: d.clock,
        };

        let seeds = self
            .seed
            .into_iter()
            .enumerate()
            .map(|(i, s)| {
                Ok(Seed {
                    run: s.run,
                    timeout: duration(&format!("seed {}: timeout", i + 1), s.timeout)?,
                    network: s.network,
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
                network: l.network,
            });
        }

        let c = self.camera.unwrap_or(RawCamera {
            mode: None,
            max_zoom: None,
            ease: None,
        });
        let max_zoom = c.max_zoom.unwrap_or(2.2);
        if !(1.0..=8.0).contains(&max_zoom) {
            return Err("camera.max_zoom must be from 1 to 8".into());
        }
        let camera = Camera {
            mode: c.mode.unwrap_or(CameraMode::Auto),
            max_zoom,
            ease: duration("camera.ease", c.ease)?.unwrap_or(Duration::from_millis(600)),
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
            scenes.push(Scene {
                caption: raw.caption,
                steps,
            });
        }

        let session = Session {
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

        Ok(Story {
            dir,
            demo,
            seeds,
            launches,
            camera,
            session,
            terminal,
            scenes,
        })
    }
}

impl RawStep {
    fn check(mut self, launches: &[Launch], dir: &Path, max_zoom: f64) -> Result<Step, String> {
        let hold = duration("hold", self.hold.take())?;
        let timeout = duration("timeout", self.timeout.take())?;

        let actions: Vec<&str> = [
            ("run", self.run.is_some()),
            ("terminal", self.terminal.is_some() && self.run.is_none()),
            ("wait_for", self.wait_for.is_some()),
            ("expect", self.expect.is_some()),
            ("hover", self.hover.is_some()),
            ("click", self.click.is_some()),
            ("drag", self.drag.is_some()),
            ("drag_slider", self.drag_slider.is_some()),
            ("type", self.type_text.is_some()),
            ("key", self.key.is_some()),
            ("camera", self.camera.is_some()),
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
        only_with("output", self.output.is_some(), &["terminal", "run"])?;
        only_with("to", self.to.is_some(), &["drag", "drag_slider"])?;
        only_with("to_role", self.to_role.is_some(), &["drag"])?;
        only_with("to_within", self.to_within.is_some(), &["drag"])?;
        let targeted = [
            "wait_for",
            "expect",
            "hover",
            "click",
            "drag",
            "drag_slider",
        ];
        let context = [
            ("role", self.role.is_some()),
            ("within", self.within.is_some()),
            ("near", self.near.is_some()),
            ("window", self.window.is_some()),
            ("in", self.app.is_some()),
        ];
        for (key, set) in context {
            only_with(key, set, &targeted)?;
        }

        // A plain `wait_for` or `expect` may be text in the terminal pane.
        let terminal_text =
            matches!(action, Some("wait_for" | "expect")) && context.iter().all(|(_, set)| !set);
        let targets_app = action.is_some_and(|a| targeted.contains(&a)) && !terminal_text
            || matches!(self.camera, Some(RawCameraMove::Focus(_)));
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
                }
            }
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
            Some("type") => Action::Type(self.type_text.take().unwrap()),
            Some("key") => Action::Key(self.key.take().unwrap()),
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
