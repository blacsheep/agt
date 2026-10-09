use super::*;
use agt_runtime::approval::ApprovalStore;

fn register(home: &Path, mode: &str) -> agt_service::ToolService {
    mount_resources(home, "a");
    let manager = hosted(home, agt_service::ResponsePolicy::Mentioned);
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/contract_mcp.py");
    manager.call("resource_write", json!({"kind":"mcp","reference":"counter","create":true,
        "data":{"mcpServers":{"counter":{"command":"python3","args":[fixture,mode,home.join("calls")]}}}})).unwrap();
    manager
}

#[test]
fn registration_probe_assembly_validation_repair_and_next_run_unmount() {
    let home = home();
    let manager = register(home.path(), "valid");
    // Static checks and assembly must not start a process.
    let check = manager
        .call("resource_test", json!({"kind":"mcp","reference":"counter"}))
        .unwrap();
    assert_eq!(check["connected"], false);
    manager
        .call(
            "resource_write",
            json!({"kind":"agent","reference":"a","create":false,"data":{"mcps":["counter"]}}),
        )
        .unwrap();
    manager
        .call(
            "resource_write",
            json!({"kind":"agent","reference":"a","create":false,"data":{"mcps":["counter"]}}),
        )
        .unwrap();
    assert!(!home.path().join("calls.starts").exists());
    let probe = manager
        .call(
            "mcp_probe",
            json!({"reference":"counter","working_directory":home.path()}),
        )
        .unwrap();
    assert_eq!(probe["schemas_valid"], true);
    assert_eq!(probe["probe"]["tool_names"], json!(["count"]));
    assert!(!home.path().join("calls").exists());
    let mut script = Script::new(vec![
        (
            "default/a",
            calls(vec![call("bad", "mcp_0_0", json!({"count":"2"}))]),
        ),
        (
            "default/a",
            calls(vec![call(
                "unmount",
                "framework_resource_write",
                json!({"kind":"agent","reference":"a","create":false,"data":{"mcps":[]}}),
            )]),
        ),
        (
            "default/a",
            calls(vec![call("good", "mcp_0_0", json!({"count":2}))]),
        ),
        ("default/a", text("done")),
    ]);
    let run = Runtime::open(home.path(), &mut script, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Completed, "{:?}", run.error);
    assert_eq!(run.tool_failures, 1);
    let failure = script.seen[1].1.last().unwrap();
    assert_eq!(failure["tool_call_id"], "bad");
    let error: Value = serde_json::from_str(failure["content"].as_str().unwrap()).unwrap();
    assert_eq!(error["error"]["field"], "/count");
    assert_eq!(
        std::fs::read_to_string(home.path().join("calls"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    let approvals = ApprovalStore::open(home.path())
        .unwrap()
        .list(None, None)
        .unwrap();
    assert_eq!(
        approvals.len(),
        2,
        "only the config update and the corrected call request approval"
    );
    manager
        .call(
            "resource_write",
            json!({"kind":"agent","reference":"a","create":false,"data":{"mcps":[]}}),
        )
        .unwrap();
    manager
        .call(
            "resource_write",
            json!({"kind":"agent","reference":"a","create":false,"data":{"mcps":[]}}),
        )
        .unwrap();
    let mut script = Script::new(vec![("default/a", text("next"))]);
    let mut req = request();
    req.request_key = "next".into();
    Runtime::open(home.path(), &mut script, &DefaultMcpClientFactory)
        .unwrap()
        .run(req, &CancellationToken::new())
        .unwrap();
    assert!(
        script.seen[0]
            .2
            .iter()
            .all(|t| !t["function"]["name"].as_str().unwrap().starts_with("mcp_"))
    );
}

#[test]
fn invalid_schema_fails_probe_and_run_before_model_request() {
    let home = home();
    let manager = register(home.path(), "bad-schema");
    let error = manager
        .call(
            "mcp_probe",
            json!({"reference":"counter","working_directory":home.path()}),
        )
        .unwrap_err();
    assert_eq!(error.code, "invalid_tool_schema");
    assert_eq!(error.field.as_deref(), Some("tools/count/inputSchema"));
    assert!(
        error
            .schema_path
            .as_ref()
            .is_some_and(|p| p.contains("count"))
    );
    manager
        .call(
            "resource_write",
            json!({"kind":"agent","reference":"a","create":false,"data":{"mcps":["counter"]}}),
        )
        .unwrap();
    let mut script = Script::default();
    let run = Runtime::open(home.path(), &mut script, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Failed);
    assert!(script.seen.is_empty());
    assert!(!home.path().join("calls").exists());
}

#[test]
fn invalid_framework_arguments_bypass_manual_approval() {
    let home = home();
    mount_resources(home.path(), "a");
    let mut config = agt_config::RuntimeConfig::load(home.path()).unwrap();
    config.permissions.mode = agt_config::runtime::PermissionMode::Manual;
    config.save(home.path()).unwrap();
    let mut script = Script::new(vec![
        (
            "default/a",
            calls(vec![call("array", "framework_resource_list", json!([]))]),
        ),
        (
            "default/a",
            calls(vec![call(
                "bad",
                "framework_resource_list",
                json!({"kind":7}),
            )]),
        ),
        ("default/a", text("corrected")),
    ]);
    let run = Runtime::open(home.path(), &mut script, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Completed);
    assert_eq!(run.tool_failures, 2);
    assert!(
        ApprovalStore::open(home.path())
            .unwrap()
            .list(None, None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn hosted_probe_requires_effective_mcp_authorization() {
    let home = home();
    let manager = register(home.path(), "valid");
    let direct = manager
        .call("direct_create", json!({"principal":"a"}))
        .unwrap();
    let sent = manager
        .call(
            "message_send",
            json!({"conversation":direct["id"],"body":"probe","request_key":"probe"}),
        )
        .unwrap();
    for allowed in [false, true] {
        let mut req = request();
        req.request_key = format!("probe-{allowed}");
        req.trigger = Some(agt_runtime::MessageTrigger {
            message_id: agt_im::MessageId::new(sent["message"]["id"].as_str().unwrap()),
            conversation_id: agt_im::ConversationId::new(direct["id"].as_str().unwrap()),
            principal: PrincipalId::new("a"),
            depth: 1,
            allow_mcp: allowed,
        });
        let mut script = Script::new(vec![("default/a", text("inspected"))]);
        let run = Runtime::open(home.path(), &mut script, &DefaultMcpClientFactory)
            .unwrap()
            .run(req, &CancellationToken::new())
            .unwrap();
        assert_eq!(run.state, RunState::Completed);
        assert_eq!(
            script.seen[0]
                .2
                .iter()
                .any(|t| t["function"]["name"] == "framework_mcp_probe"),
            allowed
        );
    }
    assert!(!home.path().join("calls.starts").exists());
}
