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
fn registration_probe_validation_repair_and_batch_boundary_unmount() {
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
            calls(vec![call(
                "bad",
                &mcp_alias("default/counter", "count"),
                json!({"count":"2"}),
            )]),
        ),
        (
            "default/a",
            calls(vec![
                call(
                    "unmount",
                    "framework_resource_write",
                    json!({"kind":"agent","reference":"a","create":false,"data":{"mcps":[]}}),
                ),
                call(
                    "good",
                    &mcp_alias("default/counter", "count"),
                    json!({"count":2}),
                ),
            ]),
        ),
        ("default/a", text("done")),
    ]);
    let run = Runtime::open(home.path(), &mut script, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Completed, "{:?}", run.error);
    assert_eq!(run.tool_failures, 1);
    assert!(
        script.seen[2]
            .2
            .iter()
            .all(|t| !t["function"]["name"].as_str().unwrap().starts_with("mcp_"))
    );
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

#[test]
fn self_mount_is_usable_in_next_model_turn_and_matches_fresh_fingerprint() {
    let home = home();
    register(home.path(), "valid");
    let alias = mcp_alias("default/counter", "count");
    let mut script = Script::new(vec![
        (
            "default/a",
            calls(vec![call(
                "mount",
                "framework_resource_write",
                json!({"kind":"agent","reference":"a","create":false,"data":{},"add":{"mcps":["counter"]}}),
            )]),
        ),
        (
            "default/a",
            calls(vec![call("use", &alias, json!({"count":2}))]),
        ),
        ("default/a", text("done")),
    ]);
    let run = Runtime::open(home.path(), &mut script, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Completed, "{:?}", run.error);
    assert!(
        !script.seen[0]
            .2
            .iter()
            .any(|t| t["function"]["name"] == alias)
    );
    assert!(
        script.seen[1]
            .2
            .iter()
            .any(|t| t["function"]["name"] == alias)
    );
    assert!(
        script.seen[1].1.last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("\"state\":\"applied\"")
    );
    assert_eq!(
        std::fs::read_to_string(home.path().join("calls"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    let mut fresh = Script::new(vec![("default/a", text("next"))]);
    let mut req = request();
    req.request_key = "fresh".into();
    let next = Runtime::open(home.path(), &mut fresh, &DefaultMcpClientFactory)
        .unwrap()
        .run(req, &CancellationToken::new())
        .unwrap();
    assert_eq!(next.state, RunState::Completed);
    assert_eq!(run.config_fingerprint, next.config_fingerprint);
}

#[test]
fn failed_refresh_retains_previous_session_and_reports_saved_but_unapplied() {
    let home = home();
    let manager = register(home.path(), "valid");
    manager
        .call(
            "resource_write",
            json!({"kind":"agent","reference":"a","create":false,"data":{"mcps":["counter"]}}),
        )
        .unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/contract_mcp.py");
    let mut script = Script::new(vec![
        (
            "default/a",
            calls(vec![call(
                "break",
                "framework_resource_write",
                json!({"kind":"mcp","reference":"counter","create":false,
                "data":{"mcpServers":{"counter":{"command":"python3","args":[fixture,"bad-schema",home.path().join("broken")]}}}}),
            )]),
        ),
        (
            "default/a",
            calls(vec![call(
                "old",
                &mcp_alias("default/counter", "count"),
                json!({"count":2}),
            )]),
        ),
        ("default/a", text("done")),
    ]);
    let run = Runtime::open(home.path(), &mut script, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Completed, "{:?}", run.error);
    assert!(
        script.seen[1].1.last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("\"state\":\"failed\"")
    );
    assert!(
        std::fs::read_to_string(home.path().join("calls"))
            .unwrap()
            .contains("2")
    );
    assert!(!home.path().join("broken").exists());
    assert!(
        RunStore::open(home.path())
            .unwrap()
            .calls(&run.id, 0, 100)
            .unwrap()
            .iter()
            .any(|c| c.tool_name.as_deref() == Some("capability_refresh")
                && c.state == agt_runtime::CallState::Failed)
    );
}

#[test]
fn self_skill_refresh_updates_instructions_but_keeps_framework_grants() {
    let home = home();
    let manager = register(home.path(), "valid");
    manager
        .call(
            "resource_write",
            json!({"kind":"agent","reference":"a","create":false,"data":{"mcps":["counter"]}}),
        )
        .unwrap();
    manager.call("resource_write", json!({"kind":"skill","reference":"helper","create":true,"data":{"description":"Unique live skill instructions"}})).unwrap();
    let mut script = Script::new(vec![
        (
            "default/a",
            calls(vec![call(
                "mount",
                "framework_resource_write",
                json!({"kind":"agent","reference":"a","create":false,"data":{"builtin_tools":[]},"add":{"skills":["helper"]}}),
            )]),
        ),
        ("default/a", text("done")),
    ]);
    let run = Runtime::open(home.path(), &mut script, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Completed, "{:?}", run.error);
    assert!(
        !script.seen[0].1[0]["content"]
            .as_str()
            .unwrap()
            .contains("Unique live skill")
    );
    assert!(
        script.seen[1].1[0]["content"]
            .as_str()
            .unwrap()
            .contains("Unique live skill")
    );
    assert!(
        script.seen[1]
            .2
            .iter()
            .any(|t| t["function"]["name"] == "framework_resource_write")
    );
    assert_eq!(
        std::fs::read_to_string(home.path().join("calls.starts"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[test]
fn self_mount_cannot_expand_host_mcp_authorization() {
    let home = home();
    let manager = register(home.path(), "valid");
    let conversation = manager
        .call("direct_create", json!({"principal":"a"}))
        .unwrap();
    let sent = manager
        .call(
            "message_send",
            json!({"conversation":conversation["id"],"body":"configure","request_key":"configure"}),
        )
        .unwrap();
    let mut req = request();
    req.trigger = Some(agt_runtime::MessageTrigger {
        message_id: agt_im::MessageId::new(sent["message"]["id"].as_str().unwrap()),
        conversation_id: agt_im::ConversationId::new(conversation["id"].as_str().unwrap()),
        principal: PrincipalId::new("a"),
        depth: 1,
        allow_mcp: false,
    });
    let mut script = Script::new(vec![
        (
            "default/a",
            calls(vec![call(
                "mount",
                "framework_resource_write",
                json!({"kind":"agent","reference":"a","create":false,"data":{"mcps":["counter"]}}),
            )]),
        ),
        ("default/a", text("done")),
    ]);
    let run = Runtime::open(home.path(), &mut script, &DefaultMcpClientFactory)
        .unwrap()
        .run(req, &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Completed, "{:?}", run.error);
    assert!(!home.path().join("calls.starts").exists());
    assert!(
        script.seen[1]
            .2
            .iter()
            .all(|t| !t["function"]["name"].as_str().unwrap().starts_with("mcp_"))
    );
    assert!(
        script.seen[1].1.last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("\"state\":\"failed\"")
    );
}

#[test]
fn provider_credential_rotation_changes_execution_fingerprint() {
    let home = home();
    let before = agt_runtime::ResolvedAgent::load(home.path(), "a")
        .unwrap()
        .fingerprint;
    agt_service::Resources::new(home.path())
        .write(
            "provider",
            "local",
            false,
            json!({"api_key":"rotated-credential"}),
        )
        .unwrap();
    let after = agt_runtime::ResolvedAgent::load(home.path(), "a")
        .unwrap()
        .fingerprint;
    assert_ne!(before, after);
    assert!(!after.contains("rotated-credential"));
}

#[test]
fn mount_reordering_keeps_tool_identity_bound_to_the_same_server() {
    let home = home();
    let manager = register(home.path(), "valid");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/contract_mcp.py");
    manager.call("resource_write", json!({"kind":"mcp","reference":"other","create":true,
        "data":{"mcpServers":{"other":{"command":"python3","args":[fixture,"valid",home.path().join("other-calls")]}}}})).unwrap();
    manager.call("resource_write", json!({"kind":"agent","reference":"a","create":false,"data":{"mcps":["counter","other"]}})).unwrap();
    let alias = mcp_alias("default/counter", "count");
    let mut script = Script::new(vec![
        (
            "default/a",
            calls(vec![call(
                "reorder",
                "framework_resource_write",
                json!({"kind":"agent","reference":"a","create":false,"data":{"mcps":["other","counter"]}}),
            )]),
        ),
        (
            "default/a",
            calls(vec![call("same-server", &alias, json!({"count":7}))]),
        ),
        ("default/a", text("done")),
    ]);
    let run = Runtime::open(home.path(), &mut script, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Completed, "{:?}", run.error);
    assert!(
        script.seen[0]
            .2
            .iter()
            .any(|tool| tool["function"]["name"] == alias)
    );
    assert!(
        script.seen[1]
            .2
            .iter()
            .any(|tool| tool["function"]["name"] == alias)
    );
    assert!(
        std::fs::read_to_string(home.path().join("calls"))
            .unwrap()
            .contains("7")
    );
    assert!(!home.path().join("other-calls").exists());
}
