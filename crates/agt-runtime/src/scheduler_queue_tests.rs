use super::*;
use crate::work::{TaskControl, WorkLedger};
use agt_service::WorkControl;

fn setup() -> (
    tempfile::TempDir,
    Scheduler,
    TaskControl,
    String,
    String,
    String,
) {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let source = scheduler.store.tasks().unwrap()[0].message.clone();
    let control = TaskControl::local(
        home.path(),
        Actor {
            principal: PrincipalId::new("user"),
            role: Role::Manager,
            automated_depth: None,
        },
    );
    let create = |key| {
        control.call("task_request",json!({"assignee":"b","conversation":chat,"source_message":source,"input":key,"request_key":key})).unwrap()["id"].as_str().unwrap().to_owned()
    };
    let a = create("A");
    let b = create("B");
    crate::work::reconcile(home.path()).unwrap();
    scheduler.scan().unwrap();
    (home, scheduler, control, chat, a, b)
}
fn ids(scheduler: &Scheduler) -> Vec<String> {
    scheduler
        .store
        .candidates()
        .unwrap()
        .into_iter()
        .filter(|t| !t.work_notification)
        .filter_map(|t| t.work_id)
        .collect()
}
fn view(control: &TaskControl, id: &str) -> Value {
    control.call("task_get", json!({"task_id":id})).unwrap()
}

#[test]
fn priority_order_is_persistent_and_old_revision_cannot_overwrite() {
    let (home, scheduler, control, _, a, b) = setup();
    assert_eq!(ids(&scheduler), vec![a.clone(), b.clone()]);
    let args =
        json!({"task_id":b,"expected_revision":1,"priority":"high","reason":"B is time sensitive"});
    control.call("task_reprioritize", args.clone()).unwrap();
    assert!(control.call("task_reprioritize", args).is_err());
    assert_eq!(ids(&scheduler), vec![b.clone(), a.clone()]);
    assert_eq!(
        WorkLedger::open(home.path())
            .unwrap()
            .get(&b)
            .unwrap()
            .priority,
        crate::work::Priority::High
    );
    let mut candidates = scheduler.store.candidates().unwrap();
    crate::work::queue::order(
        &scheduler.store.connection,
        &mut candidates,
        now_ms() + 600_000,
    )
    .unwrap();
    assert_eq!(
        candidates
            .into_iter()
            .filter_map(|t| t.work_id)
            .collect::<Vec<_>>(),
        vec![a, b]
    );
}

#[test]
fn barrier_rechecks_at_claim_and_releases_on_failed_predecessor() {
    let (home, mut scheduler, control, _, a, b) = setup();
    // Candidate snapshots can exist before the defer decision commits.
    let before = scheduler.store.get(&format!("work:{a}")).unwrap();
    control.call("task_defer",json!({"task_id":a,"expected_revision":1,"after_task":b,"deadline":now_ms()+60_000,"reason":"B first"})).unwrap();
    assert!(
        scheduler
            .store
            .claim(&before.id, &scheduler.owner, now_ms())
            .unwrap()
            .is_none()
    );
    assert_eq!(view(&control, &a)["blocked_reason"], "waiting_for_task");
    assert_eq!(view(&control, &a)["waiting_for_task"], b);
    let btask = scheduler
        .store
        .claim(&format!("work:{b}"), &scheduler.owner, now_ms())
        .unwrap()
        .unwrap();
    execute(
        home.path(),
        btask,
        &mut Answer(json!({"role":"assistant","content":"[[silent]]"})),
        &CancellationToken::new(),
    )
    .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    assert!(view(&control, &a)["blocked_reason"].is_null());
    assert!(
        scheduler
            .store
            .claim(&before.id, &scheduler.owner, now_ms())
            .unwrap()
            .is_some()
    );
}

