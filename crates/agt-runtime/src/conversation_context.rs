//! Per-principal public-conversation summaries. Private Run transcripts never enter this store.
use crate::context_store::{ContextStore, ConversationSummary};
use crate::{ModelClient, ResolvedAgent, RuntimeError, ScheduledTask};
use agt_core::CancellationToken;
use agt_im::{ConversationId, ImStore, MessageListOptions, PrincipalId};
use serde_json::{Value, json};
use std::path::Path;

pub(crate) fn visible_history(
    im: &ImStore,
    task: &ScheduledTask,
    cutoff: i64,
) -> crate::Result<Vec<Value>> {
    let mut result = Vec::new();
    let mut before = None;
    loop {
        let page = im
            .list_messages(
                &ConversationId::new(&task.conversation),
                &PrincipalId::new(&task.principal),
                MessageListOptions {
                    limit: 200,
                    before,
                    after: None,
                },
            )
            .map_err(|e| {
                RuntimeError::Im(crate::Reason::because(
                    "cannot read visible conversation history",
                    e,
                ))
            })?;
        let reached = page.messages.first().is_some_and(|m| m.seq <= cutoff);
        result.extend(
            page.messages
                .into_iter()
                .filter(|m| m.seq > cutoff && m.seq <= task.through_seq && m.deleted_at.is_none())
                .map(|m| json!({"seq":m.seq,"message_id":m.id,"sender":m.sender_id,"body":m.body})),
        );
        before = page.next_cursor;
        if reached || before.is_none() {
            break;
        }
    }
    result.sort_by_key(|m| m["seq"].as_i64().unwrap_or(0));
    Ok(result)
}
fn hash(history: &[Value]) -> String {
    crate::fingerprint(json!(history).to_string().as_bytes())
}

pub(crate) fn prepare(
    home: &Path,
    im: &ImStore,
    task: &ScheduledTask,
    agent: &ResolvedAgent,
    model: &mut dyn ModelClient,
    token: &CancellationToken,
) -> crate::Result<Value> {
    let store = ContextStore::open(home)?;
    let cutoff = crate::ScheduleStore::open(home)
        .and_then(|s| s.context_cutoff(&task.conversation))
        .map_err(|e| {
            RuntimeError::Configuration(crate::Reason::because(
                "context reset state unavailable",
                e,
            ))
        })?;
    let history = visible_history(im, task, cutoff)?;
    let tail_budget = crate::context::input_budget(agent) / 4;
    let mut start = 0;
    let mut summary = String::new();
    if let Some(saved) = store.load(&task.conversation, &task.principal)? {
        let prefix =
            history.partition_point(|m| m["seq"].as_i64().unwrap_or(0) <= saved.through_seq);
        if saved.agent_fingerprint == agent.fingerprint
            && prefix > 0
            && history[prefix - 1]["seq"] == saved.through_seq
            && hash(&history[..prefix]) == saved.source_hash
        {
            summary = saved.text;
            start = prefix;
        }
    }
    let mut split = start;
    if crate::context::estimate(&json!(&history[start..]).to_string()) > tail_budget
        || crate::context::estimate(
            &json!({"summary":summary,"history":&history[start..]}).to_string(),
        ) > crate::context::input_budget(agent) / 2
    {
        // Compact in batches, leaving headroom for several subsequent messages.
        split = history.len();
        let mut used = 0;
        for message in history[start..].iter().rev().take(20) {
            let cost = crate::context::estimate(&message.to_string());
            if used + cost > tail_budget / 2 {
                break;
            }
            used += cost;
            split -= 1;
        }
        let mut attempts = 0;
        summary = crate::context::summarize(
            model,
            agent,
            &summary,
            &json!(&history[start..split]).to_string(),
            token,
            &mut attempts,
        )?;
        // The conversation envelope reserves the other half for current triggers,
        // instructions and tools; the summary target itself is only advisory.
        crate::context::ensure_fits(
            crate::context::estimate(
                &json!({"summary":summary,"history":&history[split..]}).to_string(),
            ),
            crate::context::input_budget(agent) / 2,
            "conversation history and summary",
        )?;
        let checkpoint = ConversationSummary {
            through_seq: history[split - 1]["seq"].as_i64().unwrap(),
            source_hash: hash(&history[..split]),
            agent_fingerprint: agent.fingerprint.clone(),
            text: summary.clone(),
        };
        crate::scheduler::guarded(home, task, || {
            // A delete/leave/reset during inference must not revive stale or invisible data.
            if visible_history(im, task, cutoff)? != history {
                return Err(RuntimeError::Canceled);
            }
            store.save(&task.conversation, &task.principal, &checkpoint)
        })?;
    }
    Ok(json!({"summary":summary,"history":&history[split..],"reset_through_seq":cutoff}))
}
