use matinee::{a11y::A11y, boundary::Boundary, runner::Runner, session::Session, story::Story};
use std::time::Duration;
use std::{path::PathBuf, process::ExitCode};

const USAGE: &str = "\
usage: matinee check <storyboard.toml>...
       matinee run <storyboard.toml>
       matinee record <storyboard.toml>
       matinee shell <storyboard.toml> [-- <command>...]
       matinee snapshot <storyboard.toml> <out.png> [-- <command>...]
       matinee probe <storyboard.toml> [--all] [-- <command>...]

commands:
  check   read storyboards and report any mistakes, without running them
  run     perform a storyboard against the real apps and check every step,
          without recording
  record  perform it once per theme and write a GIF of each, named after
          demo.output with -light or -dark added
  shell   run a command, or bash, inside the isolated session a storyboard
          gets: its project at /project, a throwaway home, and no network
  snapshot  start the storyboard's apps, or a command, in that session and
          save a picture of the screen once it settles
  probe   start them the same way and list what steps can target: each
          window, and the name and role of everything showing in it";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check") if args.len() > 1 => check(args[1..].iter().map(PathBuf::from)),
        Some("run") if args.len() == 2 => run(&args[1]),
        Some("record") if args.len() == 2 => record(&args[1]),
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
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
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
    let result = Boundary::for_story(&story, &work).and_then(|session| {
        session.run(|| {
            let (program, args) = match command.split_first() {
                Some((program, args)) => (program.as_str(), args),
                None => ("bash", &[][..]),
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
        let result = Boundary::for_story(&story, &work).and_then(|boundary| {
            boundary.run(|| {
                let shot = Session::start(&boundary, &story, theme).and_then(|mut session| {
                    session.seed(&story)?;
                    match command.split_first() {
                        Some((program, args)) => session.spawn(program, program, args)?,
                        None => session.launch(&story)?,
                    }
                    let frame = session.settle(Duration::from_secs(1), Duration::from_secs(20))?;
                    frame.write_png(&std::path::Path::new(matinee::boundary::WORK).join("shot.png"))
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
    let result = Boundary::for_story(&story, &work).and_then(|boundary| {
        boundary.run(|| {
            let listed =
                Session::start(&boundary, &story, story.demo.themes[0]).and_then(|mut session| {
                    session.seed(&story)?;
                    match command.split_first() {
                        Some((program, args)) => session.spawn(program, program, args)?,
                        None => session.launch(&story)?,
                    }
                    session.settle(Duration::from_secs(1), Duration::from_secs(20))?;
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
    for window in session.compositor.windows() {
        let r = window.rect;
        println!(
            "window \"{}\" ({}) at {},{} {}x{}{}",
            window.title,
            window.app_id,
            r.loc.x,
            r.loc.y,
            r.size.w,
            r.size.h,
            if window.focused { ", focused" } else { "" }
        );
    }
    let a11y = A11y::connect(&session.buses)?;
    let apps = a11y.apps();
    if apps.is_empty() {
        println!("\nno app published an accessibility tree");
    }
    for app in apps {
        println!("\napp \"{}\" (pid {})", app.name, app.pid);
        for node in a11y.nodes(&app, all) {
            if node.depth > 0 && node.name.is_empty() && node.text.is_none() && !all {
                continue;
            }
            let at = node
                .extents
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
    // The story is the same in every theme, so one is enough to check it.
    let theme = story.demo.themes[0];
    let result = Boundary::for_story(&story, &work).and_then(|boundary| {
        boundary.run(|| {
            let performed = Session::start(&boundary, &story, theme)
                .map_err(matinee::runner::Failure::from)
                .and_then(|mut session| {
                    let mut runner = Runner::new(&mut session, &story)?;
                    runner.launch(&story)?;
                    runner.perform(&story)
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

/// Time recorded before the first step and after the last.
const LEAD_IN: Duration = Duration::from_millis(500);
const TAIL: Duration = Duration::from_millis(1500);

fn record(path: &str) -> ExitCode {
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
    let mut failed = false;
    for &theme in &story.demo.themes {
        let result = Boundary::for_story(&story, &work).and_then(|boundary| {
            boundary.run(|| {
                let recorded = Session::start(&boundary, &story, theme)
                    .map_err(matinee::runner::Failure::from)
                    .and_then(|mut session| {
                        let mut runner = Runner::new(&mut session, &story)?;
                        runner.launch(&story)?;
                        drop(runner);
                        session.start_recording();
                        session.run_for(LEAD_IN)?;
                        let mut runner = Runner::new(&mut session, &story)?;
                        let steps = runner.perform(&story)?;
                        drop(runner);
                        session.run_for(TAIL)?;
                        let recording = session.stop_recording().expect("recording was started");
                        let gif = std::path::Path::new(matinee::boundary::WORK).join("out.gif");
                        let written = recording.write_gif(&gif, story.demo.fps)?;
                        Ok((steps, written))
                    });
                match recorded {
                    Ok((steps, written)) => {
                        println!(
                            "{}: {steps} steps, {} frames, {:.1} s, {} KB",
                            theme.name(),
                            written.frames,
                            written.seconds,
                            written.bytes / 1024
                        );
                        0
                    }
                    Err(failure) => {
                        eprintln!("{path} ({}): {failure}", theme.name());
                        1
                    }
                }
            })
        });
        let target = story.dir.join(format!(
            "{}-{}.gif",
            story.demo.output.display(),
            theme.name()
        ));
        let copied = match result {
            Ok(0) => target
                .parent()
                .map(std::fs::create_dir_all)
                .transpose()
                .and_then(|_| std::fs::copy(work.join("out.gif"), &target).map(drop)),
            Ok(_) => Err(std::io::Error::other("not recorded")),
            Err(e) => Err(e),
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
    let _ = std::fs::remove_dir_all(&work);
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
