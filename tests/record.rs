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
