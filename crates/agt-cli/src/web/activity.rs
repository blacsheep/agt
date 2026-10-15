//! Public projection of runs for the page. Raw requests, output, paths and
//! approval payloads never leave this module; raw tool details are fetched
//! separately and only by the run's own identity or the operator.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use super::cli::{Ops, is_member, strings};
use super::error::ApiError;
use super::js;

const KINDS: [&str; 5] = ["command", "files", "mcp", "framework", "input"];
const STATUSES: [&str; 6] = [
    "running",
    "completed",
    "failed",
    "canceled",
    "unsupported",
    "unknown",
];

fn safe_name(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|s| {
        (1..=128).contains(&s.len())
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.:/-".contains(&b))
    })
}

/// Audit IDs (`call:N`) or hashed engine IDs; anything else is replaced.
fn stable_id(value: &str) -> bool {
    value
        .strip_prefix("call:")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        || (value.len() == 64
            && value
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

fn finite(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(n)) => Value::Number(n.clone()),
        _ => Value::Null,
    }
}

/// `a ?? b`
fn or<'a>(a: Option<&'a Value>, b: Option<&'a Value>) -> Option<&'a Value> {
    if js::nullish(a) { b } else { a }
}

fn copy(target: &mut Map<String, Value>, key: &str, value: Option<&Value>) {
    if let Some(value) = value {
        target.insert(key.to_owned(), value.clone());
    }
}

pub(crate) fn public_activities(items: Option<&Value>, settled: bool) -> Value {
    let list = items
        .and_then(Value::as_array)
        .map_or(&[][..], |a| &a[a.len().saturating_sub(40)..]);
    let mut order: Vec<String> = Vec::new();
    let mut entries: HashMap<String, Value> = HashMap::new();
    for (index, a) in list.iter().enumerate() {
        let Some(a) = a.as_object() else { continue };
        let (Some(kind), Some(status)) = (
            a.get("kind")
                .and_then(Value::as_str)
                .filter(|k| KINDS.contains(k)),
            a.get("status")
                .and_then(Value::as_str)
                .filter(|s| STATUSES.contains(s)),
        ) else {
            continue;
        };
        // Legacy snapshots without a stable ID get a local fallback.
        let id = match a.get("id").and_then(Value::as_str) {
            Some(id) if stable_id(id) => id.to_owned(),
            _ => {
                let started = finite(a.get("started_at"));
                let started = if js::truthy(Some(&started)) {
                    js::number_text(&started)
                } else {
                    "0".to_owned()
                };
                format!("legacy:{index}:{started}")
            }
        };
        let mut entry = Map::new();
        entry.insert("id".into(), json!(id));
        entry.insert("kind".into(), json!(kind));
        let status = if settled && status == "running" {
            "unknown"
        } else {
            status
        };
        entry.insert("status".into(), json!(status));
        entry.insert("started_at".into(), finite(a.get("started_at")));
        entry.insert("updated_at".into(), finite(a.get("updated_at")));
        if let Some(name) = safe_name(a.get("tool_name")) {
            entry.insert("tool_name".into(), json!(name));
        }
        if let Some(label) = a.get("label").and_then(Value::as_str) {
            entry.insert("label".into(), json!(js::prefix(label, 80)));
        }
        if let Some(summary) = a.get("summary").and_then(Value::as_str) {
            entry.insert("summary".into(), json!(js::prefix(summary, 120)));
        }
        // A repeated ID keeps its first position with the latest content.
        if !entries.contains_key(&id) {
            order.push(id.clone());
        }
        entries.insert(id, Value::Object(entry));
    }
    Value::Array(
        order
            .into_iter()
            .filter_map(|id| entries.remove(&id))
            .collect(),
    )
}

/// `id === conversation`, where an absent query parameter is `null`.
fn same_conversation(id: Option<&Value>, conversation: Option<&str>) -> bool {
    match conversation {
        None => matches!(id, Some(Value::Null)),
        Some(c) => js::is(id, c),
    }
}

