use crate::*;
use serde_json::json;
#[test]
fn tool_progress_preserves_names_and_history_without_private_arguments() {
    let home = tempfile::tempdir().unwrap();
    let mut store = RunStore::open(home.path()).unwrap();
    let (mut run, _) = store
        .claim(&RunRequest::new("test", "private input"), None)
        .unwrap();
    crate::progress::stage(&mut run, &store, None, "tool_running").unwrap();
    let mut call = store
        .start_call(
            CallRecord::new(
                &run,
                Some("framework_harness_models"),
                json!({"private":"secret"}),
            )
            .with_public_context(&json!({})),
        )
        .unwrap();
    let active = store.get(&run.id).unwrap().progress.unwrap();
    assert_eq!(
        active.activities[0].tool_name.as_deref(),
        Some("framework_harness_models")
    );
    assert_eq!(active.activities[0].status, "running");
    assert!(!serde_json::to_string(&active).unwrap().contains("secret"));
    call.summarize_result(&json!({"data":[{},{}]}));
    call.finish(CallState::Succeeded, None);
    store.finish_call(&call).unwrap();
    crate::progress::stage(&mut run, &store, None, "waiting").unwrap();
    let settled = store.get(&run.id).unwrap().progress.unwrap();
    assert_eq!(settled.activities[0].status, "completed");
    assert_eq!(
        settled.activities[0].summary.as_deref(),
        Some("找到 2 个模型")
    );
    run.progress = None;
    run.state = RunState::Completed;
    store.save(&run).unwrap();
    assert_eq!(
        store.get(&run.id).unwrap().execution.unwrap()["activity"]["items"][0]["tool_name"],
        "framework_harness_models"
    );
}

#[test]
fn unfinished_tools_are_unknown_after_run_failure() {
    let home = tempfile::tempdir().unwrap();
    let mut store = RunStore::open(home.path()).unwrap();
    let (mut run, _) = store
        .claim(&RunRequest::new("test", "input"), None)
        .unwrap();
    crate::progress::stage(&mut run, &store, None, "tool_running").unwrap();
    for name in ["framework_harness_models", "framework_resource_list"] {
        store
            .start_call(CallRecord::new(&run, Some(name), json!({})))
            .unwrap();
    }
    assert_eq!(
        store
            .get(&run.id)
            .unwrap()
            .progress
            .unwrap()
            .activities
            .len(),
        2
    );
    run.state = RunState::Failed;
    run.progress = None;
    store.save(&run).unwrap();
    let saved = store.get(&run.id).unwrap();
    let execution = saved.execution.unwrap();
    assert!(
        execution["activity"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["status"] == "unknown")
    );
}

#[test]
fn completed_execution_retains_activity_facts_but_never_the_draft() {
    let home = tempfile::tempdir().unwrap();
    let mut store = RunStore::open(home.path()).unwrap();
    let (mut run, _) = store
        .claim(&RunRequest::new("test", "private input"), None)
        .unwrap();
    crate::progress::stage(&mut run, &store, None, "generating").unwrap();
    crate::progress::update(&mut run, &store, ModelEvent::Text("private draft")).unwrap();
    crate::progress::harness_event(
        &mut run,
        &store,
        &agt_harness::Event::Activity {
            id: "private-engine-item",
            kind: "command",
            status: "completed",
            tool_name: None,
        },
    )
    .unwrap();
    crate::progress::harness_event(
        &mut run,
        &store,
        &agt_harness::Event::Activity {
            id: "unfinished-command",
            kind: "command",
            status: "running",
            tool_name: None,
        },
    )
    .unwrap();
    let activity_id = run.progress.as_ref().unwrap().activities[0].id.clone();
    crate::progress::finish(&mut run);
    run.state = RunState::Completed;
    store.save(&run).unwrap();
    let restored = store.get(&run.id).unwrap();
    assert!(restored.progress.is_none());
    let activity = &restored.execution.unwrap()["activity"];
    assert_eq!(activity["items"][0]["id"], activity_id);
    assert_eq!(activity["items"][0]["status"], "completed");
    assert_eq!(activity["items"][1]["status"], "unknown");
    assert!(activity["ended_at"].as_i64().unwrap() >= activity["started_at"].as_i64().unwrap());
    assert!(!activity.to_string().contains("private"));
}
