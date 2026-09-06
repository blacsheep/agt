use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_home() -> std::path::PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "agt-im-cli-{}-{suffix}-{counter}",
        std::process::id()
    ))
}

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agt"))
        .args(["--home", home.to_str().expect("utf8")])
        .args(args)
        .output()
        .expect("run agt")
}

fn json(output: Output) -> Value {
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON output")
}

#[test]
fn creates_direct_conversation_and_replays_messages() {
    let home = temp_home();
    assert!(
        run(&home, &["init", home.to_str().expect("utf8")])
            .status
            .success()
    );
    assert!(
        run(&home, &["im", "principal", "create", "alice"])
            .status
            .success()
    );
    assert!(
        run(&home, &["im", "principal", "create", "bob"])
            .status
            .success()
    );

    let conversation = json(run(&home, &["im", "direct", "create", "alice", "bob"]));
    let conversation_id = conversation["id"].as_str().expect("conversation id");
    let send = json(run(
        &home,
        &[
            "im",
            "message",
            "send",
            conversation_id,
            "--sender",
            "alice",
            "hello from cli",
            "--idempotency-key",
            "request-1",
        ],
    ));
    assert_eq!(send["message"]["seq"], 1);
    assert_eq!(send["duplicate"], false);

    let retry = json(run(
        &home,
        &[
            "im",
            "message",
            "send",
            conversation_id,
            "--sender",
            "alice",
            "hello from cli",
            "--idempotency-key",
            "request-1",
        ],
    ));
    assert_eq!(retry["duplicate"], true);
    assert_eq!(retry["message"]["id"], send["message"]["id"]);

    let messages = json(run(
        &home,
        &["im", "message", "list", conversation_id, "--viewer", "bob"],
    ));
    assert_eq!(messages["messages"][0]["body"], "hello from cli");
    assert_eq!(messages["messages"][0]["sender_id"], "alice");

    let _ = fs::remove_dir_all(home);
}

#[test]
fn group_member_lifecycle_is_exposed_by_cli() {
    let home = temp_home();
    assert!(
        run(&home, &["init", home.to_str().expect("utf8")])
            .status
            .success()
    );
    for id in ["owner", "member", "guest"] {
        assert!(
            run(&home, &["im", "principal", "create", id])
                .status
                .success()
        );
    }
    let group = json(run(
        &home,
        &[
            "im", "group", "create", "--owner", "owner", "--title", "team", "--member", "member",
        ],
    ));
    let group_id = group["id"].as_str().expect("group id");
    let added = run(
        &home,
        &[
            "im", "group", "members", "add", group_id, "guest", "--actor", "owner",
        ],
    );
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let members = json(run(&home, &["im", "group", "members", "list", group_id]));
    assert_eq!(members.as_array().expect("members").len(), 3);

    let removed = run(
        &home,
        &[
            "im", "group", "members", "remove", group_id, "guest", "--actor", "owner",
        ],
    );
    assert!(
        removed.status.success(),
        "{}",
        String::from_utf8_lossy(&removed.stderr)
    );
    let _ = fs::remove_dir_all(home);
}
