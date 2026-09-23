//! Explicit human authorization and conversation lifecycle controls.
use super::im;
use crate::{ScheduleStore, schedule_store::now_ms};
use agt_im::{ConversationId, ConversationState, MessageId, PrincipalId, PrincipalKind};
use agt_service::BindingStore;
use anyhow::{Result, bail};
use serde_json::json;
use std::path::Path;

/// Explicit authorization of human messages; merely starting the daemon does not
/// infer over arbitrary old inboxes. Descendants inherit the authorized round.
pub fn start_discussion(
    home: &Path,
    conversation: &str,
    message: Option<&str>,
    allow_mcp: bool,
) -> Result<()> {
    let im = im(home)?;
    let id = ConversationId::new(conversation);
    if im.get_conversation(&id)?.state != ConversationState::Active {
        bail!("conversation is not active");
    }
    let mut store = ScheduleStore::open(home)?;
    let mut messages = Vec::new();
    if let Some(message) = message {
        messages.push(im.get_message(&MessageId::new(message))?);
    } else {
        for binding in BindingStore::open(home)?.list()? {
            let mut after = 0;
            loop {
                let page = im.deliveries_after(&binding.principal, true, 200, Some(&id), after)?;
                if page.is_empty() {
                    break;
                }
                after = page.last().expect("nonempty").id;
                for delivery in page {
                    messages.push(im.get_message(&delivery.message_id)?);
                }
            }
        }
    }
    messages.sort_by_key(|m| m.seq);
    messages.dedup_by(|a, b| a.id == b.id);
    for message in messages {
        if message.conversation_id != id {
            bail!("message does not belong to conversation");
        }
        if message.seq > store.context_cutoff(conversation)?
            && message.deleted_at.is_none()
            && message.metadata["discussion_id"].is_null()
            && im.get_principal(&message.sender_id)?.kind == PrincipalKind::User
        {
            store.authorize_human(
                message.id.as_str(),
                conversation,
                message.seq,
                allow_mcp,
                now_ms(),
            )?;
        }
    }
    if message.is_none() {
        store.control(conversation, "resume", allow_mcp, now_ms())?;
    }
    Ok(())
}

pub fn delete_scheduled_conversation(
    home: &Path,
    conversation: &str,
    actor: &str,
) -> Result<agt_im::Conversation> {
    let store = ScheduleStore::open(home)?;
    store.connection.execute_batch("BEGIN IMMEDIATE")?;
    // IM checks owner/admin permissions before any discussion is modified.
    let result = im(home)?
        .delete_conversation(&ConversationId::new(conversation), &PrincipalId::new(actor))?;
    store.connection.execute(
        "UPDATE discussions SET state='ended' WHERE conversation=?1",
        [conversation],
    )?;
    store.connection.execute_batch("COMMIT")?;
    agt_storage::wake::notify(home);
    crate::HarnessSessions::open(home)?.cancel_conversation(conversation)?;
    Ok(result)
}

pub fn dissolve_scheduled_group(home: &Path, conversation: &str, actor: &str) -> Result<()> {
    let store = ScheduleStore::open(home)?;
    store.connection.execute_batch("BEGIN IMMEDIATE")?;
    im(home)?.dissolve_group(&ConversationId::new(conversation), &PrincipalId::new(actor))?;
    store.connection.execute(
        "UPDATE discussions SET state='ended' WHERE conversation=?1",
        [conversation],
    )?;
    store.connection.execute_batch("COMMIT")?;
    agt_storage::wake::notify(home);
    crate::HarnessSessions::open(home)?.cancel_conversation(conversation)?;
    Ok(())
}

/// Start a fresh model-visible epoch without deleting IM history.
pub fn reset_conversation_context(
    home: &Path,
    conversation: &str,
    actor: &str,
) -> Result<serde_json::Value> {
    let store = ScheduleStore::open(home)?;
    store.connection.execute_batch("BEGIN IMMEDIATE")?;
    let im = im(home)?;
    let id = ConversationId::new(conversation);
    let principal = PrincipalId::new(actor);
    let chat = im.get_conversation(&id)?;
    let member = im.get_member(&id, &principal)?;
    if chat.state != ConversationState::Active
        || member.left_message_seq.is_some()
        || im.get_principal(&principal)?.kind != PrincipalKind::User
        || (chat.kind == agt_im::ConversationKind::Group
            && !matches!(
                member.role,
                agt_im::MemberRole::Owner | agt_im::MemberRole::Admin
            ))
    {
        bail!("only an active human direct participant or group owner/admin may reset context");
    }
    // Epoch and discussion cancellation commit together in scheduler.db. Removing a
    // cached summary before that commit is safe: on failure it can be rebuilt from IM.
    store.connection.execute("INSERT INTO context_resets VALUES(?1,?2) ON CONFLICT(conversation) DO UPDATE SET through_seq=MAX(through_seq,excluded.through_seq)", rusqlite::params![conversation, chat.last_message_seq])?;
    store.connection.execute(
        "INSERT INTO context_reset_events(conversation,through_seq,created_at) VALUES(?1,?2,?3)",
        rusqlite::params![conversation, chat.last_message_seq, now_ms()],
    )?;
    crate::context_store::ContextStore::open(home)?.clear(conversation)?;
    store.connection.execute("UPDATE discussions SET state='ended' WHERE conversation=?1 OR id IN (SELECT discussion FROM tasks WHERE conversation=?1)", [conversation])?;
    store.connection.execute_batch("COMMIT")?;
    agt_storage::wake::notify(home);
    crate::HarnessSessions::open(home)?.cancel_conversation(conversation)?;
    Ok(
        json!({"conversation":conversation,"reset_through_seq":chat.last_message_seq,"history_preserved":true}),
    )
}
