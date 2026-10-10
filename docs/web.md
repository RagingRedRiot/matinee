# Recording a webpage

Matinee can record a live site, including an app in a lab tenant. A storyboard
launches the browser as a normal GUI app against the remote URL, then uses
the same `click`, `type`, `key`, `expect`, and camera steps as a desktop demo.
Matinee records the browser's window; kiosk or app mode makes that window show
only the page. The [`web-google` storyboard](../examples/web-google/demo.toml)
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
| Firefox | `--kiosk -profile "$profile" URL`; set `MOZ_ACCESSIBILITY_ATK2=1` and seed the fresh profile's `user.js` to suppress first-run prompts. See [launch.sh](../examples/web-google/launch.sh). |
| Chrome | `--ozone-platform=wayland --kiosk --app=URL --no-first-run --force-renderer-accessibility --user-data-dir="$profile"`; mount `/opt/google/chrome` on this installation. |
| Vivaldi | `--ozone-platform=wayland --kiosk --app=URL --force-renderer-accessibility --user-data-dir="$profile"`; app mode bypassed its welcome tour. Mount `/opt/vivaldi` on this installation. |
| Brave | Use Chromium's Wayland app-mode flags and a fresh `--user-data-dir`; seed its `Local State` with P3A disabled to remove the analytics banner. Mount `/opt/brave.com/brave` on this installation. See the [browser notes](../examples/web-google/README.md). |

For Brave, this `Local State` content removed the banner in our disposable
profile:

```json
{"brave":{"p3a":{"enabled":false,"notice_acknowledged":true}}}
```

These browsers were tested as Wayland clients. The browser supplies the
page-only window and AT-SPI tree; Matinee supplies the isolated session,
input, camera, checks, and recording. An authenticated site needs its own
sign-in steps because the browser profile starts fresh.
