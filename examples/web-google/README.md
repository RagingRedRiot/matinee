# Live Google search example

This storyboard opens the real Google homepage in Firefox kiosk mode, zooms
in on the search box, types “do penguins have knees”, and holds the completed
question briefly before zooming out and submitting it. It checks that the
results page appeared, then zooms in on the answer column and returns to the
full page.
The GIF shows only the webpage; the browser's tabs and address bar stay offscreen.

```sh
cargo build --release
target/release/matinee record examples/web-google/demo.toml
```

Firefox, network access, and `pasta` (from the `passt` package) are
required. This storyboard sets `[session] network = true`, which connects the
session to the outside through pasta while keeping Matinee's isolation; see
[Recording a webpage](../../docs/web.md). `launch.sh` creates a
disposable Firefox profile in the session to suppress
first-run prompts and expose page controls through AT-SPI; it never reads your
normal browser profile.

Google may vary its page by region, consent state, or time. The camera centers
a window-relative point on the answer column rather than a particular heading,
so wording changes do not break the camera move. If the page layout changes,
adjust `focus_at` in `demo.toml`. If this story fails,
run `matinee probe examples/web-google/demo.toml` to inspect the
controls available in your session. The checked result is the search page's
title and its “All” results link, not any particular answer or ranking.

## Other browsers

Vivaldi also completed this storyboard in a disposable profile. Launching it
with `--ozone-platform=wayland`, `--kiosk`,
`--app=https://www.google.com/`, and `--force-renderer-accessibility`
bypassed its welcome tour and kept browser controls out of the recording.
Its installed `/opt/vivaldi` directory needed a `session.mounts` entry.

Chrome completed the same storyboard with `--ozone-platform=wayland`,
`--kiosk`, `--app=https://www.google.com/`, `--no-first-run`, and
`--force-renderer-accessibility`. Its `/opt/google/chrome` directory needed
a `session.mounts` entry. The recording showed no browser controls or prompts.

Brave's fresh-profile analytics banner remained visible with `--no-first-run`
and `--app`. To suppress it, create the profile's `Local State` file before
launching Brave with `--user-data-dir` pointing at that profile:

```json
{"brave":{"p3a":{"enabled":false,"notice_acknowledged":true}}}
```

This disabled P3A and removed the banner in a full Brave recording. The
profile stays inside Matinee's disposable home; Brave's installed
`/opt/brave.com/brave` directory needs a read-only `session.mounts` entry.
