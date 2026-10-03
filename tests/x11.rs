//! An X11-only Tk window under Xwayland. Ignored by default: it needs
//! Xwayland and Python's Tk. Run it with `cargo test -- --ignored`.

use std::process::Command;

#[test]
#[ignore = "needs Xwayland and Python's Tk"]
fn an_x11_app_takes_clicks_and_typing() {
    let out = Command::new(env!("CARGO_BIN_EXE_matinee"))
        .args(["run", "tests/fixtures/x11/demo.toml"])
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    assert!(text.contains("all 4 steps passed"), "{text}");
    // Apps stop before Xwayland, so none reports a lost display.
    assert!(!text.contains("X connection"), "{text}");
}
