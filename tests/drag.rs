//! Drag steps against a small GTK 4 window that the story compiles in its
//! session. Ignored by default: they need a C compiler and GTK 4 headers.
//! Run them with `cargo test -- --ignored`.

use std::process::Command;

/// Run the drag storyboard with `steps` in place of its scene, returning
/// whether it passed and everything it printed.
fn run_drag(name: &str, steps: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("matinee-drag-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy("tests/fixtures/drag/drag.c", dir.join("drag.c")).unwrap();
    let story = std::fs::read_to_string("tests/fixtures/drag/demo.toml").unwrap();
    let head = &story[..story.find("[[scene]]").unwrap()];
    std::fs::write(
        dir.join("demo.toml"),
        format!("{head}[[scene]]\nsteps = [\n{steps}\n]\n"),
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_matinee"))
        .arg("run")
        .arg(dir.join("demo.toml"))
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

#[test]
#[ignore = "needs a C compiler and GTK 4 headers"]
fn the_drag_storyboard_passes() {
    let out = Command::new(env!("CARGO_BIN_EXE_matinee"))
        .args(["run", "tests/fixtures/drag/demo.toml"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("all 4 steps passed"), "{text}");
}

#[test]
#[ignore = "needs a C compiler and GTK 4 headers"]
fn a_slider_lands_where_it_is_dragged() {
    for (to, label) in [
        ("0.3", "volume: 30"),
        ("1.0", "volume: 100"),
        ("0.0", "volume: 0"),
    ] {
        let (ok, text) = run_drag(
            "slider",
            &format!(r#"{{ drag_slider = "Volume", to = {to} }}, {{ expect = "{label}" }}"#),
        );
        assert!(ok, "to {to}: {text}");
    }
}

#[test]
#[ignore = "needs a C compiler and GTK 4 headers"]
fn drag_slider_refuses_something_that_is_not_a_slider() {
    let (ok, text) = run_drag("notslider", r#"{ drag_slider = "Drop here", to = 0.5 }"#);
    assert!(!ok);
    assert!(
        text.contains("\"Drop here\" is not a slider that reports its value"),
        "{text}"
    );
}

#[test]
#[ignore = "needs a C compiler and GTK 4 headers"]
fn coordinate_steps_act_where_they_say() {
    // The slider spans x 52 to 772 at y 191 to 225 in the 800x500 window;
    // its value starts at 20.
    let (ok, text) = run_drag(
        "coordinates",
        r#"{ drag_from = [196, 208], drag_to = [556, 208] }, { expect = "volume: 70" }"#,
    );
    assert!(ok, "{text}");
    let (ok, text) = run_drag("outside", r#"{ click_at = [900, 10] }"#);
    assert!(!ok);
    assert!(
        text.contains("[900, 10] is outside \"Drag lab\", which is 800x500"),
        "{text}"
    );
}
