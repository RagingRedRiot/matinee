# matinee

**Scripted, reproducible demos of your software, recorded from the real thing.**

A matinee is a showing put on for an audience. This tool puts one on for
your project: you write the demo once as a script, and matinee performs it
against the real program, with real, seeded data, and records it as a GIF,
with a camera that eases in on whatever is being clicked, typed, or
dragged so small controls read clearly in a large window. Change the program,
run matinee again, and the demo is current.

```sh
matinee record demo.toml        # perform the demo and write a GIF per theme
matinee record                  # record every storyboard under this folder
matinee run demo.toml           # perform it and check every step, without recording
matinee check demo.toml         # read the storyboard and report mistakes, without running it
matinee probe demo.toml         # list the windows and what steps can target in them
matinee snapshot demo.toml shot.png   # a picture of the screen once it settles
matinee shell demo.toml         # a shell inside the isolated session the demo gets
```

> **Status:** working on Linux, not yet released. It records the README demos
> in [`examples/`](examples/), including [callboard](https://github.com/RagingRedRiot/callboard)'s
> and [cued](https://github.com/RagingRedRiot/cued)'s, from the real apps.

## Why

Hand-recorded demos go stale the day the interface changes, take a steady
hand to redo, and are never quite the same twice. Terminal tools have
[VHS](https://github.com/charmbracelet/vhs) for this. Desktop apps, and
projects that are part terminal, part GUI, have nothing comparable. matinee
aims to be that: one storyboard format, one command, for terminals and GUIs
alike.

## What it needs

Linux with unprivileged user namespaces (on Ubuntu, see
`kernel.apparmor_restrict_unprivileged_userns`), plus what any desktop that
runs GTK or Qt already has: `dbus-daemon`, at-spi2-core, libpixman, and
libxkbcommon. Stories that opt into network access also need `pasta`, from
the `passt` package, and apps launched with `x11 = true` need Xwayland. Apps that use OpenGL fall back to Mesa's software
renderer. matinee runs its own compositor, so it needs no desktop session or
GPU.
Where user namespaces are not allowed, such as some CI runners,
`--no-isolation` runs stories on the host instead: they still get a fresh
home, environment, display, and buses, but can read your files and use the
network.

```sh
cargo build --release
target/release/matinee record examples/calculator/demo.toml
```

## What the isolation is for

matinee runs each demo in an isolated session so that personal and
identifying details about you and your machine cannot end up in a recording:
your files, home folder, user and host names, other apps' windows, open
sessions, and logins. A demo shows only the project and what the storyboard
sets up.

The isolation is not a sandbox for running untrusted or dangerous software.
Do not use matinee to detonate malware or contain anything malicious. It is
not built or tested to withstand a program trying to escape, and it would not
hold up as a safety net.

## Recording a webpage

Matinee can record a live site, including an app in a lab tenant. A storyboard
launches the browser as a normal GUI app against the remote URL, then uses
the same `click`, `type`, `key`, `expect`, and camera steps as a desktop demo.
Matinee records the browser's window; kiosk or app mode makes that window show
only the page. The [`web-google` storyboard](examples/web-google/demo.toml)
is a complete example: it zooms in while typing a question, zooms out to
submit it, checks the results, then zooms in on the answer. Run it with
`matinee record examples/web-google/demo.toml`.

Live sites need an explicit network opt-in in the storyboard:

```toml
[session]
network = true

[launch]
app = "bash"
args = ["./launch.sh"]
wait_for = "Search"
```

Network access is off by default. The opt-in needs
[pasta](https://passt.top/), from the `passt` package, which connects the
session's private network to the outside. The session keeps its private
filesystem, processes, display, and buses, and cannot reach services on your
machine's loopback, but it can reach other hosts, including your local
network. Without pasta, such a story fails; `--no-isolation` runs it on the
host network instead, without isolation. The launch script creates a fresh
browser profile inside that session, so a demo does not use your regular
browser profile or its logins. Browsers
installed outside `/usr` may need their application directory listed in
`session.mounts` read-only. Matinee finds named page controls through AT-SPI;
use `matinee probe examples/web-google/demo.toml` to inspect them, or
window-relative coordinates when a control has no usable accessible name.

We recorded the Google storyboard with these browsers on Linux:

| Browser | Page-only launch and profile setup |
| --- | --- |
| Firefox | `--kiosk -profile "$profile" URL`; set `MOZ_ACCESSIBILITY_ATK2=1` and seed the fresh profile's `user.js` to suppress first-run prompts. See [launch.sh](examples/web-google/launch.sh). |
| Chrome | `--ozone-platform=wayland --kiosk --app=URL --no-first-run --force-renderer-accessibility --user-data-dir="$profile"`; mount `/opt/google/chrome` on this installation. |
| Vivaldi | `--ozone-platform=wayland --kiosk --app=URL --force-renderer-accessibility --user-data-dir="$profile"`; app mode bypassed its welcome tour. Mount `/opt/vivaldi` on this installation. |
| Brave | Use Chromium's Wayland app-mode flags and a fresh `--user-data-dir`; seed its `Local State` with P3A disabled to remove the analytics banner. Mount `/opt/brave.com/brave` on this installation. See the [browser notes](examples/web-google/README.md). |

For Brave, this `Local State` content removed the banner in our disposable
profile:

```json
{"brave":{"p3a":{"enabled":false,"notice_acknowledged":true}}}
```

These browsers were tested as Wayland clients. The browser supplies the
page-only window and AT-SPI tree; Matinee supplies the isolated session,
input, camera, checks, and recording. An authenticated site needs its own
sign-in steps because the browser profile starts fresh.

## Recording an X11 app

Apps run on Wayland. An app that cannot, such as one built with Tk or Motif,
asks for X11 in its launch:

```toml
[launch]
app = "python3"
args = ["tk_lab.py"]
x11 = true
```

matinee then starts Xwayland, from the `xwayland` package, and acts as its
window manager: X11 windows fill the window area like any other, dialogs
are centered, and clicks, typing, and the camera work the same. Only that
launch gets `DISPLAY`; other apps stay on Wayland, so an app that supports
both never picks X11 by accident. X11 apps draw at one pixel per point and
are scaled up, so they look slightly softer than Wayland apps at
`scale = 2`. Many X11-only toolkits publish no accessibility tree; target
their controls with window-relative points such as `click_at`. See the
[Tk fixture](tests/fixtures/x11/demo.toml).

## Examples

- [`calculator`](examples/calculator/demo.toml): clicks and typing in GNOME
  Calculator, checking what it shows.
- [`terminal`](examples/terminal/demo.toml): a terminal-only demo, with a
  staged `gh` command backed by a fixture and canned output.
- [`mixed`](examples/mixed/demo.toml): a terminal pane below an app window.
- [`callboard`](examples/callboard/demo.toml): callboard's README story: a
  script posts a feed, an item is dragged onto a board, and Ctrl+K jumps back
  after panning.
- [`cued`](examples/cued/demo.toml): cued's status window following a release
  pipeline as a graph, with a loop, a rollback, and a success.
- [`web-google`](examples/web-google/demo.toml): a live Google search recorded
  as a page-only demo. The included launcher uses Firefox; Chrome, Vivaldi,
  and Brave were also tested. See its [setup notes](examples/web-google/README.md).

The callboard and cued examples need those projects built in checkouts beside
this one.

## Read next

- [VISION.md](VISION.md): philosophy, goals, and the plan.
- [BUILD-DECISIONS.md](BUILD-DECISIONS.md): working contracts, experiment
  results, and the architecture.

## License

[MIT](LICENSE)
