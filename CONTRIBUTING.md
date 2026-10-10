# Contributing to matinee

We welcome reproducible bug reports, focused fixes, tests, example
storyboards, support evidence for more toolkits and themes, and documentation
improvements. AI-assisted contributions are welcome and encouraged. The same
standards apply regardless of the tools used.

## Before starting

Small fixes can go straight to a pull request. For substantial features, new
dependencies, architectural changes, or broad refactors, open an issue to
discuss the need and approach before implementing them. Check existing issues
and pull requests to avoid duplicating work. [VISION.md](VISION.md) lists what
matinee is not trying to be.

matinee keeps its runtime dependencies few and owns its core pipeline: the
compositor, the isolation boundary, and the GIF encoder (see
[BUILD-DECISIONS.md](BUILD-DECISIONS.md)). A change that adds a runtime
dependency, or hands one of those parts to an outside tool, needs a
strong case in an issue first.

If you are new to the project, start with one focused pull request and wait for
feedback before opening several more. You do not need to open an issue for every
small change.

## Reporting bugs

Include the commit you built, your Linux distribution, and whether you ran with
`--no-isolation`. For a GUI story, include the app and toolkit versions and
whether the app runs on Wayland or under Xwayland (`x11 = true`).

The most useful reproduction is a minimal storyboard. Add the output of
`matinee check` and `matinee probe` when a step cannot find its target, and the
images `--artifacts <folder>` saves when a run fails. Those images show the
session's screen; look at them before posting.

For suspected vulnerabilities, including anything from your host showing up in
a session or recording, follow [SECURITY.md](SECURITY.md) and use the private
reporting route rather than a public issue or pull request.

## Development and checks

Development targets Linux and requires Rust 1.87 or newer, with the `rustfmt`
and `clippy` components. The isolation tests need unprivileged user namespaces;
on Ubuntu, see `kernel.apparmor_restrict_unprivileged_userns`. The packages CI
installs are listed in [ci.yml](.github/workflows/ci.yml).

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

The integration tests drive real apps and need GTK 4, GNOME Calculator, Qt 5,
Tk, Xwayland, and pasta. Run them one at a time:

```sh
cargo test --locked -- --ignored --test-threads=1
```

Run the tests relevant to your change first, then broader checks as
appropriate. Report the checks you actually ran and any limitations; a
documentation change does not require exercising unrelated GUI behavior.

## Reviewing recordings

To see what a change does to recordings, `scripts/review.py` records the
terminal, calculator, and mixed examples in both themes, with the moving and
fixed cameras, and builds an HTML report with sampled frames, visual
differences, per-scene logs, and GIF sizes. It needs a release build, Python
3.11 or newer, and ffmpeg, which the review uses but matinee itself does not.

```sh
cargo build --locked --release
python3 scripts/review.py --fixed
```

Open `target/review/index.html` when it finishes. Choose a fresh `--out`
folder for every run so old files cannot pass for new results. To review the
callboard and cued examples, build those projects in checkouts beside this
one and name the stories:

```sh
python3 scripts/review.py --fixed --out target/project-review \
  --story examples/callboard/demo.toml --story examples/cued/demo.toml
```

CI runs the same review on every change and saves the report as an artifact.
Visual differences do not fail CI; story failures and errors do. Sampled
differences are a review aid rather than an assertion, since app timing, live
text, fonts, and camera alignment can all move pixels. Watch the complete GIF
and read each scene's size log before accepting a new baseline.

## Submitting patches

Explain the concrete problem, the resulting behavior, and what you checked.
Keep each pull request focused on one coherent change and avoid unrelated
formatting or refactoring.

Add or update tests for behavior changes, and documentation for anything users
need to understand. Storyboard format changes need particular care: explain
what happens to existing storyboards, and update [STORYBOARD.md](STORYBOARD.md).
A change to the isolation boundary should explain how it relates to the
contract in [BUILD-DECISIONS.md](BUILD-DECISIONS.md) and add a test to
`tests/boundary.rs` for what it protects.

Follow the surrounding code style. Pull requests do not change the release
version; maintainers handle versioning when they cut a release.

CI runs formatting, lint, the core and integration tests, and records the
examples for review. Pull requests run `main`'s copy of the workflows, so a
change to a workflow takes effect only after it merges.

The separate controls check requires a maintainer to apply `reviewed-controls`
to the current revision when a pull request changes files under `.github/`,
`.cargo/`, or `scripts/`, build files such as `Cargo.toml` and `Cargo.lock`,
the isolation boundary in `src/boundary.rs`, or existing files under `tests/`.
Adding a new file under `tests/` does not by itself require that label. A later
push requires renewed maintainer review. An expected controls failure is not a
reason to weaken or bypass the check.

## Submitting examples and support evidence

An example storyboard should show something the existing examples don't. It
should pass `matinee check` and `matinee run`, and record in both themes. Keep
each GIF under the size budget `matinee record` reports: 5 MiB per theme with
the moving camera, 2 MiB with a fixed one. Use seeded or fixture data, never a
real account or personal files, and watch the whole recording before
submitting it.

Updated GIFs for existing examples need a reason, such as a change in the app
or in how matinee records. Review the complete GIF and each scene's size log
before submitting it.

[SUPPORT.md](SUPPORT.md) lists only combinations with evidence. To add one,
state the distribution, the toolkit, theme, and app versions, and which
interactions you exercised, ideally with a storyboard that shows them.

## AI-assisted contributions

Before submitting, review the entire change, understand what it does, and verify
its behavior. You should be able to explain the change, answer review questions,
and address problems found during review. AI-generated claims about tests or
behavior are not verification.

Briefly mention substantial AI assistance in the pull request description,
along with how you checked the result. Prompt transcripts and disclosure of
routine autocomplete are unnecessary. Keep explanations concise and accurate.

Automated submissions require human review before posting. Do not submit
batches of speculative fixes, generated storyboards, or review comments. These
requirements apply to issues and review discussion as well as pull requests.

## Review expectations and conduct

Maintainer review time is limited, and there is no promised turnaround.
Contributions may be deferred or closed because of scope, insufficient
verification, duplication, or maintenance cost, even when technically correct.
Maintainers may close unsuitable contributions without a detailed review or
repairing them on the contributor's behalf.

Respond to feedback and explain revisions. A closed pull request does not
prevent a revised proposal that addresses the reason for closure. Repeated
submissions that disregard feedback may result in contribution restrictions.

Keep discussion respectful and constructive. Critique the work, avoid personal
attacks, and respect maintainers' decisions about project scope and review
capacity.

## Licensing

Contributions, including storyboards, recordings, and AI-assisted work, use the
project's [MIT license](LICENSE). Submit only material you are entitled to
contribute under those terms. Do not include secrets, personal paths, or
third-party material with incompatible licensing, and do not record apps or
sites in a way their terms forbid.
