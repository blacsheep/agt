//! Bounded live progress and retained public activity summaries; never shared IM history.
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
    #[serde(default)]
    pub local_checked_at: Option<i64>,
    #[serde(default)]
    pub protocol_at: Option<i64>,
    #[serde(default)]
    pub meaningful_at: Option<i64>,
    #[serde(default)]
    pub activities: Vec<Activity>,
}

pub(crate) fn stage(
    record: &mut RunRecord,
    store: &RunStore,
    task: Option<&ScheduledTask>,
    phase: &str,
) -> Result<()> {
    let now = crate::schedule_store::now_ms();
    let previous = record.progress.take();
    record.progress = Some(RunProgress {
        phase: phase.into(),
        attempt: 0,
        preview: String::new(),
        started_at: previous.as_ref().map(|p| p.started_at).unwrap_or(now),
        updated_at: now,
        task: task.map(|t| t.id.clone()),
        task_version: task.map(|t| t.version),
        local_checked_at: None,
        protocol_at: None,
        meaningful_at: None,
        activities: previous.map(|p| p.activities).unwrap_or_default(),
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Activity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub id: String,
    pub kind: String,
    pub status: String,
    pub started_at: i64,
    pub updated_at: i64,
}

pub(crate) fn harness_event(
    record: &mut RunRecord,
    store: &RunStore,
    event: &agt_harness::Event<'_>,
) -> Result<()> {
    use agt_harness::Event;
    let now = crate::schedule_store::now_ms();
    let progress = record.progress.as_mut().expect("stage initialized");
    let mut boundary = false;
    match event {
        Event::Tick => {
            progress.local_checked_at = Some(now);
        }
        Event::Protocol => {
            progress.protocol_at = Some(now);
        }
        Event::Text(_) => {
            progress.meaningful_at = Some(now);
        }
        Event::Activity {
            id,
            kind,
            status,
            tool_name,
        } => {
            progress.meaningful_at = Some(now);
            progress.phase = if *status == "running" {
                "tool_running"
            } else if *kind == "input" {
                "input_required"
            } else {
                "waiting"
            }
            .into();
            let id = crate::fingerprint(id.as_bytes());
            if let Some(activity) = progress.activities.iter_mut().find(|a| a.id == id) {
                activity.status = (*status).into();
                activity.updated_at = now;
            } else {
                progress.activities.push(Activity {
                    tool_name: tool_name
                        .filter(|n| {
                            n.len() <= 128
                                && n.chars()
                                    .all(|c| c.is_ascii_alphanumeric() || "_./:-".contains(c))
                        })
                        .map(str::to_owned),
                    label: None,
                    summary: None,
                    id,
                    kind: (*kind).into(),
                    status: (*status).into(),
                    started_at: now,
                    updated_at: now,
                });
                if progress.activities.len() > 40 {
                    progress.activities.remove(0);
                }
            }
            boundary = true;
        }
        Event::Turn(_) => {
            progress.meaningful_at = Some(now);
            progress.phase = "waiting".into();
            boundary = true;
        }
        _ => {}
    }
    // Tick is only local liveness. It never updates protocol/progress timestamps.
    if boundary || now - progress.updated_at >= 1000 {
        progress.updated_at = now;
        store.save(record)?;
    }
    Ok(())
}

/// Retain only public activity facts. Draft text and model reasoning are not execution history.
pub(crate) fn finish(record: &mut RunRecord) {
    let Some(mut progress) = record.progress.take() else {
        return;
    };
    let ended_at = crate::schedule_store::now_ms();
    for activity in &mut progress.activities {
        if activity.status == "running" {
            // Ending a Run is not evidence that an external operation succeeded or was canceled.
            activity.status = "unknown".into();
        }
    }
    let execution = record
        .execution
        .get_or_insert_with(|| serde_json::json!({"engine":"native"}));
    execution["activity"] = serde_json::json!({
        "items": progress.activities,
        "started_at": progress.started_at,
        "updated_at": progress.updated_at,
        "ended_at": ended_at,
        "local_checked_at": progress.local_checked_at,
        "protocol_at": progress.protocol_at,
        "meaningful_at": progress.meaningful_at,
    });
}
