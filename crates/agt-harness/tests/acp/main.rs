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
    let exe = dir.path().join("dsh");
    write_executable(&exe, include_str!("../fixtures/acp.py"));
    let config = HarnessConfig {
        engine: "deepseek-harness".into(),
        executable: exe,
        working_directory: dir.path().into(),
        model: None,
        description: String::new(),
        reasoning_effort: None,
        sandbox: Sandbox::WorkspaceWrite,
        timeout_seconds: 10,
        args: Vec::new(),
        auth_method: None,
        inherit_mcp_servers: true,
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
fn protocol(dir: &tempfile::TempDir) -> Vec<Value> {
    std::fs::read_to_string(dir.path().join("protocol.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
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
fn a_turn_mounts_agt_tools_over_http_and_streams_the_reply() {
    let (dir, config) = fixture();
    let (mut preview, mut thread, mut model, mut settled) =
        (String::new(), String::new(), String::new(), Vec::new());
    let mut event = |e: Event<'_>| {
        match e {
            Event::Text(t) => preview.push_str(t),
            Event::Thread(t) => thread = t.into(),
            Event::Model(m) => model = m.into(),
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
            None,
        )
        .unwrap();
    assert_eq!(
        output,
        "working done tool:[] model:deepseek-v4-flash session:acp-session-1"
    );
    assert_eq!(calls, 1);
    assert_eq!(thread, "acp-session-1");
    assert_eq!(model, "deepseek-v4-flash");
    assert_eq!(settled, ["completed"]);
    assert!(preview.starts_with("working done"));
    let args: Vec<String> =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("args.json")).unwrap())
            .unwrap();
    assert_eq!(args, ["--profile", "acp"]);
    let frames = protocol(&dir);
    let new = frames
        .iter()
        .find(|f| f["method"] == "session/new")
        .unwrap();
    assert_eq!(new["params"]["mcpServers"][0]["type"], "http");
    assert!(
        new["params"]["mcpServers"][0]["url"]
            .as_str()
            .unwrap()
            .starts_with("http://127.0.0.1:")
    );
    assert!(frames.iter().any(|f| f["method"] == "session/close"));
    let listed: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("tools.json")).unwrap())
            .unwrap();
    assert_eq!(listed[0]["name"], "framework_resource_list");
}

#[test]
fn resumed_turns_choose_the_harness_model_and_effort() {
    let (dir, mut config) = fixture();
    config.model = Some("deepseek-v4-pro".into());
    config.reasoning_effort = Some("max".into());
    let mut engine = agt_harness::connect(&config, &mut |_| Ok(())).unwrap();
    let output = engine
        .execute(
            &config,
            Execution {
                input: "hello",
                instructions: "role",
                thread: Some("acp-session-9"),
                tools: &[],
            },
            &mut |_| Ok(()),
            &mut |_, _| unreachable!(),
            None,
        )
        .unwrap();
    assert!(
        output.ends_with("model:deepseek-v4-pro session:acp-session-9"),
        "{output}"
    );
    let frames = protocol(&dir);
    assert!(frames.iter().any(|f| f["method"] == "session/resume"
        && f["params"]["sessionId"] == "acp-session-9"));
    let set: Vec<_> = frames
        .iter()
        .filter(|f| f["method"] == "session/set_config_option")
        .map(|f| {
            (
                f["params"]["configId"].clone(),
                f["params"]["value"].clone(),
            )
        })
        .collect();
    assert_eq!(
        set,
        [
            (
                json!("model"),
                json!("[\"deepseek-official\", \"deepseek-v4-pro\"]")
            ),
            (json!("reasoning_effort"), json!("max"))
        ]
    );

    let (_dir, mut config) = fixture();
    config.model = Some("unknown-model".into());
    let mut engine = agt_harness::connect(&config, &mut |_| Ok(())).unwrap();
    let error = engine
        .execute(
            &config,
            Execution {
                input: "hello",
                instructions: "role",
                thread: None,
                tools: &[],
            },
            &mut |_| Ok(()),
            &mut |_, _| unreachable!(),
            None,
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("deepseek-v4-pro"), "{error}");
}

