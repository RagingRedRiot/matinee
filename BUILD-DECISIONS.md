# matinee: build decisions

The contracts matinee keeps, the experiments behind them, and how it is built.
The experiments ran on an early prototype that is not part of this repository,
and their measurements come from one machine. They informed the design; the
tools used to run them were not adopted.

## What a successful run means

A storyboard must execute the commands it runs, check their exit status, find
one unambiguous target for each semantic action, and assert the resulting
application state. A missing target, repeated name with no context, failed
command, or unmet assertion fails the run with a useful diagnostic. An
explicit expected exit code is allowed for a story about failure.

A terminal step may display a different command than it runs, or play back
canned output, so a demo of `gh`-fed data needs no live account or service.
This staging is declared per step in the storyboard. The demoed program
itself is never staged: its GUI or terminal result comes from a real run.

The reproducibility promise is **the same observable story** under a
pinned app build, fixtures, theme, locale, screen geometry, and fonts. The
clock is deliberately left running: live labels such as "updated 3s ago" are
part of what a demo shows. Byte-identical GIFs across machines are not
promised; that would need the app clock, animation scheduling, terminal
rendering, compositor frames, and encoders all controlled. CI fails on story
correctness, and a visual difference produces a reviewable artifact rather
than being silently accepted or treated as an application failure.

## What the experiments show

An 800×480 logical screen at scale 2 recorded a mixed GTK and terminal story:
prelaunch seeding created a file, a real command changed it, and an accessible
GUI control changed from `waiting` to `ready`. A command that exited 1 failed
the run. The existing calculator and cued stories also completed with command
status checks and stricter target selection.

Three optimized fixed-camera runs of the mixed story each captured 72 frames.
The two underlying GUI images matched at all 72 frame indices. The final
frames matched at 70 of 72 indices, but the GIFs still differed: 19,710,
19,782, and 19,899 bytes. Three moving-camera runs also captured 72 frames:
the GUI matched at 71 indices, while the final frames matched at only 35.
Those GIFs were 112,426, 115,509, and 114,476 bytes and had different
hashes. Camera easing and real-time sampling vary even when the app's resting
state is stable. So CI checks the observable story and saves assertion
stills and visual differences for review, rather than requiring identical GIF
hashes.

In a separate optimized comparison with the same story, a fixed view produced
a 19 KB GIF and automatic 1.8× framing produced a 109 KB GIF. Capture lateness
at the 95th percentile was about 5 ms in the optimized camera run; a debug
build fell almost a second behind. The camera improves the tiny GUI label's
readability but makes GIF size depend strongly on movement. Tune camera
behavior and GIF encoding using moving stories, not only still frames.
A local H.264 version of the moving clip was 18 KB at 800×480, versus 112 KB
for its light-theme GIF; this is a size comparison, not a reason to make
video a first-release requirement.

At 400-pixel README width, the fixed view's GUI label and the prototype's
default terminal text were hard to read. A story variant with 2.2× GUI framing
and a 22-point terminal font made both clearer at that width; its GIF was
119 KB. This is a layout choice per story, not an encoder fix. Test each GIF
at the width where readers will actually see it.

## Time, themes, and interactions

A GTK fixture displayed a frozen `2026-10-01 09:00:00 UTC` while a separate
timer reached an asserted `timer active` state. It recorded visibly different
light and dark output with `GTK_THEME`. A Qt 5 fixture under Xwayland held the
same wall-clock time while its visible timer count advanced from 11 to 21.
Passing `GTK_THEME` through a GTK platform theme made its two images identical;
isolated Qt palette profiles produced distinct light and dark output. The
clock preload and Qt palette tool were experiment mechanisms, not selected
matinee dependencies. The Qt clock fixture published only its accessibility
application root, so its time/theme result is visual, not a semantic-control
success. Other Qt apps had usable trees in earlier probes. Other toolkits'
themes are tracked in [SUPPORT.md](SUPPORT.md).

One GTK action fixture completed focus and typing, drag and drop, scrolling to
an initially offscreen row, a modal dialog, a second window, clipboard copy
and paste, and selection of a seeded file in a file chooser. Every action
asserted the resulting label or control. A placeholder alone gave the entry
no accessible name; an explicit label did. One large pointer-axis event did
not scroll, while wheel-sized events moved the last row into view. Dialog and
second-window actions needed an exact per-step window title. The file chooser
opened on Recent, so selecting the seeded file first required navigation to
Home. Direct location-entry typing was unreliable in this trial; semantic
selection worked. This establishes the mechanism in GTK, not cross-toolkit
coverage for each action.

The offscreen row was still present in AT-SPI and initially had coordinates
well beyond the 800×480 window. Without a visibility check, a semantic click
could have gone to the wrong place. matinee refuses a target outside its
window and says to scroll it into view; the same action succeeds after an
explicit scroll.

## Support boundary

`probe` output and story validation distinguish three kinds of control:

