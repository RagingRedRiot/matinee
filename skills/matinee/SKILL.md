---
name: matinee
description: Write, check, and record matinee storyboards, scripted demos that matinee performs against a real program on Linux or Windows and records as light and dark GIFs for a README, with captions, highlights, and annotated stills for docs. Use when the user wants a demo GIF, screencast, README recording, or step-by-step screenshots of a terminal tool, a desktop app, or both; when they say "record a demo", "storyboard", "matinee", "re-record the GIF", "highlight", or "the demo is out of date"; and when a matinee run fails and needs diagnosing. Covers the storyboard TOML, targeting controls by accessible name, terminal staging, the camera and zoom, captions and highlights, Windows sessions and --screen, and the check, probe, run, record loop.
---

# matinee

matinee performs a storyboard, a TOML file kept beside the project, against
the real program in a private session, checks every step, and writes a GIF
per theme. It runs on Linux, where the session is a headless compositor of
its own, and on Windows, where it is a private desktop; see
[Windows](#windows). This skill is what a session needs to write storyboards
and drive matinee from the CLI.

## Commands

| Need | Use |
| --- | --- |
| Validate a storyboard without running it | `matinee check demo.toml` |
| See what steps can target in the running app | `matinee probe demo.toml` (`--all` includes hidden nodes) |
| Perform it and check every step, no GIF | `matinee run demo.toml` |
| Write the GIFs | `matinee record demo.toml` |
| Record every storyboard under a folder | `matinee record [folder]` (default: current folder) |
| A picture of the settled screen | `matinee snapshot demo.toml out.png [-- command...]` |
| A shell in the session the story gets | `matinee shell demo.toml [-- command...]` |

Options: `--artifacts <folder>` (run, record) saves a PNG at each passing
`expect` or `wait_for` and `failure.png` when a run fails. `--fixed-too`
(record) also writes `<output>-<theme>-fixed.gif` with the camera held still.
`--stills` (record) also writes `<output>-<theme>-01.png` and on, one after
each highlight step, for instructions and docs.
`--view` (run, record) opens a live window on the user's Wayland desktop; it is
for a person watching, so don't use it in an unattended session. It is not on
Windows yet. `--screen` (Windows: run, record, snapshot, probe) runs the
story's apps on the user's own screen; see [Windows](#windows).

Exit status: 0 success, 1 a failed story or error (the message says which
scene and step), 2 a usage mistake.

## The loop

1. **Start from something that works.** Copy the closest example in matinee's
   `examples/` (`terminal`, `calculator`, `mixed`, `callboard`, `cued`, or
   `powershell` on Windows) or the reference below. Put the storyboard in the user's project, conventionally
   `docs/demo.toml` with `output = "assets/demo"`, or wherever they keep
   README assets.
2. **`matinee check`** until it passes. It catches unknown keys, bad
   durations, steps with no action or two actions, and selector keys on the
   wrong action, without starting anything.
3. **`matinee probe`** to learn the real names and roles. Never guess a
   control's name from a screenshot or the source: use what probe prints.
4. **`matinee run --artifacts <scratch>/run`** until it passes. Read the saved
   PNGs to see what the session showed at each check, and `failure.png`
   after a failure. They show the whole session screen, without the camera.
5. **`matinee record`** once `run` passes. It records once per theme, so it
   takes about twice the story's length plus app startup. Run it as a
   background shell command when it will take more than a minute or two.
6. **Report the scene sizes it prints** and any budget warning, and ask the
   user to watch the GIFs. A GIF's first frame is not the recording; judge
   motion and legibility from the user's review or the artifact stills.

Write scratch storyboards and artifacts under a temporary directory, not the
user's project, until the user wants the storyboard kept.

## Storyboard reference

Paths are relative to the storyboard's folder. Durations are seconds or
milliseconds only: `"1.5s"`, `"600ms"`, `"60s"` (never `"1m"`).

```toml
[demo]
output = "assets/demo"          # writes assets/demo-light.gif and assets/demo-dark.gif
size = [800, 600]               # logical points
# scale = 2                     # 1-4; the app renders at size × scale, and the
                                # camera zooms up to it with real detail
# fps = 15                      # 1-60
# themes = ["light", "dark"]    # default both; at least one
# lossy = 6                     # 0 exact to 32; higher is smaller and streakier

[session]                       # all optional
# path = ["~/my-app/target/release"]   # host dirs, read-only, added to PATH
# mounts = ["/opt/google/chrome"]      # other host paths, read-only
# network = true                       # outside network through pasta; off by default

[[seed]]                        # before any app starts; each must exit 0
run = "mkdir -p ~/notes && cp fixtures/todo.txt ~/notes/"
# timeout = "60s"

[launch]                        # omit for a terminal-only story; [[launch]] for several
app = "gnome-calculator"
# args = ["--mode", "basic"]
# name = "calc"                 # for `in = "calc"`; defaults to the program name
wait_for = "="                  # an accessible name that shows the app is ready
# timeout = "20s"
# x11 = true                    # only for apps that need Xwayland (Tk, Motif)

[camera]                        # optional
# mode = "auto"                 # or "fixed"
# max_zoom = 2.2                # 1-8
# ease = "400ms"
# reading_width = 800           # the width readers see; smaller zooms more

[terminal]                      # optional
# position = "bottom"           # or "right"
# size = 200                    # points, along the pane's direction
# font_size = 15

[captions]                      # optional
# position = "bottom"           # "top", "auto", or a band: "below", "above"

[[scene]]
caption = "Add two numbers"     # optional, shown during the scene
# caption_position = "top"      # this scene's, in place of [captions]
steps = [
  { click = "7" },
  { click = "+" },
  { click = "8" },
  { click = "=", hold = "1s" },
  { expect = "15", role = "text box" },
]
```

Unknown keys are errors. Every storyboard needs at least one scene, and every
scene at least one step.

## Steps

Each step has exactly one action, plus optional selectors, `hold` (a pause
after the step), and `timeout`.

| Action | Example |
| --- | --- |
| Click, hover | `{ click = "Save", role = "button" }`, `{ hover = "Inbox" }` |
| Type, keys | `{ type = "hello" }`, `{ key = "ctrl+a" }` |
| Wait, assert | `{ wait_for = "Ready", timeout = "10s" }`, `{ expect = "42", role = "text box" }` |
| Drag | `{ drag = "Card", to = "Inbox" }`; `to_role` and `to_within` narrow the drop target |
| Slider | `{ drag_slider = "Volume", to = 0.7 }` (a fraction from 0 to 1) |
| Points | `{ click_at = [20, 40], window = "Tk lab" }`, `hover_at`, `{ drag_from = [20, 40], drag_to = [200, 40] }` |
| Terminal command | `{ run = "my-app status" }`, `{ run = "false", exit = 1 }` |
| Staged command | `{ terminal = "gh pr list", run = "cat fixtures/prs.txt" }` |
| Canned output | `{ terminal = "gh auth status", output = "fixtures/auth.txt" }` |
| Camera | `{ camera = { focus = "Theme", zoom = 1.8 } }`, `{ camera = { focus_at = [20, 40] } }`, `{ camera = "overview" }` |
| Highlight | `{ highlight = "Total", label = "The result", number = 1 }`, `{ highlight = { from = "== Summary", to = "Total" }, style = "spotlight" }`, `{ highlight_at = [20, 40, 200, 30], window = "Main" }` |
| Clear highlights | `{ clear = "highlights" }` |
| Pause | `{ hold = "2s" }` |

**Targets.** Names match the accessible name exactly; a trailing `*` matches a
prefix, and a `*` at both ends matches anywhere, as in a console window's
whole buffer (`{ wait_for = "*Done*", role = "document text" }`). Narrow with `role`, `within` (a named ancestor), `near` (a named
element nearby), `window` (the exact window title), and `in` (a launch name,
or `"terminal"`). A missing, ambiguous, hidden, or offscreen target fails with
a diagnostic listing what is there; scroll with keys or the app's own
controls first. `type` and `key` go to the focused window.

**Keys** are chords such as `enter`, `escape`, `tab`, `space`, `backspace`,
`delete`, arrows (`up`, `down`, `left`, `right`), `home`, `end`, `pageup`,
`pagedown`, `f1`-`f12`, letters, and digits, with `ctrl`, `shift`, `alt`, and
`super` joined by `+`.

**Assertions.** `expect` checks now (3 seconds by default) and `wait_for`
waits (10 seconds). Both read text fields' contents. Without `role` or `in`
they can also match terminal text, which proves nothing about the GUI, so
qualify any assertion that must prove a GUI result.

**Coordinates** are a fallback for apps with no usable accessibility tree,
when `probe` shows nothing to name. They are logical points within the window;
negative values count from the right or bottom. Name the window.

**Captions** show during their scene. `[captions] position` places them for
the story and `caption_position` for one scene: `bottom` (the default) or
`top` over the frame, `auto` over the frame away from the newest highlight or
the pointer, or a band `below` or `above` the frame, which covers nothing but
makes the GIF taller. Use a band when a caption would cover what the scene
shows, such as a terminal's last lines.

**Highlights** mark part of the screen until the scene ends or highlights
are cleared. A plain name marks the latest such text in the terminal pane,
then in an app's text (Windows console windows; not on Linux yet), then a
control with that name; selectors narrow it to a control. `{ from, to }`
marks a section of lines, and `highlight_at = [x, y, w, h]` a rectangle in a
window. `style` is `box` or `spotlight`; `label` adds a callout and `number`
a numbered marker. The camera frames each mark. Highlight what a reader
should look at, in the order they should read it. For docs that walk through
a screen, record with `--stills` to get a PNG after each highlight.

**Terminal stories.** `run` checks the exit status (0 unless `exit` says
otherwise). `terminal` shows one command while `run` runs another, for
example the `gh` query a user would type over a fixture that stands in for
it. `output` plays back a file with nothing run. To drive an interactive
program, start it with `{ run = "my-prompt", wait = false }`, send `type` and
`key` steps with `in = "terminal"`, and finish with
`{ wait_exit = "my-prompt" }`. Only one command can be left running.

## Rules for a truthful demo

- Stage only what surrounds the demoed program: commands that would need an
  account, a network service, or time. The program itself must really do
  what the GIF shows. Never fake its output with `output` or a stand-in.
- Seed every piece of data the demo shows. Don't depend on the user's files,
  accounts, or anything outside the project.
- End scenes on an assertion that proves the result, not only on a `hold`.
- Live clocks stay live. Don't try to freeze time; "updated 3s ago" ticking is
  expected, and recordings are not byte-identical between runs.

## The session

Commands start in the storyboard's folder. The project (the nearest folder
with `.git`) is read-only at `/project`; anything else must be declared in
`path` or `mounts`. The home folder is fresh and empty; write results to it
or to `/work`. In `[session]` paths, `~` is the host's home; in commands it is
the session's. Without `network = true` the session has only its own
loopback. Programs see the theme being recorded in `MATINEE_THEME`.

Build the program before recording, and point `path` at its build output, or
use an installed binary. matinee does not build anything.

## Windows

Terminal stories run PowerShell in the terminal pane, so `run` commands are
PowerShell. GUI stories need a signed-in, unlocked desktop session: from SSH
or a service, making the private desktop fails with "Access is denied", so
run matinee from a terminal in the user's session or a scheduled task that
runs only when the user is logged on.

- **The private desktop** (default) shows nothing on screen and leaves the
  user's mouse and keyboard alone. Clicks and typed characters act on
  controls through UI Automation, and what it can't do is posted to the
  window, so consoles, Chromium browsers, and WinUI apps work there. Keys
  other than typed characters (Enter, Tab, shortcuts) are only posted, which
  WinUI and WPF apps ignore: click into a field instead of reaching it by
  shortcut. Typing adds to the end of a field.
- **`--screen`** runs the apps on the user's own screen, with real input.
  It is needed for packaged (Store) apps, which open on the user's desktop,
  and WPF apps, which draw nothing on a private desktop. Only the story's
  windows are drawn or given input, but matinee moves the real pointer and
  types, so the user must leave the mouse and keyboard alone.
- **Packaged apps** launch by app ID, as `Get-StartApps` lists it, with
  `--screen`: `app = "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"`. They
  take no arguments, and matinee drives only an instance it started, so the
  app must not already be running.
- **Console windows**: launch `powershell.exe` as an app and it gets a
  console window of its own. Set its title in `args` to give `wait_for` a
  name. Match its output with `*text*` and `role = "document text"`.
- **Scale and zoom**: apps draw at the display's scaling, not the story's.
  Use `scale = 2` with the display at 200%, or `scale = 1` at 100%; matinee
  warns when they differ, since apps then show larger or smaller than the
  story lays them out. With `--screen`, `size` × `scale` must fit the
  display.
- **Browsers** started with a fresh profile on a machine signed in to a work
  or Microsoft account may sign in and show the account over the page; start
  them private, as with Edge's `--inprivate`.
- **Not yet on Windows:** `--view`, driving the system theme (light and dark
  look alike), and isolation: the session is a fresh profile, and commands
  can read the user's files and use the network whatever `network` says.

## Size and legibility

`record` prints each scene's seconds and kilobytes, and how much came while
the camera moved. It warns above 5 MiB per theme with a moving camera, or
2 MiB with a fixed one. Camera moves are the biggest cost; to shrink a GIF,
cut moves, shorten holds, or lower `max_zoom` before raising `lossy`. The
camera zooms no further than `scale`, so a story that needs close-ups sets
`scale = 2`. A story
meant for narrow screens sets a smaller `reading_width` and a larger
terminal `font_size`.

## Leave these to the user

- **`--no-isolation`.** It runs commands on the host, where they can read the
  user's files and use the network. Suggest it when user namespaces are
  blocked, but run it only when the user agrees.
- **`network = true`.** It lets the session reach other hosts, including the
  local network. Add it only for a story that needs a live site, and say so.
- **Host access through `path` and `mounts`.** Mount the narrowest folder
  that works, never the home folder.
- **Publishing.** Recordings show whatever was on the session's screen. The
  user watches a GIF before it replaces one in a README or gets committed.
- **Someone else's storyboard.** Its commands run with the user's privileges.
  Read it before running it, and point out anything that deletes, sends, or
  reaches outside the project.
- **Host settings.** Don't change sysctls, AppArmor, or installed packages;
  tell the user what is missing. On Windows, don't change the display's
  scaling or theme; tell the user what the story needs.
- **`--screen`.** It takes over the user's pointer and keyboard while it
  runs. Use it when the story needs it, and only when the user agrees.

## Troubleshooting

- **User namespaces are not allowed.** On Ubuntu,
  `kernel.apparmor_restrict_unprivileged_userns` blocks them. Tell the user;
  `--no-isolation` is their call.
- **A target isn't found or is ambiguous.** Run `probe` and copy the name and
  role exactly. Add `within`, `near`, or `window` for repeated names.
- **The app publishes no tree.** Probe shows a window but no controls. Use
  coordinate steps and a qualified `expect` on something observable, such as
  a file a `run` step checks.
- **Qt 5 on Wayland shows nothing.** Ubuntu 24.04's Qt 5 lacks the patch it
  needs; launch with `x11 = true` (needs Xwayland).
- **A story needing the network fails at start.** pasta, from the `passt`
  package, is missing.
- **A command can't find the program.** The session sees only `/usr`, the
  project, and declared `path` and `mounts`. Add the build folder to `path`.
- **The run passes but the GIF is too big.** See Size and legibility.
- **Windows: "creating a desktop: Access is denied".** matinee is running
  from SSH or a service; run it in the user's desktop session.
- **Windows: a packaged app is refused, or "no new process of it started".**
  Add `--screen`, and close the app if it is already running.
- **Windows: a WPF app's window is blank.** Use `--screen`.
- **Windows: clicks or keys do nothing in a WinUI or WPF app.** Click into
  the field rather than using a shortcut, or use `--screen`.
- **Windows: the scale warning.** Ask the user to set the display's scaling
  to the story's (200% for `scale = 2`), or change `scale` to match it.

Read `STORYBOARD.md` in matinee's repository for the full format, and
`SUPPORT.md` for what each toolkit supports.
