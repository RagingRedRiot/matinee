use matinee::{platform::Boundary, runner::Runner, session::Session, story::Story};
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::{path::PathBuf, process::ExitCode};

const USAGE: &str = "\
usage: matinee check <storyboard.toml>...
       matinee run <storyboard.toml>
       matinee record [<storyboard.toml or folder>...]
       matinee shell <storyboard.toml> [-- <command>...]
       matinee snapshot <storyboard.toml> <out.png> [-- <command>...]
       matinee probe <storyboard.toml> [--all] [-- <command>...]
       matinee --version

commands:
  check   read storyboards and report any mistakes, without running them
  run     perform a storyboard against the real apps and check every step,
          without recording, in its first theme or the one MATINEE_THEME
          names
  record  perform it once per theme and write a GIF of each, named after
          demo.output with -light or -dark added; with folders, or none for
          the current folder, record every storyboard found in them
  shell   run a command, or bash, inside the isolated session a storyboard
          gets: its project at /project, a throwaway home, and only its own
          loopback network, unless [session] network = true reaches out
          through pasta
  snapshot  start the storyboard's apps, or a command, in that session and
          save a picture of the screen once it settles
  probe   start them the same way and list what steps can target: each
          window, and the name and role of everything showing in it

options:
  --artifacts <folder>  with run or record, save assertion stills and the
          actual failed-session screen, grouped by story and theme
  --no-isolation  run, record, shell, snapshot, and probe on the host instead
          of in namespaces, for systems that do not allow unprivileged user
          namespaces; commands and apps still get a fresh home, environment,
          display, and buses, but can read your files and use the network
  --fixed-too  with record, also write each GIF with the camera held on
          the whole screen, from the same run, named with -fixed added
  --view  with run or record, show the session in a window on your Wayland
          desktop as it runs, with the camera's view outlined and the
          pointer, but not captions, click ripples, or key badges; the run
          never waits for the window
  --screen  on Windows, with run, record, snapshot, or probe, open apps on
          your own screen instead of a private desktop, for packaged (Store)
          apps and apps that need the screen to draw, such as WPF ones;
          matinee takes the pointer and keyboard while it runs";

/// Set by `--no-isolation`: run stories on the host, without namespaces.
static NO_ISOLATION: AtomicBool = AtomicBool::new(false);
/// Set by `--view`: show the session in a window while it runs.
static VIEW: AtomicBool = AtomicBool::new(false);
/// Set by `--screen`: open apps on the user's screen, not a private desktop.
static SCREEN: AtomicBool = AtomicBool::new(false);
/// Set by `--fixed-too`: also write a fixed-camera GIF of each recording.
static FIXED_TOO: AtomicBool = AtomicBool::new(false);
static ARTIFACTS: OnceLock<PathBuf> = OnceLock::new();

