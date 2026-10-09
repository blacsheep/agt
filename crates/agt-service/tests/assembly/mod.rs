use super::*;

fn configured(home: &std::path::Path) -> ToolService {
    let s = service(home, "a", false).with_resource_management(true);
    s.call("resource_write", json!({"kind":"provider","reference":"p","create":true,"data":{"base_url":"http://localhost/v1","api_key":"test"}})).unwrap();
    s.call("resource_write", json!({"kind":"model","reference":"m","create":true,"data":{"provider":"p","model":"fixture"}})).unwrap();
    s.call(
        "resource_write",
        json!({"kind":"agent","reference":"worker","create":true,"data":{"model":"m"}}),
    )
    .unwrap();
    s
}

#[test]
fn resource_updates_replace_mounts_and_removed_tools_are_unavailable() {
    let home = home();
    let s = configured(home.path());
    for name in ["one", "two"] {
        s.call(
            "resource_write",
            json!({"kind":"mcp","reference":name,"create":true,
            "data":{"mcpServers":{name:{"command":"must-not-run"}}}}),
        )
        .unwrap();
    }
    let update = |data| json!({"kind":"agent","reference":"worker","create":false,"data":data});
    s.call("resource_write", update(json!({"mcps":["one","two"]})))
        .unwrap();
    let before = s.call("agent_mcp_list", json!({"agent":"worker"})).unwrap();
    assert_eq!(before["mcps"].as_array().unwrap().len(), 2);
    assert_eq!(before["effective"], "next_run");
    s.call("resource_write", update(json!({"description":"updated"})))
        .unwrap();
    assert_eq!(
        s.call("agent_mcp_list", json!({"agent":"worker"})).unwrap(),
        before
    );
    assert!(
        s.call("resource_write", update(json!({"mcps":["missing"]})))
            .is_err()
    );
    assert_eq!(
        s.call("agent_mcp_list", json!({"agent":"worker"})).unwrap(),
        before
    );
    for mcps in [json!(["two"]), json!([])] {
        s.call("resource_write", update(json!({"mcps":mcps})))
            .unwrap();
        assert_eq!(
            s.call("agent_mcp_list", json!({"agent":"worker"})).unwrap()["mcps"]
                .as_array()
                .unwrap()
                .len(),
            mcps.as_array().unwrap().len()
        );
    }
    for name in ["agent_mcp_attach", "agent_mcp_detach"] {
        assert!(s.schemas().iter().all(|schema| schema["name"] != name));
        assert_eq!(
            s.call(name, json!({"agent":"worker","mcp":"one"}))
                .unwrap_err()
                .code,
            "tool_unavailable"
        );
    }
    let plain = service(home.path(), "b", false);
    for (name, args) in [
        ("resource_write", update(json!({"mcps":["one"]}))),
        ("agent_mcp_list", json!({"agent":"worker"})),
        (
            "mcp_probe",
            json!({"reference":"two","working_directory":home.path()}),
        ),
    ] {
        assert_eq!(plain.call(name, args).unwrap_err().code, "tool_unavailable");
    }
}

#[test]
fn concurrent_mounts_do_not_overwrite_each_other() {
    let home = home();
    let s = configured(home.path());
    let names = (0..8).map(|i| format!("mcp-{i}")).collect::<Vec<_>>();
    for name in &names {
        s.call(
            "resource_write",
            json!({"kind":"mcp","reference":name,"create":true,
            "data":{"mcpServers":{name:{"command":"must-not-run"}}}}),
        )
        .unwrap();
    }
    let barrier = std::sync::Barrier::new(names.len());
    std::thread::scope(|scope| {
        for name in &names {
            let barrier = &barrier;
            let home = home.path();
            scope.spawn(move || {
                barrier.wait();
                agt_agent::AgentStore::new(home)
                    .add_mcp("worker", name)
                    .unwrap();
            });
        }
    });
    assert_eq!(
        s.call("agent_mcp_list", json!({"agent":"worker"})).unwrap()["mcps"]
            .as_array()
            .unwrap()
            .len(),
        names.len()
    );
}

#[test]
fn framework_schemas_compile_and_nested_validation_rejects_wrong_array_items() {
    let home = home();
    let s = configured(home.path());
    for schema in s.schemas() {
        agt_core::tool_schema::ToolSchema::compile(&schema["inputSchema"])
            .unwrap_or_else(|e| panic!("{}: {e}", schema["name"]));
    }
    let error = s
        .call("group_create", json!({"title":"x","members":[7]}))
        .unwrap_err();
    assert_eq!(error.code, "invalid_arguments");
    assert_eq!(error.field.as_deref(), Some("members"));
}

#[test]
fn probe_requires_host_authorization_and_harness_mounts_stay_rejected() {
    let home = home();
    let s = configured(home.path());
    assert!(s.schemas().iter().all(|t| t["name"] != "mcp_probe"));
    let s = s.with_mcp_probe(true);
    assert!(s.schemas().iter().any(|t| t["name"] == "mcp_probe"));
    s.call(
        "resource_write",
        json!({"kind":"mcp","reference":"one","create":true,
        "data":{"mcpServers":{"one":{"command":"must-not-run"}}}}),
    )
    .unwrap();
    s.call("resource_write", json!({"kind":"harness","reference":"h","create":true,
        "data":{"engine":"codex","executable":std::env::current_exe().unwrap(),"working_directory":home.path(),"sandbox":"read_only"}})).unwrap();
    s.call(
        "resource_write",
        json!({"kind":"agent","reference":"external","create":true,"data":{"harness":"h"}}),
    )
    .unwrap();
    assert!(
        s.call(
            "resource_write",
            json!({"kind":"agent","reference":"external","create":false,"data":{"mcps":["one"]}})
        )
        .is_err()
    );
    assert_eq!(
        s.call("agent_mcp_list", json!({"agent":"external"}))
            .unwrap()["mcps"],
        json!([])
    );
}
