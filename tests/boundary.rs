#![cfg(target_os = "linux")]

//! The isolation contract from BUILD-DECISIONS.md, checked through
//! `matinee shell`. These need a kernel that allows unprivileged user
//! namespaces.

use std::process::{Command, Output};

const STORY: &str = "examples/calculator/demo.toml";

fn shell(script: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_matinee"))
        .args(["shell", STORY, "--", "bash", "-c", script])
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
fn host_paths_are_hidden() {
    let home = std::env::var("HOME").unwrap();
    let uid = host_uid();
    for path in [
        home.as_str(),
        "/run/dbus/system_bus_socket",
        &format!("/run/user/{uid}/bus"),
        &format!("/run/user/{uid}/wayland-0"),
        "/tmp/.X11-unix",
        "/sys",
        "/media",
        "/mnt",
        "/root",
        "/etc/shadow",
        "/etc/ssh",
        "/var/lib",
    ] {
        assert_eq!(
            stdout(&format!("test -e '{path}' && echo visible || echo hidden")),
            "hidden",
            "{path}"
        );
    }
}

#[test]
fn host_tmp_is_hidden() {
    let sentinel = std::env::temp_dir().join(format!("matinee-sentinel-{}", std::process::id()));
    std::fs::write(&sentinel, "").unwrap();
    let seen = stdout(&format!(
        "test -e '{}' && echo visible || echo hidden",
        sentinel.display()
    ));
    std::fs::remove_file(&sentinel).unwrap();
    assert_eq!(seen, "hidden");
}

#[test]
fn identity_is_synthetic() {
    assert_eq!(stdout("hostname"), "matinee");
    assert_eq!(stdout("cat /etc/hostname"), "matinee");
    assert_eq!(stdout("whoami"), "matinee");
    assert_eq!(stdout("echo $HOME"), "/home/matinee");
    assert_eq!(
        stdout("cat /etc/machine-id"),
        "6d6174696e656500000000000000000a"
    );
    assert_eq!(stdout("id -u"), host_uid().to_string());
}

#[test]
fn only_session_processes_are_visible() {
    assert_eq!(stdout("cat /proc/1/comm"), "matinee");
    assert_eq!(
        stdout("for p in /proc/[0-9]*; do echo -n \"${p#/proc/}:$(cat $p/comm) \"; done"),
        "1:matinee 2:bash"
    );
}

#[test]
fn story_and_root_are_read_only() {
    assert_eq!(stdout("test -f demo.toml && echo yes"), "yes");
    for path in ["/project/x", "demo.toml", "/x", "/usr/x", "/etc/x"] {
        assert_eq!(
            stdout(&format!(
                "touch {path} 2>/dev/null && echo wrote || echo refused"
            )),
            "refused",
            "{path}"
        );
    }
    for path in [
        "/work/x",
        "/tmp/x",
        "/home/matinee/x",
        "/var/tmp/x",
        "/dev/shm/x",
    ] {
        assert_eq!(
            stdout(&format!("touch {path} && echo wrote")),
            "wrote",
            "{path}"
        );
    }
}

#[test]
fn session_folders_are_not_writable_by_others() {
    let uid = host_uid();
    // Owner-only or sticky folders are fine; others must not be writable
    // by group or others.
    assert_eq!(
        stdout(&format!(
            "stat -c '%n %a' /run /run/user /run/user/{uid} /home /home/matinee; umask"
        )),
        format!("/run 755\n/run/user 755\n/run/user/{uid} 700\n/home 755\n/home/matinee 700\n0022")
    );
}

#[test]
fn network_is_loopback_only() {
    assert_eq!(
        stdout(
            "timeout 2 bash -c 'echo > /dev/tcp/1.1.1.1/53' 2>/dev/null && echo open || echo blocked"
        ),
        "blocked"
    );
    // A terminal command reaching a service started earlier in the session.
    assert_eq!(
        stdout(
            "python3 -c 'import socket,sys; s=socket.socket(); s.bind((\"127.0.0.1\", 8765)); s.listen(); \
             c,_=s.accept(); sys.stdout.write(c.recv(5).decode())' & \
             sleep 0.3; echo -n ready > /dev/tcp/127.0.0.1/8765; wait"
        ),
        "ready"
    );
}

#[test]
#[ignore = "needs pasta and outside network access"]
fn network_opt_in_keeps_the_host_out_of_reach() {
    // A service on the host's loopback, which the session must not reach.
    let host_service = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = host_service.local_addr().unwrap().port();
    let host_net = std::fs::read_link("/proc/self/ns/net").unwrap();
    // The host's abstract Unix sockets, such as an X display, belong to its
    // network namespace.
    let host_abstract: Vec<String> = std::fs::read_to_string("/proc/net/unix")
        .unwrap()
        .lines()
        .filter_map(|l| l.split_whitespace().nth(7))
        .filter(|name| name.starts_with('@'))
        .map(str::to_owned)
        .collect();
    let (seen, root) = outside(
        "network",
        "[session]\nnetwork = true",
        &format!(
            "readlink /proc/self/ns/net; \
             getent hosts example.com >/dev/null && echo dns; \
             test -d /etc/ssl/certs && echo certs; \
             timeout 2 bash -c 'echo > /dev/tcp/127.0.0.1/{port}' 2>/dev/null && echo open || echo closed; \
             awk 'NR > 1 && $8 ~ /^@/ {{ print $8 }}' /proc/net/unix"
        ),
    );
    std::fs::remove_dir_all(root).unwrap();
    let mut lines = seen.lines();
    assert_ne!(lines.next(), Some(host_net.to_str().unwrap()));
    assert_eq!(lines.next(), Some("dns"));
    assert_eq!(lines.next(), Some("certs"));
    assert_eq!(lines.next(), Some("closed"), "host loopback reachable");
    for name in lines {
        assert!(!host_abstract.iter().any(|h| h == name), "{name} visible");
    }
}

#[test]
fn terminals_work() {
    assert_eq!(
        stdout("python3 -c 'import os,pty; m,s=pty.openpty(); print(os.isatty(s))'"),
        "True"
    );
}

#[test]
fn exit_status_comes_back() {
    assert_eq!(shell("exit 3").status.code(), Some(3));
    assert_eq!(shell("true").status.code(), Some(0));
}

#[test]
fn detached_children_stop_with_the_session() {
    // A distinctive sleep that detaches and would outlive the shell.
    let marker = format!("987.{}", std::process::id());
    for script in [
        format!("setsid sleep {marker} & exit 0"),
        format!("setsid sleep {marker} & exit 1"),
    ] {
        shell(&script);
        let survivors: Vec<_> = std::fs::read_dir("/proc")
            .unwrap()
            .filter_map(|e| std::fs::read(e.ok()?.path().join("cmdline")).ok())
            .filter(|cmd| String::from_utf8_lossy(cmd).contains(&marker))
            .collect();
        assert!(survivors.is_empty(), "a detached child outlived: {script}");
    }
}

#[test]
fn bad_storyboard_is_reported() {
    let out = Command::new(env!("CARGO_BIN_EXE_matinee"))
        .args(["shell", "tests/story.rs"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("TOML parse error"));
}

#[test]
fn project_root_holds_the_storyboard() {
    // This repository is the project, so its files are visible and the
    // session starts in the storyboard's folder within it.
    assert_eq!(stdout("pwd"), "/project/examples/calculator");
    assert_eq!(stdout("test -f /project/Cargo.toml && echo yes"), "yes");
}

/// A storyboard at `home/proj/story` under a temporary folder outside any
/// repository, run with that `home` as the host home. Callers may prepare
/// the folder first; the caller removes it.
fn outside(name: &str, story_extra: &str, script: &str) -> (String, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("matinee-{name}-{}", std::process::id()));
    std::fs::create_dir_all(root.join("home/proj/story")).unwrap();
    std::fs::write(
        root.join("home/proj/story/demo.toml"),
        format!("[demo]\noutput = \"d\"\nsize = [10, 10]\n{story_extra}\n[[scene]]\nsteps = [{{ run = \"true\" }}]\n"),
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_matinee"))
        .args([
            "shell",
            "home/proj/story/demo.toml",
            "--",
            "bash",
            "-c",
            script,
        ])
        .current_dir(&root)
        .env("HOME", root.join("home"))
        .output()
        .unwrap();
    let text = if out.status.success() {
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    } else {
        format!("failed: {}", String::from_utf8_lossy(&out.stderr).trim())
    };
    (text, root)
}

#[test]
fn without_a_repository_only_the_storyboard_folder_is_shown() {
    let (seen, root) = outside("norepo", "", "pwd; ls /project");
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(seen, "/project\ndemo.toml");
}

#[test]
fn a_repository_at_home_is_not_a_project() {
    let root = std::env::temp_dir().join(format!("matinee-homerepo-{}", std::process::id()));
    std::fs::create_dir_all(root.join("home/.git")).unwrap();
    let (seen, root) = outside("homerepo", "", "pwd; ls -A /project");
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(seen, "/project\ndemo.toml");
}

#[test]
fn declared_mounts_are_shown_read_only() {
    let root = std::env::temp_dir().join(format!("matinee-mounts-{}", std::process::id()));
    std::fs::create_dir_all(root.join("home/tools/bin")).unwrap();
    std::fs::create_dir_all(root.join("home/proj/shared")).unwrap();
    std::fs::write(root.join("home/tools/bin/tool"), "").unwrap();
    std::fs::write(root.join("home/proj/shared/data"), "").unwrap();
    let home = root.join("home");
    let script = format!(
        "ls {home}/tools/bin {home}/proj/shared; touch {home}/tools/bin/x 2>/dev/null && echo wrote || echo refused; ls {home}",
        home = home.display()
    );
    let (seen, root) = outside(
        "mounts",
        "[session]\nmounts = [\"~/tools/bin\", \"../shared\"]",
        &script,
    );
    std::fs::remove_dir_all(root).unwrap();
    let home = home.display();
    assert_eq!(
        seen,
        format!("{home}/proj/shared:\ndata\n\n{home}/tools/bin:\ntool\nrefused\nproj\ntools")
    );
}

#[test]
fn path_folders_put_programs_on_path() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("matinee-path-{}", std::process::id()));
    std::fs::create_dir_all(root.join("home/tools/bin")).unwrap();
    let tool = root.join("home/tools/bin/greet");
    std::fs::write(&tool, "#!/bin/sh\necho hello from greet\n").unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    let home = root.join("home");
    let (seen, root) = outside(
        "path",
        "[session]\npath = [\"~/tools/bin\"]",
        "greet; echo $PATH | cut -d: -f1",
    );
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(
        seen,
        format!("hello from greet\n{}/tools/bin", home.display())
    );
}

#[test]
fn mounts_must_exist_and_stay_narrow() {
    let (missing, root) = outside("missing", "[session]\nmounts = [\"~/nope\"]", "true");
    std::fs::remove_dir_all(root).unwrap();
    assert!(missing.contains("session.mounts: ~/nope"), "{missing}");
    let (wide, root) = outside("wide", "[session]\nmounts = [\"~\"]", "true");
    std::fs::remove_dir_all(root).unwrap();
    assert!(wide.contains("would show all of"), "{wide}");
}

fn host_uid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))
        .and_then(|l| l.split_whitespace().next()?.parse().ok())
        .unwrap()
}
