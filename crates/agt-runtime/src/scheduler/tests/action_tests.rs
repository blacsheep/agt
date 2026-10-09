use super::wait_tests::{claim_work, setup, suspend};
use super::*;
use crate::work::{TaskControl, WorkLedger, WorkOutcome, WorkState};
use agt_service::WorkControl;
fn manager(home: &Path) -> TaskControl {
    TaskControl::local(
        home,
        Actor {
            principal: PrincipalId::new("user"),
            role: Role::Manager,
            automated_depth: None,
        },
    )
}
#[test]
fn queued_cancel_and_explicit_retry_preserve_history() {
    let (home, _s, _chat, parent) = setup();
    let control = manager(home.path());
    let canceled = control
        .call(
            "task_cancel",
            json!({"task_id":parent,"expected_revision":1,"reason":"changed plan"}),
        )
        .unwrap();
    assert_eq!(canceled["state"], "canceled");
    assert!(
        control
            .call(
                "task_retry",
                json!({"task_id":parent,"expected_revision":1,"reason":"retry","request_key":"new"})
            )
            .is_err()
    );
    let args = json!({"task_id":parent,"expected_revision":canceled["revision"],"reason":"retry approved","request_key":"retry-1"});
    let new = control.call("task_retry", args.clone()).unwrap();
    assert_ne!(new["id"], parent);
    assert_eq!(new["request"]["retry_of"], parent);
    assert_eq!(new["state"], "queued");
    assert_eq!(control.call("task_retry", args).unwrap()["id"], new["id"]);
    assert!(matches!(
        WorkLedger::open(home.path())
            .unwrap()
            .get(&parent)
            .unwrap()
            .state,
        WorkState::Canceled { .. }
    ));
}
#[test]
fn cancel_waiting_parent_cancels_exclusive_child() {
    let (home, mut s, chat, parent) = setup();
    let child = suspend(home.path(), &mut s, &chat, &parent);
    crate::work::reconcile(home.path()).unwrap();
    let ledger = WorkLedger::open(home.path()).unwrap();
    assert_eq!(
        ledger.get(&child).unwrap().request.parent_id.as_deref(),
        Some(parent.as_str())
    );
    manager(home.path()).call("task_cancel",json!({"task_id":parent,"expected_revision":ledger.get(&parent).unwrap().revision,"reason":"stop tree"})).unwrap();
    assert!(matches!(
        ledger.get(&parent).unwrap().state,
        WorkState::Canceled { .. }
    ));
    assert!(matches!(
        ledger.get(&child).unwrap().state,
        WorkState::Canceled { .. }
    ));
}
#[test]
fn admitted_cancellation_fences_work_without_claiming_external_success() {
    let (home, mut s, _chat, parent) = setup();
    let task = claim_work(&mut s, &parent);
    let result = manager(home.path())
        .call(
            "task_cancel",
            json!({"task_id":parent,"expected_revision":1,"reason":"stop admitted"}),
        )
        .unwrap();
    assert_eq!(result["state"], "queued");
    assert_eq!(result["cancellation_requested"], "stop admitted");
    assert!(guarded(home.path(), &task, || Ok(())).is_err());
}
#[test]
fn unknown_requires_human_evidence_before_retry_and_reports_resolution_once() {
    let (home, _s, chat, parent) = setup();
    let mut ledger = WorkLedger::open(home.path()).unwrap();
    let run = ledger.record_running(&parent, 1, "unknown-run").unwrap();
    let unknown = ledger
        .record_outcome(
            &parent,
            run.revision,
            "unknown-run",
            WorkOutcome::Unknown("lost connection".into()),
        )
        .unwrap();
    let control = manager(home.path());
    assert!(control.call("task_retry",json!({"task_id":parent,"expected_revision":unknown.revision,"reason":"guess","request_key":"bad-retry"})).is_err());
    let verified=control.call("task_resolve",json!({"task_id":parent,"expected_revision":unknown.revision,"outcome":"failed","evidence":"inspected process and repository; task did not complete"})).unwrap();
    assert_eq!(verified["state"], "outcome_unknown");
    assert_eq!(verified["resolution"]["outcome"], "failed");
    crate::work::reconcile(home.path()).unwrap();
    crate::work::reconcile(home.path()).unwrap();
    let messages = im(home.path())
        .unwrap()
        .list_messages(
            &ConversationId::new(chat),
            &PrincipalId::new("user"),
            MessageListOptions {
                limit: 200,
                before: None,
                after: None,
            },
        )
        .unwrap()
        .messages;
    let notices: Vec<_> = messages
        .iter()
        .filter(|m| m.metadata["work_kind"] == "result")
        .collect();
    assert_eq!(notices.len(), 2);
    for message in notices {
        assert!(
            crate::work::route(&ScheduleStore::open(home.path()).unwrap(), message)
                .unwrap()
                .is_some()
        );
    }
    assert!(control.call("task_retry",json!({"task_id":parent,"expected_revision":verified["revision"],"reason":"retry after verification","request_key":"verified-retry"})).is_ok());
}
#[test]
fn unhosted_agent_cannot_cancel_shared_work() {
    let (home, _s, _chat, parent) = setup();
    let control = TaskControl::local(
        home.path(),
        Actor {
            principal: PrincipalId::new("a"),
            role: Role::Participant,
            automated_depth: None,
        },
    );
    assert!(
        control
            .call(
                "task_cancel",
                json!({"task_id":parent,"expected_revision":1,"reason":"unhosted"})
            )
            .is_err()
    );
}

