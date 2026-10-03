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
libxkbcommon. Apps that use OpenGL fall back to Mesa's software renderer.
matinee runs its own compositor, so it needs no desktop session or GPU.
Where user namespaces are not allowed, such as some CI runners,
`--no-isolation` runs stories on the host instead: they still get a fresh
home, environment, display, and buses, but can read your files and use the
network.

```sh
cargo build --release
target/release/matinee record examples/calculator/demo.toml
```

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

The callboard and cued examples need those projects built in checkouts beside
this one.

## Read next

- [VISION.md](VISION.md): philosophy, goals, and the plan.
- [BUILD-DECISIONS.md](BUILD-DECISIONS.md): working contracts, experiment
  results, and the architecture.

## License

[MIT](LICENSE)