/// `trigger.conversation_id === conversation`
fn in_conversation(trigger: Option<&Value>, conversation: Option<&str>) -> bool {
    same_conversation(trigger.and_then(|t| t.get("conversation_id")), conversation)
}

/// The live draft of a run, only while its scheduler task is still the one
/// replying in an active or paused round of the selected conversation.
fn live_progress(
    record: &Value,
    scheduler: &Value,
    visible: bool,
    conversation: Option<&str>,
) -> Value {
    let p = record.get("progress");
    let trigger = record.get("request").and_then(|r| r.get("trigger"));
    if !visible
        || !js::truthy(scheduler.get("online"))
        || !js::truthy(p)
        || !js::is(record.get("state"), "running")
        || js::truthy(record.get("parent_run_id"))
        || !in_conversation(trigger, conversation)
    {
        return Value::Null;
    }
    let p = p.expect("truthy progress");
    let conversation = conversation.map(|c| json!(c)).unwrap_or(Value::Null);
    let tasks = scheduler
        .get("tasks")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let Some(task) = tasks.iter().find(|t| {
        js::same(t.get("id"), p.get("task"))
            && js::same(t.get("version"), p.get("task_version"))
            && js::is(t.get("state"), "responding")
            && js::same(t.get("principal"), trigger.and_then(|t| t.get("principal")))
            && js::same(t.get("conversation"), Some(&conversation))
    }) else {
        return Value::Null;
    };
    let discussions = scheduler
        .get("discussions")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let round = discussions
        .iter()
        .find(|d| js::same(d.get("id"), task.get("discussion")));
    if !round.is_some_and(|d| js::is(d.get("state"), "active") || js::is(d.get("state"), "paused"))
    {
        return Value::Null;
    }
    let mut live = Map::new();
    copy(&mut live, "phase", p.get("phase"));
    copy(&mut live, "attempt", p.get("attempt"));
    let preview = p
        .get("preview")
        .and_then(Value::as_str)
        .map_or("", |s| js::suffix(s, 4096));
    live.insert("preview".into(), json!(preview));
    copy(&mut live, "started_at", p.get("started_at"));
    copy(&mut live, "updated_at", p.get("updated_at"));
    copy(&mut live, "principal", task.get("principal"));
    if matches!(p.get("protocol_at"), Some(Value::Number(_))) {
        for key in ["protocol_at", "local_checked_at", "meaningful_at"] {
            copy(&mut live, key, p.get(key));
        }
    }
    live.insert(
        "activities".into(),
        public_activities(p.get("activities"), false),
    );
    Value::Object(live)
}

