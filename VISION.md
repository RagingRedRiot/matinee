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
   cannot show (the pointer, a terminal pane, captions). What surrounds the
   program can be staged: a terminal step can show the command a user would
   type, such as a `gh` query, while running a fixture-backed stand-in, or
   show canned output for a service the demo should not depend on. Staging is
   declared in the storyboard, and whatever does run is checked.
3. **Find things by meaning, not by pixels.** Steps name what they act on,
   "click Inbox", through the accessibility tree, the same names a screen
   reader uses. Scripts survive layout changes, and a missing or ambiguous
   target fails loudly. Window-relative coordinates are an explicit fallback
   for apps without useful accessibility data.
4. **The same script tells the same story.** Fixed window size, scale,
   and theme; seeded data in a throwaway home; a camera that moves the same
   way every run. The clock is left alone: "updated 3s ago" ticking is part
   of what makes a demo look alive, so recordings are compared by the story
   they tell, not byte for byte.
5. **Small and sharp.** Recordings belong in READMEs, so they are encoded to
   load fast: only changed regions stored, holds as single long frames, one
   palette per clip. Video can be explored later if a story cannot stay small
   enough as a GIF.
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

[[seed]]                             # before app launch, in a throwaway $HOME
run = "callboard put reviews < fixtures/reviews.json"

[[seed]]
run = "callboard board add Inbox"

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
  # shown in the terminal / actually run
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

## How it works

- **Session:** each demo runs in a headless desktop session, with seeded
  state and a process and filesystem boundary that has been
  tested against apps that ignore XDG paths. The compositor and the
  boundary are built into matinee, not external tools.
- **Targets:** the accessibility tree (AT-SPI on Linux) finds widgets by name,
  role, and context where the app publishes one. An explicit window-relative
  fallback serves apps without a usable tree.
- **Input:** real pointer and keyboard events injected into the session,
  with eased pointer movement so it reads as a person.
- **Terminal:** a real pseudo-terminal rendered by matinee, either as the
  whole frame (a terminal demo) or as an overlay pane (a mixed story). A
  step can display a different command than it runs, or play back canned
  output, so demos need no live accounts or services.
- **Capture and encode:** frames from the compositor, deduplicated, then
  a GIF (palette per clip, changed rectangles only, holds merged into long
  frames). Video and a poster are possible later outputs.
- **In-process driver (deferred):** for apps that can link a test harness (egui with
  egui_kittest, as in the prototype), a precise mode with no compositor at
  all.

## The camera

The app is rendered at twice the output resolution (or more), and every
output frame is a crop of that canvas, so a zoomed view stays sharp instead
of upscaled. The camera is a rectangle over the canvas, moved by a smoothing
spring rather than cut, and it is planned from the script, not guessed from
the pixels. Camera and pointer motion run on the recording's frame timeline,
not wall-clock time, so the same story produces the same camera path:

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

Zooming changes every pixel of a frame, which GIF stores expensively. Keep
camera moves short and few, and measure whether the result is legible at
README width. matinee should report GIF size per scene so camera behavior
can be tuned against a concrete size budget. Video is a possible later
escape hatch, not a requirement for the first release.

## Plan

The core runner, storyboard authoring tools, external GUI and terminal drivers,
frame pipeline, camera, and encoder below are implemented. The CI recipe now
lives in [.github/workflows/ci.yml](.github/workflows/ci.yml); its hosted run
still needs confirmation. The in-process driver is deferred beyond the first
release. The current remaining gates are tracked in [RELEASE.md](RELEASE.md),
with support evidence in [SUPPORT.md](SUPPORT.md).

The original implementation order was:

1. **Carry the proven contracts into the runner.** Use callboard's GitHub GIFs
   as the output reference. Reproduce the tested isolation boundary with the
   chosen implementation and treat story correctness as the hard CI gate;
   attach visual changes for review.
2. **One complete story.** Seed before launch, run a real terminal command,
   check its exit status, select one GUI target, assert the result, and record
   it with both fixed and moving camera views.
3. **Storyboard and authoring.** Make target context, windows, timeouts,
   assertions, fixture paths, themes, and useful failure diagnostics part of
   one format. Let `probe` show what a writer can target.
4. **Drivers.** Support a terminal-only story, an in-process driver where an
   app can link one, and a headless Linux session for external GUI apps.
5. **Frame pipeline and camera.** Supersampled capture, calm automatic and
   explicit framing, overlays after cropping, frame deduplication, GIF
   output, and per-scene size reporting. Decide encoder details using
   moving-camera stories.
6. **CI recipe.** Re-record on release, fail on broken stories, and present
   visual changes for review.

## Not goals

- Screen recording of a live desktop. matinee performs scripts; it does not
  capture what you happen to do.
- Video editing. Captions and overlays, yes; timelines and effects, no.
- Faking the program. Inputs from outside it may be staged, but what the
  demoed program shows, it really did.
