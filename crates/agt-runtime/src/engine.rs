use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agt_core::CancellationToken;
use agt_im::{ImStore, PrincipalId, PrincipalInput, PrincipalKind, SendMessageRequest};
use agt_mcp::McpClientFactory;
use serde::Deserialize;
use serde_json::json;

use crate::store::database_path;
use crate::{
    ModelClient, ResolvedAgent, Result, RunRecord, RunRequest, RunState, RunStore, RuntimeError,
};

mod dispatch;
mod native;
pub(crate) use dispatch::EngineKind;

pub struct Runtime<'a> {
    home: PathBuf,
    model: &'a mut dyn ModelClient,
    factory: &'a dyn McpClientFactory,
    store: RunStore,
    pub(crate) discussion: Option<String>,
    pub(crate) scheduled_task: Option<crate::ScheduledTask>,
}

impl<'a> Runtime<'a> {
    pub fn open(
        home: &Path,
        model: &'a mut dyn ModelClient,
        factory: &'a dyn McpClientFactory,
    ) -> Result<Self> {
        Ok(Self {
            home: home.to_owned(),
            model,
            factory,
            store: RunStore::open(home)?,
            discussion: None,
            scheduled_task: None,
        })
    }

    pub fn runs(&self) -> &RunStore {
        &self.store
    }

    /// A replay returns the stored state, including failed or interrupted runs.
    pub fn run(
        &mut self,
        request: RunRequest,
        cancellation: &CancellationToken,
    ) -> Result<RunRecord> {
        let request = request.normalize()?;
        let (mut record, claimed) = self.store.claim(&request, None)?;
        if let Some(task) = &self.scheduled_task {
            crate::work::bind_run(&self.home, task, &record)
                .map_err(|_| RuntimeError::Configuration("cannot bind work Run"))?;
        }
        if !claimed {
            return Ok(record);
        }
        let mut step_count = 0;
        let result = self.execute_claimed(&request, &mut record, &mut step_count, cancellation);
        record.model_steps = step_count;
        self.finish(record, result)
    }

    fn finish(&self, mut record: RunRecord, result: Result<String>) -> Result<RunRecord> {
        crate::progress::finish(&mut record);
        match result {
            Ok(_) if record.state == RunState::Suspended => {}
            Ok(output) => {
                record.state = RunState::Completed;
                record.output = Some(output);
            }
            Err(error) => {
                record.state = crate::termination::finish(&mut record, &error);
                record.error = Some(error.to_string());
            }
        }
        self.store.save(&record)?;
        self.store.get(&record.id)
    }

    fn drive(
        &mut self,
        record: &mut RunRecord,
        agents: &BTreeMap<String, ResolvedAgent>,
        step_count: &mut u32,
        cancellation: &CancellationToken,
    ) -> Result<String> {
        match native::NativeRun::prepare(self, record, agents, step_count, cancellation)?.run()? {
            native::LoopOutcome::Complete(reply) => Ok(reply.into_output()),
            // The loop has committed its checkpoint before returning this outcome.
            native::LoopOutcome::Suspended => Ok(String::new()),
        }
    }