pub(crate) async fn snapshot(
    ops: Ops<'_>,
    conversation: Option<&str>,
    viewer: &str,
) -> Result<Value, ApiError> {
    let (records, scheduler) = tokio::try_join!(
        ops.run(strings(&["run", "list", "--limit", "50"])),
        ops.run(strings(&["schedule", "status"])),
    )?;
    let members = match conversation {
        Some(c) if !c.is_empty() => ops.members(c, viewer).await.unwrap_or_else(|_| json!([])),
        _ => json!([]),
    };
    let visible = is_member(&members, viewer);
    let records = records.as_array().ok_or_else(ApiError::internal)?;
    let mut runs: Vec<Value> = records
        .iter()
        .map(|r| {
            let request = r.get("request");
            let trigger = request.and_then(|q| q.get("trigger"));
            let progress = live_progress(r, &scheduler, visible, conversation);
            let scoped = visible
                && !js::truthy(r.get("parent_run_id"))
                && in_conversation(trigger, conversation);
            let saved = r.get("execution").and_then(|e| e.get("activity"));
            let items = if progress.is_null() {
                if scoped && !js::is(r.get("state"), "running") {
                    public_activities(saved.and_then(|s| s.get("items")), true)
                } else {
                    json!([])
                }
            } else {
                progress["activities"].clone()
            };
            let mut row = Map::new();
            copy(&mut row, "id", r.get("id"));
            copy(&mut row, "agent", request.and_then(|q| q.get("agent")));
            copy(
                &mut row,
                "conversation",
                trigger.and_then(|t| t.get("conversation_id")),
            );
            copy(&mut row, "state", r.get("state"));
            copy(&mut row, "model_steps", r.get("model_steps"));
            copy(&mut row, "tool_failures", r.get("tool_failures"));
            let saved_at = |key| saved.and_then(|s| s.get(key));
            row.insert(
                "started_at".into(),
                finite(or(
                    r.get("progress").and_then(|p| p.get("started_at")),
                    saved_at("started_at"),
                )),
            );
            row.insert("ended_at".into(), finite(saved_at("ended_at")));
            row.insert(
                "error".into(),
                if js::truthy(r.get("error")) {
                    json!("执行失败，请查看消息投递了解原因。")
                } else {
                    Value::Null
                },
            );
            let live = (!progress.is_null()).then_some(&progress);
            let has_items = items.as_array().is_some_and(|a| !a.is_empty());
            let activity = if scoped && (live.is_some() || has_items) {
                let mut a = Map::new();
                copy(
                    &mut a,
                    "principal",
                    trigger.and_then(|t| t.get("principal")),
                );
                let codex = js::is(r.get("execution").and_then(|e| e.get("engine")), "codex");
                a.insert(
                    "engine".into(),
                    json!(if codex { "codex" } else { "native" }),
                );
                a.insert("items".into(), items);
                let live_at = |key| live.and_then(|p| p.get(key));
                a.insert(
                    "started_at".into(),
                    finite(or(live_at("started_at"), saved_at("started_at"))),
                );
                a.insert(
                    "updated_at".into(),
                    finite(or(live_at("updated_at"), saved_at("updated_at"))),
                );
                a.insert("ended_at".into(), finite(saved_at("ended_at")));
                a.insert("approvals".into(), json!([]));
                Value::Object(a)
            } else {
                Value::Null
            };
            row.insert("progress".into(), progress);
            row.insert("tool_activity".into(), activity);
            Value::Object(row)
        })
        .collect();
    // Only ask for approvals while a visible execution is live. The approval
    // store rechecks scope.
    if runs.iter().any(|r| !r["progress"].is_null()) {
        let approvals = ops
            .run(strings(&["permission", "list", "--principal", viewer]))
            .await
            .ok()
            .and_then(|a| a.as_array().cloned());
        for row in runs.iter_mut().filter(|r| !r["progress"].is_null()) {
            let id = row.get("id").cloned();
            let activity = &mut row["tool_activity"];
            match &approvals {
                Some(list) => {
                    activity["approvals"] = list
                        .iter()
                        .filter(|a| {
                            js::same(a.get("run_id"), id.as_ref())
                                && same_conversation(a.get("conversation"), conversation)
                                && js::is(a.get("delivery"), "not_sent")
                                && (js::is(a.get("state"), "reviewing") || js::is(a.get("state"), "awaiting_human"))
                        })
                        .map(|a| json!({ "id": a.get("id"), "source": a.get("source"), "state": a.get("state") }))
                        .collect();
                }
                None => activity["approvals_unavailable"] = json!(true),
            }
        }
    }
    Ok(json!({ "scheduler": scheduler, "runs": runs }))
}

