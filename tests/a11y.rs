#![cfg(target_os = "linux")]

//! Accessibility trees through the session's buses. These launch a real
//! GTK app, so they are ignored by default: run them with
//! `cargo test -- --ignored` on a machine with gnome-calculator.

use std::process::Command;

#[test]
#[ignore = "needs gnome-calculator"]
fn menu_coordinates_open_a_verified_dialog_and_return_focus() {
    let out = Command::new(env!("CARGO_BIN_EXE_matinee"))
        .args(["run", "tests/fixtures/calculator-menu/demo.toml"])
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    assert!(text.contains("all 6 steps passed"), "{text}");
}

#[test]
#[ignore = "needs gnome-calculator"]
fn probe_lists_a_gtk_apps_controls() {
    let out = Command::new(env!("CARGO_BIN_EXE_matinee"))
        .args([
            "probe",
            "examples/calculator/demo.toml",
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
    assert!(
        text.contains("control: semantic targets published"),
        "{text}"
    );
    for control in ["button \"7\"", "button \"=\"", "button \"Main Menu\""] {
        assert!(text.contains(control), "{control} missing:\n{text}");
    }
}

/// An RGB PNG, decoded.
struct Picture {
    rgb: Vec<u8>,
    line: usize,
}

impl Picture {
    fn open(path: &std::path::Path) -> Picture {
        let decoder =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let mut rgb = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut rgb).unwrap();
        Picture {
            rgb,
            line: info.line_size,
        }
    }

    /// The brightness, 0 to 255, of one pixel.
    fn brightness(&self, x: usize, y: usize) -> u32 {
        let i = y * self.line + x * 3;
        (self.rgb[i] as u32 + self.rgb[i + 1] as u32 + self.rgb[i + 2] as u32) / 3
    }
}

#[test]
#[ignore = "needs gnome-calculator"]
fn snapshot_renders_each_theme_and_settles_quickly() {
    let dir = std::env::temp_dir().join(format!("matinee-themes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let start = std::time::Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_matinee"))
        .args(["snapshot", "examples/calculator/demo.toml"])
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
    let light = Picture::open(&dir.join("calc-light.png")).brightness(100, 600);
    let dark = Picture::open(&dir.join("calc-dark.png")).brightness(100, 600);
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
    assert!(stdout.contains("all 10 steps passed"), "{stdout}");
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

#[test]
#[ignore = "needs gnome-calculator"]
fn clicks_find_the_app_not_matching_terminal_text() {
    // The terminal shows `7` and `8` in the typed command; clicks must
    // still go to the calculator's buttons.
    let out = Command::new(env!("CARGO_BIN_EXE_matinee"))
        .args(["run", "examples/mixed/demo.toml"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[ignore = "needs gnome-calculator"]
fn a_target_behind_the_terminal_pane_is_refused() {
    let dir = std::env::temp_dir().join(format!("matinee-behind-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let story = dir.join("demo.toml");
    // The calculator is taller than the 300 points left above the pane, so
    // its buttons are refused as soon as the launch looks for one.
    std::fs::write(
        &story,
        r#"
[demo]
output = "x"
size = [800, 600]
[terminal]
size = 300
[launch]
app = "gnome-calculator"
wait_for = "="
[[scene]]
steps = [{ run = "true" }, { click = "+", timeout = "1s" }]
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
    // The window's height follows the installed fonts, so only its width,
    // the screen's, is fixed.
    assert!(
        stderr.contains("is behind the terminal pane: the window is 800x"),
        "{stderr}"
    );
}

#[test]
#[ignore = "needs gnome-calculator"]
fn the_calculator_lays_out_its_entry_at_full_height() {
    // GNOME Calculator measures its entry against the first size it is
    // given. Configured straight to the screen's size, the entry stayed a
    // sliver and hid what was typed; first drawing at its own size and then
    // being resized gives it its real height.
    let dir = std::env::temp_dir().join(format!("matinee-entry-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let story = dir.join("demo.toml");
    std::fs::write(
        &story,
        "[demo]\noutput = \"x\"\nsize = [800, 600]\nthemes = [\"light\"]\n\
         [launch]\napp = \"gnome-calculator\"\n[[scene]]\nsteps = [{ hold = \"1s\" }]\n",
    )
    .unwrap();
    let shot = dir.join("shot.png");
    let out = Command::new(env!("CARGO_BIN_EXE_matinee"))
        .arg("snapshot")
        .arg(&story)
        .arg(&shot)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Count white rows down the middle of the display, in device pixels.
    let picture = Picture::open(&shot);
    let white = (300..1100)
        .filter(|&y| picture.brightness(600, y) == 255)
        .count();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(white >= 100, "the entry is {white} device pixels tall");
}

#[test]
#[ignore = "needs gnome-calculator"]
fn expect_reads_what_a_text_field_contains() {
    let dir = std::env::temp_dir().join(format!("matinee-field-{}", std::process::id()));
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
  { click = "7" }, { click = "+" }, { click = "8" },
  { expect = "7+8" },
  { expect = "7+9", timeout = "1s" },
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
    // The right contents pass; the wrong ones fail, naming what is there.
    assert!(
        stderr.contains("scene 1, step 5: nothing showing matches \"7+9\""),
        "{stderr}"
    );
    assert!(stderr.contains("text box \"7+8\""), "{stderr}");
}
