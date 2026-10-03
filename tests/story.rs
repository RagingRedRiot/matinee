use matinee::story::{Action, CameraMove, Story, Theme, parse_duration};
use std::{path::PathBuf, time::Duration};

const HEAD: &str = r#"
[demo]
output = "demo"
size = [800, 480]

[launch]
app = "/usr/bin/watch-file"
"#;

fn parse(steps: &str) -> Result<Story, String> {
    Story::parse(
        &format!("{HEAD}\n[[scene]]\nsteps = [\n{steps}\n]\n"),
        PathBuf::from("tests"),
    )
}

fn error(steps: &str) -> String {
    parse(steps).expect_err("storyboard should be rejected")
}

#[test]
fn vision_example_loads() {
    let story = Story::load("tests/fixtures/vision.toml".as_ref()).unwrap();
    assert_eq!(story.demo.themes, [Theme::Light, Theme::Dark]);
    assert_eq!(story.seeds.len(), 2);
    assert_eq!(story.launches[0].name, "callboard-gui");
    assert_eq!(story.scenes.len(), 3);
    assert_eq!(story.steps().filter(|s| s.action.is_staged()).count(), 1);
    let Action::Drag { from, to } = &story.scenes[1].steps[0].action else {
        panic!("expected a drag");
    };
    assert_eq!(from.within.as_deref(), Some("Inbox"));
    assert_eq!(from.near.as_deref(), Some("Fix flaky upload retry"));
    assert_eq!(to.name, "Todos");
    assert_eq!(
        story.scenes[2].steps[2].action,
        Action::Camera(CameraMove::Overview)
    );
}