/// Settled root runs of one conversation, which the page anchors to messages.
/// Same visibility rule as live activity.
pub(crate) async fn conversation_runs(
    ops: Ops<'_>,
    conversation: Option<&str>,
    viewer: &str,
) -> Result<Value, ApiError> {
    let Some(conversation) = conversation.filter(|c| !c.is_empty()) else {
        return Ok(json!({ "runs": [] }));
    };
    let members = ops
        .members(conversation, viewer)
        .await
        .unwrap_or_else(|_| json!([]));
    if !is_member(&members, viewer) {
        return Ok(json!({ "runs": [] }));
    }
    let records = ops
        .run(strings(&[
            "run",
            "list",
            "--conversation",
            conversation,
            "--limit",
            "200",
        ]))
        .await?;
    let records = records.as_array().ok_or_else(ApiError::internal)?;
    let runs: Vec<Value> = records
        .iter()
        .filter(|r| !js::is(r.get("state"), "running"))
        .map(|r| {
            let request = r.get("request");
            let trigger = request.and_then(|q| q.get("trigger"));
            let saved = r.get("execution").and_then(|e| e.get("activity"));
            let saved_at = |key| saved.and_then(|s| s.get(key));
            let output = r.get("output").and_then(Value::as_str).unwrap_or("");
            let codex = js::is(r.get("execution").and_then(|e| e.get("engine")), "codex");
            let mut row = Map::new();
            copy(&mut row, "id", r.get("id"));
            copy(&mut row, "agent", request.and_then(|q| q.get("agent")));
            copy(
                &mut row,
                "principal",
                trigger.and_then(|t| t.get("principal")),
            );
            copy(
                &mut row,
                "trigger_message",
                trigger.and_then(|t| t.get("message_id")),
            );
            copy(&mut row, "state", r.get("state"));
            row.insert("silent".into(), json!(output.trim() == "[[silent]]"));
            copy(&mut row, "tool_failures", r.get("tool_failures"));
            row.insert(
                "engine".into(),
                json!(if codex { "codex" } else { "native" }),
            );
            row.insert(
                "started_at".into(),
                finite(or(
                    saved_at("started_at"),
                    r.get("progress").and_then(|p| p.get("started_at")),
                )),
            );
            row.insert("ended_at".into(), finite(saved_at("ended_at")));
            row.insert("items".into(), public_activities(saved_at("items"), true));
            Value::Object(row)
        })
        .collect();
    Ok(json!({ "runs": runs }))
}

