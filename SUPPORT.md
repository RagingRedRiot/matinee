# Supported configurations and evidence

This matrix distinguishes automated evidence from
manual trials and work still needed before making broader claims. A working
accessibility tree depends on the app's build, labels, and runtime settings,
not just its toolkit.

| Configuration | Control and assertions | Themes | Evidence |
| --- | --- | --- | --- |
| Terminal only | Real PTY commands, input, exit status, output assertions, declared staging | Light/dark | `tests/terminal.rs`, `tests/no_isolation.rs`, `tests/record.rs` |
| GTK 4/libadwaita, Wayland | Named controls and text contents; reviewed menu points where menu children are absent | Light/dark checked | GNOME Calculator and menu/dialog/focus in `tests/a11y.rs`; drag, sliders, and points in `tests/drag.rs` |
| Qt 5, X11 | Named controls when AT-SPI is published | Not verified | Qt 5.15.13 fixture in `tests/x11.rs` |
| Tk, X11 | Window-relative points and keys; result checked through a real file | Not verified | Tk 8.6 fixture in `tests/x11.rs` |
| Qt 5, Wayland | Build dependent; Ubuntu 24.04's Qt lacks the accessibility-bus patch | Not verified | Existing probe findings; coordinate fallback is available |
| GTK 3, Qt 6, Electron | Experimental; validate the particular app | Not verified | No committed automated toolkit coverage yet |
| Firefox, Chrome, Vivaldi, Brave, Wayland | Named page controls where the browser exposes them; point fallback | Site/browser dependent | Manual Google story trials; not a CI gate |
| callboard and cued | Mixed terminal and semantic GUI stories | Both checked locally | Real-app examples rerun with fixed and moving cameras |
| Windows terminal (PowerShell) | Real pseudo-console commands, input, exit status, output assertions | Light/dark pane | `examples/powershell`; manual runs on Windows 11 |
| Windows console windows | Typed input; buffer text matched with `*text*`; text highlights through UI Automation | Not driven | Manual runs on Windows 11 |
| Win32 and WinUI apps, private desktop | Named controls through UI Automation; clicks and typed characters act through it | Not driven | Notepad (`examples/notepad`) and Task Manager, manual runs on Windows 11 |
| Edge, private desktop and `--screen` | Clicks and typing; start it with `--inprivate` | Not driven | Manual runs on Windows 11 |
| Packaged apps, `--screen` | Real input sent only to the story's windows | Not driven | Calculator by app ID, manual runs on Windows 11 |

Three levels of GUI use apply:

1. **Semantic control:** uniquely select a named accessible element using role,
   app, ancestor, nearby element, and window context. Assertions check actual
   visible state or text-field contents.
2. **Explicit control:** choose a window-relative point and inject real pointer
   or keyboard input. A coordinate action alone does not prove its result;
   add an observable GUI, terminal, or file assertion.
3. **Capture only:** show the app while other real actions run. Recording a GUI
   alone does not verify its behavior.

`probe` lists windows and accessible controls and reports whether named
targets are published or coordinate control is the available fallback. This
does not guarantee every widget has a useful name: the tested Calculator
build displays a menu but publishes no menu children. Its menu/dialog fixture
uses a point for Preferences, then asserts actual dialog and calculator state.
`check` reports declared semantic, coordinate, keyboard, or capture-only use,
and flags unqualified assertions that may match terminal text. It validates
without starting the app, so it cannot prove the named controls will exist.

Isolation tests cover host filesystem/socket visibility, synthetic identity,
read-only project and declared mounts, private networking, outside networking
through pasta, and detached-child cleanup. This is privacy isolation for demo
recording, not a sandbox for malicious software. `--no-isolation` gives fresh
session state but leaves host files and networking reachable.

Local integration baseline: Pop!_OS 24.04, GTK 4.14.5, Qt 5.15.13, Tk 8.6,
on 2026-10-07. No cross-machine or byte-identical GIF promise is made. The
new GitHub-hosted CI baseline remains to be confirmed.

Windows evidence is manual runs on Windows 11 in October 2026, plus unit
tests that CI runs on Windows Server 2022. A Windows session gives commands a
fresh profile and environment but no isolation, and matinee does not drive the
Windows theme yet.
