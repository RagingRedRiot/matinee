# matinee: vision

## The goal

Every project's demo is a script in its repository. One command performs
each script against the real software and records it, so demos are reviewed
like code, re-recorded on every release, and never drift from what the
software does.

matinee should record:

- **terminal programs**, as VHS does;
- **desktop GUI apps** on Linux, of any toolkit;
- **mixed stories**, where a command in a terminal changes what a window
  shows, which is how tools like cued and callboard are actually used.

## Philosophy

1. **Demos are code.** A demo is a storyboard file, versioned and reviewed
   with the project. Nothing is recorded by hand, so nothing needs a steady
   hand to redo.
2. **The real thing, never a mockup.** matinee drives the actual program
   against seeded, isolated state. Overlays draw only what a recording
   cannot show (the pointer, a terminal pane, captions), and anything an
   overlay shows happening must really happen.
3. **Find things by meaning, not by pixels.** Steps name what they act on,
   "click Inbox", through the accessibility tree, the same names a screen
   reader uses. Scripts survive layout changes, and a demo that cannot find
   its target fails loudly instead of clicking the wrong spot.
4. **The same script makes the same recording.** Fixed window size, scale,
   and theme; a pinned clock so "updated 3s ago" never ticks; seeded data in
   a throwaway home. A re-record changes only what the software changed.
5. **Small and sharp.** Recordings belong in READMEs, so they are encoded to
   load fast: only changed regions stored, holds as single long frames, one
   palette per clip, and video formats where GIF is too heavy.
6. **Both themes.** Every demo records in light and dark, for a README
   `<picture>` that matches the reader.
7. **Quiet to run.** One command, no desktop session, no interaction, no
   window flashing on screen. It runs the same on a laptop and in CI.
8. **Show the viewer where to look.** A large window in a README is a wall of
   small text. A virtual camera follows the action, easing in on the field
   being typed into, the slider being dragged, the button about to be
   clicked, and back out to show the result in context, the way a
   professional screencast does, and with no hand-editing.

## The storyboard

A sketch of the format, to be refined by building it:

```toml
[demo]
output = "docs/assets/demo"          # demo-light.gif, demo-dark.gif
size = [1200, 720]
themes = ["light", "dark"]
clock = "2026-10-01T09:00:00Z"        # pinned

[setup]                              # off camera, in a throwaway $HOME
run = ["callboard put reviews < fixtures/reviews.json", "callboard board add Inbox"]

[launch]
app = "callboard-gui"
wait_for = "Inbox"                   # an accessible name

[camera]
mode = "auto"                        # follow the action; "fixed" to never zoom
max_zoom = 2.2
ease = "0.6s"

[[scene]]
caption = "A script posts its feed"
steps = [
  { terminal = "gh search prs … | callboard put reviews", run = "callboard put reviews < fixtures/new-pr.json" },
  { wait_for = "Fix flaky upload retry" },
  { hover = "Fix flaky upload retry", hold = "2.4s" },
]

[[scene]]
caption = "Promote it to a todo"
steps = [
  { drag = "Drag to reorder or promote", near = "Fix flaky upload retry", to = "Todos", within = "Inbox" },
  { click = "Book the room for Thursday's retro" },
  { key = "ctrl+k" }, { type = "inbox" }, { key = "enter" },
]

[[scene]]
caption = "Settings"
steps = [
  { camera = { focus = "Theme", zoom = 1.8 } },   # explicit framing
  { drag_slider = "Font size", to = 0.7 },
  { camera = "overview" },                        # ease back out
]
```

## How it works (planned)

- **Session:** each demo runs in a headless desktop session (a nested Wayland
  compositor such as cage or weston in headless mode, or Xvfb), with an
  isolated `$HOME` and XDG directories and a pinned clock.
- **Targets:** the accessibility tree (AT-SPI on Linux) finds widgets by name
  and role, and gives their on-screen rectangles. egui, GTK, Qt, and browsers
  all publish one.
- **Input:** real pointer and keyboard events injected into the session,
  with eased pointer movement so it reads as a person.
- **Terminal:** a real pseudo-terminal rendered by matinee, either as the
  whole frame (a terminal demo) or as an overlay pane (a mixed story).
- **Capture and encode:** frames from the compositor, deduplicated, then
  GIF (palette per clip, changed rectangles only, holds merged into long
  frames) or WebM/MP4, plus a poster PNG.
- **In-process driver:** for apps that can link a test harness (egui with
  egui_kittest, as in the prototype), a precise mode with no compositor at
  all.

## The camera

The app is rendered at twice the output resolution (or more), and every
output frame is a crop of that canvas, so a zoomed view stays sharp instead
of upscaled. The camera is a rectangle over the canvas, moved by a smoothing
spring rather than cut, and it is planned from the script, not guessed from
the pixels:

- **Auto framing.** Before each step matinee knows the target's rectangle
  from the accessibility tree. Clicking a button, typing into a field, or
  dragging a slider frames that element with some context around it; a drag
  frames both ends; a step that changes much of the window (a pan, a new
  card, a dialog) eases back out to the whole window.
- **Calm motion.** The camera moves only when the next target would fall
  outside a comfortable inner region, holds still while the pointer works
  within it, never zooms during fast pointer travel, and waits a moment after
  an action so the result is seen in context. Zoom stays within limits set
  per demo.
- **Readable at the size people view it.** The goal is that the text being
  acted on is legible in a README at its rendered width, so the camera can
  pick the zoom level from the target's text size rather than a fixed factor.
- **Explicit control.** Scripts can override: frame a named element at a
  given zoom, follow the pointer, return to the overview, or fix the camera
  for a scene.
- **Overlays stay crisp.** The pointer, click ripples, keystroke badges, and
  captions are drawn after cropping, at output resolution, so they do not
  grow or blur with the zoom.

Zooming changes every pixel of a frame, which GIF stores expensively. Demos
with a busy camera are a better fit for WebM or MP4 wherever the page can
play video (which hosts play video embedded in a README, and how, is to be
checked when building this); GIF output should keep camera moves short and
few. matinee should report each output's size per scene so this trade-off is
visible.

## Plan

1. **Encoder and frame pipeline.** Lift the prototype's encoding into a
   library: transparent-margin cropping, frame deduplication into long
   frames, GIF and WebM output, light and dark. Measure against the
   prototype's 1.3 MB.
2. **Storyboard format and runner** for the in-process egui driver, so
   callboard's demo becomes a TOML file instead of Rust.
3. **Terminal driver**, so cued's VHS tape can move to matinee.
4. **Headless session driver** with AT-SPI targets and input injection, for
   any Linux GUI.
5. **Mixed stories**, terminal pane plus GUI, as one scene.
6. **The camera:** supersampled capture, spring-smoothed crop, auto framing
   from target rectangles, explicit camera steps, and overlays drawn after
   the crop. The in-process egui driver can prototype it first, since it
   already renders at any scale and knows every target's rectangle.
7. **CI recipe:** re-record on release and fail when a target goes missing.

## Not goals

- Screen recording of a live desktop. matinee performs scripts; it does not
  capture what you happen to do.
- Video editing. Captions and overlays, yes; timelines and effects, no.
- Faking output. If the demo shows it, the software did it.
