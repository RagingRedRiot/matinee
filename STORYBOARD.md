# Writing a storyboard

Start with an [example](examples/), run `matinee check demo.toml`, and use
`matinee probe demo.toml` to inspect the app's windows, names, and roles.
Then run `matinee run demo.toml` to check the real story before recording it.
`check` reports the control mechanisms the story declares; `probe` reports
what the app publishes at runtime. An unqualified assertion in a mixed story
can match terminal text and does not by itself prove a GUI result.

```toml
[demo]
output = "demo"                 # demo-light.gif and demo-dark.gif
size = [800, 600]                # logical points; scale defaults to 2
themes = ["light", "dark"]

[launch]
app = "gnome-calculator"
wait_for = "="

[[scene]]
caption = "Calculate a sum"
steps = [
  { click = "7" }, { click = "+" }, { click = "8" },
  { click = "=", hold = "1s" },
  { expect = "15", role = "text box" },
]
```

## Session and launch

Paths and fixture files are relative to the storyboard directory. The nearest
repository is mounted read-only at `/project`; without a repository only the
storyboard directory is shown. Commands start beside the storyboard. A fresh
home holds app state, and `/work` is writable for session results.

Use `[[seed]] run = "..."` to prepare state before launching apps. Seed commands
must succeed. Installed binaries outside `/usr` can be declared in
`[session] path = ["~/my-app/target/release"]`; those directories are mounted
read-only and added to `PATH`. `[session] mounts` adds other narrow read-only
host paths. Here `~` means the host home; in commands it means the fresh session
home. Network access needs `[session] network = true` and pasta.

`[launch]` accepts `app`, `args`, `name`, `wait_for`, `timeout`, and `x11`.
On Windows, `app` can also be a packaged app's ID, recorded with `--screen`;
see [docs/windows.md](docs/windows.md).
Use `[[launch]]` for multiple apps and `in = "launch-name"` in named steps.
Set `x11 = true` only for a launch needing Xwayland. Omit launches for a
terminal-only story.

## Actions and assertions

Each step has one action. Durations use seconds or milliseconds, such as
`hold = "1.5s"` and `timeout = "500ms"`.