#[test]
fn dependency_cycles_and_stale_changes_rollback_atomically() {
    let (_home, scheduler, control, _, a, b) = setup();
    control.call("task_defer",json!({"task_id":a,"expected_revision":1,"after_task":b,"deadline":now_ms()+60_000,"reason":"B first"})).unwrap();
    assert!(control.call("task_defer",json!({"task_id":b,"expected_revision":1,"after_task":a,"deadline":now_ms()+60_000,"reason":"cycle"})).is_err());
    assert_eq!(view(&control, &b)["revision"], 1);
    assert!(view(&control, &b)["deferred"].is_null());
    assert_eq!(
        scheduler
            .store
            .connection
            .query_row("SELECT COUNT(*) FROM work_dependencies", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(control.call("task_defer",json!({"task_id":b,"expected_revision":1,"after_task":b,"deadline":now_ms()+60_000,"reason":"self"})).is_err());
    assert!(control.call("task_defer",json!({"task_id":a,"expected_revision":1,"not_before":now_ms()+60_000,"reason":"stale"})).is_err());
}

#[test]
fn timed_wait_survives_reopen_and_admitted_work_cannot_be_deferred() {
    let (home, mut scheduler, control, _, a, _) = setup();
    let at = now_ms() + 1000;
    control
        .call(
            "task_defer",
            json!({"task_id":a,"expected_revision":1,"not_before":at,"reason":"wait one second"}),
        )
        .unwrap();
    assert!(
        scheduler
            .store
            .claim(&format!("work:{a}"), &scheduler.owner, at - 1)
            .unwrap()
            .is_none()
    );
    let reopened = ScheduleStore::open(home.path()).unwrap();
    assert_eq!(
        crate::work::queue::blocker(
            &reopened.connection,
            &WorkLedger::open(home.path()).unwrap().get(&a).unwrap(),
            at - 1
        )
        .unwrap(),
        Some("deferred_until")
    );
    assert!(
        scheduler
            .store
            .claim(&format!("work:{a}"), &scheduler.owner, at)
            .unwrap()
            .is_some()
    );
    assert!(
        control
            .call(
                "task_defer",
                json!({"task_id":a,"expected_revision":2,"not_before":at+1000,"reason":"too late"})
            )
            .is_err()
    );
}

#[test]
fn hosted_assignee_can_sort_its_queue_but_cannot_grant_urgent() {
    let (home, mut scheduler, control, chat, a, b) = setup();
    make_quiet(&mut scheduler);
    let ordinary = scheduler
        .store
        .tasks()
        .unwrap()
        .into_iter()
        .find(|t| t.principal == "b" && t.work_id.is_none())
        .unwrap();
    let lead = scheduler
        .store
        .claim(&ordinary.id, &scheduler.owner, now_ms())
        .unwrap()
        .unwrap();
    let hosted = TaskControl::hosted(
        home.path(),
        Actor {
            principal: PrincipalId::new("b"),
            role: Role::Participant,
            automated_depth: Some(1),
        },
        lead,
    );
    let updated = hosted
        .call(
            "task_reprioritize",
            json!({"task_id":b,"expected_revision":1,"priority":"high","reason":"handle B first"}),
        )
        .unwrap();
    assert_eq!(updated["priority"], "high");
    assert!(
        hosted
            .call(
                "task_reprioritize",
                json!({"task_id":a,"expected_revision":1,"priority":"urgent","reason":"escalate"})
            )
            .is_err()
    );
    assert!(hosted.call("task_request",json!({"assignee":"a","conversation":chat,"input":"urgent","request_key":"urgent","priority":"urgent"})).is_err());
    hosted.call("task_defer",json!({"task_id":a,"expected_revision":1,"after_task":b,"deadline":now_ms()+60_000,"reason":"B first"})).unwrap();
    assert_eq!(view(&control, &a)["deferred"]["reason"], "B first");
    let peer = TaskControl::local(
        home.path(),
        Actor {
            principal: PrincipalId::new("a"),
            role: Role::Participant,
            automated_depth: None,
        },
    );
    assert!(
        peer.call(
            "task_reprioritize",
            json!({"task_id":a,"expected_revision":2,"priority":"low","reason":"unauthorized"})
        )
        .is_err()
    );
}

#[test]
fn expired_or_unknown_dependencies_do_not_release_barriers() {
    let (home, _, control, _, a, b) = setup();
    let deadline = now_ms() + 1000;
    control.call("task_defer",json!({"task_id":a,"expected_revision":1,"after_task":b,"deadline":deadline,"reason":"ordered"})).unwrap();
    let store = ScheduleStore::open(home.path()).unwrap();
    let ledger = WorkLedger::open(home.path()).unwrap();
    let item = ledger.get(&a).unwrap();
    assert_eq!(
        crate::work::queue::blocker(&store.connection, &item, deadline).unwrap(),
        Some("dependency_deadline_exceeded")
    );
    drop(ledger);
    let mut ledger = WorkLedger::open(home.path()).unwrap();
    ledger.record_running(&b, 1, "unknown-run").unwrap();
    ledger
        .record_outcome(
            &b,
            2,
            "unknown-run",
            crate::work::WorkOutcome::Unknown("lost owner".into()),
        )
        .unwrap();
    assert_eq!(
        crate::work::queue::blocker(&store.connection, &item, deadline - 1).unwrap(),
        Some("dependency_outcome_unknown")
    );
}

#[test]
fn native_queue_tool_preserves_guard_without_nested_transaction() {
    struct Defer {
        step: usize,
        target: String,
    }
    impl ModelClient for Defer {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            messages: &[Value],
            tools: &[Value],
        ) -> crate::Result<Value> {
            assert!(
                tools
                    .iter()
                    .any(|t| t["function"]["name"] == "framework_task_defer")
            );
            self.step += 1;
            if self.step == 1 {
                Ok(
                    json!({"role":"assistant","tool_calls":[{"id":"defer","type":"function","function":{"name":"framework_task_defer","arguments":json!({"task_id":self.target,"expected_revision":1,"not_before":now_ms()+60_000,"reason":"finish current discussion first"}).to_string()}}]}),
                )
            } else {
                let result: Value =
                    serde_json::from_str(messages.last().unwrap()["content"].as_str().unwrap())
                        .unwrap();
                assert_eq!(result["blocked_reason"], "deferred_until");
                Ok(json!({"role":"assistant","content":"queued work deferred"}))
            }
        }
    }
    let (home, mut scheduler, control, _, a, _) = setup();
    AgentStore::new(home.path())
        .update_in(&agt_core::CategoryName::default(), "b", |config| {
            config
                .builtin_tools
                .push(agt_agent::BuiltinTool::TaskCollaboration);
            Ok(())
        })
        .unwrap();
    make_quiet(&mut scheduler);
    let ordinary = scheduler
        .store
        .tasks()
        .unwrap()
        .into_iter()
        .find(|t| t.principal == "b" && t.work_id.is_none())
        .unwrap();
    let task = scheduler
        .store
        .claim(&ordinary.id, &scheduler.owner, now_ms())
        .unwrap()
        .unwrap();
    let mut model = Defer {
        step: 0,
        target: a.clone(),
    };
    execute(home.path(), task, &mut model, &CancellationToken::new()).unwrap();
    assert_eq!(model.step, 2);
    assert_eq!(view(&control, &a)["blocked_reason"], "deferred_until");
}

#[test]
fn human_urgent_beats_aging_and_end_still_cancels_deferred_work() {
    let (home, mut scheduler, control, chat, a, b) = setup();
    control.call("task_reprioritize",json!({"task_id":b,"expected_revision":1,"priority":"urgent","reason":"human escalation"})).unwrap();
    let mut candidates = scheduler.store.candidates().unwrap();
    crate::work::queue::order(
        &scheduler.store.connection,
        &mut candidates,
        now_ms() + 1_000_000,
    )
    .unwrap();
    assert_eq!(
        candidates
            .into_iter()
            .filter_map(|t| t.work_id)
            .collect::<Vec<_>>(),
        vec![b, a.clone()]
    );
    control.call("task_defer",json!({"task_id":a,"expected_revision":1,"not_before":now_ms()+60_000,"reason":"later"})).unwrap();
    scheduler
        .store
        .control(&chat, "end", false, now_ms())
        .unwrap();
    scheduler.tick(false).unwrap();
    assert_eq!(view(&control, &a)["state"], "canceled");
    assert!(
        control
            .call("task_list", json!({"queued_only":true}))
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        crate::RunStore::open(home.path())
            .unwrap()
            .list(10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn expired_barrier_notifies_once_without_releasing_queued_work() {
    let (home, scheduler, control, _, a, b) = setup();
    control.call("task_defer",json!({"task_id":a,"expected_revision":1,"after_task":b,"deadline":now_ms()+60000,"reason":"B first"})).unwrap();
    scheduler
        .store
        .connection
        .execute(
            "UPDATE work_items SET record=json_set(record,'$.deferred.deadline',0) WHERE id=?1",
            [&a],
        )
        .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    crate::work::reconcile(home.path()).unwrap();
    let item = view(&control, &a);
    assert_eq!(item["state"], "queued");
    assert_eq!(item["blocked_reason"], "dependency_deadline_exceeded");
    let count: i64 = scheduler
        .store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM work_outbox WHERE work_id=?1 AND kind='dependency_expired'",
            [&a],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(item["publication"], "published");
    assert_eq!(item["report"]["state"], "not_ready");
}
