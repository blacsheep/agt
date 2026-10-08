#![cfg(unix)]
use agt_harness::{Codex, Event, Execution, HarnessConfig, Sandbox, Tool};
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
fn fixture() -> (tempfile::TempDir, HarnessConfig) {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("codex");
    std::fs::write(&exe, include_str!("fixtures/codex.py")).unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = HarnessConfig {
        engine: "codex".into(),
        executable: exe,
        working_directory: dir.path().into(),
        model: None,
        description: String::new(),
        reasoning_effort: None,
        sandbox: Sandbox::ReadOnly,
        timeout_seconds: 10,
    };
    (dir, config)
}
#[test]
fn streams_final_and_bridges_tools_without_model_retries() {
    let (_dir, config) = fixture();
    let mut preview = String::new();
    let mut thread = String::new();
    let mut event = |e: Event<'_>| {
        match e {
            Event::Text(t) => preview.push_str(t),
            Event::Thread(t) => thread = t.into(),
            _ => {}
        };
        Ok(())
    };
    let mut client = Codex::connect(&config, &mut event).unwrap();
    let tools = [Tool {
        name: "framework_resource_list".into(),
        description: "list".into(),
        input_schema: json!({"type":"object"}),
    }];
    let mut calls = 0;
    let output = client
        .execute(
            &config,
            Execution {
                input: "TOOL",
                instructions: "role",
                thread: None,
                tools: &tools,
            },
            &mut event,
            &mut |name, args| {
                assert_eq!(name, "framework_resource_list");
                assert_eq!(args["kind"], "agent");
                calls += 1;
                Ok(json!([]))
            },
        )
        .unwrap();
    assert_eq!(output, "tool completed");
    assert_eq!(calls, 1);
    assert_eq!(preview, "working preview");
    assert!(!thread.is_empty());
    drop(client);
    let mut event = |_: Event<'_>| Ok(());
    let mut client = Codex::connect(&config, &mut event).unwrap();
    assert_eq!(
        client
            .execute(
                &config,
                Execution {
                    input: "continue",
                    instructions: "role",
                    thread: Some(&thread),
                    tools: &tools
                },
                &mut event,
                &mut |_, _| unreachable!()
            )
            .unwrap(),
        "fixture final answer"
    );
}
#[test]
fn failures_empty_and_approvals_are_not_replayed() {
    for input in ["FAIL", "EMPTY", "APPROVE"] {
        let (dir, config) = fixture();
        let mut event = |_: Event<'_>| Ok(());
        let mut client = Codex::connect(&config, &mut event).unwrap();
        assert!(
            client
                .execute(
                    &config,
                    Execution {
                        input,
                        instructions: "",
                        thread: None,
                        tools: &[]
                    },
                    &mut event,
                    &mut |_, _| unreachable!()
                )
                .is_err()
        );
        drop(client);
        let log = std::fs::read_to_string(dir.path().join("protocol.jsonl")).unwrap();
        assert_eq!(log.lines().filter(|l| l.contains("turn/start")).count(), 1);
    }
}
#[test]
fn cancellation_during_silent_execution_reaps_process() {
    let (dir, config) = fixture();
    let start = std::time::Instant::now();
    let mut event = |_: Event<'_>| {
        if start.elapsed().as_millis() > 250 {
            anyhow::bail!("canceled");
        }
        Ok(())
    };
    let mut client = Codex::connect(&config, &mut event).unwrap();
    assert!(
        client
            .execute(
                &config,
                Execution {
                    input: "HANG",
                    instructions: "",
                    thread: None,
                    tools: &[]
                },
                &mut event,
                &mut |_, _| unreachable!()
            )
            .is_err()
    );
    drop(client);
    assert!(start.elapsed().as_secs() < 3);
    let pid = std::fs::read_to_string(dir.path().join("started"))
        .unwrap()
        .parse::<i32>()
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
}

#[test]
fn control_operations_query_engine_without_sending_chat_messages() {
    let (dir, config) = fixture();
    let models = Codex::models(&config).unwrap();
    assert_eq!(
        models["data"][0]["supportedReasoningEfforts"][1]["reasoningEffort"],
        "xhigh"
    );
    assert_eq!(
        Codex::inspect(&config, "skills").unwrap()["data"][0]["skills"][0]["name"],
        "fixture-skill"
    );
    assert_eq!(
        Codex::inspect(&config, "mcp").unwrap()["data"][0]["name"],
        "fixture-mcp"
    );
    Codex::rename(&config, "engine-id", "Project review").unwrap();
    Codex::compact(&config, "engine-id", &mut |_| Ok(())).unwrap();
    let log = std::fs::read_to_string(dir.path().join("protocol.jsonl")).unwrap();
    assert!(!log.contains("turn/start"));
    assert!(log.contains("thread/compact/start"));
}

#[test]
fn activity_is_structured_and_disconnect_has_no_confirmed_terminal_event() {
    for input in ["PROGRESS", "DISCONNECT"] {
        let (_dir, config) = fixture();
        let mut submitted = false;
        let mut settled = false;
        let mut activity = Vec::new();
        let mut event = |e: Event<'_>| {
            match e {
                Event::Submitted => submitted = true,
                Event::Settled(_) => settled = true,
                Event::Activity { kind, status, .. } => {
                    activity.push((kind.to_string(), status.to_string()))
                }
                _ => {}
            };
            Ok(())
        };
        let mut client = Codex::connect(&config, &mut event).unwrap();
        let result = client.execute(
            &config,
            Execution {
                input,
                instructions: "",
                thread: None,
                tools: &[],
            },
            &mut event,
            &mut |_, _| unreachable!(),
        );
        assert!(submitted);
        assert_eq!(settled, input == "PROGRESS");
        assert_eq!(result.is_ok(), input == "PROGRESS");
        if input == "PROGRESS" {
            assert_eq!(
                activity,
                vec![
                    ("command".into(), "running".into()),
                    ("command".into(), "completed".into())
                ]
            );
        }
    }
}

#[path = "codex/cancellation.rs"]
mod cancellation;

#[path = "codex/tools.rs"]
mod tools;

#[path = "codex/approvals.rs"]
mod approvals;