1. **Semantic control:** an accessible target can be located uniquely by
   name, role, app, named ancestor, and window. This is the preferred mode.
2. **Explicit control:** a storyboard opts into window-relative coordinates
   when the app has no usable accessibility tree. The coordinate and window
   are reviewable, and a missing or ambiguous window fails the run.
3. **Capture only:** an app can be shown alongside real terminal actions, but
   its GUI cannot be clicked or asserted by meaning. Such scenes cannot claim
   a verified GUI result without another observable assertion.

`check` reports which a story uses. matinee does not claim semantic control
of every Linux toolkit: whether a tree exists depends on the app's build and
settings.

## Isolation contract

The isolation exists so that personal and identifying details about the user
and their machine cannot end up in a recording: host files, the home folder,
user and host names, other windows, sessions, and logins. It is not a sandbox
for untrusted or malicious software, such as malware detonation, and must not
be presented as one. It is not designed or tested to withstand a program
trying to escape, and it would not hold up as that kind of safety net.

The prototype runner used a fresh home and runtime directory, started GUI apps
in that home, disabled their network by default, and reaped detached children.
That was useful state isolation, not a containment boundary: apps could still
access host paths, fixed socket locations, and some system services. Its
per-process network namespaces also prevented terminal and GUI apps from
talking over localhost. The current built-in boundary addresses these gaps and
is covered by `tests/boundary.rs`, which CI runs on every change.

A local feasibility experiment wrapped the **entire** runner and its children
in one private user, mount, PID, and network boundary. A first version with a
read-only host root still exposed readable credentials; that is insufficient.
A narrower version mounted `/usr` read-only, selected system configuration
files, a synthetic machine ID, and the prototype's own files read-only, with
private `/tmp`, `/run`, and `/var`. A known host home path, host `/tmp` sentinel,
host `/etc/hostname`, and system-bus socket were invisible; a write to the
prototype mount and an external network connection failed. The mixed GTK
story still ran, including a real loopback TCP message from terminal to a
service that updated the GUI. A detached child writing a heartbeat stopped
after both a successful run and an intentional command failure. The experiment
used `bwrap` as a stand-in; matinee now creates the same boundary itself (see
Architecture).

What a story sees of the host: the project, meaning the nearest folder above
the storyboard that holds `.git`, read-only at `/project`, with commands
starting in the storyboard's folder within it. Without a repository, or
when that folder would be the home folder or `/`, only the storyboard's
folder is shown. Anything else, such as an installed binary, is declared in
`[session] mounts` and shown read-only at its host path. Results come back
through a writable `/work`.

`[session] network = true` gives the session outside access, including DNS
and HTTPS, for stories such as web demos. It is off by default; without it,
the session has only its own loopback interface. The opt-in keeps the
session's private network namespace and connects it to the outside with
`pasta`, from the passt project, which carries traffic through the host's
sockets. Like Xwayland, pasta is an optional dependency that only these
stories need; without it, such a story fails and suggests `--no-isolation`.

Sharing the host's network namespace instead was tried and rejected: the
host's abstract Unix sockets belong to that namespace, and a process in the
session could open the host's X display. Pasta runs with `--no-map-gw` and no
port forwarding in either direction, so the host's loopback services stay out
of reach and the session's own localhost stays private. DNS goes to a
forwarding address that pasta relays to the host's resolver, since that
resolver often listens on the host's loopback. Other hosts, including those on
the local network and the host's own non-loopback addresses, are reachable.

A temporary `HOME` alone must never be presented as full isolation.

`--no-isolation` is that kind of opt-out, for systems that do not allow
unprivileged user namespaces. It skips the namespaces only: the session,
steps, and recording are unchanged, and commands still get a fresh home,
runtime folder, and environment. Leftover processes are stopped by a
subreaper rather than PID 1. matinee warns on every use that host files and
the network are reachable.

## Output contract

