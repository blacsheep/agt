use agt_im::{
    DeliveryState, ImStore, MessageListOptions, PrincipalId, PrincipalInput, PrincipalKind,
};
use agt_service::{Actor, Role, ToolService};
use serde_json::{Value, json};

fn home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    agt_config::initialize_home(Some(home.path().to_owned())).unwrap();
    agt_prompt::PromptStore::new(home.path())
        .ensure_default_prompt()
        .unwrap();
    let im = ImStore::open(home.path().join("im/im.db")).unwrap();
    for name in ["owner", "a", "b", "outsider"] {
        im.create_principal(PrincipalInput {
            id: PrincipalId::new(name),
            kind: PrincipalKind::Agent,
            display_name: name.into(),
            metadata: json!({}),
        })
        .unwrap();
    }
    home
}
fn service(home: &std::path::Path, principal: &str, manager: bool) -> ToolService {
    ToolService::open(
        home,
        Actor {
            principal: PrincipalId::new(principal),
            role: if manager {
                Role::Manager
            } else {
                Role::Participant
            },
            automated_depth: None,
        },
    )
    .unwrap()
}

#[test]
fn group_broadcast_is_atomic_deduplicated_and_separate_from_read_cursor() {
    let home = home();
    let owner = service(home.path(), "owner", true);
    let group = owner
        .call("group_create", json!({"title":"group","members":["a","b"]}))
        .unwrap();
    let args = json!({"conversation":group["id"],"body":"hello","request_key":"same"});
    let message = owner.call("message_send", args.clone()).unwrap();
    assert_eq!(owner.call("message_send", args).unwrap()["duplicate"], true);
    let im = ImStore::open(home.path().join("im/im.db")).unwrap();
    assert!(
        im.deliveries(&PrincipalId::new("owner"), true, 50)
            .unwrap()
            .is_empty()
    );
    for name in ["a", "b"] {
        let deliveries = im.deliveries(&PrincipalId::new(name), true, 50).unwrap();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].message_id.as_str(), message["message"]["id"]);
    }
    let cid = agt_im::ConversationId::new(group["id"].as_str().unwrap());
    im.mark_read(&cid, &PrincipalId::new("a"), 1).unwrap();
    assert_eq!(
        im.deliveries(&PrincipalId::new("a"), true, 50).unwrap()[0].state,
        DeliveryState::Pending
    );
    owner
        .call(
            "member_add",
            json!({"conversation":group["id"],"principal":"outsider"}),
        )
        .unwrap();
    assert!(
        im.deliveries(&PrincipalId::new("outsider"), true, 50)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        im.list_messages(
            &cid,
            &PrincipalId::new("outsider"),
            MessageListOptions::default()
        )
        .unwrap()
        .messages
        .len(),
        1
    );
    owner
        .call(
            "member_remove",
            json!({"conversation":group["id"],"principal":"a"}),
        )
        .unwrap();
    assert!(
        im.deliveries(&PrincipalId::new("a"), true, 50)
            .unwrap()
            .is_empty()
    );
    owner
        .call(
            "member_add",
            json!({"conversation":group["id"],"principal":"a"}),
        )
        .unwrap();
    assert!(
        im.deliveries(&PrincipalId::new("a"), true, 50)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn tool_identity_and_management_permissions_are_enforced() {
    let home = home();
    let owner = service(home.path(), "owner", true);
    let a = service(home.path(), "a", false);
    let outsider = service(home.path(), "outsider", false);
    let group = owner
        .call("group_create", json!({"title":"group","members":["a"]}))
        .unwrap();
    assert!(
        a.call(
            "message_send",
            json!({"conversation":group["id"],"body":"x","request_key":"x","sender_id":"owner"})
        )
        .is_err()
    );
    assert!(
        a.call(
            "message_send",
            json!({"conversation":group["id"],"body":"x","request_key":"x","automated_depth":0})
        )
        .is_err()
    );
    assert!(
        a.call(
            "principal_create",
            json!({"id":"fake","name":"fake","kind":"agent"})
        )
        .is_err()
    );
    assert!(a.call("resource_write", json!({})).is_err());
    assert!(
        a.call(
            "member_add",
            json!({"conversation":group["id"],"principal":"outsider"})
        )
        .is_err()
    );
    assert!(
        outsider
            .call("message_history", json!({"conversation":group["id"]}))
            .is_err()
    );
    assert!(
        outsider
            .call("member_list", json!({"conversation":group["id"]}))
            .is_err()
    );
    let sent = a
        .call(
            "message_send",
            json!({"conversation":group["id"],"body":"x","request_key":"x"}),
        )
        .unwrap();
    assert_eq!(sent["message"]["sender_id"], "a");
    let delivery = owner.call("message_inbox", json!({})).unwrap();
    assert!(
        outsider
            .call("message_ack", json!({"id":delivery[0]["id"]}))
            .is_err()
    );
    owner
        .call("message_ack", json!({"id":delivery[0]["id"]}))
        .unwrap();
}

#[test]
fn tools_create_update_mount_and_delete_resources_with_safe_summaries() {
    let home = home();
    let manager = service(home.path(), "owner", true);
    agt_provider::ProviderStore::new(home.path())
        .create(
            &agt_provider::ProviderConfig::openai_compatible(
                "p",
                "http://127.0.0.1:1/v1",
                "super-secret",
            )
            .unwrap(),
        )
        .unwrap();
    assert!(
        !manager
            .call("resource_read", json!({"kind":"provider","reference":"p"}))
            .unwrap()
            .to_string()
            .contains("super-secret")
    );
    manager.call("resource_write",json!({"kind":"model","reference":"m","create":true,"data":{"provider":"p","model":"test"}})).unwrap();
    manager.call("resource_write",json!({"kind":"prompt","reference":"review","create":true,"data":{"text":"Review carefully"}})).unwrap();
    manager.call("resource_write",json!({"kind":"skill","reference":"@team/review","create":true,"data":{"description":"review"}})).unwrap();
    let result=manager.call("resource_write",json!({"kind":"mcp","reference":"helper","create":true,"data":{"mcpServers":{"helper":{"command":"echo","env":{"TOKEN":"mcp-secret"}}}}})).unwrap();
    assert!(!result.to_string().contains("mcp-secret"));
    let agent=manager.call("resource_write",json!({"kind":"agent","reference":"reviewer","create":true,"data":{"model":"m","prompt":"review","skills":["@team/review"],"mcps":["helper"]}})).unwrap();
    assert_eq!(agent["skills"].as_array().unwrap().len(), 1);
    manager.call("resource_write",json!({"kind":"agent","reference":"reviewer","create":false,"data":{"skills":[],"mcps":[]}})).unwrap();
    assert!(
        manager
            .call("resource_delete", json!({"kind":"model","reference":"m"}))
            .is_err()
    );
    let plan = manager
        .call(
            "resource_delete",
            json!({"kind":"model","reference":"m","recursive":true}),
        )
        .unwrap();
    assert_eq!(
        plan["items"],
        json!(["agent/default/reviewer", "model/default/m"])
    );
    manager
        .call(
            "resource_delete",
            json!({"kind":"model","reference":"m","recursive":true,"confirm":plan["items"]}),
        )
        .unwrap();
    assert!(
        manager
            .call(
                "resource_read",
                json!({"kind":"agent","reference":"reviewer"})
            )
            .is_err()
    );
    assert!(
        manager
            .call("resource_read", json!({"kind":"provider","reference":"p"}))
            .is_ok()
    );
}

#[test]
fn schema_migrates_v1_without_replaying_history_and_refuses_future_versions() {
    let home = home();
    let path = home.path().join("im/im.db");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("DROP TABLE message_deliveries; PRAGMA user_version=1;")
        .unwrap();
    drop(db);
    let im = ImStore::open(&path).unwrap();
    assert!(
        im.deliveries(&PrincipalId::new("a"), true, 50)
            .unwrap()
            .is_empty()
    );
    drop(im);
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        2
    );
    db.pragma_update(None, "user_version", 99).unwrap();
    assert!(ImStore::open(&path).is_err());
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        99
    );
}

