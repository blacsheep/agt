//! One bounded snapshot per Run, cleared on completion; never shared IM history.
use crate::{ModelEvent, Result, RunRecord, RunStore, ScheduledTask};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunProgress {
    pub phase: String,
    pub attempt: u32,
    pub preview: String,
    pub started_at: i64,
    pub updated_at: i64,
    pub task: Option<String>,
    pub task_version: Option<i64>,
}

pub(crate) fn stage(
    record: &mut RunRecord,
    store: &RunStore,
    task: Option<&ScheduledTask>,
    phase: &str,
) -> Result<()> {
    let now = crate::schedule_store::now_ms();
    record.progress = Some(RunProgress {
        phase: phase.into(),
        attempt: 0,
        preview: String::new(),
        started_at: now,
        updated_at: now,
        task: task.map(|t| t.id.clone()),
        task_version: task.map(|t| t.version),
    });
    store.save(record)
}

pub(crate) fn update(
    record: &mut RunRecord,
    store: &RunStore,
    event: ModelEvent<'_>,
) -> Result<()> {
    let progress = record.progress.as_mut().expect("stage initialized");
    let old_phase = progress.phase.clone();
    let force = matches!(event, ModelEvent::Attempt(_) | ModelEvent::Retrying);
    match event {
        ModelEvent::Attempt(attempt) => {
            progress.attempt = attempt;
            progress.preview.clear();
            progress.phase = "waiting".into();
        }
        ModelEvent::Retrying => {
            progress.preview.clear();
            progress.phase = "retrying".into();
        }
        ModelEvent::Text(text) => {
            progress.phase = "generating".into();
            progress.preview.push_str(text);
            // Keep the latest 4 KiB at a UTF-8 boundary.
            if progress.preview.len() > 4096 {
                let mut cut = progress.preview.len() - 4096;
                while !progress.preview.is_char_boundary(cut) {
                    cut += 1;
                }
                progress.preview.drain(..cut);
            }
        }
        ModelEvent::Reasoning => {
            progress.phase = "reasoning".into();
        }
        ModelEvent::Tool => {
            progress.phase = "tool_pending".into();
        }
    }
    let now = crate::schedule_store::now_ms();
    if force || old_phase != progress.phase || now - progress.updated_at >= 300 {
        progress.updated_at = now;
        store.save(record)?;
    }
    Ok(())
}
