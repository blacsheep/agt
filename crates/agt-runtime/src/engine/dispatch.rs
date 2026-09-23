//! Engine selection after Run claim; protocol-specific execution remains isolated.
use super::{Runtime, check_canceled};
use crate::{ResolvedAgent, Result, RunRecord, RunRequest, RuntimeError};
use agt_core::CancellationToken;
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EngineKind {
    Native,
    Harness,
}

impl EngineKind {
    pub(crate) fn resolve(home: &Path, agent: &str) -> Result<Self> {
        Ok(
            if crate::harness::agent_config(home, agent)?.harness.is_some() {
                Self::Harness
            } else {
                Self::Native
            },
        )
    }
}

impl Runtime<'_> {
    pub(super) fn execute_claimed(
        &mut self,
        request: &RunRequest,
        record: &mut RunRecord,
        step_count: &mut u32,
        cancellation: &CancellationToken,
    ) -> Result<String> {
        check_canceled(cancellation)?;
        if EngineKind::resolve(&self.home, &request.agent)? == EngineKind::Harness {
            if let Some(task) = &self.scheduled_task
                && !task.work_notification
                && let Some(id) = &task.work_id
                && crate::work::WorkLedger::open(&self.home)
                    .and_then(|l| l.get(id))
                    .map_err(|_| RuntimeError::Configuration("cannot read continuation"))?
                    .resume_from
                    .is_some()
            {
                let item = crate::work::WorkLedger::open(&self.home)
                    .and_then(|l| l.get(id))
                    .map_err(|_| RuntimeError::Configuration("cannot read continuation"))?;
                if !matches!(
                    self.store
                        .checkpoint(item.resume_from.as_deref().unwrap())?,
                    crate::work::wait::Checkpoint::Codex { .. }
                ) {
                    return Err(RuntimeError::Configuration(
                        "native continuation cannot switch to a harness",
                    ));
                }
            }
            return crate::harness::execute(
                &self.home,
                record,
                &mut self.store,
                cancellation,
                self.scheduled_task.as_ref(),
                self.discussion.as_deref(),
            );
        }
        let agents = std::iter::once(&request.agent)
            .chain(&request.delegates)
            .map(|name| Ok((name.clone(), ResolvedAgent::load(&self.home, name)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        self.drive(record, &agents, step_count, cancellation)
    }
}