#[test]
fn preparation_and_terminal_state_survive_stale_workers() {
    let home = home();
    let owner = service(home.path(), "owner", true);
    let direct = owner
        .call("direct_create", json!({"principal":"a"}))
        .unwrap();
    owner
        .call(
            "message_send",
            json!({"conversation":direct["id"],"body":"x","request_key":"x"}),
        )
        .unwrap();
    let im = ImStore::open(home.path().join("im/im.db")).unwrap();
    let id = im.deliveries(&PrincipalId::new("a"), true, 1).unwrap()[0].id;
    im.prepare_delivery(id, &json!({"first":true})).unwrap();
    assert_eq!(
        im.prepare_delivery(id, &json!({"second":true}))
            .unwrap()
            .request,
        Some(json!({"first":true}))
    );
    im.update_delivery(id, DeliveryState::Pending, None, Some("stale"))
        .unwrap();
    assert_eq!(
        im.deliveries(&PrincipalId::new("a"), true, 1).unwrap()[0].state,
        DeliveryState::Processing
    );
    im.update_delivery(id, DeliveryState::Completed, Some("run"), None)
        .unwrap();
    im.update_delivery(id, DeliveryState::Processing, None, Some("late"))
        .unwrap();
    assert_eq!(
        im.deliveries(&PrincipalId::new("a"), false, 1).unwrap()[0].state,
        DeliveryState::Completed
    );
}

