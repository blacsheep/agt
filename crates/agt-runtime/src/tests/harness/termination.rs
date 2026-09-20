//! End-to-end ledger/Run/protocol facts, without a model endpoint.
use super::*;
use crate::{RunState, StopCode};

fn setup(
    input: &str,
) -> (
    tempfile::TempDir,
    Scheduler,
    TaskControl,
    String,
    ScheduledTask,
) {
    let (home, user, chat) = external_fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let source = scheduler
        .store
        .tasks()
        .unwrap()
        .into_iter()
        .find(|t| t.principal == "a")
        .unwrap()
        .message;
    let control = TaskControl::local(
        home.path(),
        Actor {
            principal: PrincipalId::new("user"),
            role: Role::Manager,
            automated_depth: None,
        },
    );
    let item = control.call("task_request", json!({"assignee":"a","conversation":chat,"input":input,"request_key":"termination","source_message":source})).unwrap();
    let id = item["id"].as_str().unwrap().to_owned();
    crate::work::reconcile(home.path()).unwrap();
    scheduler.scan().unwrap();
    let task = claim_work(&mut scheduler, &id);
    (home, scheduler, control, id, task)
}
fn wait_started(home: &Path) {
    let limit = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !home.join("started").exists() {
        assert!(std::time::Instant::now() < limit);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
#[test]
fn codex_known_interruption_failure_and_disconnect_have_distinct_results() {
    for (input, expected, code) in [
        ("INTERRUPTED", "interrupted", StopCode::EngineInterrupted),
        ("FAIL", "failed", StopCode::EngineFailed),
        ("DISCONNECT", "outcome_unknown", StopCode::ConnectionLost),
        ("EMPTY", "outcome_unknown", StopCode::ResultMissing),
    ] {
        let (home, _scheduler, control, id, task) = setup(input);
        execute(
            home.path(),
            task,
            &mut Answer(json!({})),
            &CancellationToken::new(),
        )
        .unwrap();
        crate::work::reconcile(home.path()).unwrap();
        let item = control.call("task_get", json!({"task_id":id})).unwrap();
        assert_eq!(item["state"], expected, "{input}");
        assert_eq!(item["termination"]["code"], json!(code));
        assert_eq!(item["publication"], "published");
        assert!(!item.to_string().contains("fixture-secret"));
        assert!(item.get("diagnostics").is_none());
        let run = crate::RunStore::open(home.path())
            .unwrap()
            .get(item["run_id"].as_str().unwrap())
            .unwrap();
        if input == "DISCONNECT" {
            assert_eq!(run.state, RunState::OutcomeUnknown);
            let diagnostic = &run.execution.as_ref().unwrap()["diagnostics"];
            assert!(!diagnostic.to_string().contains("fixture-secret"));
            assert_eq!(diagnostic["exit_code"], 7);
        }
        if input == "INTERRUPTED" {
            assert_eq!(run.state, RunState::Interrupted);
            assert_eq!(item["termination"]["confirmation"], "engine_interrupted");
            let retry = control.call("task_retry", json!({"task_id":id,"expected_revision":item["revision"],"reason":"inspect complete","request_key":"retry"})).unwrap();
            assert_eq!(retry["state"], "queued");
        }
    }
}
#[test]
fn user_cancel_is_pending_until_confirmed_and_keeps_actor_and_reason() {
    let (home, _scheduler, control, id, task) = setup("CANCELABLE");
    let path = home.path().to_owned();
    let handle = std::thread::spawn(move || {
        execute(
            &path,
            task,
            &mut Answer(json!({})),
            &CancellationToken::new(),
        )
    });
    wait_started(home.path());
    let item = control.call("task_get", json!({"task_id":id})).unwrap();
    control
        .call(
            "task_cancel",
            json!({"task_id":id,"expected_revision":item["revision"],"reason":"stop this work"}),
        )
        .unwrap();
    handle.join().unwrap().unwrap();
    crate::work::reconcile(home.path()).unwrap();
    let item = control.call("task_get", json!({"task_id":id})).unwrap();
    assert_eq!(item["state"], "canceled");
    assert_eq!(item["termination"]["actor"], "user");
    assert_eq!(item["termination"]["code"], "task_canceled");
    assert_eq!(item["termination"]["confirmation"], "engine_interrupted");
    assert_eq!(item["termination"]["side_effects"], "may_have_changes");
    assert!(item["termination"]["interrupt_requested_at"].is_number());
    assert!(item["termination"]["confirmed_at"].is_number());
    assert_eq!(item["publication"], "published");
}
#[test]
fn timeout_with_engine_ack_is_failed_not_unknown() {
    let (home, _scheduler, control, id, task) = setup("CANCELABLE");
    execute(
        home.path(),
        task,
        &mut Answer(json!({})),
        &CancellationToken::new(),
    )
    .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    let item = control.call("task_get", json!({"task_id":id})).unwrap();
    assert_eq!(item["state"], "failed");
    assert_eq!(item["termination"]["code"], "harness_timeout");
    assert_eq!(item["termination"]["confirmation"], "engine_interrupted");
}
#[test]
fn active_codex_completes_after_discussion_expiry() {
    let (home, scheduler, control, id, task) = setup("DELAYED");
    let path = home.path().to_owned();
    let handle = std::thread::spawn(move || {
        execute(
            &path,
            task,
            &mut Answer(json!({})),
            &CancellationToken::new(),
        )
    });
    wait_started(home.path());
    scheduler
        .store
        .connection
        .execute("UPDATE discussions SET expires=0,state='limited'", [])
        .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    assert_eq!(
        control.call("task_get", json!({"task_id":id})).unwrap()["state"],
        "running"
    );
    handle.join().unwrap().unwrap();
    crate::work::reconcile(home.path()).unwrap();
    let item = control.call("task_get", json!({"task_id":id})).unwrap();
    assert_eq!(item["state"], "succeeded");
    assert_eq!(item["output"], "completed after expiry");
    assert_eq!(item["publication"], "published");
    assert!(item["notification_blocked_reason"].is_null());
}
