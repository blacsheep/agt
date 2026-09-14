#![cfg(unix)]
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
fn call(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_agt"))
        .args(["--home", home.to_str().unwrap(), "--output", "json"])
        .args(args)
        .output()
        .unwrap()
}
fn ok(home: &std::path::Path, args: &[&str]) -> Value {
    let out = call(home, args);
    assert!(
        out.status.success(),
        "{} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
#[test]
fn external_agent_crud_execution_permissions_and_dependency_protection() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    ok(home, &["init", home.to_str().unwrap()]);
    let exe = home.join("codex");
    fs::write(
        &exe,
        include_str!("../../agt-harness/tests/fixtures/codex.py"),
    )
    .unwrap();
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o700)).unwrap();
    let config =
        json!({"engine":"codex","executable":exe,"working_directory":home,"sandbox":"read_only"});
    let file = home.join("environment.json");
    fs::write(&file, config.to_string()).unwrap();
    ok(
        home,
        &[
            "environment",
            "create",
            "local",
            "--file",
            file.to_str().unwrap(),
        ],
    );
    let a = ok(
        home,
        &[
            "agent",
            "create",
            "coder",
            "--environment",
            "local",
            "--builtin-tool",
            "resource_management",
        ],
    );
    assert!(a["model"].is_null());
    assert_eq!(a["environment"]["name"], "local");
    assert!(
        !call(home, &["environment", "delete", "local", "--yes"])
            .status
            .success()
    );
    assert!(
        !call(
            home,
            &[
                "agent",
                "create",
                "invalid",
                "--environment",
                "local",
                "--model",
                "fake"
            ]
        )
        .status
        .success()
    );
    assert_eq!(
        ok(home, &["environment", "test", "local"])["status"],
        "connected"
    );
    let run = ok(
        home,
        &[
            "agent",
            "run",
            "coder",
            "--input",
            "TOOL",
            "--request-key",
            "one",
        ],
    );
    assert_eq!(run["output"], "tool completed");
    let replay = ok(
        home,
        &[
            "agent",
            "run",
            "coder",
            "--input",
            "TOOL",
            "--request-key",
            "one",
        ],
    );
    assert_eq!(run["id"], replay["id"]);
    let log = fs::read_to_string(home.join("protocol.jsonl")).unwrap();
    assert_eq!(log.lines().filter(|l| l.contains("turn/start")).count(), 1);
    ok(home, &["agent", "delete", "coder", "--yes"]);
    ok(home, &["environment", "delete", "local", "--yes"]);
}

#[test]
fn terminating_cli_cancels_and_reaps_external_process() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    ok(home, &["init", home.to_str().unwrap()]);
    let exe = home.join("codex");
    fs::write(
        &exe,
        include_str!("../../agt-harness/tests/fixtures/codex.py"),
    )
    .unwrap();
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o700)).unwrap();
    let file = home.join("environment.json");
    fs::write(
        &file,
        json!({"engine":"codex","executable":exe,"working_directory":home,"sandbox":"read_only"})
            .to_string(),
    )
    .unwrap();
    ok(
        home,
        &[
            "environment",
            "create",
            "local",
            "--file",
            file.to_str().unwrap(),
        ],
    );
    ok(
        home,
        &["agent", "create", "coder", "--environment", "local"],
    );
    let mut process = Command::new(env!("CARGO_BIN_EXE_agt"))
        .args([
            "--home",
            home.to_str().unwrap(),
            "--output",
            "json",
            "agent",
            "run",
            "coder",
            "--input",
            "HANG",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !home.join("started").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    unsafe {
        libc::kill(process.id() as i32, libc::SIGTERM);
    }
    while process.try_wait().unwrap().is_none() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let pid = fs::read_to_string(home.join("started"))
        .unwrap()
        .parse::<i32>()
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(ok(home, &["run", "list"])[0]["state"], "canceled");
}
