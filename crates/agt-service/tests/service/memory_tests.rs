use super::*;
#[test]
fn cross_chat_recall_is_identity_scoped_and_source_checked() {
    let home = home();
    let a = service(home.path(), "a", false);
    let b = service(home.path(), "b", false);
    let private = a
        .call("direct_create", json!({"principal":"owner"}))
        .unwrap();
    let msg = a
        .call(
            "message_send",
            json!({"conversation":private["id"],"body":"麻将手牌 123m","request_key":"cards"}),
        )
        .unwrap();
    let id = msg["message"]["id"].clone();
    assert_eq!(
        a.call("memory_search", json!({"query":"麻将"})).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        b.call("memory_search", json!({"query":"麻将"})).unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let note = json!({"id":"game","topic":"牌局规则","summary":"自己的初始手牌","source_messages":[id],"expected_revision":0});
    assert!(a.call("memory_save", note.clone()).is_ok());
    assert!(a.call("memory_save", note.clone()).is_err());
    assert!(b.call("memory_read", json!({"id":"game"})).is_err());
    assert!(b.call("memory_save", note).is_err());
    assert!(
        a.call("memory_search", json!({"kind":"notes","query":"牌局"}))
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .len()
            == 1
    );
    let im = ImStore::open(home.path().join("im/im.db")).unwrap();
    im.delete_message(
        &agt_im::MessageId::new(id.as_str().unwrap()),
        &PrincipalId::new("a"),
    )
    .unwrap();
    assert!(a.call("memory_read", json!({"id":"game"})).is_err());
    assert!(
        a.call("memory_search", json!({"kind":"notes"})).unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        a.call("memory_forget", json!({"id":"game","expected_revision":1}))
            .is_ok()
    );
}
#[test]
fn search_dates_pagination_and_disable_are_explicit() {
    let home = home();
    let a = service(home.path(), "a", false);
    let chat = a
        .call("direct_create", json!({"principal":"owner"}))
        .unwrap();
    for n in 0..3 {
        a.call("message_send",json!({"conversation":chat["id"],"body":format!("topic {n}"),"request_key":format!("m{n}")})).unwrap();
    }
    let first = a
        .call("memory_search", json!({"query":"topic","limit":2}))
        .unwrap();
    assert_eq!(first["items"].as_array().unwrap().len(), 2);
    let second = a
        .call(
            "memory_search",
            json!({"query":"topic","limit":2,"before":first["next_cursor"]}),
        )
        .unwrap();
    assert_eq!(second["items"].as_array().unwrap().len(), 1);
    assert!(
        a.call("memory_search", json!({"day":"1970-01-01"}))
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(a.call("memory_search", json!({"day":"bad"})).is_err());
    let mut config = agt_config::RuntimeConfig::default();
    config.memory.enabled = false;
    config.save(home.path()).unwrap();
    assert!(a.call("memory_search", json!({})).is_err());
}
