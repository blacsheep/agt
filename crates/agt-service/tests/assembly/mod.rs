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

#[test]
fn every_resource_rejects_stale_edits_and_returns_safe_revisions() {
    let home = home();
    let s = configured(home.path());
    let resources = agt_service::Resources::new(home.path());
    let fixtures = [
        (
            "provider",
            "p",
            json!({"base_url":"http://localhost/updated"}),
        ),
        ("model", "m", json!({"temperature":0.4})),
        ("agent", "worker", json!({"description":"updated"})),
        ("prompt", "instructions", json!({"text":"updated prompt"})),
        ("skill", "helper", Value::Null),
        (
            "mcp",
            "one",
            json!({"mcpServers":{"one":{"command":"updated-command"}}}),
        ),
        (
            "harness",
            "external",
            json!({"description":"updated harness"}),
        ),
    ];
    resources
        .write(
            "prompt",
            "instructions",
            true,
            json!({"text":"initial prompt"}),
        )
        .unwrap();
    resources
        .write("skill", "helper", true, json!({"description":"helper"}))
        .unwrap();
    resources
        .write(
            "mcp",
            "one",
            true,
            json!({"mcpServers":{"one":{"command":"must-not-run"}}}),
        )
        .unwrap();
    resources.write("harness", "external", true, json!({"engine":"codex","executable":std::env::current_exe().unwrap(),"working_directory":home.path(),"sandbox":"read_only"})).unwrap();
    for (kind, reference, mut data) in fixtures {
        let before = resources.read(kind, reference).unwrap();
        let revision = before["revision"].as_str().unwrap();
        if kind == "skill" {
            data = json!({"text":format!("{}\nUpdated instructions.\n", before["content"].as_str().unwrap())});
        }
        let args = json!({"kind":kind,"reference":reference,"create":false,"expected_revision":revision,"data":data});
        let updated = s.call("resource_write", args.clone()).unwrap();
        assert_ne!(updated["revision"], before["revision"], "{kind}");
        assert_eq!(
            s.call("resource_write", args).unwrap_err().code,
            "conflict",
            "{kind}"
        );
        assert_eq!(
            resources.read(kind, reference).unwrap()["revision"],
            updated["revision"]
        );
        assert!(!updated.to_string().contains("api_key\":\"test"));
    }
}

#[test]
fn collection_updates_are_atomic_normalized_and_shared_by_concurrent_writers() {
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
    std::thread::scope(|scope| {
        for name in ["one", "two"] {
            let home = home.path();
            scope.spawn(move || {
                let options = agt_service::WriteOptions {
                    add: std::collections::BTreeMap::from([("mcps".into(), vec![name.into()])]),
                    ..Default::default()
                };
                agt_service::Resources::new(home)
                    .write_with("agent", "worker", false, json!({}), &options)
                    .unwrap();
            });
        }
    });
    let before = s
        .call(
            "resource_read",
            json!({"kind":"agent","reference":"worker"}),
        )
        .unwrap();
    assert_eq!(before["mcps"].as_array().unwrap().len(), 2);
    s.call("resource_write", json!({"kind":"agent","reference":"worker","create":false,"data":{},"add":{"mcps":["default/one"]}})).unwrap();
    assert_eq!(
        s.call(
            "resource_read",
            json!({"kind":"agent","reference":"worker"})
        )
        .unwrap()["revision"],
        before["revision"]
    );
    for args in [
        json!({"kind":"agent","reference":"worker","create":false,"data":{"mcps":[]},"add":{"mcps":["one"]}}),
        json!({"kind":"agent","reference":"worker","create":false,"data":{"description":"must not persist"},"remove":{"mcps":["missing"]}}),
        json!({"kind":"agent","reference":"worker","create":false,"data":{},"add":{"mcps":["one"]},"remove":{"mcps":["default/one"]}}),
        json!({"kind":"provider","reference":"p","create":false,"data":{},"add":{"mcps":["one"]}}),
    ] {
        assert!(s.call("resource_write", args).is_err());
    }
    assert_eq!(
        s.call(
            "resource_read",
            json!({"kind":"agent","reference":"worker"})
        )
        .unwrap(),
        before
    );
    s.call(
        "resource_write",
        json!({"kind":"agent","reference":"worker","create":false,"data":{},"clear":["mcps"]}),
    )
    .unwrap();
    assert_eq!(
        s.call(
            "resource_read",
            json!({"kind":"agent","reference":"worker"})
        )
        .unwrap()["mcps"],
        json!([])
    );
}

#[test]
fn oversized_skill_update_is_rejected_before_replacing_document() {
    let home = home();
    let resources = agt_service::Resources::new(home.path());
    resources
        .write("skill", "helper", true, json!({"description":"helper"}))
        .unwrap();
    let before = resources.read("skill", "helper").unwrap();
    let content = format!(
        "{}\n{}",
        before["content"].as_str().unwrap(),
        "x".repeat(agt_skill::SKILL_DOCUMENT_MAX_BYTES as usize)
    );
    assert!(
        resources
            .write("skill", "helper", false, json!({"text":content}))
            .is_err()
    );
    assert_eq!(resources.read("skill", "helper").unwrap(), before);
}
