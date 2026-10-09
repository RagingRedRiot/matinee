//! `matinee record` over folders: finding storyboards, recording each, and
//! refusing clashes. These start real sessions, with bash, dbus-daemon, and
//! the AT-SPI registry from the host.

use std::{path::Path, process::Command};

fn story(output: &str, text: &str) -> String {
    format!(
        "[demo]\noutput = \"{output}\"\nsize = [320, 180]\nthemes = [\"dark\"]\n\
         [[scene]]\nsteps = [{{ run = \"echo {text}\" }}]\n"
    )
}

fn folder(name: &str, files: &[(&str, String)]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("matinee-rec-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (file, text) in files {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}

fn record(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_matinee"))
        .arg("record")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

#[test]
fn assertion_and_failure_stills_come_back_from_run_and_record() {
    let text = story("failure", "ready").replace(
        "steps = [{ run = \"echo ready\" }]",
        "steps = [{ run = \"echo ready\" }, { wait_for = \"ready\" }, { expect = \"ready\" }, { expect = \"wrong\", timeout = \"200ms\" }]",
    );
    let dir = folder("artifacts", &[("demo.toml", text)]);
    for (command, isolation) in [
        ("run", true),
        ("record", true),
        ("run", false),
        ("record", false),
    ] {
        let directory = format!("{command}-{isolation}");
        let mut process = Command::new(env!("CARGO_BIN_EXE_matinee"));
        if !isolation {
            process.arg("--no-isolation");
        }
        let out = process
            .args([command, "demo.toml", "--artifacts", &directory])
            .current_dir(&dir)
            .output()
            .unwrap();
        let diagnostic = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{diagnostic}");
        assert!(diagnostic.contains("scene 1, step 4"), "{diagnostic}");
        let collected = std::fs::read_dir(dir.join(directory))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        for name in ["scene-1-step-2.png", "scene-1-step-3.png", "failure.png"] {
            let decoder = png::Decoder::new(std::io::BufReader::new(
                std::fs::File::open(collected.join(name)).unwrap(),
            ));
            let mut reader = decoder.read_info().unwrap();
            assert_eq!((reader.info().width, reader.info().height), (640, 360));
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            let frame = reader.next_frame(&mut pixels).unwrap();
            let pixels = &pixels[..frame.buffer_size()];
            assert!(
                pixels.chunks_exact(3).any(|p| p != &pixels[..3]),
                "blank {name}"
            );
        }
    }
    assert!(!dir.join("failure-dark.gif").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn artifacts_need_a_directory_and_a_running_story() {
    for args in [
        vec!["run", "demo.toml", "--artifacts"],
        vec![
            "check",
            "examples/terminal/demo.toml",
            "--artifacts",
            "unused",
        ],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_matinee"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&out.stderr)
                .contains("--artifacts <folder> applies to run and record")
        );
    }
}

#[test]
fn every_storyboard_in_the_folder_is_recorded() {
    let dir = folder(
        "all",
        &[
            ("one/demo.toml", story("one", "first")),
            ("two/tour.toml", story("tour", "second")),
            // An input file a demo might use; not a storyboard.
            ("two/release.toml", "name = \"release\"\n".into()),
        ],
    );
    let (ok, text) = record(&dir, &[]);
    assert!(ok, "{text}");
    assert!(dir.join("one/one-dark.gif").is_file(), "{text}");
    assert!(dir.join("two/tour-dark.gif").is_file(), "{text}");
    assert!(text.contains("recorded 2 of 2 demos"), "{text}");

    // A folder named on the command line is searched the same way.
    let (ok, text) = record(&dir, &["two"]);
    assert!(ok, "{text}");
    assert!(!text.contains("== "), "one demo needs no headings: {text}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn clashing_outputs_and_mistakes_stop_before_recording() {
    let dir = folder(
        "clash",
        &[
            ("a.toml", story("same", "a")),
            ("b.toml", story("./same", "b")),
        ],
    );
    let (ok, text) = record(&dir, &[]);
    assert!(!ok);
    assert!(text.contains("both write"), "{text}");
    assert!(!dir.join("same-dark.gif").exists());

    std::fs::write(
        dir.join("b.toml"),
        story("other", "b").replace("size", "sise"),
    )
    .unwrap();
    let (ok, text) = record(&dir, &[]);
    assert!(!ok);
    assert!(text.contains("b.toml"), "{text}");
    assert!(
        !dir.join("same-dark.gif").exists(),
        "nothing recorded: {text}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_folder_with_no_storyboards_is_an_error() {
    let dir = folder(
        "none",
        &[("Cargo.toml", "[package]\nname = \"x\"\n".into())],
    );
    let (ok, text) = record(&dir, &[]);
    assert!(!ok);
    assert!(text.contains("no storyboards in ."), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}
