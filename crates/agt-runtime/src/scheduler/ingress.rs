//! Translate durable IM deliveries into independent scheduler tasks.
use super::im;
use crate::{ScheduleStore, ScheduledTask, schedule_store::now_ms};
use agt_im::{DeliveryState, PrincipalKind};
use agt_service::BindingStore;
use anyhow::Result;
use std::path::Path;

pub(crate) fn scan_inputs(home: &Path, store: &mut ScheduleStore) -> Result<()> {
    let im = im(home)?;
    for binding in BindingStore::open(home)?.list()? {
        let mut after = 0;
        loop {
            let page = im.deliveries_after(&binding.principal, true, 200, None, after)?;
            if page.is_empty() {
                break;
            }
            after = page.last().expect("nonempty").id;
            for delivery in page {
                if store.assigned(delivery.id)? {
                    continue;
                }
                let message = im.get_message(&delivery.message_id)?;
                let route = match crate::work::route(store, &message) {
                    Ok(route) => route,
                    Err(_) => {
                        im.update_delivery(
                            delivery.id,
                            DeliveryState::Skipped,
                            None,
                            Some("invalid task notification"),
                        )?;
                        continue;
                    }
                };
                if route
                    .as_ref()
                    .is_some_and(|(_, target, _)| target != binding.principal.as_str())
                {
                    im.update_delivery(
                        delivery.id,
                        DeliveryState::Skipped,
                        None,
                        Some("directed work message for another recipient"),
                    )?;
                    continue;
                }
                if let Some((id, _, true)) = &route
                    && !matches!(
                        message.metadata["work_event"].as_str(),
                        Some("input_requested" | "resolved" | "dependency_expired")
                    )
                    && crate::work::wait::consumed_notice(&store.connection, id)?
                {
                    im.update_delivery(
                        delivery.id,
                        DeliveryState::Completed,
                        None,
                        Some("result consumed by work continuation"),
                    )?;
                    continue;
                }
                if message.seq <= store.context_cutoff(message.conversation_id.as_str())? {
                    continue;
                }
                let discussion = if let Some(id) = message.metadata["discussion_id"].as_str() {
                    Some(id.to_string())
                } else {
                    store.human_discussion(message.id.as_str())?
                };
                let Some(discussion) = discussion else {
                    continue;
                };
                if store.discussion(&discussion)?.is_none() {
                    continue;
                }
                let peer = im.get_principal(&message.sender_id)?.kind != PrincipalKind::User;
                let mentioned = message.metadata["mentions"].as_array().is_some_and(|a| {
                    a.iter()
                        .any(|p| p.as_str() == Some(binding.principal.as_str()))
                });
                let now = now_ms();
                store.ingest(ScheduledTask {
                    id: route
                        .as_ref()
                        .map(|(id, _, notice)| {
                            if *notice {
                                format!("notice:{}", message.id)
                            } else {
                                format!("work:{id}")
                            }
                        })
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                    discussion,
                    principal: binding.principal.to_string(),
                    conversation: message.conversation_id.to_string(),
                    state: "queued".into(),
                    deliveries: vec![delivery.id],
                    message: message.id.to_string(),
                    through_seq: message.seq,
                    usage_epoch: 0,
                    triggers: vec![message.id.to_string()],
                    decision_required: route.is_none() && peer && !mentioned,
                    quiet: false,
                    due_at: message.created_at,
                    deadline: now + 60_000,
                    quiet_ms: 0,
                    version: 0,
                    owner: None,
                    run_id: None,
                    reason: None,
                    last_decision: None,
                    work_id: route.as_ref().map(|(id, _, _)| id.clone()),
                    work_notification: route.as_ref().is_some_and(|(_, _, notice)| *notice),
                    work_event: message.metadata["work_event"].as_str().map(str::to_owned),
                    work_input_id: message.metadata["input_request_id"]
                        .as_str()
                        .map(str::to_owned),
                })?;
            }
        }
    }
    Ok(())
}
