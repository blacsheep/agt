//! Orchestrate one claimed task: admission, context, decision or Run, settlement.
use super::{admission, backend::ScheduledEngine, completion, guard::ScheduledModel};
use crate::schedule_state::ClaimDeferral;
use crate::{
    MessageTrigger, ModelClient, RunRequest, Runtime, ScheduleStore, ScheduledTask,
    schedule_store::now_ms,
};
use agt_core::CancellationToken;
use agt_mcp::DefaultMcpClientFactory;
use anyhow::Result;
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
    let engine = ScheduledEngine::load(home, &admitted.binding.agent, admitted.engine)?;
    let mut scheduled_model = ScheduledModel {
        inner: model,
        home: home.into(),
        task: task.clone(),
        token: token.clone(),
    };
    if engine.triage(home, &task, &mut scheduled_model)? {
        task.defer_claim(ClaimDeferral::QueueTriage);
        store.update(&task, now_ms())?;
        return Ok(());
    }
    let Some(input) = prepare_input(
        home,
        &mut store,
        &mut task,
        &admitted,
        &engine,
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
        return engine.evaluate(&mut store, &mut task, &mut scheduled_model, &input);
    }
    if engine.needs_mcp_authorization(binding.allow_mcp && round.allow_mcp) {
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
    engine: &ScheduledEngine,
    model: &mut dyn ModelClient,
    token: &CancellationToken,
) -> Result<Option<String>> {
    let Some(context) =
        engine.prepare_context(home, store, task, &admitted.messaging.im, model, token)?
    else {
        return Ok(None);
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
    let history = engine.input_history(&context, task);
    let instruction =
        crate::conversation_instructions::execution(task.work_id.is_some(), task.work_notification);
    let report_obligation = crate::work::notification_obligation(home, store, task)?;
    let input=json!({"instruction":instruction,"report_obligation":report_obligation,"discussion":task.discussion,"last_decision":task.reason,"now":now_ms(),"your_principal":task.principal,"conversation":task.conversation,"trigger":{"id":admitted.trigger.id},"messages":admitted.batch,"history":history,"conversation_summary":context["summary"],"context_reset_through_seq":context["reset_through_seq"],"presence":presence}).to_string();
    engine.validate_input(&input)?;
    Ok(Some(input))
}
