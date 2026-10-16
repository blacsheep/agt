#![cfg(unix)]
use agt_harness::{
    ApprovalHandler, ApprovalRequest, DeliveryProgress, Event, Execution, HarnessConfig, Sandbox,
    Tool,
};
use serde_json::{Value, json};

/// See the Codex tests: write the script from a child to avoid ETXTBSY.
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
    let exe = dir.path().join("claude");
    write_executable(&exe, include_str!("../fixtures/claude.py"));
    let config = HarnessConfig {
        engine: "claude-code".into(),
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
fn tools() -> Vec<Tool> {
    vec![Tool {
        name: "framework_resource_list".into(),
        description: "list".into(),
        input_schema: json!({"type":"object","properties":{"kind":{"type":"string"}}}),
    }]
}
fn args(dir: &tempfile::TempDir) -> Vec<String> {
    serde_json::from_str(&std::fs::read_to_string(dir.path().join("args.json")).unwrap()).unwrap()
}

#[derive(Default)]
struct Human {
    requests: Vec<ApprovalRequest>,
    deliveries: Vec<DeliveryProgress>,
    choice: Option<Value>,
}
impl ApprovalHandler for Human {
    fn request(&mut self, r: &ApprovalRequest) -> anyhow::Result<String> {
        self.requests.push(r.clone());
        Ok("host-approval".into())
    }
    fn poll(&mut self, _: &str) -> anyhow::Result<Option<Value>> {
        Ok(self.choice.clone())
    }
    fn delivery(&mut self, _: &str, progress: DeliveryProgress) -> anyhow::Result<()> {
        self.deliveries.push(progress);
        Ok(())
    }
}

#[test]
fn a_turn_streams_text_serves_agt_tools_and_reports_the_session() {
    let (dir, config) = fixture();
    let mut preview = String::new();
    let mut thread = String::new();
    let mut settled = Vec::new();
    let mut event = |e: Event<'_>| {
        match e {
            Event::Text(t) => preview.push_str(t),
            Event::Thread(t) => thread = t.into(),
            Event::Settled(s) => settled.push(s),
            _ => {}
        }
        Ok(())
    };
    let mut engine = agt_harness::connect(&config, &mut event).unwrap();
    let tools = tools();
    let mut calls = 0;
    let output = engine
        .execute(
            &config,
            Execution {
                input: "TOOL",
                instructions: "You are the reviewer.",
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
            None,
        )
        .unwrap();
    assert_eq!(output, "done tool:[] session:session-new");
    assert_eq!(calls, 1);
    assert_eq!(preview, "working ");
    assert_eq!(thread, "session-new");
    assert_eq!(settled, ["completed"]);
    let args = args(&dir);
    let after = |flag: &str| args[args.iter().position(|a| a == flag).unwrap() + 1].clone();
    assert_eq!(after("--append-system-prompt"), "You are the reviewer.");
    assert_eq!(after("--allowed-tools"), "mcp__agt__*");
    assert_eq!(after("--permission-prompts"), "none");
    assert_eq!(after("--permission-mode"), "manual");
    assert!(after("--disallowed-tools").contains("Edit"));
    assert!(!args.contains(&"--resume".to_owned()));
    let listed: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("tools.json")).unwrap())
            .unwrap();
    assert_eq!(listed[0]["name"], "framework_resource_list");
}

#[test]
fn resumed_turns_pass_the_session_model_and_effort() {
    let (dir, mut config) = fixture();
    config.model = Some("opus".into());
    config.reasoning_effort = Some("high".into());
    config.sandbox = Sandbox::WorkspaceWrite;
    let mut engine = agt_harness::connect(&config, &mut |_| Ok(())).unwrap();
    let output = engine
        .execute(
            &config,
            Execution {
                input: "hello",
                instructions: "role",
                thread: Some("session-7"),
                tools: &[],
            },
            &mut |_| Ok(()),
            &mut |_, _| unreachable!(),
            None,
        )
        .unwrap();
    assert!(output.ends_with("session:session-7"));
    let args = args(&dir);
    let after = |flag: &str| args[args.iter().position(|a| a == flag).unwrap() + 1].clone();
    assert_eq!(after("--resume"), "session-7");
    assert_eq!(after("--model"), "opus");
    assert_eq!(after("--effort"), "high");
    assert_eq!(after("--permission-mode"), "acceptEdits");
}