#[test]
fn report_is_a_separate_durable_obligation_and_publication_is_idempotent() {
    let (home, mut s, chat, parent) = setup();
    let child = suspend(home.path(), &mut s, &chat, &parent);
    let mut ledger = WorkLedger::open(home.path()).unwrap();
    let queued = ledger.get(&child).unwrap();
    let running = ledger
        .record_running(&child, queued.revision, "report-test-run")
        .unwrap();
    ledger
        .record_outcome(
            &child,
            running.revision,
            "report-test-run",
            WorkOutcome::Succeeded("private child result".into()),
        )
        .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    let control = manager(home.path());
    let item = control.call("task_get", json!({"task_id":child})).unwrap();
    assert_eq!(item["report"]["state"], "awaiting_report");
    s.store
        .connection
        .execute(
            "UPDATE work_reports SET created_at=0 WHERE work_id=?1",
            [&child],
        )
        .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    crate::work::reconcile(home.path()).unwrap();
    let count: i64 = s
        .store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM change_outbox WHERE kind='attention'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "overdue reminder is emitted only once");

    let args = json!({"task_id":child,"expected_revision":item["revision"],"body":"reviewed report for original conversation"});
    let submitted = control.call("task_report", args.clone()).unwrap();
    assert_eq!(submitted["report"]["state"], "pending");
    assert!(control.call("task_report", args.clone()).is_ok());
    crate::work::reconcile(home.path()).unwrap();
    let item = control.call("task_get", json!({"task_id":child})).unwrap();
    assert_eq!(item["report"]["state"], "published");
    let message = item["report"]["message_id"].as_str().unwrap();
    // Crash after IM commit but before report acknowledgement: replay the stable message key.
    s.store
        .connection
        .execute(
            "UPDATE work_reports SET message_id=NULL WHERE work_id=?1",
            [&child],
        )
        .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    assert_eq!(
        control.call("task_get", json!({"task_id":child})).unwrap()["report"]["message_id"],
        message
    );
    let mut conflict = args;
    conflict["body"] = json!("replace already delivered report");
    assert!(control.call("task_report", conflict).is_err());
}

#[test]
fn workspace_incidents_exclude_private_payloads_and_require_owner() {
    let (home, _s, chat, parent) = setup();
    let im = im(home.path()).unwrap();
    im.ensure_workspace_owner(&PrincipalId::new("user"))
        .unwrap();
    let mut ledger = WorkLedger::open(home.path()).unwrap();
    let running = ledger.record_running(&parent, 1, "unknown-test").unwrap();
    ledger
        .record_outcome(
            &parent,
            running.revision,
            "unknown-test",
            WorkOutcome::Unknown("private failure detail".into()),
        )
        .unwrap();
    let control = manager(home.path());
    let attention = control.call("task_attention", json!({})).unwrap();
    assert_eq!(attention[0]["reasons"][0], "outcome_unknown");
    assert!(!attention.to_string().contains("private failure"));
    assert!(!attention.to_string().contains("delegate then integrate"));
    assert!(!attention.to_string().contains(&chat));
    let outsider = TaskControl::local(
        home.path(),
        Actor {
            principal: PrincipalId::new("a"),
            role: Role::Manager,
            automated_depth: None,
        },
    );
    assert!(outsider.call("task_attention", json!({})).is_err());
}
