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
| Pause | `{ hold = "2s" }` |

Names match exactly; a trailing `*` matches a prefix. Use `role`, `within`
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

## Camera and output

`[camera]` supports `mode = "auto"` or `"fixed"`, `max_zoom`, `ease`, and
`reading_width` (800 pixels by default). Automatic framing keeps the target
readable while avoiding unnecessary camera moves. Set a smaller reading width
for recordings meant for narrow screens, then review the resulting GIF size.

Override framing with `{ camera = { focus = "Save", zoom = 1.8 } }`,
`{ camera = { focus_at = [20, 40], zoom = 1.8 } }`, or `{ camera = "overview" }`.
Coordinates can include a `window` selector. Captions, pointer ripples, and
key badges are drawn at output resolution after camera cropping.

`[terminal]` supports `position = "bottom"` or `"right"`, `size`, and
`font_size`. `[demo]` also supports `scale`, `fps`, and `lossy` (6 by default,
0 for exact LZW color handling; GIF palette quantization still applies).
Recording prints each scene's duration and encoded size, including camera
movement cost, and warns above 2 MiB per theme with a fixed camera or 5 MiB
with movement. Review legibility at the actual display width.

`matinee record` discovers storyboards recursively. It validates all files and
checks output collisions before recording. Choose explicit story paths when
a folder also contains experimental fixtures or needs unavailable apps.
Use the [release review recipe](RELEASE.md) for side-by-side baseline artifacts.

For diagnosis, `matinee run demo.toml --artifacts target/assertions` and
`matinee record demo.toml --artifacts target/assertions` save a PNG for each
successful `expect` or `wait_for` and a `failure.png` if the running story fails. The images
come from that session without advancing its timeline or relaunching the app.
They show the full session at capture resolution and are grouped by story,
theme, and invocation so older failures cannot masquerade as new results.