    fn delegate(
        &mut self,
        parent: &RunRecord,
        args: Delegation,
        call_id: &str,
        agents: &BTreeMap<String, ResolvedAgent>,
        step_count: &mut u32,
        cancellation: &CancellationToken,
    ) -> Result<String> {
        check_canceled(cancellation)?;
        let request = RunRequest {
            agent: args.agent,
            input: args.input,
            request_key: call_id.to_owned(),
            delegates: Vec::new(),
            max_steps: parent.request.max_steps,
            working_directory: parent.request.working_directory.clone(),
            trigger: None,
        };
        let im = ImStore::open(database_path(&self.home, "im", "im.db")?)
            .map_err(|_| RuntimeError::Im("cannot open message store"))?;
        let sender = bind_principal(&im, &parent.request.agent)?;
        let receiver = bind_principal(&im, &request.agent)?;
        let conversation = im
            .create_direct(&sender, &receiver)
            .map_err(|_| RuntimeError::Im("cannot create direct conversation"))?;
        let (mut child, claimed) = self.store.claim(&request, Some(&parent.id))?;
        let initial_steps = *step_count;
        let preparation = check_canceled(cancellation).and_then(|()| {
            im.send_message(SendMessageRequest {
                conversation_id: conversation.id.clone(),
                sender_id: sender,
                body: request.input.clone(),
                client_message_id: Some(format!("run:{}:input", child.id)),
                reply_to_id: None,
                metadata: json!({"run_id":child.id,"parent_run_id":parent.id}),
                idempotency_key: None,
            })
            .map_err(|_| RuntimeError::Im("cannot publish delegation input"))
        });
        if claimed {
            let result = match preparation.as_ref() {
                Ok(_) => self.drive(&mut child, agents, step_count, cancellation),
                Err(RuntimeError::Canceled) => Err(RuntimeError::Canceled),
                Err(RuntimeError::Stopped(code)) => Err(RuntimeError::Stopped(*code)),
                Err(_) => Err(RuntimeError::Im("cannot publish delegation input")),
            };
            child.model_steps = *step_count - initial_steps;
            child = self.finish(child, result)?;
        }
        let input_message = preparation?;
        match child.state {
            RunState::Running | RunState::Suspended | RunState::OutcomeUnknown => {
                return Err(RuntimeError::Busy);
            }
            RunState::Canceled => return Err(RuntimeError::Canceled),
            RunState::Failed | RunState::Interrupted => {
                return Err(RuntimeError::Tool(
                    "delegated run failed; inspect child run",
                ));
            }
            RunState::Completed => {}
        }
        let output = child
            .output
            .ok_or(RuntimeError::Tool("completed child has no output"))?;
        check_canceled(cancellation)?;
        im.send_message(SendMessageRequest {
            conversation_id: conversation.id,
            sender_id: receiver,
            body: output.clone(),
            client_message_id: Some(format!("run:{}:output", child.id)),
            reply_to_id: Some(input_message.message.id),
            metadata: json!({"run_id":child.id,"parent_run_id":parent.id}),
            idempotency_key: None,
        })
        .map_err(|_| RuntimeError::Im("child completed but reply publication failed"))?;
        Ok(output)
    }

    fn check_schedule(&self) -> Result<()> {
        if let Some(task) = &self.scheduled_task {
            crate::scheduler::guarded(&self.home, task, || Ok(()))?;
        }
        Ok(())
    }
}

fn check_canceled(token: &CancellationToken) -> Result<()> {
    if token.is_canceled() {
        Err(RuntimeError::Stopped(crate::StopCode::token(token)))
    } else {
        Ok(())
    }
}

pub(crate) fn bind_principal(im: &ImStore, agent: &str) -> Result<PrincipalId> {
    let id = PrincipalId::new(format!("runtime:{agent}"));
    let metadata = json!({"agent_ref":agent,"binding":"agt-runtime-v1"});
    match im.create_principal(PrincipalInput {
        id: id.clone(),
        kind: PrincipalKind::Agent,
        display_name: agent.to_owned(),
        metadata: metadata.clone(),
    }) {
        Ok(_) => {}
        Err(agt_im::ImError::AlreadyExists(_)) => {
            let principal = im
                .get_principal(&id)
                .map_err(|_| RuntimeError::Im("cannot load participant"))?;
            if principal.kind != PrincipalKind::Agent || principal.metadata != metadata {
                return Err(RuntimeError::Im(
                    "runtime participant identity is already bound differently",
                ));
            }
        }
        Err(_) => return Err(RuntimeError::Im("cannot bind participant")),
    }
    Ok(id)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Delegation {
    agent: String,
    input: String,
}
