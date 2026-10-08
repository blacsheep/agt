//! Private, bounded tool payloads. Never part of Run snapshots, audit summaries or IM history.
use crate::{Result, RunStore};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

const FIELD_BYTES: usize = 64 * 1024;

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct ToolDetail {
    pub fields: BTreeMap<String, ToolDetailField>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolDetailField {
    pub text: String,
    pub truncated: bool,
}
impl RunStore {
    /// Host-only payload; callers exposing this remotely must check execution scope.
    pub fn tool_detail(&self, run: &str, activity: &str) -> Result<Option<ToolDetail>> {
        self.get(run)?;
        let json: Option<String> = self
            .connection
            .query_row(
                "SELECT detail_json FROM tool_details WHERE run_id=?1 AND activity_id=?2",
                params![run, activity],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|text| serde_json::from_str(&text).map_err(Into::into))
            .transpose()
    }
    pub(crate) fn save_tool_detail(
        &self,
        run: &str,
        activity: &str,
        fields: &[(&str, &Value)],
    ) -> Result<()> {
        let mut detail = self.tool_detail(run, activity)?.unwrap_or_default();
        for (name, value) in fields {
            let mut text = match value {
                Value::String(text) => text.clone(),
                value => serde_json::to_string_pretty(value)?,
            };
            let truncated = text.len() > FIELD_BYTES;
            if truncated {
                let mut end = FIELD_BYTES;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                text.truncate(end);
            }
            detail
                .fields
                .insert((*name).into(), ToolDetailField { text, truncated });
        }
        self.connection.execute(
            "INSERT INTO tool_details VALUES(?1,?2,?3) ON CONFLICT(run_id,activity_id) DO UPDATE SET detail_json=excluded.detail_json",
            params![run, activity, serde_json::to_string(&detail)?],
        )?;
        Ok(())
    }
    pub(crate) fn save_harness_detail(&self, run: &str, id: &str, item: &Value) -> Result<()> {
        // Engine payloads are not copied wholesale: exclude reasoning and protocol internals.
        let fields = [
            "command",
            "cwd",
            "arguments",
            "aggregatedOutput",
            "changes",
            "result",
            "error",
            "exitCode",
        ]
        .iter()
        .filter_map(|key| {
            item.get(*key)
                .filter(|v| !v.is_null())
                .map(|value| (*key, value))
        })
        .collect::<Vec<_>>();
        self.save_tool_detail(run, &crate::fingerprint(id.as_bytes()), &fields)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn details_merge_survive_reopen_and_stay_out_of_snapshots() {
        let home = tempfile::tempdir().unwrap();
        let mut store = RunStore::open(home.path()).unwrap();
        let (run, _) = store
            .claim(&crate::RunRequest::new("test", "input"), None)
            .unwrap();
        store
            .save_harness_detail(
                &run.id,
                "command",
                &json!({"command":"echo private-command", "reasoning":"do not record"}),
            )
            .unwrap();
        store
            .save_harness_detail(
                &run.id,
                "command",
                &json!({"command":null,"aggregatedOutput":"private-output", "exitCode":0}),
            )
            .unwrap();
        store
            .save_harness_detail(
                &run.id,
                "file",
                &json!({"changes":[{"path":"src/a.rs","diff":"-old\n+new"}]}),
            )
            .unwrap();
        store
            .save_tool_detail(
                &run.id,
                "call:1",
                &[("output", &json!("中".repeat(30_000)))],
            )
            .unwrap();
        assert!(
            !serde_json::to_string(&store.get(&run.id).unwrap())
                .unwrap()
                .contains("private")
        );
        assert!(store.calls(&run.id, 0, 100).unwrap().is_empty());
        drop(store);
        let store = RunStore::open(home.path()).unwrap();
        let detail = store
            .tool_detail(&run.id, &crate::fingerprint(b"command"))
            .unwrap()
            .unwrap();
        assert_eq!(detail.fields["command"].text, "echo private-command");
        assert_eq!(detail.fields["aggregatedOutput"].text, "private-output");
        assert_eq!(detail.fields["exitCode"].text, "0");
        assert!(!detail.fields.contains_key("reasoning"));
        assert!(
            store
                .tool_detail(&run.id, &crate::fingerprint(b"file"))
                .unwrap()
                .unwrap()
                .fields["changes"]
                .text
                .contains("+new")
        );
        let bounded = store.tool_detail(&run.id, "call:1").unwrap().unwrap();
        assert!(bounded.fields["output"].truncated);
        assert!(bounded.fields["output"].text.len() <= FIELD_BYTES);
        assert!(store.tool_detail(&run.id, "legacy").unwrap().is_none());
    }
}
