#![cfg(target_os = "linux")]

//! `--no-isolation`: stories on the host, for systems without unprivileged
//! user namespaces. What isolation adds is checked in `boundary.rs`; these
//! check what must hold either way.

use std::process::{Command, Output};

const STORY: &str = "examples/calculator/demo.toml";

fn shell(script: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_matinee"))
        .args(["shell", "--no-isolation", STORY, "--", "bash", "-c", script])
        .output()
        .unwrap()
}

fn stdout(script: &str) -> String {
    let out = shell(script);
    assert!(
        out.status.success(),
        "{script}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

#[test]
fn commands_get_a_fresh_home_and_start_beside_the_storyboard() {
    let host_home = std::env::var("HOME").unwrap();
    let home = stdout("echo $HOME");
    assert_ne!(home, host_home);
    assert!(home.ends_with("/session/home"), "{home}");
    let cwd = stdout("pwd");
    let story_dir = std::fs::canonicalize("examples/calculator").unwrap();
    assert_eq!(cwd, story_dir.display().to_string());
    // Nothing written to one session's home is there in the next.
    assert_eq!(stdout("touch ~/left-behind && ls ~"), "left-behind");
    assert_eq!(stdout("ls -A ~"), "");
    assert_eq!(stdout("stat -c %a ~ $XDG_RUNTIME_DIR"), "700\n700");
}

#[test]
fn the_warning_is_shown_and_the_exit_status_comes_back() {
    let out = shell("exit 3");
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("running without isolation"));
}

#[test]
fn detached_children_stop_with_the_session() {
    let marker = format!("986.{}", std::process::id());
    // Their output goes elsewhere, so a survivor cannot hold the shell's
    // pipes open and the check fails rather than hangs.
    for script in [
        format!("setsid sleep {marker} >/dev/null 2>&1 & exit 0"),
        format!("(setsid sleep {marker} >/dev/null 2>&1 &) ; sleep 0.2; exit 1"),
    ] {
        shell(&script);
        let survivors: Vec<String> = std::fs::read_dir("/proc")
            .unwrap()
            .filter_map(|e| {
                let path = e.ok()?.path();
                let cmd = std::fs::read(path.join("cmdline")).ok()?;
                String::from_utf8_lossy(&cmd)
                    .contains(&marker)
                    .then(|| path.file_name().unwrap().to_string_lossy().into_owned())
            })
            .collect();
        for pid in &survivors {
            let _ = Command::new("kill").arg(pid).status();
        }
        assert!(survivors.is_empty(), "a detached child outlived: {script}");
    }
}

#[test]
fn a_terminal_story_passes() {
    let out = Command::new(env!("CARGO_BIN_EXE_matinee"))
        .args(["--no-isolation", "run", "examples/terminal/demo.toml"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("all 6 steps passed"), "{text}");
}

#[test]
fn check_refuses_the_flag() {
    let out = Command::new(env!("CARGO_BIN_EXE_matinee"))
        .args(["check", "--no-isolation", STORY])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}
