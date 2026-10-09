use super::*;
#[test]
fn reset_markers_persist_without_chat_messages_and_obey_membership() {
    let (home, user, chat) = fixture();
    let sent = user
        .call(
            "message_send",
            json!({"conversation":chat,"body":"before reset","request_key":"before"}),
        )
        .unwrap();
    let seq = sent["message"]["seq"].as_i64().unwrap();
    reset_conversation_context(home.path(), &chat, "user").unwrap();
    reset_conversation_context(home.path(), &chat, "user").unwrap();
    let records = crate::context_reset_history(home.path(), &chat, "user", 0, seq).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["through_seq"], seq);
    assert_ne!(records[0]["id"], records[1]["id"]);
    assert!(crate::context_reset_history(home.path(), &chat, "missing", 0, seq).is_err());
    assert!(
        crate::context_reset_history(home.path(), &chat, "user", seq + 1, seq + 2)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        im(home.path())
            .unwrap()
            .get_conversation(&ConversationId::new(&chat))
            .unwrap()
            .last_message_seq,
        seq
    );
    assert!(reset_conversation_context(home.path(), &chat, "a").is_err());
    assert_eq!(
        crate::context_reset_history(home.path(), &chat, "user", 0, seq)
            .unwrap()
            .len(),
        2
    );
}
