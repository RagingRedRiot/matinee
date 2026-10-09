# First release checklist

The first release targets Linux terminal stories, GTK 4/libadwaita apps,
and explicit X11 control, with semantic Qt 5 X11 control where the app
publishes a usable tree. Other toolkit/theme combinations are experimental;
see [SUPPORT.md](SUPPORT.md). An in-process driver, video output, and frozen
clocks are deferred. The clock stays live by design.

## Correctness baseline

Validated locally on Pop!_OS 24.04, 2026-10-07:

- [x] Core suite: 112 tests passed; dependency-gated tests run separately.
- [x] Full dependency-gated integration suite: 18 tests passed, including GTK
      accessibility, themes, text entry, and failure diagnostics.
- [x] GTK drag/drop, sliders, coordinate bounds, and slow-seed regression.
- [x] Tk coordinate input and observable file assertion under Xwayland.
- [x] Qt 5 named controls under Xwayland.
- [x] Network opt-in: DNS works; host loopback and abstract sockets stay hidden.
- [x] Host paths, synthetic identity, read-only mounts, private localhost,
      and detached-process cleanup; the no-isolation fallback is also tested.
- [x] Formatting and Clippy with warnings denied.
- [x] Rust 1.87 checks every target locally. Rust 1.85 fails in locked fontdue
      0.9.4, so the declared minimum and CI check now use 1.87.
- [ ] Confirm the new CI workflow passes on a GitHub-hosted runner.

Slow setup exposed a timing bug: overdue frames rushed input before GTK
could process a drop. The scheduler now rebases wall time while retaining
the frame index. The slow-seed test guards the resulting behavior.

## Recordings and visual review

- [x] CI recipe records terminal, calculator, and mixed stories in both themes,
      with automatic and fixed camera variants.
- [x] Review generator preserves checked-in GIFs and produces an HTML report,
      sampled PNGs, pixel differences, per-scene logs, and JSON byte sizes.
      It uses a fixed copy of the recording binary and saves its SHA-256.
- [x] Inspect sampled recording frames at 800 and 400 pixels wide. The current
      desktop stories are intended for 800-pixel README columns; narrow views
      make app text small. A mobile-focused story needs its own framing/font
      choices rather than merely scaling these GIFs down.
- [ ] Remeasure callboard's fixed-camera GIFs. The story gained the zoom-out
      and ticket-watch scenes after the last fixed-camera measurement.
- [x] Rerun callboard and cued in both themes against the local binaries
      identified below; all stories passed.
- [ ] Compare a second machine's recordings with this baseline. GitHub-hosted
      artifacts can provide that second environment after CI runs.

Visual differences do not fail CI. Story failures and artifact-generation
errors do. Sampled differences are a review aid, not an assertion: app timing,
live text, fonts, and camera alignment can affect them. Review the complete
GIF and each scene's size log before accepting a baseline update.

Local 2026-10-07 measurements, in MiB per theme:

| Story | Automatic light / dark | Fixed light / dark |
| --- | --- | --- |
| callboard | 4.53 / 4.47 | not yet remeasured |
| cued | 1.29 / 1.30 | 0.33 / 0.33 |

Both stories pass under the moving-camera 5 MiB budget, callboard's with
little room to spare; cued also passes the fixed-camera 2 MiB budget.
Fixed camera intentionally suppresses explicit focus moves too. Runs can have
slightly different durations because GUI updates and terminal commands use
wall time; this is not a byte-stability benchmark.

The local checks used existing release binaries, identified by SHA-256 rather
than assuming they match the current source checkout:

```text
callboard      5701ea49fdc0ba5fe304cde84ccb35f23e6c8c57e831d66579af0dded11a9541
callboard-gui  bcd10b57bb6d20f05377c337736963a0fc18353e92fcca7baf19a15e34573059
cued           d4271a6038cb13efb15879585867c8cc077b0c8d1f3fd79b619f0f2bb1014f91
cued-gui       0e919d840a57fb4bae5eb0e52142de62b1399580282bc73aeded9f28113db8a6
```

## Authoring and compatibility

- [x] Document storyboard authoring and the evidence behind the support matrix.
- [x] `check` reports declared control requirements, including capture-only
      stories and assertions that may match terminal text. `probe` reports
      whether usable named controls are published or coordinates are needed;
      individual selector roles and uniqueness still need validation.
- [x] Add a real Calculator menu/dialog/focus story. This build publishes no
      menu children, so the story uses a reviewed point for Preferences and
      verifies the opened dialog and calculator result after closing it.
- [ ] Add supported real-app stories for menus, toolkit focus, scrolling,
      dialogs, file pickers, clipboard, and multiple windows. Prototype evidence
      alone is insufficient to claim production support for every interaction.
- [ ] Verify GTK 3, Qt 5/6, and Electron themes before adding them to the supported
      theme matrix. Qt 5 Wayland accessibility varies by distributor build.
- [x] `run` and `record` accept `--artifacts <folder>` and save successful
      assertion stills before their holds, plus the actual failed-session
      screen. The CI review script requests these and links them in the report.
      They show the whole session at capture resolution, without camera cropping.

## Distribution

- [x] Source-only first release; Linux x86_64 is the validated baseline.
      Prebuilt binaries, other architecture claims, and Cargo publication are deferred.
- [x] Install and check the source-built binary in a fresh local installation root.
- [ ] Verify installation in the clean GitHub-hosted environment.
- [x] Draft [release notes](CHANGELOG.md) and identify reviewed demo baselines.
- [ ] Tag 0.1.0 after the hosted checks and release review pass.
- [ ] Publish the approved release. Cargo publishing remains disabled.

## Run the release checks

Install the dependencies listed in [.github/workflows/ci.yml](.github/workflows/ci.yml),
and use a host that permits unprivileged user namespaces:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo test --locked -- --ignored --test-threads=1
cargo +1.87.0 check --locked --all-targets
cargo build --locked --release
python3 scripts/review.py --fixed
```

The review script needs Python 3.11+ and ffmpeg. ffmpeg is only a review-tool
dependency; it is not part of Matinee's capture or encoding pipeline. Open
`target/review/index.html` locally after downloading the CI artifact. Choose
a fresh `--out` directory for every run to prevent stale files being mistaken
for new results.

For real-project release checks, build callboard and cued in their documented
locations, then add their stories explicitly:

```sh
python3 scripts/review.py --fixed --out target/project-review \
  --story examples/callboard/demo.toml --story examples/cued/demo.toml
```

CI runs on pushes, pull requests, and manual dispatch, including release-tag
pushes. It uploads artifacts even if checks fail, whenever artifacts exist.
It has read-only repository permissions and never commits GIF updates or
publishes a release. The workflow enables user namespaces on its disposable
Ubuntu runner so isolation checks exercise the real boundary.

The artifact retention and upload behavior follows GitHub's
[workflow artifact documentation](https://docs.github.com/en/actions/concepts/workflows-and-actions/workflow-artifacts).
