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