/// The boundary a command runs `story` in, isolated unless asked not to be.
fn boundary_for(story: &Story, work: &std::path::Path) -> std::io::Result<Boundary> {
    let mut boundary = Boundary::for_story(story, work)?;
    if NO_ISOLATION.load(Ordering::Relaxed) {
        boundary = boundary.without_isolation()?;
    }
    if SCREEN.load(Ordering::Relaxed) {
        boundary = boundary.on_screen()?;
    }
    Ok(boundary)
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let end = args.iter().position(|a| a == "--").unwrap_or(args.len());
    if let Some(at) = args[..end].iter().position(|a| a == "--artifacts") {
        if at + 1 >= end || args[at + 1].starts_with("--") {
            eprintln!("matinee: --artifacts <folder> applies to run and record\n{USAGE}");
            return ExitCode::from(2);
        }
        let directory = PathBuf::from(&args[at + 1]);
        args.drain(at..at + 2);
        if !matches!(
            args.iter()
                .find(|a| {
                    !matches!(
                        a.as_str(),
                        "--no-isolation" | "--view" | "--fixed-too" | "--screen"
                    )
                })
                .map(String::as_str),
            Some("run" | "record")
        ) {
            eprintln!("matinee: --artifacts <folder> applies to run and record\n{USAGE}");
            return ExitCode::from(2);
        }
        let directory = if directory.is_absolute() {
            directory
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(directory),
                Err(e) => {
                    eprintln!("matinee: {e}");
                    return ExitCode::FAILURE;
                }
            }
        };
        ARTIFACTS.set(directory).expect("artifacts configured once");
    }
    // `--no-isolation` may come anywhere before a `--` that starts a
    // command.
    let end = args.iter().position(|a| a == "--").unwrap_or(args.len());
    if let Some(at) = args[..end].iter().position(|a| a == "--no-isolation") {
        args.remove(at);
        if args.first().is_some_and(|c| c == "check") {
            eprintln!("matinee: check runs nothing, so --no-isolation does not apply");
            return ExitCode::from(2);
        }
        NO_ISOLATION.store(true, Ordering::Relaxed);
        eprintln!(
            "matinee: running without isolation: commands and apps get a fresh home and \
             environment, but can read your files and use the network"
        );
    }
    let end = args.iter().position(|a| a == "--").unwrap_or(args.len());
    if let Some(at) = args[..end].iter().position(|a| a == "--view") {
        args.remove(at);
        if !matches!(args.first().map(String::as_str), Some("run" | "record")) {
            eprintln!("matinee: --view applies to run and record\n{USAGE}");
            return ExitCode::from(2);
        }
        VIEW.store(true, Ordering::Relaxed);
    }
    let end = args.iter().position(|a| a == "--").unwrap_or(args.len());
    if let Some(at) = args[..end].iter().position(|a| a == "--screen") {
        args.remove(at);
        if !matches!(
            args.first().map(String::as_str),
            Some("run" | "record" | "snapshot" | "probe")
        ) {
            eprintln!("matinee: --screen applies to run, record, snapshot, and probe\n{USAGE}");
            return ExitCode::from(2);
        }
        SCREEN.store(true, Ordering::Relaxed);
        if cfg!(windows) {
            eprintln!(
                "matinee: running on your screen: apps open on your own desktop, and matinee \
                 moves the pointer and types while it runs; leave the mouse and keyboard alone"
            );
        }
    }
    let end = args.iter().position(|a| a == "--").unwrap_or(args.len());
    if let Some(at) = args[..end].iter().position(|a| a == "--fixed-too") {
        args.remove(at);
        if args.first().is_none_or(|c| c != "record") {
            eprintln!("matinee: --fixed-too applies to record\n{USAGE}");
            return ExitCode::from(2);
        }
        FIXED_TOO.store(true, Ordering::Relaxed);
    }
    let code = match args.first().map(String::as_str) {
        Some("check") if args.len() > 1 => check(args[1..].iter().map(PathBuf::from)),
        Some("run") if args.len() == 2 => run(&args[1]),
        Some("record") => record(&args[1..]),
        Some("shell") if args.len() == 2 => shell(&args[1], &[]),
        Some("shell") if args.len() > 3 && args[2] == "--" => shell(&args[1], &args[3..]),
        Some("probe") if args.len() >= 2 => {
            let all = args.get(2).is_some_and(|a| a == "--all");
            let rest = &args[if all { 3 } else { 2 }..];
            match rest.split_first() {
                None => probe(&args[1], all, &[]),
                Some((dashes, command)) if dashes == "--" && !command.is_empty() => {
                    probe(&args[1], all, command)
                }
                _ => {
                    eprintln!("{USAGE}");
                    ExitCode::from(2)
                }
            }
        }
        Some("snapshot") if args.len() == 3 => snapshot(&args[1], &args[2], &[]),
        Some("snapshot") if args.len() > 4 && args[3] == "--" => {
            snapshot(&args[1], &args[2], &args[4..])
        }
        Some("-V" | "--version") => {
            println!("matinee {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    };
    matinee::platform::viewport::close();
    code
}

/// Open the viewport window for `stories` if `--view` asked for one, with
/// room for the largest of their frames.
fn open_viewport(stories: &[Story]) -> std::io::Result<()> {
    if !VIEW.load(Ordering::Relaxed) {
        return Ok(());
    }
    let pixels = stories
        .iter()
        .map(|s| {
            let [w, h] = s.demo.size.map(|n| (n * s.demo.scale) as usize);
            w * h
        })
        .max()
        .unwrap_or(0);
    let [w, h] = stories[0].demo.size;
    matinee::platform::viewport::open(pixels, (w, h))
}

/// Assertion and failure images are written inside the same session as the
/// story. Copy them out before deleting its work area; never relaunch an app
/// to reconstruct a failed state. The hash distinguishes equal output names
/// in different project folders.
fn collect_artifacts(
    work: &std::path::Path,
    story: &Story,
    theme: matinee::story::Theme,
) -> std::io::Result<()> {
    let Some(root) = ARTIFACTS.get() else {
        return Ok(());
    };
    let source = work.join("artifacts");
    if !source.exists() {
        return Ok(());
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let directory = if story.dir.as_os_str().is_empty() {
        std::path::Path::new(".")
    } else {
        &story.dir
    };
    std::fs::canonicalize(directory)?.hash(&mut hash);
    story.demo.output.hash(&mut hash);
    let name = story
        .demo
        .output
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let invocation = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_nanos();
    let destination = root.join(format!(
        "{name}-{:016x}-{}-{invocation}",
        hash.finish(),
        theme.name()
    ));
    std::fs::create_dir_all(&destination)?;
    for entry in std::fs::read_dir(&source)? {
        let entry = entry?;
        std::fs::copy(entry.path(), destination.join(entry.file_name()))?;
    }
    std::fs::remove_dir_all(source)?;
    println!("artifacts: {}", destination.display());
    Ok(())
}

fn check(paths: impl Iterator<Item = PathBuf>) -> ExitCode {
    let mut failed = false;
    for path in paths {
        match Story::load(&path) {
            Ok(story) => {
                let steps = story.steps().count();
                let staged = story.steps().filter(|s| s.action.is_staged()).count();
                let themes: Vec<_> = story.demo.themes.iter().map(|t| t.name()).collect();
                println!(
                    "{}: {} scenes, {steps} steps ({staged} staged), {}",
                    path.display(),
                    story.scenes.len(),
                    themes.join(" and "),
                );
                println!("  control: {}", story.control_usage().join("; "));
            }
            Err(e) => {
                eprintln!("{e}");
                failed = true;
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn shell(path: &str, command: &[String]) -> ExitCode {
    let path = PathBuf::from(path);
    let story = match Story::load(&path) {
        Ok(story) => story,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let work = std::env::temp_dir().join(format!("matinee-{}", std::process::id()));
    if let Err(e) = std::fs::create_dir(&work) {
        eprintln!("matinee: {}: {e}", work.display());
        return ExitCode::FAILURE;
    }
    let result = boundary_for(&story, &work).and_then(|session| {
        session.run(|| {
            let (program, args) = match command.split_first() {
                Some((program, args)) => (program.as_str(), args),
                None => (matinee::platform::SHELL, &[][..]),
            };
            match session.command(program).args(args).status() {
                Ok(status) => status.code().unwrap_or(1),
                Err(e) => {
                    eprintln!("matinee: {program}: {e}");
                    127
                }
            }
        })
    });
    let _ = std::fs::remove_dir_all(&work);
    match result {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(e) => {
            eprintln!("matinee: {e}");
            ExitCode::FAILURE
        }
    }
}

fn snapshot(path: &str, out: &str, command: &[String]) -> ExitCode {
    let story = match Story::load(path.as_ref()) {
        Ok(story) => story,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    if command.is_empty() && story.launches.is_empty() {
        eprintln!("matinee: {path} launches no app; name a command after --");
        return ExitCode::FAILURE;
    }
    let work = std::env::temp_dir().join(format!("matinee-{}", std::process::id()));
    if let Err(e) = std::fs::create_dir(&work) {
        eprintln!("matinee: {}: {e}", work.display());
        return ExitCode::FAILURE;
    }
    // One picture per theme, each from a fresh session.
    let several = story.demo.themes.len() > 1;
    let mut written = vec![];
    let mut failure = None;
    for &theme in &story.demo.themes {
        let result = boundary_for(&story, &work).and_then(|boundary| {
            boundary.run(|| {
                let shot = Session::start(&boundary, &story, theme).and_then(|mut session| {
                    session.seed(&story)?;
                    match command.split_first() {
                        Some((program, args)) => session.spawn(program, program, args)?,
                        None => session.launch(&story)?,
                    }
                    let frame = session.settle(Duration::from_secs(1), Duration::from_secs(8))?;
                    frame.write_png(&boundary.work().join("shot.png"))
                });
                match shot {
                    Ok(()) => 0,
                    Err(e) => {
                        eprintln!("matinee: {e}");
                        1
                    }
                }
            })
        });
        let target = if several {
            themed(out, theme)
        } else {
            PathBuf::from(out)
        };
        let copied = match result {
            Ok(0) => std::fs::copy(work.join("shot.png"), &target).map(drop),
            Ok(_) => Err(std::io::Error::other("no snapshot was taken")),
            Err(e) => Err(e),
        };
        match copied {
            Ok(()) => written.push(target),
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    for path in &written {
        println!("{}", path.display());
    }
    match failure {
        None => ExitCode::SUCCESS,
        Some(e) => {
            eprintln!("matinee: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `shot.png` as `shot-dark.png`.
fn themed(out: &str, theme: matinee::story::Theme) -> PathBuf {
    let path = PathBuf::from(out);
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let name = match path.extension() {
        Some(ext) => format!("{stem}-{}.{}", theme.name(), ext.to_string_lossy()),
        None => format!("{stem}-{}", theme.name()),
    };
    path.with_file_name(name)
}

fn probe(path: &str, all: bool, command: &[String]) -> ExitCode {
    let story = match Story::load(path.as_ref()) {
        Ok(story) => story,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    if command.is_empty() && story.launches.is_empty() {
        eprintln!("matinee: {path} launches no app; name a command after --");
        return ExitCode::FAILURE;
    }
    let work = std::env::temp_dir().join(format!("matinee-{}", std::process::id()));
    if let Err(e) = std::fs::create_dir(&work) {
        eprintln!("matinee: {}: {e}", work.display());
        return ExitCode::FAILURE;
    }
    let result = boundary_for(&story, &work).and_then(|boundary| {
        boundary.run(|| {
            let listed =
                Session::start(&boundary, &story, story.demo.themes[0]).and_then(|mut session| {
                    session.seed(&story)?;
                    match command.split_first() {
                        Some((program, args)) => session.spawn(program, program, args)?,
                        None => session.launch(&story)?,
                    }
                    session.settle(Duration::from_secs(1), Duration::from_secs(8))?;
                    print_tree(&session, all)
                });
            match listed {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("matinee: {e}");
                    1
                }
            }
        })
    });
    let _ = std::fs::remove_dir_all(&work);
    match result {
        Ok(0) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("matinee: {e}");
            ExitCode::FAILURE
        }
    }
}

fn print_tree(session: &Session, all: bool) -> std::io::Result<()> {
    let windows = session.windows();
    for window in &windows {
        println!(
            "window \"{}\" ({}) at {},{} {}x{}{}",
            window.title,
            window.app_id,
            window.x,
            window.y,
            window.width,
            window.height,
            if window.focused { ", focused" } else { "" }
        );
    }
    let a11y = session.accessibility()?;
    let apps = a11y.apps();
    if apps.is_empty() {
        println!("\nno app published an accessibility tree");
        if !windows.is_empty() {
            println!("control: window coordinates available; semantic targets unavailable");
        }
    }
    for app in apps {
        println!("\napp \"{}\" (pid {})", app.name, app.pid);
        let nodes = a11y.nodes(&app, all);
        let named = nodes.iter().any(|n| {
            n.depth > 0
                && n.showing
                && (!n.name.is_empty() || n.text.as_ref().is_some_and(|t| !t.is_empty()))
        });
        if named {
            println!(
                "  control: semantic targets published; verify each selector's role and uniqueness"
            );
        } else {
            println!(
                "  control: no named controls published; use window coordinates or capture only"
            );
        }
        for (i, node) in nodes.iter().enumerate() {
            if node.depth > 0 && node.name.is_empty() && node.text.is_none() && !all {
                continue;
            }
            let size = windows
                .iter()
                .find(|w| w.title == node.window)
                .or_else(|| (windows.len() == 1).then(|| &windows[0]))
                .map_or((0, 0), |w| (w.width, w.height));
            let at = matinee::platform::in_points(&nodes, i, size, session.scale() as i32)
                .map(|(x, y, w, h)| format!(" at {x},{y} {w}x{h}"))
                .unwrap_or_default();
            let text = node
                .text
                .as_ref()
                .map(|t| format!(" containing {t:?}"))
                .unwrap_or_default();
            println!(
                "{}{} \"{}\"{text}{at}{}{}",
                "  ".repeat(node.depth + 1),
                node.role,
                node.name,
                if node.focused { ", focused" } else { "" },
                if node.showing { "" } else { ", hidden" },
            );
        }
    }
    Ok(())
}

fn run(path: &str) -> ExitCode {
    let story = match Story::load(path.as_ref()) {
        Ok(story) => story,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let work = std::env::temp_dir().join(format!("matinee-{}", std::process::id()));
    if let Err(e) = std::fs::create_dir(&work) {
        eprintln!("matinee: {}: {e}", work.display());
        return ExitCode::FAILURE;
    }
    // The story is the same in every theme, so one is enough to check it:
    // the first, or the one a matinee session around this run is recording.
    let theme = std::env::var("MATINEE_THEME")
        .ok()
        .and_then(|name| story.demo.themes.iter().find(|t| t.name() == name))
        .copied()
        .unwrap_or(story.demo.themes[0]);
    if let Err(e) = open_viewport(std::slice::from_ref(&story)) {
        let _ = std::fs::remove_dir_all(&work);
        eprintln!("matinee: {e}");
        return ExitCode::FAILURE;
    }
    let result = boundary_for(&story, &work).and_then(|boundary| {
        boundary.run(|| {
            let performed = Session::start(&boundary, &story, theme)
                .map_err(matinee::runner::Failure::from)
                .and_then(|mut session| {
                    session.enable_artifacts(ARTIFACTS.get().is_some());
                    let result = (|| {
                        let mut runner = Runner::new(&mut session, &story)?;
                        runner.launch(&story)?;
                        runner.perform(&story)
                    })();
                    if result.is_err() {
                        if let Err(e) = session.save_artifact("failure.png") {
                            eprintln!("matinee: could not save failure screen: {e}");
                        }
                    }
                    result
                });
            match performed {
                Ok(steps) => {
                    println!("{path}: all {steps} steps passed");
                    0
                }
                Err(failure) => {
                    eprintln!("{path}: {failure}");
                    1
                }
            }
        })
    });
    let collected = collect_artifacts(&work, &story, theme);
    let _ = std::fs::remove_dir_all(&work);
    if let Err(e) = collected {
        eprintln!("matinee: collecting artifacts: {e}");
        return ExitCode::FAILURE;
    }
    match result {
        Ok(0) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("matinee: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Time recorded before the first step and after the last.
const LEAD_IN: Duration = Duration::from_millis(500);
const TAIL: Duration = Duration::from_millis(1500);

/// Record every storyboard in `targets`, files or folders to search, or
/// in the current folder when there are none.
fn record(targets: &[String]) -> ExitCode {
    let roots: Vec<PathBuf> = if targets.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        targets.iter().map(PathBuf::from).collect()
    };
    let mut paths = vec![];
    for root in &roots {
        match matinee::discover::storyboards(root) {
            Ok(found) if found.is_empty() => {
                eprintln!("matinee: no storyboards in {}", root.display());
                return ExitCode::FAILURE;
            }
            Ok(found) => paths.extend(found),
            Err(e) => {
                eprintln!("matinee: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    paths.sort();
    paths.dedup();

    // Read them all first, so a mistake, or two demos writing the same
    // GIF, stops everything before anything is recorded.
    let mut stories = vec![];
    let mut mistaken = false;
    for path in &paths {
        match Story::load(path) {
            Ok(story) => stories.push(story),
            Err(e) => {
                eprintln!("{e}");
                mistaken = true;
            }
        }
    }
    if mistaken {
        return ExitCode::FAILURE;
    }
    let mut outputs: Vec<(PathBuf, &PathBuf)> = vec![];
    for (story, path) in stories.iter().zip(&paths) {
        let output = normalize(&story.dir.join(&story.demo.output));
        if let Some((_, other)) = outputs.iter().find(|(o, _)| *o == output) {
            eprintln!(
                "matinee: {} and {} both write {}-*.gif; give one a different demo.output",
                other.display(),
                path.display(),
                output.display()
            );
            return ExitCode::FAILURE;
        }
        outputs.push((output, path));
    }
    if let Err(e) = open_viewport(&stories) {
        eprintln!("matinee: {e}");
        return ExitCode::FAILURE;
    }

    if let ([path], [story]) = (&paths[..], &stories[..]) {
        return if record_story(path, story) {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    let mut failed = vec![];
    for (path, story) in paths.iter().zip(&stories) {
        println!("== {}", path.display());
        if !record_story(path, story) {
            failed.push(path);
        }
    }
    println!(
        "recorded {} of {} demos",
        paths.len() - failed.len(),
        paths.len()
    );
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        for path in failed {
            eprintln!("matinee: failed: {}", path.display());
        }
        ExitCode::FAILURE
    }
}

/// Print what was written for the GIF called `name`, and warn when it is
/// over its size budget.
fn report(name: &str, steps: usize, written: &matinee::record::Written) {
    println!(
        "{name}: {steps} steps, {} frames, {:.1} s, {} KB",
        written.frames,
        written.seconds,
        written.bytes / 1024
    );
    for part in &written.segments {
        let camera = if part.camera_bytes > 0 {
            format!(", {} KB while the camera moved", part.camera_bytes / 1024)
        } else {
            String::new()
        };
        println!(
            "  {:>5.1} s {:>6} KB  {}{camera}",
            part.seconds,
            part.bytes / 1024,
            part.label
        );
    }
    let budget = if written.camera_moved {
        matinee::record::CAMERA_BUDGET
    } else {
        matinee::record::BUDGET
    };
    if written.bytes > budget {
        eprintln!(
            "matinee: warning: the {name} GIF is {} KB, over its {} KB budget{}",
            written.bytes / 1024,
            budget / 1024,
            if written.camera_moved {
                " for a moving camera"
            } else {
                ""
            }
        );
    }
}

/// `path` with `.` and `..` worked out, without touching the filesystem,
/// so two spellings of one output compare equal.
fn normalize(path: &std::path::Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir if out.file_name().is_some() => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Record one storyboard in each of its themes, and say whether every
/// theme was written.
fn record_story(path: &std::path::Path, story: &Story) -> bool {
    let path = path.display();
    let work = std::env::temp_dir().join(format!("matinee-{}", std::process::id()));
    if let Err(e) = std::fs::create_dir(&work) {
        eprintln!("matinee: {}: {e}", work.display());
        return false;
    }
    let mut failed = false;
    for &theme in &story.demo.themes {
        let result = boundary_for(story, &work).and_then(|boundary| {
            boundary.run(|| {
                let recorded = Session::start(&boundary, story, theme)
                    .map_err(matinee::runner::Failure::from)
                    .and_then(|mut session| {
                        session.enable_artifacts(ARTIFACTS.get().is_some());
                        let result = (|| {
                            let mut runner = Runner::new(&mut session, story)?;
                            runner.launch(story)?;
                            drop(runner);
                            session.start_recording(FIXED_TOO.load(Ordering::Relaxed));
                            session.run_for(LEAD_IN)?;
                            let mut runner = Runner::new(&mut session, story)?;
                            let steps = runner.perform(story)?;
                            drop(runner);
                            // End on the whole screen, where the loop restarts.
                            session.mark_segment("end");
                            session.camera.overview();
                            session.run_for(TAIL)?;
                            let (recording, fixed) =
                                session.stop_recording().expect("recording was started");
                            let gif = boundary.work().join("out.gif");
                            let written = recording.write_gif(&gif, story.demo.fps)?;
                            let fixed = match fixed {
                                Some(fixed) => {
                                    let gif = boundary.work().join("out-fixed.gif");
                                    Some(fixed.write_gif(&gif, story.demo.fps)?)
                                }
                                None => None,
                            };
                            Ok((steps, written, fixed))
                        })();
                        if result.is_err() {
                            if let Err(e) = session.save_artifact("failure.png") {
                                eprintln!("matinee: could not save failure screen: {e}");
                            }
                        }
                        result
                    });
                match recorded {
                    Ok((steps, written, fixed)) => {
                        report(theme.name(), steps, &written);
                        if let Some(fixed) = fixed {
                            report(&format!("{} fixed", theme.name()), steps, &fixed);
                        }
                        0
                    }
                    Err(failure) => {
                        eprintln!("{path} ({}): {failure}", theme.name());
                        1
                    }
                }
            })
        });
        if let Err(e) = collect_artifacts(&work, story, theme) {
            eprintln!("matinee: collecting artifacts: {e}");
            failed = true;
            break;
        }
        let mut gifs = vec![("out.gif", "")];
        if FIXED_TOO.load(Ordering::Relaxed) {
            gifs.push(("out-fixed.gif", "-fixed"));
        }
        let result = result.and_then(|code| match code {
            0 => Ok(()),
            _ => Err(std::io::Error::other("not recorded")),
        });
        for (written, suffix) in gifs {
            let target = story.dir.join(format!(
                "{}-{}{suffix}.gif",
                story.demo.output.display(),
                theme.name()
            ));
            let copied = match &result {
                Ok(()) => target
                    .parent()
                    .map(std::fs::create_dir_all)
                    .transpose()
                    .and_then(|_| std::fs::copy(work.join(written), &target).map(drop)),
                Err(e) => Err(std::io::Error::new(e.kind(), e.to_string())),
            };
            match copied {
                Ok(()) => println!("{}", target.display()),
                Err(e) => {
                    eprintln!("matinee: {}: {e}", target.display());
                    failed = true;
                    break;
                }
            }
        }
        if failed {
            break;
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    !failed
}
