# Decisions before building matinee

These are working product contracts and tests for the first implementation.
They are based on local experiments, not on adopting the tools used to run
those experiments. The prototype source stays outside version control.

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

The first reproducibility promise is **the same observable story** under a
pinned app build, fixtures, theme, locale, screen geometry, fonts, and clock.
We will measure visual stability separately. Byte-identical GIFs across
machines are not a release promise until the app clock, animation scheduling,
terminal rendering, compositor frames, and encoders are controlled and tested.
CI should fail on story correctness; a visual difference should produce a
reviewable artifact rather than being silently accepted or automatically
treated as an application failure.

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
state is stable. These are same-machine measurements; another machine remains
untested. CI should check the observable story and save assertion stills and
visual diffs for review, not require identical GIF hashes.

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
success. Other Qt apps had usable trees in earlier probes. Electron, egui,
and other toolkit clocks/themes remain to be tested with real apps.

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
could have gone to the wrong place. The prototype now rejects that action with
an offscreen diagnostic; the same action succeeds after an explicit scroll.

## Support boundary

For the first release, distinguish these capabilities in `probe` output and
story validation:

1. **Semantic control:** an accessible target can be located uniquely by
   name, role, app, named ancestor, and window. This is the preferred mode.
2. **Explicit control:** a storyboard opts into window-relative coordinates
   when the app has no usable accessibility tree. The coordinate and window
   are reviewable, and a missing or ambiguous window fails the run.
3. **Capture only:** an app can be shown alongside real terminal actions, but
   its GUI cannot be clicked or asserted by meaning. Such scenes cannot claim
   a verified GUI result without another observable assertion.

The tool should report which tier a story uses. It should not claim semantic
control of every Linux toolkit: whether a tree exists depends on the app's
build and settings.

## Isolation contract to prove

The current runner uses a fresh home and runtime directory, starts GUI apps
in that home, disables their network by default, and reaps detached children.
This is useful state isolation, not yet a containment boundary. Apps can
still access host paths, fixed socket locations, and some system services.
Separate per-process network namespaces also prevent a terminal and GUI app
from talking over localhost unless they share a namespace.

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

Before release, repeat these checks with the chosen implementation and a
broader set of host Unix sockets and system paths. Document explicit opt-ins
for stories that need outside access. A temporary `HOME` alone must never be
presented as full isolation.

## Output contract

GIF is the primary output. The existing
[callboard README](https://github.com/RagingRedRiot/callboard) uses `<picture>`
with separate light and dark GIFs, each about 1.3 MB. Use that as the GitHub
README pattern; no new hosting repository is needed to choose the format.
Matinee should report GIF size per scene, set a practical per-story budget,
and check legibility at README width. A particularly motion-heavy story may
motivate optional video support later, but video playback is not a build gate.
`record` warns when a GIF is over budget: 2 MiB per theme with a still
camera, and 5 MiB when the camera moves. Size alone is not a pass: the
text acted on must remain legible at 400-pixel display width.

The camera's moves are most of a moving GIF's size: a move changes nearly
every pixel of each frame it spans (callboard's demo measured 3.1 of
4.5 MB in 42 such frames). So the camera stays put whenever it can. It
zooms only as far as text needs to read at an 800-pixel display width, so
an output no wider than that never zooms on its own. It stays while the
next target is comfortably in view and readable. Each move eases over
0.4 seconds by default and is stored at half the frame rate. With these,
callboard's 40-second demo is about 4 MB per theme, against 2.4 MB with a
fixed camera.

Light and dark come from the session, not the app: matinee serves the desktop
portal's settings on the session bus (the `color-scheme` that libadwaita,
GTK 4, and Qt 6 follow) and writes the same GNOME interface settings to a
GSettings keyfile, which GTK reads outside a sandbox. Neither bus starts
services on demand, so the session does not depend on what the host has
installed. This put GTK 4 and libadwaita into both themes; GTK 3, Qt 5,
Qt 6, and Electron still need checking. A terminal theme alone does not
change the GUI. Reduced-motion
behavior can use a still poster where a host supports it, without blocking
GIF output.

## Architecture

The point of matinee is control over every stage of a recording, so the core
pipeline is built into the binary rather than borrowed from external tools.
The prototype's stand-ins (sway, ffmpeg, `bwrap`) are not runtime
dependencies of the finished tool.

- **Compositor:** a headless Wayland compositor built into matinee. The
  prototype already captures and injects input only through standard
  wlroots protocols (screencopy, virtual pointer, virtual keyboard), so that
  client code carries over. Owning the compositor also gives matinee control
  of frame timing and output geometry.
- **Isolation:** matinee creates the user, mount, PID, and network namespaces
  itself, with the mount policy tested above.
- **D-Bus and accessibility:** a private session bus and accessibility bus,
  each the system's `dbus-daemon`, plus the system's `at-spi2-registryd`.
  Any desktop that runs AT-SPI already has both. matinee answers
  `org.a11y.Bus` and `org.a11y.Status` itself. An in-process bus (busd) was
  tried and dropped: zbus, which it is built on, rewrites a message whose
  only argument is a struct as separate arguments, and the AT-SPI registry
  crashes on the first app that registers.
- **Encoder:** a GIF encoder in matinee, built for this output: changed
  regions only, holds merged into long frames, one palette per clip, and
  per-scene size reporting.
- **Timeline:** recording time is a frame index, not wall-clock time. The
  camera path, eased pointer motion, and capture schedule are planned on
  that timeline, so the same story yields the same camera path. Waiting for
  the app (window appearance, assertions, timeouts) still uses real time.

The first slice may run on the stand-ins while these are built, but each
stage sits behind its own module boundary so it can be replaced without
reworking the runner.

## First implementation slice

Build one story through seeding, launch, real terminal command, checked exit
code, unambiguous GUI action or assertion, camera, and a reviewable GIF.
Keep a fixed-camera variant for size and timing comparisons. Use
`probe` output to author selectors and include the available names, roles,
windows, and nearby matches in failure messages.

The GTK fixture now exercises focus, scrolling, drag, dialogs, a file picker,
clipboard, and multiple windows. Repeat those actions on supported real apps;
menus and toolkit-specific focus behavior still need stories. Test time and
theme controls in each supported toolkit, and repeat the visual-stability
measurement on another machine. Implement and retest the chosen containment
boundary before calling a session isolated. Then write the release CI
recipe.