GIF is the primary output, as separate light and dark files for a README
`<picture>` element, the pattern
[callboard's README](https://github.com/RagingRedRiot/callboard) uses. matinee
reports GIF size per scene against a practical budget, and legibility at README
width is checked by eye. A particularly motion-heavy story may motivate
optional video support later.
`record` warns when a GIF is over budget: 2 MiB per theme with a still
camera, and 5 MiB when the camera moves. Size alone is not a pass: the
text acted on must remain legible at the width the recording is read at,
800 pixels by default (about a README column on a desktop), or what the
story sets as `[camera] reading_width`. Measured on callboard's demo, text
reads at 800 pixels; at 400 only the captions do, so a story meant for
narrow screens sets a smaller reading width and accepts more zoom.

The camera's moves are most of a moving GIF's size: a move changes nearly
every pixel of each frame it spans (callboard's demo measured 3.1 of
4.5 MB in 42 such frames). So the camera stays put whenever it can. It
zooms only as far as text needs to read at the reading width, so an
output no wider than that never zooms on its own. It stays while the
next target is comfortably in view and readable. Each move eases over
0.4 seconds by default and is stored at half the frame rate. With these,
callboard's 40-second demo was about 4 MB per theme, against 2.4 MB with a
fixed camera. Three encoder changes have since brought it to about 2.4 MB
per theme, with captions, click ripples, and key badges included: pixels
that did not change inside a changed rectangle are stored as transparent,
or as their own color where that continues a run; the palette is chosen by
median cut and k-means from how long each color is on screen; and LZW
compression may let a color move by up to 6 in any channel (`[demo] lossy`,
0 for exact), which looked the same in side-by-side frames (at 12, streaks
showed in flat light areas). `record` prints each scene's seconds and kilobytes, and how much of that came
while the camera moved.

Light and dark come from the session, not the app: matinee serves the desktop
portal's settings on the session bus (the `color-scheme` that libadwaita,
GTK 4, and Qt 6 follow) and writes the same GNOME interface settings to a
GSettings keyfile, which GTK reads outside a sandbox. Neither bus starts
services on demand, so the session does not depend on what the host has
installed. This puts GTK 4 and libadwaita into both themes; GTK 3, Qt 5,
Qt 6, and Electron still need checking (see [SUPPORT.md](SUPPORT.md)). A terminal theme alone does not
change the GUI. Reduced-motion
behavior can use a still poster where a host supports it, without blocking
GIF output.

## Architecture

The point of matinee is control over every stage of a recording, so the core
pipeline is built into the binary rather than borrowed from external tools.
The experiments used sway, ffmpeg, and `bwrap` as stand-ins; none of them is
a runtime dependency.

- **Compositor:** a headless Wayland compositor built into matinee. Owning
  the compositor gives matinee control of frame timing and output geometry.
- **Isolation:** matinee creates the user, mount, PID, network, IPC, and
  hostname namespaces itself, with the mount policy described above.
- **X11 apps:** Xwayland, started only when a launch sets `x11 = true`,
  with matinee as its window manager through smithay's X11 support. It is
  an optional runtime dependency, like pasta. X11 is chosen per launch,
  never per session, so apps that also support Wayland stay on Wayland; it
  exists for apps that require X11. X11 apps draw at one pixel per point
  and are scaled up: most X11-only toolkits cannot be told to draw larger,
  and a correct size matters more than sharpness. matinee sets the root
  window's `AT_SPI_BUS` property, as desktops do, because Qt 5 finds the
  accessibility bus there; its other route, asking the session bus, makes
  the registration call before its connection is up and loses it. On
  Wayland, Qt 5 has only that route unless its build carries KDE's patch
  for `AT_SPI_BUS_ADDRESS` (Ubuntu 24.04's 5.15.13 does not).
- **D-Bus and accessibility:** a private session bus and accessibility bus,
  each the system's `dbus-daemon`, plus the system's `at-spi2-registryd`.
  Any desktop that runs AT-SPI already has both. matinee answers
  `org.a11y.Bus` and `org.a11y.Status` itself. An in-process bus (busd) was
  tried and dropped: zbus, which it is built on, rewrites a message whose
  only argument is a struct as separate arguments, and the AT-SPI registry
  crashes on the first app that registers.
- **Encoder:** a GIF encoder in matinee, built for this output: changed
  regions only, with unchanged pixels inside them transparent, holds merged
  into long frames, one palette per clip from its colors' time on screen,
  slightly lossy LZW of its own, and per-scene size reporting. The `gif`
  crate writes only the file's structure.
- **Timeline:** recording time is a frame index, not wall-clock time. The
  camera path, eased pointer motion, and capture schedule are planned on
  that timeline, so the same story yields the same camera path. Waiting for
  the app (window appearance, assertions, timeouts) still uses real time.
- **Platform layer:** the compositor, the boundary, the buses, the
  accessibility client, the shell behind the terminal pane, and the
  viewport window are the parts that depend on the desktop platform. They
  live under `src/platform`, one implementation per platform selected at
  compile time, behind the same names: a display that renders frames and
  takes input, a boundary that runs commands, an accessibility tree, and a
  shell. The storyboard, session, runner, camera, overlays, and encoder are
  the same everywhere. Linux has the full pipeline. Windows runs terminal
  stories through PowerShell on a pseudo console, and GUI stories by
  launching apps onto a private desktop of the session's own, in a job
  that ends them with the session, placing and capturing each window with
  `PrintWindow`, posting them input, and reading their UI Automation trees
  for named targets. Packaged apps, which Windows opens on the user's own
  desktop, and WinUI and WPF apps, which ignore posted input, need `--screen`
  instead: apps open on the user's screen and take real input through the
  input queue, sent only when it lands on the story's windows, and
  packaged apps start through the shell by app ID. The boundary is a fresh
  profile, not yet isolation. Still missing on Windows: driving the system
  theme, so light and dark look alike; and a canvas larger than the app
  for the zoom camera, so stories record at one pixel per point.
  [docs/windows.md](docs/windows.md) is the guide.
