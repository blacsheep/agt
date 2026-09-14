use agt_im::*;
use serde_json::json;
fn identity(store: &ImStore, id: &str, kind: PrincipalKind) -> PrincipalId {
    store
        .create_principal(PrincipalInput {
            id: id.into(),
            kind,
            display_name: id.into(),
            metadata: json!({}),
        })
        .unwrap()
        .id
}
fn send(store: &ImStore, chat: &ConversationId, sender: &PrincipalId, body: &str) -> Message {
    store
        .send_message(SendMessageRequest {
            conversation_id: chat.clone(),
            sender_id: sender.clone(),
            body: body.into(),
            client_message_id: None,
            reply_to_id: None,
            metadata: json!({}),
            idempotency_key: Some(body.into()),
        })
        .unwrap()
        .message
}
fn fixture() -> (ImStore, PrincipalId, PrincipalId, ConversationId) {
    let s = ImStore::open_in_memory().unwrap();
    let owner = identity(&s, "owner", PrincipalKind::User);
    let member = identity(&s, "member", PrincipalKind::User);
    let chat = s
        .create_group(&owner, Some("group".into()), vec![member.clone()])
        .unwrap()
        .id;
    (s, owner, member, chat)
}
#[test]
fn personal_entry_hiding_is_independent_and_only_new_messages_reveal_it() {
    let (s, owner, member, chat) = fixture();
    send(&s, &chat, &owner, "first");
    s.set_conversation_hidden(&chat, &member, true).unwrap();
    assert!(s.list_conversations(&member).unwrap().is_empty());
    assert_eq!(s.list_conversations(&owner).unwrap().len(), 1);
    assert_eq!(
        s.list_conversation_entries(&member, true, true)
            .unwrap()
            .len(),
        1
    );
    send(&s, &chat, &owner, "first"); // duplicate must not reveal hidden entries
    assert!(s.list_conversations(&member).unwrap().is_empty());
    send(&s, &chat, &owner, "second");
    assert_eq!(s.list_conversations(&member).unwrap().len(), 1);
    s.leave_conversation(&chat, &member).unwrap();
    s.set_conversation_hidden(&chat, &member, true).unwrap();
    send(&s, &chat, &owner, "third");
    assert!(s.list_conversations(&member).unwrap().is_empty());
    s.set_conversation_hidden(&chat, &member, false).unwrap();
    assert!(
        s.list_conversations(&member).unwrap()[0]
            .left_message_seq
            .is_some()
    );
    assert!(
        s.list_conversation_entries(&member, true, true)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn membership_windows_filter_messages_events_status_and_pagination() {
    let (s, owner, member, chat) = fixture();
    let a = send(&s, &chat, &owner, "before leave");
    s.leave_conversation(&chat, &member).unwrap();
    let b = send(&s, &chat, &owner, "private while absent");
    assert_eq!(
        s.list_messages(&chat, &member, MessageListOptions::default())
            .unwrap()
            .messages
            .len(),
        1
    );
    assert!(s.visible_message(&b.id, &member).is_err());
    s.add_member(&chat, &owner, &member).unwrap();
    let c = send(&s, &chat, &owner, "after rejoin");
    let first = s
        .list_messages(
            &chat,
            &member,
            MessageListOptions {
                limit: 1,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(first.messages[0].id, c.id);
    let second = s
        .list_messages(
            &chat,
            &member,
            MessageListOptions {
                limit: 1,
                before: first.next_cursor,
                after: None,
            },
        )
        .unwrap();
    assert_eq!(second.messages[0].id, a.id);
    assert!(second.next_cursor.is_none());
    assert!(
        !s.list_events(&chat, &member, None, 200)
            .unwrap()
            .events
            .iter()
            .any(|e| e.payload.to_string().contains("private while absent"))
    );
    assert!(s.visible_message(&b.id, &member).is_err());
    assert!(
        s.send_message(SendMessageRequest {
            conversation_id: chat.clone(),
            sender_id: member.clone(),
            body: "reply".into(),
            client_message_id: None,
            reply_to_id: Some(b.id),
            metadata: json!({}),
            idempotency_key: None
        })
        .is_err()
    );
    assert_eq!(s.deliveries(&member, true, 100).unwrap().len(), 1); // first message after rejoin included
    s.leave_conversation(&chat, &member).unwrap();
    assert_eq!(
        s.list_messages(&chat, &member, MessageListOptions::default())
            .unwrap()
            .messages
            .len(),
        2
    );
}
#[test]
fn agent_groups_require_persistent_owner_and_cannot_remove_or_dissolve_them() {
    let s = ImStore::open_in_memory().unwrap();
    let agent = identity(&s, "lead", PrincipalKind::Agent);
    let owner = identity(&s, "human", PrincipalKind::User);
    let other = identity(&s, "other", PrincipalKind::User);
    assert!(
        s.create_group(&agent, Some("missing owner".into()), vec![])
            .is_err()
    );
    assert_eq!(s.ensure_workspace_owner(&owner).unwrap(), owner);
    assert_eq!(s.ensure_workspace_owner(&other).unwrap(), owner);
    let chat = s.create_group(&agent, Some("team".into()), vec![]).unwrap();
    assert_eq!(chat.owner_id, Some(owner.clone()));
    assert_eq!(
        s.get_member(&chat.id, &agent).unwrap().role,
        MemberRole::Admin
    );
    assert_eq!(
        s.get_member(&chat.id, &owner).unwrap().role,
        MemberRole::Owner
    );
    assert!(s.remove_member(&chat.id, &agent, &owner).is_err());
    assert!(s.dissolve_group(&chat.id, &agent).is_err());
    assert!(s.delete_conversation(&chat.id, &agent).is_err());
    send(&s, &chat.id, &agent, "record");
    s.dissolve_group(&chat.id, &owner).unwrap();
    assert!(s.list_conversations(&agent).unwrap()[0].dissolved);
    assert_eq!(
        s.list_messages(&chat.id, &agent, MessageListOptions::default())
            .unwrap()
            .messages
            .len(),
        1
    );
    assert!(s.reactivate_conversation(&chat.id, &owner).is_err());
    assert!(s.add_member(&chat.id, &owner, &other).is_err());
}
#[test]
fn v2_migration_reconstructs_departure_windows_and_agent_ownership() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("im.db");
    let s = ImStore::open(&path).unwrap();
    let owner = identity(&s, "owner", PrincipalKind::User);
    let agent = identity(&s, "agent", PrincipalKind::Agent);
    let member = identity(&s, "member", PrincipalKind::User);
    let chat = s
        .create_group(
            &owner,
            Some("old".into()),
            vec![member.clone(), agent.clone()],
        )
        .unwrap();
    send(&s, &chat.id, &owner, "before");
    s.leave_conversation(&chat.id, &member).unwrap();
    send(&s, &chat.id, &owner, "gap");
    s.add_member(&chat.id, &owner, &member).unwrap();
    send(&s, &chat.id, &owner, "after");
    drop(s);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("DROP TRIGGER membership_first; DROP TRIGGER membership_leave; DROP TRIGGER membership_rejoin; DROP TRIGGER message_reveals_sessions;
        DROP TABLE membership_history; DROP TABLE managed_groups; DROP TABLE workspace_owner;
        ALTER TABLE conversation_members DROP COLUMN hidden; ALTER TABLE conversation_members DROP COLUMN last_viewed_at; ALTER TABLE conversations DROP COLUMN dissolved;
        UPDATE conversation_members SET role='member' WHERE principal_id='owner';
        UPDATE conversation_members SET role='owner' WHERE principal_id='agent';
        UPDATE conversations SET owner_id='agent',state='deleted'; PRAGMA user_version=2;").unwrap();
    drop(db);
    let s = ImStore::open(&path).unwrap();
    let page = s
        .list_messages(&chat.id, &member, MessageListOptions::default())
        .unwrap();
    assert_eq!(
        page.messages
            .iter()
            .map(|m| m.body.as_str())
            .collect::<Vec<_>>(),
        vec!["before", "after"]
    );
    assert!(
        !s.list_events(&chat.id, &member, None, 200)
            .unwrap()
            .events
            .iter()
            .any(|e| e.payload["body"] == "gap")
    );
    s.ensure_workspace_owner(&owner).unwrap();
    s.reactivate_conversation(&chat.id, &owner).unwrap();
    assert_eq!(
        s.get_conversation(&chat.id).unwrap().owner_id,
        Some(owner.clone())
    );
    assert_eq!(
        s.get_member(&chat.id, &agent).unwrap().role,
        MemberRole::Admin
    );
    s.set_conversation_hidden(&chat.id, &member, true).unwrap();
    drop(s);
    let s = ImStore::open(&path).unwrap();
    assert!(s.list_conversations(&member).unwrap().is_empty());
    assert_eq!(s.workspace_owner().unwrap(), Some(owner));
}

#[test]
fn explicit_visits_are_persistent_personal_and_do_not_change_history_or_hidden_state() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("im.db");
    let s = ImStore::open(&path).unwrap();
    let owner = identity(&s, "owner", PrincipalKind::User);
    let member = identity(&s, "member", PrincipalKind::User);
    let outsider = identity(&s, "outsider", PrincipalKind::User);
    let chat = s
        .create_group(&owner, None, vec![member.clone()])
        .unwrap()
        .id;
    send(&s, &chat, &owner, "retained");
    s.dissolve_group(&chat, &owner).unwrap();
    s.set_conversation_hidden(&chat, &owner, true).unwrap();
    assert!(s.mark_conversation_viewed(&chat, &outsider).is_err());
    let first = s.mark_conversation_viewed(&chat, &owner).unwrap();
    s.list_messages(&chat, &owner, MessageListOptions::default())
        .unwrap();
    let entry = s
        .list_conversation_entries(&owner, true, false)
        .unwrap()
        .remove(0);
    assert_eq!(entry.last_viewed_at, Some(first));
    assert!(entry.hidden && entry.dissolved);
    assert_eq!(
        s.list_conversations(&member).unwrap()[0].last_viewed_at,
        None
    );
    assert!(s.mark_conversation_viewed(&chat, &owner).unwrap() > first);
    drop(s);
    let s = ImStore::open(&path).unwrap();
    assert!(
        s.list_conversation_entries(&owner, true, false).unwrap()[0]
            .last_viewed_at
            .unwrap()
            > first
    );
    assert_eq!(
        s.list_messages(&chat, &owner, MessageListOptions::default())
            .unwrap()
            .messages[0]
            .body,
        "retained"
    );
    // Existing v3 homes migrate additively on next open.
    drop(s);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(
        "ALTER TABLE conversation_members DROP COLUMN last_viewed_at; PRAGMA user_version=3;",
    )
    .unwrap();
    drop(db);
    let s = ImStore::open(&path).unwrap();
    assert_eq!(
        s.list_conversation_entries(&owner, true, false).unwrap()[0].last_viewed_at,
        None
    );
    assert_eq!(
        s.list_messages(&chat, &owner, MessageListOptions::default())
            .unwrap()
            .messages
            .len(),
        1
    );
}
