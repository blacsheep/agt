#![cfg(unix)]

use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn long_home_events_keep_polling_and_stop_cleanly() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("long-home-".repeat(14));
    agt_config::initialize_home(Some(home.clone())).unwrap();
    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_agt"))
            .arg("--home")
            .arg(&home)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    run(&["im", "principal", "create", "alice"]);
    run(&["im", "principal", "create", "bob"]);
    let chat: serde_json::Value =
        serde_json::from_slice(&run(&["im", "direct", "create", "alice", "bob"]).stdout).unwrap();
    let chat = chat["id"].as_str().unwrap();
    let mut process = Process(
        Command::new(env!("CARGO_BIN_EXE_agt"))
            .arg("--home")
            .arg(&home)
            .args(["events", "watch", "--principal", "alice"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = process.0.stdout.take().unwrap();
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if send.send(line).is_err() {
                break;
            }
        }
    });
    let page = || -> serde_json::Value {
        let line = receive
            .recv_timeout(Duration::from_secs(25))
            .unwrap()
            .unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let initial = page();
    run(&[
        "im",
        "message",
        "send",
        chat,
        "--sender",
        "bob",
        "after subscription",
    ]);
    let changed = page();
    assert_ne!(changed["cursor"], initial["cursor"]);
    assert!(
        changed["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|change| change["conversation"] == chat)
    );
    unsafe {
        libc::kill(process.0.id() as i32, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = process.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "events watch ignored cancellation"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    reader.join().unwrap();
    let mut diagnostic = String::new();
    std::io::Read::read_to_string(&mut process.0.stderr.take().unwrap(), &mut diagnostic).unwrap();
    assert_eq!(diagnostic.matches("wake hints unavailable").count(), 1);
}
