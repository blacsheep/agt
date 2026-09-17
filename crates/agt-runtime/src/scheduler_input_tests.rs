use super::wait_tests::{claim_work, setup, suspend, tool};
use super::*;
use crate::work::{TaskControl, WorkLedger, WorkState};
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
struct One {
    name: &'static str,
    args: Value,
    calls: usize,
}
impl ModelClient for One {
    fn complete(&mut self, _: &ResolvedAgent, _: &[Value], _: &[Value]) -> crate::Result<Value> {
        self.calls += 1;
        assert_eq!(self.calls, 1);
        Ok(tool(self.name, self.args.clone()))
    }
}
struct Resume {
    expected: &'static str,
}
impl ModelClient for Resume {
    fn complete(&mut self, _: &ResolvedAgent, m: &[Value], _: &[Value]) -> crate::Result<Value> {
        assert!(
            m.last().unwrap()["content"]
                .as_str()
                .unwrap()
                .contains(self.expected)
        );
        Ok(json!({"role":"assistant","content":"finished after input"}))
    }
}
#[test]
fn input_reply_is_idempotent_and_resumes_saved_context() {
    let (home, mut s, _, parent) = setup();
    let task = claim_work(&mut s, &parent);
    execute(
        home.path(),
        task,
        &mut One {
            name: "task_input",
            args: json!({"question":"Which branch?","deadline":now_ms()+60000}),
            calls: 0,
        },
        &CancellationToken::new(),
    )
    .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    let ledger = WorkLedger::open(home.path()).unwrap();
    let item = ledger.get(&parent).unwrap();
    let WorkState::WaitingInput {
        ref run_id,
        ref question,
        ..
    } = item.state
    else {
        panic!("not waiting")
    };
    assert_eq!(question, "Which branch?");
    let args = json!({"task_id":parent,"input_request_id":run_id,"expected_revision":item.revision,"answer":"feature/next","request_key":"answer-1"});
    let control = manager(home.path());
    control.call("task_reply", args.clone()).unwrap();
    control.call("task_reply", args.clone()).unwrap();
    let mut conflicting = args.clone();
    conflicting["answer"] = json!("different");
    assert!(control.call("task_reply", conflicting).is_err());
    crate::work::reconcile(home.path()).unwrap();
    let task = claim_work(&mut s, &parent);
    execute(
        home.path(),
        task,
        &mut Resume {
            expected: "feature/next",
        },
        &CancellationToken::new(),
    )
    .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    assert!(matches!(
        ledger.get(&parent).unwrap().state,
        WorkState::Succeeded { .. }
    ));
    assert!(control.call("task_reply", args).is_ok());
}
#[test]
fn yielding_and_input_timeout_resume_without_an_answer() {
    for input in [false, true] {
        let (home, mut s, _, parent) = setup();
        let task = claim_work(&mut s, &parent);
        let (name, args, expected) = if input {
            (
                "task_input",
                json!({"question":"Need input","deadline":now_ms()+60000}),
                "input_timeout",
            )
        } else {
            (
                "task_yield",
                json!({"reason":"let other work proceed","not_before":now_ms()+60000}),
                "timer",
            )
        };
        execute(
            home.path(),
            task,
            &mut One {
                name,
                args,
                calls: 0,
            },
            &CancellationToken::new(),
        )
        .unwrap();
        crate::work::reconcile(home.path()).unwrap();
        assert!(matches!(
            WorkLedger::open(home.path())
                .unwrap()
                .get(&parent)
                .unwrap()
                .state,
            WorkState::WaitingUntil { .. } | WorkState::WaitingInput { .. }
        ));
        assert!(
            s.store
                .claim(&format!("work:{parent}"), &s.owner, now_ms())
                .unwrap()
                .is_none()
        );
        s.store
            .connection
            .execute("UPDATE work_waits SET deadline=0", [])
            .unwrap();
        crate::work::reconcile(home.path()).unwrap();
        let task = claim_work(&mut s, &parent);
        execute(
            home.path(),
            task,
            &mut Resume { expected },
            &CancellationToken::new(),
        )
        .unwrap();
    }
}
#[test]
fn awaiting_parent_still_receives_child_input_notification() {
    let (home, mut s, chat, parent) = setup();
    let child = suspend(home.path(), &mut s, &chat, &parent);
    AgentStore::new(home.path())
        .update_in(&agt_core::CategoryName::default(), "b", |c| {
            c.builtin_tools
                .push(agt_agent::BuiltinTool::TaskCollaboration);
            Ok(())
        })
        .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    s.scan().unwrap();
    let task = claim_work(&mut s, &child);
    execute(
        home.path(),
        task,
        &mut One {
            name: "task_input",
            args: json!({"question":"Clarify scope?","deadline":now_ms()+60000}),
            calls: 0,
        },
        &CancellationToken::new(),
    )
    .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    s.scan().unwrap();
    let notice = s
        .store
        .tasks()
        .unwrap()
        .into_iter()
        .find(|t| {
            t.work_id.as_deref() == Some(&child)
                && t.work_event.as_deref() == Some("input_requested")
        })
        .unwrap();
    assert!(crate::work::runnable(home.path(), &s.store, &notice).unwrap());
    assert!(
        s.store
            .claim(&notice.id, &s.owner, now_ms())
            .unwrap()
            .is_some()
    );
}

#[test]
fn scheduler_wake_deadline_tracks_suspended_work_without_busy_scanning() {
    let (home, mut scheduler, _, parent) = setup();
    let task = claim_work(&mut scheduler, &parent);
    execute(
        home.path(),
        task,
        &mut One {
            name: "task_yield",
            args: json!({"reason":"timer test", "not_before":now_ms()+5000}),
            calls: 0,
        },
        &CancellationToken::new(),
    )
    .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    let delay = scheduler.next_wake_delay().unwrap();
    assert!(delay > Duration::from_secs(1));
    assert!(delay <= Duration::from_secs(5));
}
