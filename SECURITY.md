# Security policy

## Reporting a vulnerability

Report suspected vulnerabilities through
[GitHub's private vulnerability reporting](https://github.com/RagingRedRiot/matinee/security/advisories/new).
Do not post exploit details in a public issue or pull request before
maintainers have assessed the report.

Please include:

- The commit you built, your Linux distribution and kernel, and whether
  unprivileged user namespaces are restricted on that host.
- A minimal storyboard that reproduces the problem, and any `[session]` options
  it uses, such as `mounts` or `network`.
- What appeared in the session or recording, what you expected to stay out, and
  the potential impact.
- What you verified and what remains uncertain.

Use a throwaway account, synthetic files, and placeholder names where possible.
A report that shows a host detail leaking can use a planted sentinel file or
value instead of real personal data. AI-assisted reports are welcome; verify
generated claims and distinguish observed behavior from speculation. See the
[contribution policy](CONTRIBUTING.md#ai-assisted-contributions).

## What matinee protects

matinee's isolation exists for privacy: personal and identifying details about
you and your machine should not end up in a recording. Each story runs in a
private user, mount, PID, IPC, UTS, and network namespace with its own
compositor and buses. Inside it, a story sees:

- The project, read-only at `/project`, and anything listed in
  `[session] mounts`, read-only at its host path.
- A writable `/work`, a fresh home, and a synthetic user, host name, and
  machine ID.
- Its own loopback network only, unless the story sets `network = true`.

Everything else on the host should be out of reach: other files and the real
home folder, user and host names, the desktop's windows and display sockets,
the host's session and system buses, logins, and host services on loopback.
Processes a story leaves behind should stop when the session ends. With
`--view`, frames are copied out to a separate viewer process; the session
should not be able to reach the desktop through it.

Reports are welcome when matinee's own setup breaks one of these promises in
its default, isolated mode. Examples include a host path, socket, or identity
detail that is visible inside the session; a write that escapes `/work`; an
undeclared network route; a host loopback service or abstract socket reachable
with `network = true`; a process that outlives its session; or a way for the
session to act on the desktop through the viewport.

The repository's CI is also in scope. Pull requests run on
`pull_request_target`, so every run uses `main`'s copy of the workflows, and
their safety rests on the rules listed at the top of
[ci.yml](.github/workflows/ci.yml): a read-only token, no repository secrets,
no Actions cache, and no trusted use of CI artifacts. A pull request that can
obtain write access or secrets, poison something a later trusted run uses, or
merge a change to a guarded file without the `reviewed-controls` label is a
vulnerability.

## What it does not protect

The isolation is not a sandbox. It is not built or tested to withstand a
program that is trying to escape, and it must not be used to run untrusted or
malicious software. A program that breaks out through a kernel or user
namespace flaw is outside what matinee claims to resist; report those flaws to
the kernel or your distribution.

A storyboard is code. Its `run`, `seed`, and launch commands execute programs
with your privileges inside the session, and `--no-isolation` runs them on the
host. Read a storyboard before running it, as you would a build script.

Reports that demonstrate only these documented behaviors are not
vulnerabilities in matinee:

- `--no-isolation` exposing host files, sockets, or the network. It keeps a
  fresh home and environment but skips the namespaces by design.
- `network = true` reaching other hosts, including the local network and the
  host's non-loopback addresses.
- Project files, including untracked files under the project folder, or
  declared mounts appearing in a session. The whole project is visible
  read-only, so keep secrets out of it, or out of what the story shows.
- What the recorded program itself displays: a live web page, an account the
  storyboard signs into, or data the storyboard seeds.
- Images saved with `--artifacts`, or the `--view` window, showing the session
  screen. They show exactly what the session shows.

Recordings show whatever was on the session's screen. Watch a GIF before you
publish it.

## Supported versions

matinee has not had its first release yet. Security fixes target `main`. After
0.1.0, fixes target `main` and the latest release; older releases do not have
guaranteed backports.

## Response and disclosure

Maintainer capacity is limited, and there is no guaranteed response or
resolution time. Maintainers will use the private report to discuss
reproduction, impact, and a fix or mitigation where appropriate.

Please coordinate public disclosure through the private report so users can
receive a fix before exploit details are published.