| Action | Example and behavior |
| --- | --- |
| Named pointer | `{ click = "Save", role = "button", window = "Editor" }`; `hover` uses the same selectors |
| Typing and keys | `{ type = "hello" }`, `{ key = "ctrl+a" }`; target the focused GUI window |
| Wait and assert | `{ wait_for = "Ready", timeout = "10s" }`, `{ expect = "42", role = "text box" }` |
| Drag and slider | `{ drag = "Item", to = "Inbox" }`, `{ drag_slider = "Volume", to = 0.7 }` |
| Coordinates | `{ click_at = [20, 40], window = "Tk lab" }`, `{ drag_from = [20, 40], drag_to = [200, 40] }` |
| Terminal command | `{ run = "my-app status", exit = 0 }`; checks the exit status |
| Staged command | `{ terminal = "gh pr list", run = "cat fixtures/prs.txt" }`; displays the first command and runs the second |
| Playback | `{ terminal = "gh auth status", output = "fixtures/auth.txt" }`; declared canned output, no command runs |
| Highlight | `{ highlight = "Total", label = "The result", number = 1 }`; see [captions and highlights](#captions-and-highlights) |
| Clear highlights | `{ clear = "highlights" }` |
| Pause | `{ hold = "2s" }` |

Names match exactly; a trailing `*` matches a prefix, and a `*` at both
ends matches anywhere, as in a console window's whole buffer. Use `role`, `within`
(named ancestor), `near` (named nearby element), `window` (exact window title),
and `in` (launch name) to disambiguate. Nested matches such as a button and
its own label count as one target. Missing, ambiguous, hidden, and offscreen
targets fail with diagnostics. Scroll with key input or use an app's controls
before targeting an offscreen item.

`expect` and `wait_for` can match a text field's contents. Without selector
context they can also match terminal text; add `role` or `in` when an assertion
must prove GUI state. Coordinate steps use logical window points; negative
coordinates count from the right or bottom. They check bounds and require an
unambiguous window. [SUPPORT.md](SUPPORT.md) explains control and assertion limits.

To interact with a terminal program, use `{ run = "my-prompt", wait = false }`,
then `type` and `key` steps with `in = "terminal"` in a mixed story. A later
`{ wait_exit = "my-prompt" }` checks completion; otherwise status is checked
at story end. Only one terminal command can remain running at a time.

## Captions and highlights

A scene's `caption` shows while the scene plays. `[captions]` sets where,
and a scene's `caption_position` overrides it for that scene:

| `position` | Where the caption goes |
| --- | --- |
| `"bottom"` | Over the frame, at its bottom; the default |
| `"top"` | Over the frame, at its top |
| `"auto"` | Over the frame, on the half away from the newest highlight, or else the pointer |
| `"below"` | In a band added below the frame, where it covers nothing |
| `"above"` | In a band added above the frame |

```toml
[captions]
position = "below"

[[scene]]
caption = "The summary explains the result"
caption_position = "top"
steps = [ ... ]
```

A band makes the GIF taller by the band's height, for the whole recording,
when any scene's caption goes in it.

A `highlight` step marks part of the screen until the scene ends or a
`{ clear = "highlights" }` step. It marks:

- **text or a control, by name:** `{ highlight = "PASSED" }` marks the
  latest text in the terminal pane, else in an app's text, such as a
  Windows console window's (not read on Linux yet), else the control with
  that name. `role`, `within`, `near`, `window`, and `in` narrow it to a
  control, as for a click.
- **a section of lines:** `{ highlight = { from = "== Summary", to = "Total" } }`
  marks the lines from the latest holding `from` through the next holding
  `to`, as wide as the terminal pane or the text holding them.
- **a rectangle in a window:** `{ highlight_at = [x, y, width, height], window = "Main" }`,
  in points; negative `x` and `y` count from the right and bottom.

`style = "box"`, the default, outlines the mark; `"spotlight"` also dims the
rest of the frame. `label` adds a callout beside the mark, or below or above
it when the frame has no room beside. `number` adds a numbered marker, for a
sequence of steps. `color = "#dc2626"` draws this mark in its own color in
place of the style's accent, such as red for a problem and green for a good
result. The camera frames the mark, the step waits up to its
`timeout` (5 seconds by default) for the mark to show, and the mark shows for
a moment before the step's `hold` starts.

`matinee record --stills` also writes a PNG at the end of each highlight
step, after its hold, named `<output>-<theme>-01.png` and on, with the caption
and highlights fully shown. Stills suit instructions and docs that walk
through a screen one part at a time.

`[style]` sets how captions, key badges, highlights, and click ripples look,
for every theme, and `[style.light]` and `[style.dark]` override any of its
keys for one theme, since a color that reads on a dark recording often does
not on a light one. Every key is optional; one left out keeps its default.

```toml
[style]
accent = "#2563eb"           # highlights, their labels, and numbers
ripple = "#3b82f6"           # click ripples
spotlight_dim = 0.55         # how dark a spotlight makes the rest, 0 to 1
caption_text = "#0f172a"     # caption and badge text
caption_backing = "#ffffff"  # behind captions and badges over the frame
caption_border = "#cbd5e1"
band = "#f8fafc"             # a caption band's background
text_size = 18               # caption and label text, in points, 8 to 72

[style.dark]
accent = "#60a5fa"
caption_text = "#f8fafc"
caption_backing = "#1e293b"
```

Colors are `#rrggbb`. Text on a highlight's label or number is dark or white,
whichever reads on its color. Without `text_size`, text is sized from the
frame's width; captions do not wrap, so check that long ones still fit.

## Camera and output

`[camera]` supports `mode = "auto"` or `"fixed"`, `max_zoom`, `ease`, and
`reading_width` (800 pixels by default). Automatic framing keeps the target
readable while avoiding unnecessary camera moves. Set a smaller reading width
for recordings meant for narrow screens, then review the resulting GIF size.

Override framing with `{ camera = { focus = "Save", zoom = 1.8 } }`,
`{ camera = { focus_at = [20, 40], zoom = 1.8 } }`, or `{ camera = "overview" }`.
Coordinates can include a `window` selector. Captions, highlights, pointer
ripples, and key badges are drawn at output resolution after camera
cropping.

`[terminal]` supports `position = "bottom"` or `"right"`, `size`, and
`font_size`. A story that launches no app gives the terminal the whole
screen, unless it sets `position` or `size`: then the pane takes only that
part, and the rest is left for windows that its commands open. `[demo]` also supports `scale`, `fps`, and `lossy` (6 by default,
0 for exact LZW color handling; GIF palette quantization still applies).

`matinee record --fixed-too` also writes each GIF with the camera held on
the whole screen, as `<output>-<theme>-fixed.gif`. Both come from the same
run, so they show the same moments and differ only in the camera, which
makes it easy to judge what the moving camera adds and what it costs.

Recording prints each scene's duration and encoded size, including camera
movement cost, and warns above 2 MiB per theme with a fixed camera or 5 MiB
with movement. Review legibility at the actual display width.

`matinee record` discovers storyboards recursively. It validates all files and
checks output collisions before recording. Choose explicit story paths when
a folder also contains experimental fixtures or needs unavailable apps.
Use the [review recipe](CONTRIBUTING.md#reviewing-recordings) for side-by-side baseline artifacts.

For diagnosis, `matinee run demo.toml --artifacts target/assertions` and
`matinee record demo.toml --artifacts target/assertions` save a PNG for each
successful `expect` or `wait_for` and a `failure.png` if the running story fails. The images
come from that session without advancing its timeline or relaunching the app.
They show the full session at capture resolution and are grouped by story,
theme, and invocation so older failures cannot masquerade as new results.

To watch a story as it runs, add `--view` to `run` or `record`. A window on
your desktop, Wayland or Windows, shows the whole session, with the camera's view outlined
and the rest dimmed, so you can see where the camera goes and when. The
pointer shows once a step has used it, as in recordings. Captions, click
ripples, and key badges do not: recordings draw them when the GIF is
written, after the run. The window lives outside the isolation boundary:
matinee copies each frame out to it, and nothing inside the session can
reach your desktop. The session never waits for the window, so watching does
not change how a story runs or what is recorded. Closing the window leaves
the run going.

`matinee run` performs a story in its first theme. Programs in a matinee
session see the theme being recorded in `MATINEE_THEME`, and `run` uses that
theme when the story has it, so a run recorded inside another story matches
the take it appears in.