#[test]
fn mcp_protocol_rejects_preinit_and_invalid_calls_without_secret_echo() {
    let home = home();
    let service = service(home.path(), "owner", true);
    let messages = [
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2025-11-25","clientInfo":{"name":"test","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"resource_write","arguments":{"kind":"provider","reference":"x","create":true,"data":{"api_key":"super-secret"}}}}),
    ];
    let input = messages
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let mut output = Vec::new();
    agt_service::mcp_server::serve(&service, input.as_bytes(), &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(!text.contains("super-secret"));
    let values = text
        .lines()
        .map(|s| serde_json::from_str::<Value>(s).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 3);
    assert_eq!(values[0]["error"]["code"], -32002);
    assert_eq!(values[2]["result"]["isError"], true);
}

#[test]
fn deleting_hosted_agent_requires_explicit_unbinding() {
    let home = home();
    let manager = service(home.path(), "owner", true);
    agt_provider::ProviderStore::new(home.path())
        .create(
            &agt_provider::ProviderConfig::openai_compatible(
                "p",
                "http://127.0.0.1:1/v1",
                "secret",
            )
            .unwrap(),
        )
        .unwrap();
    manager.call("resource_write",json!({"kind":"model","reference":"m","create":true,"data":{"provider":"p","model":"test"}})).unwrap();
    manager
        .call(
            "resource_write",
            json!({"kind":"agent","reference":"a","create":true,"data":{"model":"m"}}),
        )
        .unwrap();
    manager.call("agent_bind",json!({"principal":"a","agent":"a","working_directory":home.path(),"max_steps":3,"max_depth":1})).unwrap();
    assert!(
        manager
            .call(
                "resource_delete",
                json!({"kind":"provider","reference":"p","recursive":true})
            )
            .is_err()
    );
    manager
        .call("agent_unbind", json!({"principal":"a"}))
        .unwrap();
    assert!(
        manager
            .call(
                "resource_delete",
                json!({"kind":"provider","reference":"p","recursive":true})
            )
            .is_ok()
    );
}
