use super::*;

#[test]
fn unavailable_and_failed_tools_return_failure_without_replaying_the_turn() {
    for allowed in [false, true] {
        let (dir, config) = fixture();
        let tools = if allowed {
            vec![Tool {
                name: "framework_resource_list".into(),
                description: "list".into(),
                input_schema: json!({"type":"object"}),
            }]
        } else {
            vec![]
        };
        let mut statuses = Vec::new();
        let mut event = |event: Event<'_>| {
            if let Event::Activity {
                kind: "framework",
                status,
                ..
            } = event
            {
                statuses.push(status.to_owned());
            }
            Ok(())
        };
        let mut client = Codex::connect(&config, &mut event).unwrap();
        let mut calls = 0;
        let result = client
            .execute(
                &config,
                Execution {
                    input: "TOOL",
                    instructions: "",
                    thread: None,
                    tools: &tools,
                },
                &mut event,
                &mut |_, _| {
                    calls += 1;
                    anyhow::bail!("fixture tool failure")
                },
            )
            .unwrap();
        assert_eq!(result, "tool completed");
        assert_eq!(calls, usize::from(allowed));
        assert_eq!(statuses, ["running", "failed"]);
        let protocol = std::fs::read_to_string(dir.path().join("protocol.jsonl")).unwrap();
        let frames: Vec<serde_json::Value> = protocol
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let reply = frames.iter().find(|v| v["id"] == "tool").unwrap();
        assert_eq!(reply["result"]["success"], false);
        assert_eq!(
            reply["result"]["contentItems"][0]["text"],
            if allowed {
                "fixture tool failure"
            } else {
                "tool unavailable"
            }
        );
        assert_eq!(
            frames
                .iter()
                .filter(|v| v["method"] == "turn/start")
                .count(),
            1
        );
    }
}

#[test]
fn command_details_are_reported_without_replacing_activity_events() {
    let (_dir, config) = fixture();
    let mut details = Vec::new();
    let mut event = |e: Event<'_>| {
        if let Event::ToolDetail { id, item } = e {
            details.push((id.to_owned(), item.clone()));
        }
        Ok(())
    };
    let mut client = Codex::connect(&config, &mut event).unwrap();
    client
        .execute(
            &config,
            Execution {
                input: "PROGRESS",
                instructions: "",
                thread: None,
                tools: &[],
            },
            &mut event,
            &mut |_, _| unreachable!(),
        )
        .unwrap();
    assert_eq!(details.len(), 2);
    assert_eq!(details[0].0, "command-1");
    assert_eq!(details[0].1["command"], "secret-command");
    assert_eq!(details[1].1["aggregatedOutput"], "private-output");
}

#[test]
fn schema_failure_returns_tool_error_without_invoking_host_callback() {
    let (dir, config) = fixture();
    let mut event = |_: Event<'_>| Ok(());
    let mut client = Codex::connect(&config, &mut event).unwrap();
    let tools = [Tool {
        name: "framework_resource_list".into(),
        description: "list".into(),
        input_schema: json!({"type":"object","properties":{"kind":{"type":"integer"}},"required":["kind"]}),
    }];
    let result = client
        .execute(
            &config,
            Execution {
                input: "TOOL",
                instructions: "",
                thread: None,
                tools: &tools,
            },
            &mut event,
            &mut |_, _| panic!("invalid arguments reached callback"),
        )
        .unwrap();
    assert_eq!(result, "tool completed");
    let log = std::fs::read_to_string(dir.path().join("protocol.jsonl")).unwrap();
    let reply = log
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|v| v["id"] == "tool")
        .unwrap();
    assert_eq!(reply["result"]["success"], false);
    assert!(
        reply["result"]["contentItems"][0]["text"]
            .as_str()
            .unwrap()
            .contains("invalid_arguments")
    );
}
