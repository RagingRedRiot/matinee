# Recording on Windows

Terminal stories run PowerShell on a pseudo console, as on Linux they run
bash. GUI stories run their apps in one of two places: a private desktop
of the session's own, by default, or your own screen, with `--screen`.

GUI stories need a signed-in, unlocked desktop session. Windows gives
programs started from SSH or as a service no desktop to make, so run
matinee from a terminal in your session, or from a scheduled task that
runs only when the user is logged on. Terminal stories run anywhere.

## The private desktop

By default each session makes a desktop of its own, which nothing shows,
and launches its apps there. matinee places each window, captures it with
`PrintWindow`, posts it input as messages, and reads its controls through
UI Automation. Your pointer and keyboard are left alone, and nothing
appears on screen.

Classic Win32 apps, console windows, and Chromium-based browsers draw,
publish their controls, and take posted clicks and typing there. WinUI
apps, such as Task Manager, and WPF apps draw and publish their controls,
so `wait_for` and `expect` work, but they ignore posted clicks and keys.
Packaged (Store) apps, launched through the shell, open on your own
desktop rather than the private one. Use `--screen` for both.

matinee places only the windows an app shows. Launched apps, and
everything they start, run in a job that ends them when the session ends,
even if matinee itself is stopped, and an app counts as running while
anything it started does, so a launcher that starts the real app and
exits is followed.

A browser started with a fresh profile on a machine signed in to a work
or Microsoft account may sign in to that account and show it in a
welcome dialog over the page. Start it private, as with Edge's
`--inprivate`, to keep the account off screen.

## On the screen

```sh
matinee run --screen demo.toml
matinee record --screen demo.toml
```

With `--screen`, apps open on your own desktop, above your windows, and
take real input: matinee moves the pointer and types, so leave the mouse
and keyboard alone while it runs. Clicks and keys go only to the story's
windows. If something else is in the way, such as a window that came to
the front, the press is not sent and matinee prints a warning. Frames are
still captured window by window, so your other windows never appear in a
recording. The story's screen is laid out from the top-left corner of your
primary screen and must fit on it.

`--screen` applies to `run`, `record`, `snapshot`, and `probe`. Linux has
no need of it: every app there runs in the session's own compositor.

## Packaged apps

A packaged app is launched by its app ID, which `Get-StartApps` lists:

```toml
[launch]
app = "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"
wait_for = "Calculator"
```

matinee asks the shell to start it, as the Start menu does, and drives the
process that starts. It takes no arguments, and it gets Windows' own
environment rather than the session's fresh profile. matinee drives only an
instance it started, so close the app first if it is already running.
Without `--screen`, a story that launches an app ID is refused.

## Console windows

A console program launched as an app gets a console window of its own,
typed into and checked like any other window:

```toml
[launch]
app = "powershell.exe"
args = ["-NoLogo", "-NoExit", "-Command", "$Host.UI.RawUI.WindowTitle = 'Windows PowerShell'"]
wait_for = "Windows PowerShell"
```

A console publishes its whole buffer as one text field, so match its
output anywhere in it with a `*` at both ends, as in
`{ wait_for = "*Done*", role = "document text" }`.

## Not yet

matinee does not drive the Windows theme yet, so light and dark recordings
of an app look alike, and there is no canvas larger than the app for the
zoom camera, so stories record at one pixel per point. The session's
boundary is a fresh profile, not isolation: commands and apps can read
your files and use the network.