#[test]
fn permission_prompts_go_through_agt_approvals() {
    for (choice, expected) in [
        ("accept", Ok("permission:allow")),
        ("decline", Ok("permission:deny")),
        ("cancel", Err(())),
    ] {
        let (dir, config) = fixture();
        let mut human = Human {
            choice: Some(json!(choice)),
            ..Default::default()
        };
        let mut engine = agt_harness::connect(&config, &mut |_| Ok(())).unwrap();
        let result = engine.execute(
            &config,
            Execution {
                input: "PERMISSION",
                instructions: "role",
                thread: None,
                tools: &[],
            },
            &mut |_| Ok(()),
            &mut |_, _| unreachable!(),
            Some(&mut human),
        );
        match expected {
            Ok(note) => assert!(result.unwrap().contains(note), "{choice}"),
            Err(()) => {
                let error = result.unwrap_err();
                let failure = error.downcast_ref::<agt_harness::Failure>().unwrap();
                assert!(matches!(
                    failure.code,
                    agt_harness::FailureCode::Interrupted
                ));
            }
        }
        let request = &human.requests[0];
        assert_eq!(request.method, "claude/can_use_tool");
        assert_eq!(request.params["tool_name"], "Bash");
        assert_eq!(request.params["input"]["command"], "ls");
        let offered: Vec<_> = request.options.iter().map(|o| o["value"].clone()).collect();
        assert_eq!(
            offered,
            [json!("accept"), json!("decline"), json!("cancel")]
        );
        assert_eq!(
            human.deliveries,
            [DeliveryProgress::Sending, DeliveryProgress::Sent]
        );
        let a = args(&dir);
        assert_eq!(
            a[a.iter().position(|x| x == "--permission-prompts").unwrap() + 1],
            "host"
        );
    }
}

#[test]
fn cancellation_interrupts_the_turn_and_failures_are_reported() {
    let (dir, config) = fixture();
    let hanging = dir.path().join("hanging");
    let mut settled = Vec::new();
    let mut interrupt_requested = false;
    let mut engine = agt_harness::connect(&config, &mut |_| Ok(())).unwrap();
    let result = engine.execute(
        &config,
        Execution {
            input: "HANG",
            instructions: "role",
            thread: None,
            tools: &[],
        },
        &mut |e| {
            match e {
                Event::Settled(s) => settled.push(s),
                Event::InterruptRequested => interrupt_requested = true,
                Event::Tick | Event::Protocol if hanging.exists() => anyhow::bail!("canceled"),
                _ => {}
            }
            Ok(())
        },
        &mut |_, _| unreachable!(),
        None,
    );
    assert!(result.is_err());
    assert!(interrupt_requested);
    assert_eq!(settled, ["interrupted"]);
    let protocol = std::fs::read_to_string(dir.path().join("protocol.jsonl")).unwrap();
    assert!(protocol.contains("\"subtype\": \"interrupt\""));

    let (_dir, config) = fixture();
    let mut settled = Vec::new();
    let mut engine = agt_harness::connect(&config, &mut |_| Ok(())).unwrap();
    let error = engine
        .execute(
            &config,
            Execution {
                input: "FAIL",
                instructions: "role",
                thread: None,
                tools: &[],
            },
            &mut |e| {
                if let Event::Settled(s) = e {
                    settled.push(s);
                }
                Ok(())
            },
            &mut |_, _| unreachable!(),
            None,
        )
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<agt_harness::Failure>().unwrap().code,
        agt_harness::FailureCode::EngineFailed
    ));
    assert_eq!(settled, ["failed"]);
}

#[test]
fn engine_operations_dispatch_by_engine() {
    let (_dir, mut config) = fixture();
    let models = agt_harness::models(&config).unwrap();
    assert!(
        models["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["model"] == "opus")
    );
    assert!(agt_harness::inspect(&config, "skills").is_err());
    assert!(agt_harness::compact(&config, "session-new", &mut |_| Ok(())).is_err());
    agt_harness::rename(&config, "session-new", "title").unwrap();
    assert_eq!(
        agt_harness::capabilities()["engines"]["claude-code"]["engine"],
        "claude-code"
    );
    config.engine = "unknown".into();
    assert!(config.validate().is_err());
}
