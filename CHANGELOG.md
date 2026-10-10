# Release notes

## 0.1.0

Matinee performs versioned storyboard files against real Linux programs and
records light and dark GIFs for READMEs. Terminal, GUI, and mixed stories share
one format, with seeded state, checked command status, and observable assertions.

- Built-in headless Wayland compositor, input driver, frame timeline, camera,
  overlays, terminal renderer, and GIF encoder; no desktop session or GPU needed.
- Private Linux namespaces keep host files, identity, sessions, and sockets out
  of recordings. Explicit network opt-in uses pasta; `--no-isolation` provides
  a documented fallback for hosts that prohibit namespaces.
- Semantic AT-SPI targets with context and ambiguity diagnostics, plus reviewed
  window coordinates for controls without usable accessibility data. Optional
  Xwayland supports Tk input and Qt 5 named controls.
- `check`, `probe`, `run`, `record`, `snapshot`, and `shell` authoring commands;
  project-wide recording validates stories and output collisions first.
- GIF scene-size reporting and practical size budgets. Automatic framing targets
  readability at the intended README width; fixed-camera comparisons are supported.
- `--artifacts` captures real assertion and failure screens. CI checks story
  correctness and saves recordings, stills, and visual differences for review.
- Input pacing resumes normally after slow setup or rendering instead of rushing
  overdue frames and losing interactions such as GTK drops.
- `--view` on `run` and `record` shows the session live in a window on a
  Wayland desktop, with the camera's view outlined and the pointer. Captions,
  click ripples, and key badges stay in the GIF only. Frames are copied out of
  the boundary, so the desktop stays out of reach, and the run never waits for
  the window.
- `record --fixed-too` also writes a fixed-camera GIF of each theme from the
  same run, for comparing the moving camera against the whole screen.
- A story with no app can place its terminal pane with `position` or `size`,
  leaving the rest of the screen for windows its commands open.
- OpenGL apps go straight to Mesa's software renderer instead of probing for a
  GPU the session does not have, so runs no longer print EGL and Zink warnings.
  With `--no-isolation`, apps also render in software rather than on the host GPU.
- `matinee --version` prints the version, for bug reports.

The release includes a prebuilt binary for x86_64 Linux with glibc 2.35 or
newer, and source builds require Rust 1.87 or newer. Other architectures and
toolkit/theme combinations need their own evidence; see [SUPPORT.md](SUPPORT.md).

In-process drivers, video output, and Cargo registry publication are deferred.
Time remains live, and byte-identical GIFs across machines are not a promise.
