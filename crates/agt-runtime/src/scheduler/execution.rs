//! Orchestrate one claimed task: admission, context, decision or Run, settlement.
use super::{admission, completion, decision, guard::ScheduledModel};
use crate::schedule_state::ClaimDeferral;
use crate::{
    MessageTrigger, ModelClient, ResolvedAgent, RunRequest, Runtime, RuntimeError, ScheduleStore,
    ScheduledTask, schedule_store::now_ms,
};
use agt_core::CancellationToken;
use agt_mcp::DefaultMcpClientFactory;
use anyhow::{Result, bail};
use serde_json::json;
use std::path::Path;

pub(crate) fn execute(
    home: &Path,
    mut task: ScheduledTask,
    model: &mut dyn ModelClient,
    token: &CancellationToken,
) -> Result<()> {
    let mut store = ScheduleStore::open(home)?;
    let Some(admitted) = admission::prepare(home, &mut store, &mut task, token)? else {
        return Ok(());
    };
    let agent = if admitted.external {
        None
    } else {
        Some(ResolvedAgent::load(home, &admitted.binding.agent)?)
    };
    let mut scheduled_model = ScheduledModel {
        inner: model,
        home: home.into(),
        task: task.clone(),
        token: token.clone(),
    };
    if let Some(agent) = &agent
        && crate::work::triage::run(home, &task, agent, &mut scheduled_model)?
    {
        task.defer_claim(ClaimDeferral::QueueTriage);
        store.update(&task, now_ms())?;
        return Ok(());
    }
    let Some(input) = prepare_input(
        home,
        &mut store,
        &mut task,
        &admitted,
        agent.as_ref(),
        &mut scheduled_model,
        token,
    )?
    else {
        return Ok(());
    };
    let admission::Admission {
        binding,
        messaging,
        trigger,
        round,
        ..
    } = admitted;
    if task.state == "evaluating" {
        return decision::evaluate(
            &mut store,
            &mut task,
            &mut scheduled_model,
            agent.as_ref().unwrap(),
            &input,
        );
    }
    if agent
        .as_ref()
        .is_some_and(|agent| !(agent.mcps.is_empty() || binding.allow_mcp && round.allow_mcp))
    {
        task.block_for_mcp();
        store.update(&task, now_ms())?;
        return Ok(());
    }
    let depth = trigger.metadata["automated_depth"]
        .as_u64()
        .unwrap_or(0)
        .saturating_add(1)
        .min(u64::from(u32::MAX)) as u32;
    let mut request = RunRequest::new(&binding.agent, input);
    request.request_key = if !task.work_notification {
        task.work_id
            .as_ref()
            .map(|id| {
                crate::work::WorkLedger::open(home)
                    .and_then(|l| l.get(id))
                    .map(|i| crate::work::execution_key(&i))
            })
            .transpose()?
            .unwrap_or_else(|| format!("schedule:{}:{}", task.id, task.version))
    } else {
        format!("schedule:{}:{}", task.id, task.version)
    };
    request.max_steps = binding.max_steps;
    request.working_directory = binding.working_directory;
    request.trigger = Some(MessageTrigger {
        message_id: trigger.id.clone(),
        conversation_id: trigger.conversation_id.clone(),
        principal: binding.principal.clone(),
        depth,
        allow_mcp: binding.allow_mcp && round.allow_mcp,
    });
    scheduled_model.task = task.clone();
    let mut runtime = Runtime::open(home, &mut scheduled_model, &DefaultMcpClientFactory)?;
    runtime.discussion = Some(task.discussion.clone());
    runtime.scheduled_task = Some(task.clone());
    let record = runtime.run(request, token)?;
    drop(runtime);
    completion::settle(
        home,
        &mut store,
        &mut task,
        record,
        &messaging,
        (binding.principal, trigger.conversation_id),
        token,
    )
}

fn prepare_input(
    home: &Path,
    store: &mut ScheduleStore,
    task: &mut ScheduledTask,
    admitted: &admission::Admission,
    agent: Option<&ResolvedAgent>,
    model: &mut dyn ModelClient,
    token: &CancellationToken,
) -> Result<Option<String>> {
    let admission::Admission {
        messaging,
        trigger,
        batch,
        external,
        ..
    } = admitted;
    let external = *external;
    let context = if external {
        let cutoff = store.context_cutoff(&task.conversation)?;
        json!({"history":crate::conversation_context::visible_history(&messaging.im,task,cutoff)?,"summary":"","reset_through_seq":cutoff})
    } else {
        match crate::conversation_context::prepare(
            home,
            &messaging.im,
            task,
            agent.unwrap(),
            model,
            token,
        ) {
            Ok(context) => context,
            Err(error) => {
                task.state =
                    if matches!(error, RuntimeError::Canceled | RuntimeError::Stopped(_)) {
                        "canceled"
                    } else {
                        "failed"
                    }
                    .into();
                task.reason = Some(format!("conversation context: {error}"));
                store.update(task, now_ms())?;
                return Ok(None);
            }
        }
    };
    let presence: Vec<_> = store
        .tasks()?
        .into_iter()
        .filter(|t| {
            t.conversation == task.conversation
                && t.id != task.id
                && matches!(t.state.as_str(), "evaluating" | "ready_to_reply" | "responding" | "waiting")
        })
        .map(|t| json!({"principal":t.principal,"state":t.state,"wake_at":t.due_at,"waiting_for_quiet":t.quiet}))
        .collect();
    let history = if external {
        context["history"].clone()
    } else {
        json!(
            context["history"]
                .as_array()
                .unwrap_or(&Vec::new())
                .iter()
                .filter(|m| !task
                    .triggers
                    .iter()
                    .any(|id| m["message_id"] == id.as_str()))
                .collect::<Vec<_>>()
        )
    };
    let instruction =
        crate::conversation_instructions::execution(task.work_id.is_some(), task.work_notification);
    let report_obligation = crate::work::notification_obligation(home, store, task)?;
    let input=json!({"instruction":instruction,"report_obligation":report_obligation,"discussion":task.discussion,"last_decision":task.reason,"now":now_ms(),"your_principal":task.principal,"conversation":task.conversation,"trigger":{"id":trigger.id},"messages":batch,"history":history,"conversation_summary":context["summary"],"context_reset_through_seq":context["reset_through_seq"],"presence":presence}).to_string();
    if let Some(agent) = agent
        && crate::context::estimate(&input) + crate::context::estimate(&agent.system_prompt)
            > crate::context::input_budget(agent)
    {
        bail!(
            "conversation input exceeds context budget; shorten the message batch or increase model context_window"
        );
    }
    Ok(Some(input))
}
