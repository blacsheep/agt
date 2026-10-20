#![cfg(unix)]
use agt_harness::{Codex, Event, Execution, HarnessConfig, Sandbox, Tool};
use serde_json::json;

/// Writes an executable script from a child process. Were this process to hold
/// the file open for writing, a child forked meanwhile by a parallel test could
/// inherit the descriptor, and running the script would fail with ETXTBSY.
fn write_executable(path: &std::path::Path, script: &str) {
    use std::io::Write;
    let mut child = std::process::Command::new("sh")
        .args(["-c", "cat > \"$0\" && chmod 700 \"$0\""])
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
}
fn fixture() -> (tempfile::TempDir, HarnessConfig) {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("codex");
    write_executable(&exe, include_str!("../fixtures/codex.py"));
    let config = HarnessConfig {
        engine: "codex".into(),
        executable: exe,
        working_directory: dir.path().into(),
        model: None,
        description: String::new(),
        reasoning_effort: None,
        sandbox: Sandbox::ReadOnly,
        timeout_seconds: 10,
        args: Vec::new(),
        auth_method: None,
        inherit_mcp_servers: true,
    };
    (dir, config)
}
#[test]
fn without_inherited_mcp_servers_codex_restarts_with_each_one_disabled() {
    let launches = |inherit: bool| {
        let (dir, mut config) = fixture();
        config.inherit_mcp_servers = inherit;
        drop(Codex::connect(&config, &mut |_| Ok(())).unwrap());
        let argv = std::fs::read_to_string(dir.path().join("argv.jsonl")).unwrap();
        argv.lines()
            .map(|l| serde_json::from_str::<Vec<String>>(l).unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(launches(true).len(), 1, "the default reads no config");
    let launches = launches(false);
    assert_eq!(launches.len(), 2);
    assert!(!launches[0].contains(&"-c".to_owned()));
    let overrides = launches[1]
        .windows(2)
        .filter(|w| w[0] == "-c")
        .map(|w| w[1].as_str())
        .collect::<Vec<_>>();
    assert_eq!(overrides, ["mcp_servers.fetch.enabled=false"]);
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

mod cancellation;

mod tools;

mod approvals;
