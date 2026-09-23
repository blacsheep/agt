//! One native execution, isolated from scheduler admission and Run ownership.
//! The outer loop drains follow-ups; the inner loop advances tools and steering.
//! Scheduler tasks remain the durable queue and authorization boundary.
use super::{Runtime, check_canceled};
use crate::{ResolvedAgent, Result, RunRecord, RunState, RuntimeError};
use agt_core::CancellationToken;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

mod framework;
mod inputs;
mod mcp;
mod prepare;
mod request;
mod tools;
use mcp::ToolSession;

pub(super) enum FinalReply {
    Text(String),
    Silent,
}

impl FinalReply {
    // Keep the existing publication protocol at the host boundary.
    pub(super) fn into_output(self) -> String {
        match self {
            Self::Text(text) => text,
            Self::Silent => "[[silent]]".into(),
        }
    }
}

pub(super) enum LoopOutcome {
    Complete(FinalReply),
    Suspended,
}

enum ModelTurn {
    Final {
        message: Value,
        reported_tokens: Option<usize>,
    },
    Tools {
        message: Value,
        calls: Vec<ToolCall>,
        reported_tokens: Option<usize>,
    },
}

enum ToolFlow {
    Continue,
    Suspend,
}

pub(super) struct NativeRun<'run, 'host> {
    runtime: &'run mut Runtime<'host>,
    record: &'run mut RunRecord,
    agents: &'run BTreeMap<String, ResolvedAgent>,
    agent: &'run ResolvedAgent,
    step_count: &'run mut u32,
    cancellation: &'run CancellationToken,
    tools: ToolSession,
    communication: Option<agt_service::ToolService>,
    communication_names: BTreeSet<String>,
    messages: Vec<Value>,
    input_messages: Vec<Value>,
    completed_turns: usize,
    reply: Option<(agt_im::MessageId, u32)>,
    usage: crate::context::ContextUsage,
    seen_calls: BTreeSet<String>,
    published: bool,
    sent_messages: BTreeMap<String, String>,
    initial_steps: u32,
    model_steps: Option<u32>,
}

impl NativeRun<'_, '_> {
    pub(super) fn run(mut self) -> Result<LoopOutcome> {
        use agt_service::InputMode;
        loop {
            // Steering may also arrive during follow-up admission.
            if let Some(input) = self.next_input(InputMode::Steering)? {
                self.accept_input(input);
            }
            let output = loop {
                match self.request_turn()? {
                    ModelTurn::Final {
                        message,
                        reported_tokens,
                    } => {
                        let output = message["content"]
                            .as_str()
                            .expect("validated final text")
                            .to_owned();
                        self.messages.push(message);
                        self.usage.observe(reported_tokens, self.messages.len());
                        if let Some(input) = self.next_input(InputMode::Steering)? {
                            self.accept_input(input);
                            continue;
                        }
                        break output;
                    }
                    ModelTurn::Tools {
                        message,
                        calls,
                        reported_tokens,
                    } => {
                        self.messages.push(message);
                        self.usage.observe(reported_tokens, self.messages.len());
                        if let ToolFlow::Suspend = self.execute_tool_batch(calls)? {
                            self.suspend()?;
                            return Ok(LoopOutcome::Suspended);
                        }
                        if let Some(input) = self.next_input(InputMode::Steering)? {
                            self.accept_input(input);
                        }
                    }
                }
            };
            if let Some(input) = self.next_input(InputMode::FollowUp)? {
                self.publish_completed_turn(output, &input.task.id)?;
                self.accept_input(input);
            } else {
                return Ok(LoopOutcome::Complete(self.finish_turn(output)));
            }
        }
    }

    fn finish_turn(&self, output: String) -> FinalReply {
        if self.published || output == "[[silent]]" {
            FinalReply::Silent
        } else {
            FinalReply::Text(output)
        }
    }

    fn suspend(self) -> Result<()> {
        check_canceled(self.cancellation)?;
        self.runtime.check_schedule()?;
        self.record.state = RunState::Suspended;
        self.record.progress = None;
        self.runtime.store.save_checkpoint(
            self.record,
            &crate::work::wait::Checkpoint::Native {
                messages: self.messages,
                seen_calls: self.seen_calls,
                published: self.published,
                sent_messages: self.sent_messages,
                config_fingerprint: self.record.config_fingerprint.clone(),
            },
        )
    }
}

#[derive(Deserialize)]
struct ToolCall {
    id: String,
    function: FunctionCall,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct FunctionCall {
    name: String,
    arguments: String,
}
