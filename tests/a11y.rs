//! Accessibility trees through the session's buses. These launch a real
//! GTK app, so they are ignored by default: run them with
//! `cargo test -- --ignored` on a machine with gnome-calculator.

use std::process::Command;

#[test]
#[ignore = "needs gnome-calculator"]
fn probe_lists_a_gtk_apps_controls() {
    let out = Command::new(env!("CARGO_BIN_EXE_matinee"))
        .args([
            "probe",
            "examples/callboard/demo.toml",
            "--",
            "gnome-calculator",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("window \"Calculator\""), "{text}");
    assert!(text.contains("app \"gnome-calculator\""), "{text}");
    for control in ["button \"7\"", "button \"=\"", "button \"Main Menu\""] {
        assert!(text.contains(control), "{control} missing:\n{text}");
    }
}

/// The brightness, 0 to 255, of one pixel of a PNG.
fn brightness(path: &std::path::Path, x: usize, y: usize) -> u32 {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    let i = y * info.line_size + x * 3;
    (buf[i] as u32 + buf[i + 1] as u32 + buf[i + 2] as u32) / 3
}

#[test]
#[ignore = "needs gnome-calculator"]
fn snapshot_renders_each_theme_and_settles_quickly() {
    let dir = std::env::temp_dir().join(format!("matinee-themes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let start = std::time::Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_matinee"))
        .args(["snapshot", "examples/callboard/demo.toml"])
        .arg(dir.join("calc.png"))
        .args(["--", "gnome-calculator"])
        .output()
        .unwrap();
    let elapsed = start.elapsed();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The window background, left of the keypad.
    let light = brightness(&dir.join("calc-light.png"), 100, 600);
    let dark = brightness(&dir.join("calc-dark.png"), 100, 600);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(light > 200 && dark < 60, "light {light}, dark {dark}");
    // A steady caret lets each session settle soon after the window shows.
    assert!(elapsed.as_secs() < 15, "took {elapsed:?}");
}

#[test]
#[ignore = "needs gnome-calculator"]
fn run_clicks_and_types_through_the_calculator_story() {
    let out = Command::new(env!("CARGO_BIN_EXE_matinee"))
        .args(["run", "examples/calculator/demo.toml"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("all 8 steps passed"), "{stdout}");
}

#[test]
#[ignore = "needs gnome-calculator"]
fn run_fails_a_wrong_expectation_and_names_what_is_showing() {
    let dir = std::env::temp_dir().join(format!("matinee-wrong-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let story = dir.join("demo.toml");
    std::fs::write(
        &story,
        r#"
[demo]
output = "x"
size = [800, 600]
[launch]
app = "gnome-calculator"
wait_for = "="
[[scene]]
steps = [
  { click = "7" }, { click = "+" }, { click = "8" }, { click = "=" },
  { expect = "16", timeout = "1s" },
]
"#,
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_matinee"))
        .arg("run")
        .arg(&story)
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(
        stderr.contains("scene 1, step 5: nothing showing matches \"16\""),
        "{stderr}"
    );
    assert!(stderr.contains("\"15\""), "{stderr}");
}
