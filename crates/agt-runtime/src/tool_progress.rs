//! Public progress is a bounded projection of the durable tool audit, not a second lifecycle.
use crate::{CallState, Result, RunRecord, RunStore};
use serde_json::json;
impl RunStore {
    pub(crate) fn merge_tool_progress(&self, record: &mut RunRecord) -> Result<()> {
        let calls = self.recent_tool_calls(&record.id)?;
        if calls.is_empty() {
            return Ok(());
        }
        let tools: Vec<_> = calls
            .into_iter()
            .map(|c| {
                let name = c.tool_name.unwrap_or_default();
                let framework = name.starts_with("framework_")
                    || c.public_label.is_some()
                    || c.kind == "framework";
                let tool_name = (name.len() <= 128
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_./:-".contains(c)))
                .then_some(name);
                crate::progress::Activity {
                    id: format!("call:{}", c.id),
                    kind: if framework { "framework" } else { "mcp" }.into(),
                    status: match c.state {
                        CallState::Started if record.state == crate::RunState::Running => "running",
                        CallState::Started => "unknown",
                        CallState::Succeeded => "completed",
                        CallState::Failed => "failed",
                        CallState::Canceled => "canceled",
                    }
                    .into(),
                    tool_name,
                    label: c.public_label,
                    summary: c.public_summary,
                    started_at: c.started_at as i64,
                    updated_at: c.finished_at.unwrap_or(c.started_at) as i64,
                }
            })
            .collect();
        if let Some(p) = &mut record.progress {
            p.activities
                .retain(|a| !a.id.starts_with("call:") && a.kind != "framework");
            p.activities.extend(tools);
            p.activities.sort_by_key(|a| a.started_at);
            if p.activities.len() > 40 {
                p.activities.drain(..p.activities.len() - 40);
            }
            p.updated_at = crate::schedule_store::now_ms();
        } else {
            let execution = record
                .execution
                .get_or_insert_with(|| json!({"engine":"native"}));
            let mut entries = execution["activity"]["items"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            entries.retain(|a| {
                a["kind"] != "framework" && !a["id"].as_str().unwrap_or("").starts_with("call:")
            });
            entries.extend(tools.into_iter().map(|a| serde_json::to_value(a).unwrap()));
            if entries.len() > 40 {
                entries.drain(..entries.len() - 40);
            }
            execution["activity"]["items"] = json!(entries);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
