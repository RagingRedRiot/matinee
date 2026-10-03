//! The terminal pane, through `matinee run`. These start a real session,
//! with bash, dbus-daemon, and the AT-SPI registry from the host.

use std::process::{Command, Output};

fn run(story: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_matinee"))
        .args(["run", story])
        .output()
        .unwrap()
}

/// Run a storyboard written to a temporary folder beside `files`.
fn run_text(name: &str, text: &str, files: &[(&str, &str)]) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("matinee-term-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (file, body) in files {
        std::fs::write(dir.join(file), body).unwrap();
    }
    std::fs::write(dir.join("demo.toml"), text).unwrap();
    let out = run(dir.join("demo.toml").to_str().unwrap());
    std::fs::remove_dir_all(&dir).unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), all)
}

const HEAD: &str = "[demo]\noutput = \"t\"\nsize = [640, 360]\n";

#[test]
fn the_terminal_example_passes() {
    let out = run("examples/terminal/demo.toml");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("all 6 steps passed"), "{text}");
}

#[test]
fn a_failing_command_fails_the_story_and_shows_the_terminal() {
    let (ok, text) = run_text(
        "fail",
        &format!("{HEAD}[[scene]]\nsteps = [{{ run = \"echo before; ls /nope\" }}]\n"),
        &[],
    );
    assert!(!ok, "{text}");
    assert!(
        text.contains("scene 1, step 1: `echo before; ls /nope` exited with 2, expected 0"),
        "{text}"
    );
    assert!(text.contains("| before"), "{text}");
}

#[test]
fn shell_state_carries_between_steps() {
    let (ok, text) = run_text(
        "state",
        &format!(
            "{HEAD}[[scene]]\nsteps = [\n  {{ run = \"cd /tmp && export GREETING=hi\" }},\n  {{ run = \"test $PWD = /tmp && test $GREETING = hi\" }},\n]\n"
        ),
        &[],
    );
    assert!(ok, "{text}");
}

#[test]
fn a_staged_step_shows_its_command_but_runs_the_other() {
    let story = format!(
        "{HEAD}[[scene]]\nsteps = [\n  {{ terminal = \"gh pr list\", run = \"cat prs.txt\" }},\n  {{ expect = \"Fix flaky\" }},\n  {{ expect = \"cat prs.txt\", timeout = \"500ms\" }},\n]\n"
    );
    let (ok, text) = run_text(
        "staged",
        &story,
        &[("prs.txt", "#142 Fix flaky upload retry\n")],
    );
    // The fixture's output shows; the command that produced it does not.
    assert!(!ok, "{text}");
    assert!(
        text.contains("scene 1, step 3: nothing showing matches \"cat prs.txt\""),
        "{text}"
    );
}

#[test]
fn seeds_run_before_the_story_and_fail_it_when_they_fail() {
    let (ok, text) = run_text(
        "seed",
        &format!(
            "{HEAD}[[seed]]\nrun = \"echo seeded > ~/mark\"\n[[scene]]\nsteps = [{{ run = \"cat ~/mark\" }}, {{ expect = \"seeded\" }}]\n"
        ),
        &[],
    );
    assert!(ok, "{text}");
    let (ok, text) = run_text(
        "badseed",
        &format!(
            "{HEAD}[[seed]]\nrun = \"echo oops >&2; exit 3\"\n[[scene]]\nsteps = [{{ run = \"true\" }}]\n"
        ),
        &[],
    );
    assert!(!ok, "{text}");
    assert!(
        text.contains("seed 1: `echo oops >&2; exit 3` failed"),
        "{text}"
    );
    assert!(text.contains("oops"), "{text}");
}

#[test]
fn keys_reach_a_program_left_running() {
    // `read` echoes what is typed, through the terminal's own echo, and
    // the subshell prints what it read once Enter arrives.
    let command = "(read -p 'name? ' n && echo hello $n && read -n 1 && exit 3)";
    let story = format!(
        "{HEAD}[[scene]]\nsteps = [\n  {{ run = \"{command}\", wait = false, exit = 3 }},\n  {{ wait_for = \"name?\" }},\n  {{ type = \"Ada\" }},\n  {{ key = \"enter\" }},\n  {{ expect = \"hello Ada\" }},\n  {{ key = \"q\" }},\n  {{ wait_exit = \"{command}\" }},\n]\n"
    );
    let (ok, text) = run_text("keys", &story, &[]);
    assert!(ok, "{text}");
}

#[test]
fn a_program_left_running_is_checked_at_the_end() {
    let (ok, text) = run_text(
        "end",
        &format!("{HEAD}[[scene]]\nsteps = [{{ run = \"(exit 4)\", wait = false }}]\n"),
        &[],
    );
    assert!(!ok, "{text}");
    assert!(
        text.contains("end of story: `(exit 4)` exited with 4, expected 0"),
        "{text}"
    );
}

#[test]
fn keys_to_a_program_that_has_exited_fail() {
    let (ok, text) = run_text(
        "exited",
        &format!(
            "{HEAD}[[scene]]\nsteps = [\n  {{ run = \"true\", wait = false }},\n  {{ hold = \"300ms\" }},\n  {{ type = \"x\" }},\n]\n"
        ),
        &[],
    );
    assert!(!ok, "{text}");
    assert!(text.contains("`true` already exited with 0"), "{text}");
}