#[test]
fn permission_requests_go_through_agt_approvals() {
    for (choice, expected) in [
        ("accept", Ok("permission:allow")),
        ("decline", Ok("permission:reject")),
        ("cancel", Err(())),
    ] {
        let (_dir, config) = fixture();
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
            Err(()) => assert!(matches!(
                result
                    .unwrap_err()
                    .downcast_ref::<agt_harness::Failure>()
                    .unwrap()
                    .code,
                agt_harness::FailureCode::Interrupted
            )),
        }
        let request = &human.requests[0];
        assert_eq!(request.method, "acp/session/request_permission");
        assert_eq!(request.params["tool_call"]["title"], "bash");
        let offered: Vec<_> = request.options.iter().map(|o| o["value"].clone()).collect();
        assert_eq!(
            offered,
            [json!("accept"), json!("decline"), json!("cancel")]
        );
        assert_eq!(
            human.deliveries,
            [DeliveryProgress::Sending, DeliveryProgress::Sent]
        );
    }
}

#[test]
fn cancellation_and_refusals_end_the_turn() {
    let (dir, config) = fixture();
    let hanging = dir.path().join("hanging");
    let mut settled = Vec::new();
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
                Event::Tick | Event::Protocol if hanging.exists() => anyhow::bail!("canceled"),
                _ => {}
            }
            Ok(())
        },
        &mut |_, _| unreachable!(),
        None,
    );
    assert!(result.is_err());
    assert_eq!(settled, ["interrupted"]);
    assert!(
        protocol(&dir)
            .iter()
            .any(|f| f["method"] == "session/cancel")
    );

    let (_dir, config) = fixture();
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
            &mut |_| Ok(()),
            &mut |_, _| unreachable!(),
            None,
        )
        .unwrap_err();
    assert!(error.to_string().contains("refusal"));
}

#[test]
fn the_connection_check_runs_no_model() {
    let (_dir, config) = fixture();
    let checked = agt_harness::test(&config).unwrap();
    assert_eq!(checked["status"], "connected");
    assert_eq!(checked["agent"]["name"], "deepseek-harness-acp");
    assert!(agt_harness::models(&config).is_err());
}

#[test]
fn a_custom_acp_agent_gets_its_arguments_and_auth_method() {
    let (dir, mut config) = fixture();
    config.engine = "acp".into();
    config.args = vec!["--require-auth".into()];
    let run = |config: &HarnessConfig| {
        let mut engine = agt_harness::connect(config, &mut |_| Ok(())).unwrap();
        engine.execute(
            config,
            Execution {
                input: "hello",
                instructions: "role",
                thread: None,
                tools: &[],
            },
            &mut |_| Ok(()),
            &mut |_, _| unreachable!(),
            None,
        )
    };
    let error = run(&config).unwrap_err().to_string();
    assert!(
        error.contains("needs authentication") && error.contains("api-key"),
        "{error}"
    );
    config.auth_method = Some("api-key".into());
    assert!(run(&config).unwrap().contains("session:acp-session-1"));
    let args: Vec<String> =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("args.json")).unwrap())
            .unwrap();
    assert_eq!(args, ["--require-auth"]);
    assert!(
        protocol(&dir)
            .iter()
            .any(|f| f["method"] == "authenticate" && f["params"]["methodId"] == "api-key")
    );
    assert_eq!(
        agt_harness::test(&config).unwrap()["auth_methods"],
        json!(["api-key"])
    );
    config.auth_method = Some("browser".into());
    assert!(agt_harness::test(&config).is_err());
    // Launch settings never apply to other engines: a Codex harness only ever runs `codex app-server`.
    for engine in ["codex", "claude-code", "deepseek-harness"] {
        config.engine = engine.into();
        assert!(config.validate().is_err(), "{engine}");
    }
}
