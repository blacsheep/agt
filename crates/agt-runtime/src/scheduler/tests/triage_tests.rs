use super::wait_tests::{claim_work, setup};
use super::*;
use crate::work::{Priority, TaskControl, WorkLedger};
use agt_service::WorkControl;

#[test]
fn automatic_triage_orders_b_before_a_without_project_execution() {
    struct Decide {
        a: String,
        b: String,
        calls: usize,
    }
    impl ModelClient for Decide {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            _: &[Value],
            tools: &[Value],
        ) -> crate::Result<Value> {
            self.calls += 1;
            assert_eq!(tools.len(), 1);
            assert_eq!(tools[0]["function"]["name"], "queue_decision");
            Ok(
                json!({"role":"assistant","tool_calls":[{"id":"triage","type":"function","function":{"name":"queue_decision","arguments":json!({"decisions":[{"task_id":self.b,"priority":"high","reason":"B blocks the release"},{"task_id":self.a,"reason":"A follows B","after_task":self.b,"deadline":now_ms()+60000}]}).to_string()}}]}),
            )
        }
    }
    let (home, mut s, chat, a) = setup();
    let source = s
        .store
        .tasks()
        .unwrap()
        .into_iter()
        .find(|t| t.work_id.is_none())
        .unwrap()
        .message;
    let manager = TaskControl::local(
        home.path(),
        Actor {
            principal: PrincipalId::new("user"),
            role: Role::Manager,
            automated_depth: None,
        },
    );
    let b=manager.call("task_request",json!({"assignee":"a","conversation":chat,"source_message":source,"input":"release blocker","request_key":"B"})).unwrap()["id"].as_str().unwrap().to_string();
    crate::work::reconcile(home.path()).unwrap();
    s.scan().unwrap();
    let task = claim_work(&mut s, &a);
    let mut model = Decide {
        a: a.clone(),
        b: b.clone(),
        calls: 0,
    };
    execute(home.path(), task, &mut model, &CancellationToken::new()).unwrap();
    assert_eq!(model.calls, 1);
    assert!(
        crate::RunStore::open(home.path())
            .unwrap()
            .list(10)
            .unwrap()
            .is_empty()
    );
    let ledger = WorkLedger::open(home.path()).unwrap();
    assert_eq!(ledger.get(&b).unwrap().priority, Priority::High);
    assert_eq!(
        ledger.get(&a).unwrap().deferred.unwrap().after_task,
        Some(b.clone())
    );
    assert_eq!(ledger.get(&a).unwrap().triage.unwrap().source, "model");
    assert!(
        s.store
            .claim(&format!("work:{a}"), &s.owner, now_ms())
            .unwrap()
            .is_none()
    );
    let task = claim_work(&mut s, &b);
    execute(
        home.path(),
        task,
        &mut Answer(json!({"role":"assistant","content":"B complete"})),
        &CancellationToken::new(),
    )
    .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    assert!(
        s.store
            .claim(&format!("work:{a}"), &s.owner, now_ms())
            .unwrap()
            .is_some()
    );
}
#[test]
fn malformed_triage_falls_back_once_without_executing_requested_tools() {
    let (home, mut s, chat, a) = setup();
    let source = s
        .store
        .tasks()
        .unwrap()
        .into_iter()
        .find(|t| t.work_id.is_none())
        .unwrap()
        .message;
    let manager = TaskControl::local(
        home.path(),
        Actor {
            principal: PrincipalId::new("user"),
            role: Role::Manager,
            automated_depth: None,
        },
    );
    let b=manager.call("task_request",json!({"assignee":"a","conversation":chat,"source_message":source,"input":"other","request_key":"B"})).unwrap()["id"].as_str().unwrap().to_string();
    crate::work::reconcile(home.path()).unwrap();
    s.scan().unwrap();
    let task = claim_work(&mut s, &a);
    execute(home.path(),task,&mut Answer(json!({"role":"assistant","tool_calls":[{"function":{"name":"framework_task_cancel","arguments":"{}"}}]})),&CancellationToken::new()).unwrap();
    let ledger = WorkLedger::open(home.path()).unwrap();
    for id in [&a, &b] {
        assert_eq!(
            ledger.get(id).unwrap().triage.unwrap().source,
            "framework_fallback"
        );
        assert_eq!(ledger.get(id).unwrap().priority, Priority::Normal);
    }
    let task = claim_work(&mut s, &a);
    execute(
        home.path(),
        task,
        &mut Answer(json!({"role":"assistant","content":"normal execution"})),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        crate::RunStore::open(home.path())
            .unwrap()
            .list(10)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn four_notifications_yield_next_opportunity_to_runnable_work() {
    let (home, s, _, parent) = setup();
    let work = s.store.get(&format!("work:{parent}")).unwrap();
    let mut notice = work.clone();
    notice.id = "notice:test".into();
    notice.work_notification = true;
    s.store
        .connection
        .execute("INSERT INTO work_fairness VALUES('a',3)", [])
        .unwrap();
    let mut tasks = vec![notice.clone(), work.clone()];
    crate::work::queue::order(&s.store.connection, &mut tasks, now_ms()).unwrap();
    assert!(tasks[0].work_notification);
    s.store
        .connection
        .execute("UPDATE work_fairness SET notices=4 WHERE principal='a'", [])
        .unwrap();
    let reopened = ScheduleStore::open(home.path()).unwrap();
    crate::work::queue::order(&reopened.connection, &mut tasks, now_ms()).unwrap();
    assert!(!tasks[0].work_notification);
}
