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
            kind: if name == "owner" {
                PrincipalKind::User
            } else {
                PrincipalKind::Agent
            },
            display_name: name.into(),
            metadata: json!({}),
        })
        .unwrap();
    }
    im.ensure_workspace_owner(&PrincipalId::new("owner"))
        .unwrap();
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
fn im_rejects_unsupported_schema_without_changing_it() {
    for version in [1, 99] {
        let home = home();
        let path = home.path().join("im/im.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.pragma_update(None, "user_version", version).unwrap();
        assert!(ImStore::open(&path).is_err());
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                .unwrap(),
            version
        );
    }
}

#[test]
fn terminal_delivery_state_survives_stale_updates() {
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
    im.update_delivery(id, DeliveryState::Processing, None, None)
        .unwrap();
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
    manager
        .call(
            "agent_bind",
            json!({"principal":"a","agent":"a","working_directory":home.path(),"max_steps":3}),
        )
        .unwrap();
    // Existing v1 bindings migrate as enabled; disabling survives reopen and keeps settings.
    let db = rusqlite::Connection::open(home.path().join("runtime/bindings.db")).unwrap();
    db.execute_batch("ALTER TABLE bindings DROP COLUMN enabled; PRAGMA user_version=1;")
        .unwrap();
    drop(db);
    let principal = PrincipalId::new("a");
    let store = agt_service::BindingStore::open(home.path()).unwrap();
    assert!(store.get(&principal).unwrap().is_some());
    assert_eq!(
        service(home.path(), "a", false)
            .call(
                "agent_set_enabled",
                json!({"principal":"a","enabled":false})
            )
            .unwrap_err()
            .code,
        "tool_unavailable"
    );
    manager
        .call(
            "agent_set_enabled",
            json!({"principal":"a","enabled":false}),
        )
        .unwrap();
    let reopened = agt_service::BindingStore::open(home.path()).unwrap();
    assert!(reopened.get(&principal).unwrap().is_none());
    assert!(reopened.list().unwrap().is_empty());
    let reconfigured = manager
        .call(
            "agent_bind",
            json!({"principal":"a","agent":"a","working_directory":home.path(),"max_steps":3}),
        )
        .unwrap();
    assert_eq!(reconfigured["enabled"], false);
    assert!(
        reconfigured["next_step"]
            .as_str()
            .unwrap()
            .contains("agent_set_enabled")
    );
    let saved = manager.call("binding_list", json!({})).unwrap();
    assert_eq!(saved[0]["enabled"], false);
    assert_eq!(saved[0]["max_steps"], 3);
    manager
        .call("agent_set_enabled", json!({"principal":"a","enabled":true}))
        .unwrap();
    assert_eq!(store.get(&principal).unwrap().unwrap().max_steps, 3);
    manager
        .call(
            "agent_set_enabled",
            json!({"principal":"a","enabled":false}),
        )
        .unwrap();
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

#[test]
fn optional_reply_and_cursor_values_are_consistent_and_idempotent() {
    let home = home();
    let owner = service(home.path(), "owner", true);
    let direct = owner
        .call("direct_create", json!({"principal":"a"}))
        .unwrap();
    let base = json!({"conversation":direct["id"],"body":"hello","request_key":"optional"});
    let first = owner.call("message_send", base.clone()).unwrap();
    for empty in [Value::Null, json!(""), json!(" \t ")] {
        let mut args = base.clone();
        args["reply_to"] = empty.clone();
        assert_eq!(owner.call("message_send", args).unwrap()["duplicate"], true);
        let page = owner
            .call(
                "message_history",
                json!({"conversation":direct["id"],"before":empty}),
            )
            .unwrap();
        assert_eq!(page["messages"].as_array().unwrap().len(), 1);
    }
    let reply = owner.call("message_send", json!({"conversation":direct["id"],"body":"reply","request_key":"reply","reply_to":first["message"]["id"]})).unwrap();
    assert_eq!(reply["message"]["reply_to_id"], first["message"]["id"]);
    for target in [json!("missing-secret-id"), json!(true)] {
        let error = owner.call("message_send", json!({"conversation":direct["id"],"body":"private-body","request_key":"bad","reply_to":target})).unwrap_err();
        assert_eq!(error.code, "invalid_arguments");
        assert_eq!(error.field.as_deref(), Some("reply_to"));
        assert!(!error.as_value().to_string().contains("secret"));
    }
    let other = owner
        .call("direct_create", json!({"principal":"b"}))
        .unwrap();
    let error = owner.call("message_send", json!({"conversation":other["id"],"body":"cross","request_key":"cross","reply_to":first["message"]["id"]})).unwrap_err();
    assert_eq!(error.field.as_deref(), Some("reply_to"));
    assert_eq!(
        owner
            .call("message_history", json!({"conversation":other["id"]}))
            .unwrap()["messages"],
        json!([])
    );
    let schemas = owner.schemas();
    for (name, field, types) in [
        ("message_send", "reply_to", json!(["string", "null"])),
        ("message_history", "before", json!(["string", "null"])),
        ("resource_delete", "confirm", json!(["array", "null"])),
    ] {
        assert_eq!(
            schemas.iter().find(|s| s["name"] == name).unwrap()["inputSchema"]["properties"][field]
                ["type"],
            types
        );
    }
}

#[test]
fn argument_errors_identify_safe_fields_without_echoing_untrusted_values() {
    let home = home();
    let owner = service(home.path(), "owner", true);
    for (name, args, field) in [
        ("message_ack", json!({"id":0}), Some("id")),
        ("message_ack", json!({"id":-1}), Some("id")),
        ("message_inbox", json!({"limit":201}), Some("limit")),
        ("message_inbox", json!({"limit":-1}), Some("limit")),
        ("message_inbox", json!({"limit":null}), Some("limit")),
        ("direct_create", json!({}), Some("principal")),
        (
            "direct_create",
            json!({"principal":true}),
            Some("principal"),
        ),
        (
            "principal_list",
            json!({"secret-unknown-key":"secret-value"}),
            None,
        ),
        ("principal_list", json!([]), None),
        (
            "group_create",
            json!({"title":"test","members":[1]}),
            Some("members"),
        ),
    ] {
        let error = owner.call(name, args).unwrap_err();
        assert_eq!(error.code, "invalid_arguments");
        assert_eq!(error.field.as_deref(), field);
        assert!(!error.as_value().to_string().contains("secret"));
    }
    let direct = owner
        .call("direct_create", json!({"principal":"a"}))
        .unwrap();
    let error = service(home.path(), "outsider", false)
        .call("message_history", json!({"conversation":direct["id"]}))
        .unwrap_err();
    assert_eq!(error.code, "permission_denied");
    assert_eq!(
        owner
            .call("direct_create", json!({"principal":"missing-secret"}))
            .unwrap_err()
            .code,
        "not_found"
    );
    assert_eq!(
        service(home.path(), "a", false)
            .call("resource_write", json!({}))
            .unwrap_err()
            .code,
        "tool_unavailable"
    );
    assert_eq!(
        owner
            .call(
                "resource_read",
                json!({"kind":"prompt","reference":"missing"})
            )
            .unwrap_err()
            .code,
        "not_found"
    );
    let args = json!({"kind":"prompt","reference":"p","create":true,"data":{"text":"private"}});
    owner.call("resource_write", args.clone()).unwrap();
    assert_eq!(
        owner.call("resource_write", args).unwrap_err().code,
        "already_exists"
    );
    for confirm in [None, Some(Value::Null)] {
        let mut args = json!({"kind":"prompt","reference":"p"});
        if let Some(confirm) = confirm {
            args["confirm"] = confirm;
        }
        assert_eq!(
            owner.call("resource_delete", args).unwrap()["requires_confirmation"],
            true
        );
    }
    assert_eq!(
        owner
            .call(
                "resource_delete",
                json!({"kind":"prompt","reference":"p","confirm":[]})
            )
            .unwrap_err()
            .code,
        "confirmation_mismatch"
    );
}

#[test]
fn mcp_returns_the_same_safe_structured_error_as_the_service() {
    let home = home();
    let service = service(home.path(), "owner", true);
    let args = json!({"id":-1});
    let expected = service
        .call("message_ack", args.clone())
        .unwrap_err()
        .as_value();
    let input = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","clientInfo":{"name":"test"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"message_ack","arguments":args}}),
    ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n")+"\n";
    let mut output = Vec::new();
    agt_service::mcp_server::serve(&service, input.as_bytes(), &mut output).unwrap();
    let output = String::from_utf8(output).unwrap();
    let response: Value = serde_json::from_str(output.lines().last().unwrap()).unwrap();
    assert_eq!(response["result"]["isError"], true);
    assert_eq!(
        serde_json::from_str::<Value>(response["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap(),
        expected
    );
}

#[test]
fn deep_messages_follow_policy_without_a_depth_limit() {
    let home = home();
    let owner = service(home.path(), "owner", true);
    let group = owner
        .call("group_create", json!({"title":"deep","members":["a"]}))
        .unwrap();
    let sender = agt_service::Messaging::open(
        home.path(),
        Actor {
            principal: PrincipalId::new("owner"),
            role: Role::Participant,
            automated_depth: Some(u32::MAX),
        },
    )
    .unwrap();
    let sent = sender
        .send(agt_service::MessageInput {
            conversation: agt_im::ConversationId::new(group["id"].as_str().unwrap()),
            body: "keep going".into(),
            request_key: "deep".into(),
            mentions: vec![],
            reply_to: None,
        })
        .unwrap();
    let receiver = agt_service::Messaging::open(
        home.path(),
        Actor {
            principal: PrincipalId::new("a"),
            role: Role::Participant,
            automated_depth: None,
        },
    )
    .unwrap();
    assert!(
        receiver
            .may_respond(&sent.message, agt_service::ResponsePolicy::All)
            .unwrap()
    );
    assert!(
        !receiver
            .may_respond(&sent.message, agt_service::ResponsePolicy::Manual)
            .unwrap()
    );
    assert!(
        !receiver
            .may_respond(&sent.message, agt_service::ResponsePolicy::Mentioned)
            .unwrap()
    );
}

#[test]
fn resource_capability_is_separate_from_identity_administration() {
    let home = home();
    let plain = service(home.path(), "a", false);
    let resources = service(home.path(), "a", false).with_resource_management(true);
    let manager = service(home.path(), "owner", true);
    assert!(
        resources
            .call(
                "principal_create",
                json!({"id":"fake-user","name":"Fake","kind":"user"})
            )
            .is_err()
    );
    assert!(
        ImStore::open(home.path().join("im/im.db"))
            .unwrap()
            .get_principal(&PrincipalId::new("fake-user"))
            .is_err()
    );
    resources
        .call(
            "principal_create",
            json!({"id":"new-agent","name":"New","kind":"agent"}),
        )
        .unwrap();
    assert_eq!(
        manager
            .call("principal_list", json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["id"] == "new-agent")
            .count(),
        1
    );
    for schema in resources.schemas() {
        let name = schema["name"].as_str().unwrap();
        if name.starts_with("resource_") || name == "provider_models" {
            assert_eq!(
                plain.call(name, json!({})).unwrap_err().code,
                "tool_unavailable"
            );
            assert!(manager.schemas().iter().any(|s| s == &schema));
        }
    }
    for name in [
        "principal_create",
        "agent_bind",
        "agent_unbind",
        "agent_set_enabled",
        "binding_list",
    ] {
        assert_eq!(
            plain.call(name, json!({})).unwrap_err().code,
            "tool_unavailable"
        );
    }
    for kind in ["provider", "model", "prompt", "skill", "mcp", "agent"] {
        resources
            .call(
                "resource_category_create",
                json!({"kind":kind,"category":"team"}),
            )
            .unwrap();
        assert!(
            resources
                .call("resource_category_list", json!({"kind":kind}))
                .unwrap()
                .to_string()
                .contains("team")
        );
        resources
            .call(
                "resource_category_delete",
                json!({"kind":kind,"category":"team"}),
            )
            .unwrap();
        assert!(
            resources
                .call(
                    "resource_category_delete",
                    json!({"kind":kind,"category":"default"})
                )
                .is_err()
        );
    }
    let create = |data| {
        resources.call(
            "resource_write",
            json!({"kind":"provider","reference":"p","create":true,"data":data}),
        )
    };
    for data in [
        json!({"base_url":"http://localhost/v1"}),
        json!({"base_url":"http://localhost/v1","api_key":" "}),
        json!({"base_url":"http://localhost/v1","api_key":"private-key","api_key_env":"ENV"}),
    ] {
        assert!(
            !create(data)
                .unwrap_err()
                .as_value()
                .to_string()
                .contains("private-key")
        );
    }
    assert_eq!(
        create(json!({"base_url":"http://localhost/v1","api_key":"private-key"})).unwrap()["api_key_configured"],
        true
    );
    resources.call("resource_write", json!({"kind":"provider","reference":"p","create":false,"data":{"api_key":"replacement-key"}})).unwrap();
    resources.call("resource_write", json!({"kind":"provider","reference":"p","create":false,"data":{"base_url":"http://localhost/v2"}})).unwrap();
    assert_eq!(
        agt_provider::ProviderStore::new(home.path())
            .read("p")
            .unwrap()
            .config
            .api_key,
        "replacement-key"
    );
    let tested = resources
        .call("resource_test", json!({"kind":"provider","reference":"p"}))
        .unwrap();
    assert!(!tested.to_string().contains("replacement-key"));
    resources.call("resource_write", json!({"kind":"mcp","reference":"safe","create":true,"data":{"mcpServers":{"safe":{"command":"this-command-must-not-run","env":{"TOKEN":"private-key"}}}}})).unwrap();
    let tested = resources
        .call("resource_test", json!({"kind":"mcp","reference":"safe"}))
        .unwrap();
    assert_eq!(tested["connected"], false);
    assert!(!tested.to_string().contains("private-key"));
    resources.call("resource_write", json!({"kind":"model","reference":"m","create":true,"data":{"provider":"p","model":"test"}})).unwrap();
    let agent = resources.call("resource_write", json!({"kind":"agent","reference":"configured","create":true,"data":{"model":"m","builtin_tools":["resource_management"]}})).unwrap();
    assert_eq!(agent["builtin_tools"], json!(["resource_management"]));
    let agent = resources.call("resource_write", json!({"kind":"agent","reference":"configured","create":false,"data":{"description":"updated"}})).unwrap();
    assert_eq!(agent["builtin_tools"], json!(["resource_management"]));
    let agent = resources.call("resource_write", json!({"kind":"agent","reference":"configured","create":false,"data":{"builtin_tools":[]}})).unwrap();
    assert_eq!(agent["builtin_tools"], json!([]));
    for tools in [
        json!(["unknown"]),
        json!(["resource_management", "resource_management"]),
    ] {
        assert!(resources.call("resource_write", json!({"kind":"agent","reference":"configured","create":false,"data":{"builtin_tools":tools}})).is_err());
    }
    assert!(
        resources
            .call(
                "resource_delete",
                json!({"kind":"prompt","reference":"default"})
            )
            .is_err()
    );
    assert!(
        resources
            .call(
                "resource_delete",
                json!({"kind":"provider","reference":"p"})
            )
            .is_err()
    );
    let plan = resources
        .call(
            "resource_delete",
            json!({"kind":"agent","reference":"configured"}),
        )
        .unwrap();
    resources
        .call(
            "resource_delete",
            json!({"kind":"agent","reference":"configured","confirm":plan["items"]}),
        )
        .unwrap();
}

#[test]
fn mcp_protocol_accepts_direct_provider_keys_and_exposes_resource_extensions() {
    let home = home();
    let service = service(home.path(), "owner", true);
    let input = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","clientInfo":{"name":"test","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"resource_write","arguments":{"kind":"provider","reference":"p","create":true,"data":{"base_url":"http://localhost/v1","api_key":"protocol-private-key"}}}}),
    ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n";
    let mut output = Vec::new();
    agt_service::mcp_server::serve(&service, input.as_bytes(), &mut output).unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(!output.contains("protocol-private-key"));
    for name in [
        "provider_models",
        "resource_test",
        "resource_category_create",
    ] {
        assert!(output.contains(name));
    }
    assert!(
        agt_provider::ProviderStore::new(home.path())
            .read("p")
            .is_ok()
    );
}

#[test]
fn resource_agents_can_reference_but_cannot_write_execution_environments() {
    let home = home();
    let config = json!({"engine":"codex","executable":std::env::current_exe().unwrap(),"working_directory":home.path(),"sandbox":"read_only"});
    let owner = service(home.path(), "owner", true);
    let agent = service(home.path(), "a", false).with_resource_management(true);
    let args = json!({"kind":"environment","reference":"local","create":true,"data":config});
    assert!(agent.call("resource_write", args.clone()).is_err());
    owner.call("resource_write", args).unwrap();
    let result=agent.call("resource_write",json!({"kind":"agent","reference":"external","create":true,"data":{"environment":"local"}})).unwrap();
    assert!(result["model"].is_null());
    assert_eq!(result["activation"]["enabled"], false);
    assert!(agent.call("resource_write",json!({"kind":"agent","reference":"bad","create":true,"data":{"environment":"local","model":"missing"}})).is_err());
    assert!(
        owner
            .call(
                "resource_delete",
                json!({"kind":"environment","reference":"local"})
            )
            .is_err()
    );
}