/// Raw tool payloads, only for the run's own identity or the operator, inside
/// the run's conversation while still a member. `None` means denied.
pub(crate) async fn tool_detail(
    ops: Ops<'_>,
    viewer: &str,
    conversation: &str,
    id: &str,
    activity: &str,
) -> Result<Option<Value>, ApiError> {
    let record = ops.run(strings(&["run", "info", id])).await?;
    let trigger = record.get("request").and_then(|r| r.get("trigger"));
    if js::truthy(record.get("parent_run_id"))
        || !in_conversation(trigger, Some(conversation))
        || conversation.is_empty()
        || (viewer != ops.operator && !js::is(trigger.and_then(|t| t.get("principal")), viewer))
    {
        return Ok(None);
    }
    let members = ops
        .members(conversation, viewer)
        .await
        .unwrap_or_else(|_| json!([]));
    if !is_member(&members, viewer) {
        return Ok(None);
    }
    ops.run(strings(&["run", "tool-detail", id, "--activity", activity]))
        .await
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::cli::{Backend, Reply};
    use std::sync::Mutex;

    type Handler = Box<dyn Fn(&[String]) -> Value + Send + Sync>;
    struct Fake {
        handler: Handler,
        seen: Mutex<Vec<Vec<String>>>,
    }
    impl Backend for Fake {
        fn call(&self, args: Vec<String>, _: Vec<(String, String)>) -> Reply<'_> {
            let value = (self.handler)(&args);
            self.seen.lock().unwrap().push(args);
            Box::pin(async move { Ok(value) })
        }
    }

    struct Fixture {
        record: Value,
        scheduler: Value,
        approvals: Value,
        allowed: bool,
    }

    impl Fixture {
        fn new() -> Self {
            let items = json!([{"id":"call:7","kind":"framework","tool_name":"framework_resource_list","label":"查看资源","summary":"找到 3 项资源","status":"running","started_at":10,"updated_at":20,"arguments":"SECRET","output":"SECRET"}]);
            Self {
                scheduler: json!({"online":true,"tasks":[{"id":"task","version":2,"state":"responding","principal":"agent","conversation":"chat","discussion":"round"}],"discussions":[{"id":"round","state":"active"}]}),
                record: json!({"id":"run","request":{"agent":"default/agent","trigger":{"conversation_id":"chat","principal":"agent"},"input":"SECRET"},"state":"running","progress":{"task":"task","task_version":2,"principal":"agent","phase":"tool_running","started_at":10,"updated_at":20,"preview":"draft","activities":items},"execution":{"engine":"codex","activity":{"items":items,"started_at":10,"ended_at":30}},"output":"SECRET"}),
                approvals: json!([{"id":"approval","run_id":"run","conversation":"chat","source":"native","state":"awaiting_human","delivery":"not_sent","request":{"arguments":"SECRET"},"reason":"SECRET"}]),
                allowed: true,
            }
        }

        fn backend(&self) -> Fake {
            let (record, scheduler, approvals, allowed) = (
                self.record.clone(),
                self.scheduler.clone(),
                self.approvals.clone(),
                self.allowed,
            );
            Fake {
                handler: Box::new(move |args| match args[0].as_str() {
                    "run" => json!([record]),
                    "schedule" => scheduler.clone(),
                    "permission" => approvals.clone(),
                    _ if allowed => json!([{"principal_id": args[3], "left_message_seq": null}]),
                    _ => json!([]),
                }),
                seen: Mutex::new(Vec::new()),
            }
        }

        async fn read(&self, conversation: Option<&str>) -> Value {
            let backend = self.backend();
            let ops = Ops {
                backend: &backend,
                operator: "operator",
            };
            snapshot(ops, conversation, "operator").await.unwrap()["runs"][0].clone()
        }
    }

    #[tokio::test]
    async fn stable_tool_identities_and_safe_summaries_survive_completion() {
        let mut f = Fixture::new();
        let live = f.read(Some("chat")).await;
        assert_eq!(live["progress"]["activities"][0]["id"], "call:7");
        assert_eq!(
            live["tool_activity"]["approvals"],
            json!([{"id":"approval","source":"native","state":"awaiting_human"}])
        );
        let text = live.to_string();
        assert!(
            !text.contains("SECRET") && !text.contains("arguments") && !text.contains("output")
        );
        f.record["state"] = json!("completed");
        f.record["progress"] = Value::Null;
        f.record["execution"]["activity"]["items"][0]["status"] = json!("completed");
        f.scheduler["online"] = json!(false);
        f.scheduler["discussions"][0]["state"] = json!("ended");
        let settled = f.read(Some("chat")).await;
        assert!(settled["progress"].is_null());
        assert_eq!(settled["tool_activity"]["items"][0]["id"], "call:7");
        assert_eq!(settled["tool_activity"]["items"][0]["status"], "completed");
        assert_eq!(settled["tool_activity"]["ended_at"], 30);
        assert!(!settled.to_string().contains("draft") && !settled.to_string().contains("SECRET"));
        assert_eq!(
            f.read(Some("chat")).await["tool_activity"],
            settled["tool_activity"]
        );
    }

    #[tokio::test]
    async fn finished_work_keeps_membership_conversation_and_root_scope() {
        let mut f = Fixture::new();
        f.record["state"] = json!("failed");
        f.record["progress"] = Value::Null;
        assert_eq!(
            f.read(Some("chat")).await["tool_activity"]["items"][0]["status"],
            "unknown"
        );
        assert!(f.read(Some("another")).await["tool_activity"].is_null());
        assert!(f.read(None).await["tool_activity"].is_null());
        f.record["parent_run_id"] = json!("parent");
        assert!(f.read(Some("chat")).await["tool_activity"].is_null());
        f.record["parent_run_id"] = Value::Null;
        f.allowed = false;
        assert!(f.read(Some("chat")).await["tool_activity"].is_null());
    }

    #[tokio::test]
    async fn stale_running_snapshots_are_not_history_or_current_approval() {
        let mut f = Fixture::new();
        f.scheduler["online"] = json!(false);
        let row = f.read(Some("chat")).await;
        assert!(row["progress"].is_null() && row["tool_activity"].is_null());
        f.scheduler["online"] = json!(true);
        f.record["progress"]["task_version"] = json!(1);
        let row = f.read(Some("chat")).await;
        assert!(row["progress"].is_null() && row["tool_activity"].is_null());
        f.record["progress"]["task_version"] = json!(2);
        f.scheduler["discussions"][0]["state"] = json!("ended");
        let row = f.read(Some("chat")).await;
        assert!(row["progress"].is_null() && row["tool_activity"].is_null());
        f.scheduler["discussions"][0]["state"] = json!("paused");
        assert_eq!(
            f.read(Some("chat")).await["tool_activity"]["items"][0]["id"],
            "call:7"
        );
        f.approvals[0]["conversation"] = json!("private");
        assert_eq!(
            f.read(Some("chat")).await["tool_activity"]["approvals"],
            json!([])
        );
    }

    #[test]
    fn projection_bounds_history_deduplicates_and_strips_unknown_fields() {
        let value = public_activities(
            Some(&json!([
                {"id":"SECRET command path","kind":"command","status":"running","started_at":1,"updated_at":2,"command":"SECRET"},
                {"id":"call:2","kind":"mcp","tool_name":"search","status":"running"},
                {"id":"call:2","kind":"mcp","tool_name":"search","status":"failed"},
                {"id":"call:3","kind":"private","status":"completed"}
            ])),
            true,
        );
        let list = value.as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["status"], "unknown");
        assert_eq!(list[0]["id"], "legacy:0:1");
        assert_eq!(list[1]["status"], "failed");
        assert!(!value.to_string().contains("SECRET") && !value.to_string().contains("private"));
        let many: Vec<Value> = (0..45)
            .map(|i| json!({"id":format!("call:{i}"),"kind":"command","status":"completed"}))
            .collect();
        assert_eq!(
            public_activities(Some(&json!(many)), false)
                .as_array()
                .unwrap()
                .len(),
            40
        );
    }

    #[tokio::test]
    async fn message_threads_list_settled_root_runs_without_private_content() {
        let mut f = Fixture::new();
        f.record["state"] = json!("completed");
        f.record["progress"] = Value::Null;
        f.record["request"]["trigger"]["message_id"] = json!("m1");
        let read = |record: Value, conversation: Option<&'static str>, revoked: bool| async move {
            let mut live = record.clone();
            live["id"] = json!("live");
            live["state"] = json!("running");
            let backend = Fake {
                handler: Box::new(move |args| match args[0].as_str() {
                    "run" => json!([record, live]),
                    _ if revoked => json!([]),
                    _ => json!([{"principal_id": args[3], "left_message_seq": null}]),
                }),
                seen: Mutex::new(Vec::new()),
            };
            let ops = Ops {
                backend: &backend,
                operator: "operator",
            };
            let value = conversation_runs(ops, conversation, "operator")
                .await
                .unwrap();
            (value, backend.seen.into_inner().unwrap())
        };
        let (value, seen) = read(f.record.clone(), Some("chat"), false).await;
        assert!(seen.contains(&strings(&[
            "run",
            "list",
            "--conversation",
            "chat",
            "--limit",
            "200"
        ])));
        let runs = value["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0]["trigger_message"], "m1");
        assert_eq!(runs[0]["silent"], false);
        assert_eq!(runs[0]["items"][0]["status"], "unknown");
        assert!(!value.to_string().contains("SECRET") && !value.to_string().contains("draft"));
        f.record["output"] = json!(" [[silent]] ");
        assert_eq!(
            read(f.record.clone(), Some("chat"), false).await.0["runs"][0]["silent"],
            true
        );
        assert_eq!(
            read(f.record.clone(), Some("chat"), true).await.0,
            json!({"runs": []})
        );
        assert_eq!(
            read(f.record.clone(), None, false).await.0,
            json!({"runs": []})
        );
    }
}