#[test]
fn defaults() {
    let story = parse(r#"{ click = "Go" }"#).unwrap();
    assert_eq!(story.demo.scale, 2);
    assert_eq!(story.demo.fps, 15);
    assert_eq!(story.demo.themes, [Theme::Light, Theme::Dark]);
    assert_eq!(story.camera.ease, Duration::from_millis(400));
    assert_eq!(story.camera.reading_width, 800);
    assert_eq!(story.demo.lossy, 6);
    assert_eq!(story.launches[0].name, "watch-file");
}

#[test]
fn staged_run_shows_one_command_and_runs_another() {
    let story = parse(r#"{ terminal = "gh pr list", run = "cat prs.txt", exit = 0 }"#).unwrap();
    let step = &story.scenes[0].steps[0];
    assert!(step.action.is_staged());
    assert_eq!(
        step.action,
        Action::Run {
            command: "cat prs.txt".into(),
            shown: Some("gh pr list".into()),
            exit: 0,
            wait: true,
        }
    );
}

#[test]
fn plain_run_is_not_staged() {
    let story = parse(r#"{ run = "false", exit = 1 }"#).unwrap();
    assert!(!story.scenes[0].steps[0].action.is_staged());
}

#[test]
fn playback_reads_a_file_beside_the_storyboard() {
    let story = parse(r#"{ terminal = "gh auth status", output = "story.rs" }"#).unwrap();
    assert_eq!(
        story.scenes[0].steps[0].action,
        Action::Playback {
            shown: "gh auth status".into(),
            output: PathBuf::from("story.rs"),
        }
    );
    assert!(error(r#"{ terminal = "gh", output = "missing.txt" }"#).contains("does not exist"));
}

#[test]
fn terminal_alone_is_rejected() {
    assert!(error(r#"{ terminal = "gh pr list" }"#).contains("add `run`"));
}

#[test]
fn output_with_run_is_rejected() {
    assert!(
        error(r#"{ terminal = "gh", run = "true", output = "story.rs" }"#)
            .contains("without `run`")
    );
}

#[test]
fn one_action_per_step() {
    let e = error(r#"{ click = "A", key = "enter" }"#);
    assert!(e.starts_with("scene 1, step 1:"), "{e}");
    assert!(e.contains("click, key"), "{e}");
}

#[test]
fn context_keys_need_a_target() {
    assert!(error(r#"{ key = "enter", within = "Inbox" }"#).contains("`within` only applies"));
    assert!(error(r#"{ run = "true", in = "watch-file" }"#).contains("`in` only applies"));
}

#[test]
fn unknown_app_is_named() {
    let e = error(r#"{ click = "Go", in = "editor" }"#);
    assert!(e.contains("launched: watch-file"), "{e}");
}

#[test]
fn targets_need_a_launched_app() {
    let e = Story::parse(
        "[demo]\noutput = \"d\"\nsize = [1, 1]\n[[scene]]\nsteps = [{ click = \"Go\" }]\n",
        PathBuf::new(),
    )
    .unwrap_err();
    assert!(e.contains("launches none"), "{e}");
}

#[test]
fn plain_expectations_may_check_terminal_text() {
    let story = |step: &str| {
        Story::parse(
            &format!("[demo]\noutput = \"d\"\nsize = [1, 1]\n[[scene]]\nsteps = [{step}]\n"),
            PathBuf::new(),
        )
    };
    assert!(story(r#"{ expect = "done" }"#).is_ok());
    assert!(story(r#"{ wait_for = "ready" }"#).is_ok());
    let e = story(r#"{ expect = "OK", role = "button" }"#).unwrap_err();
    assert!(e.contains("launches none"), "{e}");
}

#[test]
fn drag_and_slider_check_to() {
    assert!(error(r#"{ drag = "Card" }"#).contains("needs `to`"));
    assert!(error(r#"{ drag = "Card", to = 0.5 }"#).contains("names the drop target"));
    assert!(error(r#"{ drag_slider = "Size", to = 1.5 }"#).contains("0 to 1"));
    assert!(error(r#"{ drag_slider = "Size", to = "Big" }"#).contains("0 to 1"));
}

#[test]
fn camera_moves() {
    assert!(error(r#"{ camera = "closeup" }"#).contains("not a camera move"));
    assert!(error(r#"{ camera = { focus = "Theme", zoom = 3.0 } }"#).contains("max_zoom"));
}

#[test]
fn hold_alone_is_a_pause() {
    let story = parse(r#"{ hold = "1s" }"#).unwrap();
    assert_eq!(story.scenes[0].steps[0].action, Action::Pause);
    assert!(error("{}").contains("does nothing"));
}

#[test]
fn unknown_keys_are_rejected() {
    assert!(error(r#"{ clik = "Go" }"#).contains("clik"));
}

#[test]
fn durations() {
    assert_eq!(parse_duration("2.4s"), Ok(Duration::from_millis(2400)));
    assert_eq!(parse_duration("600ms"), Ok(Duration::from_millis(600)));
    assert!(parse_duration("2.4").is_err());
    assert!(parse_duration("-1s").is_err());
    assert!(error(r#"{ click = "Go", hold = "2" }"#).contains("hold:"));
}

#[test]
fn coordinate_steps() {
    use matinee::story::Spot;
    let story = parse(
        r#"{ click_at = [10, -20], window = "Main" },
{ hover_at = [5.5, 6] },
{ drag_from = [-20, -20], drag_to = [560, 230] }"#,
    )
    .unwrap();
    let steps = &story.scenes[0].steps;
    assert_eq!(
        steps[0].action,
        Action::ClickAt(Spot {
            x: 10.0,
            y: -20.0,
            window: Some("Main".into())
        })
    );
    assert_eq!(
        steps[1].action,
        Action::HoverAt(Spot {
            x: 5.5,
            y: 6.0,
            window: None
        })
    );
    assert!(matches!(
        &steps[2].action,
        Action::DragAt { from, to } if from.x == -20.0 && to.y == 230.0
    ));
    assert!(error(r#"{ drag_from = [1, 2] }"#).contains("needs `drag_to`"));
    assert!(error(r#"{ drag_to = [1, 2] }"#).contains("`drag_to` only applies"));
    assert!(error(r#"{ click_at = [1, 2], role = "button" }"#).contains("`role` only applies"));
    let e = Story::parse(
        "[demo]\noutput = \"d\"\nsize = [1, 1]\n[[scene]]\nsteps = [{ click_at = [1, 1] }]\n",
        PathBuf::new(),
    )
    .unwrap_err();
    assert!(e.contains("launches none"), "{e}");
}

/// A terminal-only storyboard, with no app launched.
fn parse_terminal(steps: &str) -> Result<Story, String> {
    Story::parse(
        &format!("[demo]\noutput = \"t\"\nsize = [640, 360]\n[[scene]]\nsteps = [\n{steps}\n]\n"),
        PathBuf::from("tests"),
    )
}

#[test]
fn keys_go_to_a_program_left_running_in_the_terminal() {
    let story = parse(
        r#"{ run = "less notes.txt", wait = false },
           { type = "/retry", in = "terminal" },
           { key = "q", in = "terminal" },
           { wait_exit = "less notes.txt" },
           { key = "ctrl+k" }"#,
    )
    .unwrap();
    let actions: Vec<&Action> = story.steps().map(|s| &s.action).collect();
    assert!(matches!(actions[0], Action::Run { wait: false, .. }));
    assert_eq!(
        actions[1],
        &Action::Type {
            text: "/retry".into(),
            terminal: true
        }
    );
    assert_eq!(
        actions[4],
        &Action::Key {
            chord: "ctrl+k".into(),
            terminal: false
        },
        "without `in`, keys go to the app"
    );

    // With no app, keys go to the terminal on their own.
    let story =
        parse_terminal(r#"{ run = "cat", wait = false }, { type = "hi" }, { key = "ctrl+d" }"#)
            .unwrap();
    assert!(story.steps().skip(1).all(|s| matches!(
        s.action,
        Action::Type { terminal: true, .. } | Action::Key { terminal: true, .. }
    )));
}

#[test]
fn wait_exit_may_name_the_command_shown() {
    parse_terminal(
        r#"{ terminal = "htop", run = "top -b -n 1", wait = false }, { wait_exit = "htop" }"#,
    )
    .unwrap();
}

#[test]
fn terminal_steps_run_one_command_at_a_time() {
    let error = |steps| parse_terminal(steps).expect_err("storyboard should be rejected");
    assert!(
        error(r#"{ run = "cat", wait = false }, { run = "ls" }"#)
            .contains("scene 1, step 2: this runs a command while `cat` is still running"),
    );
    assert!(
        error(r#"{ run = "cat", wait = false }, { wait_exit = "ls" }"#)
            .contains("names no running command; `cat` is running")
    );
    assert!(error(r#"{ wait_exit = "cat" }"#).contains("no command is running"));
    assert!(error(r#"{ type = "hi" }"#).contains("nothing is running there"));
    assert!(
        error(r#"{ run = "cat", wait = false, timeout = "5s" }"#)
            .contains("put it on the `wait_exit` step")
    );
    assert!(
        error(r#"{ run = "cat", wait = false }, { key = "super+a" }"#)
            .contains("does not reach a program in the terminal")
    );
    assert!(error(r#"{ wait = false, click = "Go" }"#).contains("`wait` only applies to run"));
    assert!(
        parse(r#"{ type = "hi", in = "watch-file" }"#)
            .unwrap_err()
            .contains("keys go to the focused window, or to the terminal")
    );
}

#[test]
fn reading_width_and_lossy_can_be_set() {
    // Settings in the `demo` and `camera` tables, before a minimal story.
    let story = |demo: &str, camera: &str| {
        Story::parse(
            &format!(
                "[demo]\noutput = \"d\"\nsize = [800, 480]\n{demo}\n[camera]\n{camera}\n\
                 [launch]\napp = \"x\"\n[[scene]]\nsteps = [{{ click = \"Go\" }}]\n"
            ),
            PathBuf::from("tests"),
        )
    };
    assert_eq!(story("lossy = 0", "").unwrap().demo.lossy, 0);
    assert!(story("lossy = 40", "").unwrap_err().contains("demo.lossy"));
    let narrow = story("", "reading_width = 400").unwrap();
    assert_eq!(narrow.camera.reading_width, 400);
    assert!(
        story("", "reading_width = 50")
            .unwrap_err()
            .contains("camera.reading_width")
    );
}
