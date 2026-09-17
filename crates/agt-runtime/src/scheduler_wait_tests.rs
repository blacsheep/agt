use super::*;
use crate::work::{TaskControl, WorkLedger, WorkState};
use agt_service::WorkControl;

fn setup() -> (tempfile::TempDir, Scheduler, String, String) {
    let (home, user, chat) = fixture();
    AgentStore::new(home.path())
        .update_in(&agt_core::CategoryName::default(), "a", |c| {
            c.builtin_tools
                .push(agt_agent::BuiltinTool::TaskCollaboration);
            Ok(())
        })
        .unwrap();
    let mut scheduler = begin(home.path(), &user, &chat);
    let source = scheduler.store.tasks().unwrap()[0].message.clone();
    let manager = TaskControl::local(
        home.path(),
        Actor {
            principal: PrincipalId::new("user"),
            role: Role::Manager,
            automated_depth: None,
        },
    );
    let parent = manager.call("task_request",json!({"assignee":"a","conversation":chat,"source_message":source,"input":"delegate then integrate","request_key":"parent"})).unwrap()["id"].as_str().unwrap().to_owned();
    crate::work::reconcile(home.path()).unwrap();
    scheduler.scan().unwrap();
    (home, scheduler, chat, parent)
}
fn claim_work(s: &mut Scheduler, id: &str) -> ScheduledTask {
    s.store
        .claim(&format!("work:{id}"), &s.owner, now_ms())
        .unwrap()
        .unwrap()
}
fn tool(name: &str, args: Value) -> Value {
    json!({"role":"assistant","tool_calls":[{"id":name,"type":"function","function":{"name":format!("framework_{name}"),"arguments":args.to_string()}}]})
}
struct Suspend {
    step: usize,
    chat: String,
    child: Option<String>,
}
impl ModelClient for Suspend {
    fn complete(
        &mut self,
        _: &ResolvedAgent,
        messages: &[Value],
        schemas: &[Value],
    ) -> crate::Result<Value> {
        assert!(
            schemas
                .iter()
                .any(|t| t["function"]["name"] == "framework_task_await")
        );
        self.step += 1;
        match self.step {
            1 => Ok(tool(
                "task_request",
                json!({"assignee":"b","conversation":self.chat,"input":"child work","request_key":"child-once"}),
            )),
            2 => {
                let result: Value =
                    serde_json::from_str(messages.last().unwrap()["content"].as_str().unwrap())
                        .unwrap();
                let id = result["id"].as_str().unwrap().to_owned();
                self.child = Some(id.clone());
                Ok(tool(
                    "task_await",
                    json!({"task_id":id,"deadline":now_ms()+60_000}),
                ))
            }
            _ => panic!("model called after await instead of suspending"),
        }
    }
}
fn suspend(home: &Path, s: &mut Scheduler, chat: &str, parent: &str) -> String {
    let task = claim_work(s, parent);
    let mut model = Suspend {
        step: 0,
        chat: chat.into(),
        child: None,
    };
    execute(home, task, &mut model, &CancellationToken::new()).unwrap();
    assert_eq!(model.step, 2);
    assert_eq!(
        s.store.get(&format!("work:{parent}")).unwrap().state,
        "suspended"
    );
    model.child.unwrap()
}
struct Continue {
    called: bool,
    kind: &'static str,
}
impl ModelClient for Continue {
    fn complete(
        &mut self,
        _: &ResolvedAgent,
        messages: &[Value],
        _: &[Value],
    ) -> crate::Result<Value> {
        assert!(!self.called);
        self.called = true;
        assert!(
            messages.last().unwrap()["content"]
                .as_str()
                .unwrap()
                .contains(self.kind)
        );
        let completed: Vec<_> = messages.iter().filter(|m| m["role"] == "tool").collect();
        assert_eq!(
            completed.len(),
            2,
            "restore successful tools without replay"
        );
        assert_eq!(completed[0]["tool_call_id"], "task_request");
        assert_eq!(completed[1]["tool_call_id"], "task_await");
        Ok(json!({"role":"assistant","content":"integrated result"}))
    }
}
#[test]
fn native_wait_restarts_and_continues_once_without_replaying_tools() {
    let (home, mut s, chat, parent) = setup();
    let child = suspend(home.path(), &mut s, &chat, &parent);
    crate::work::reconcile(home.path()).unwrap();
    assert!(matches!(
        WorkLedger::open(home.path())
            .unwrap()
            .get(&parent)
            .unwrap()
            .state,
        WorkState::WaitingDependency { .. }
    ));
    // A different task for the same principal can claim the released slot.
    let other = claim(&mut s, "a");
    let mut done = other.clone();
    done.state = "completed".into();
    s.store.update(&done, now_ms()).unwrap();
    drop(s);
    let mut s = Scheduler::open(home.path(), 3).unwrap();
    s.scan().unwrap();
    let task = claim_work(&mut s, &child);
    execute(
        home.path(),
        task,
        &mut Answer(json!({"role":"assistant","content":"child done"})),
        &CancellationToken::new(),
    )
    .unwrap();
    // Child is newer than parent: a second reconciliation observes its terminal commit.
    crate::work::reconcile(home.path()).unwrap();
    crate::work::reconcile(home.path()).unwrap();
    let ledger = WorkLedger::open(home.path()).unwrap();
    assert_eq!(ledger.get(&parent).unwrap().state, WorkState::Queued);
    let revision = ledger.get(&parent).unwrap().revision;
    crate::work::reconcile(home.path()).unwrap();
    assert_eq!(ledger.get(&parent).unwrap().revision, revision);
    s.scan().unwrap();
    for t in s
        .store
        .candidates()
        .unwrap()
        .iter()
        .filter(|t| t.work_notification && t.work_id.as_deref() == Some(&child))
    {
        assert!(!crate::work::runnable(home.path(), &s.store, t).unwrap());
    }
    let task = claim_work(&mut s, &parent);
    let mut model = Continue {
        called: false,
        kind: "resolved",
    };
    execute(home.path(), task, &mut model, &CancellationToken::new()).unwrap();
    assert!(model.called);
    crate::work::reconcile(home.path()).unwrap();
    assert!(matches!(
        ledger.get(&parent).unwrap().state,
        WorkState::Succeeded { .. }
    ));
    let runs = crate::RunStore::open(home.path())
        .unwrap()
        .list(20)
        .unwrap();
    assert_eq!(runs.len(), 3);
    assert_eq!(
        runs.iter()
            .filter(|r| r.state == RunState::Suspended)
            .count(),
        1
    );
    assert!(ledger.inbox("a", 0, 200).unwrap().is_empty());
}
#[test]
fn checkpoint_cannot_resume_until_old_execution_releases_slot() {
    let (home, mut s, chat, parent) = setup();
    let child = suspend(home.path(), &mut s, &chat, &parent);
    // Simulate the gap between checkpoint commit and scheduler's terminal update.
    let mut adapter = s.store.get(&format!("work:{parent}")).unwrap();
    adapter.state = "responding".into();
    crate::schedule_store::save_task(&s.store.connection, &adapter).unwrap();
    let mut ledger = WorkLedger::open(home.path()).unwrap();
    let c = ledger.get(&child).unwrap();
    let c = ledger
        .record_running(&child, c.revision, "early-result")
        .unwrap();
    ledger
        .record_outcome(
            &child,
            c.revision,
            "early-result",
            crate::work::WorkOutcome::Failed("failed early".into()),
        )
        .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    assert!(matches!(
        ledger.get(&parent).unwrap().state,
        WorkState::Running { .. }
    ));
    adapter.state = "suspended".into();
    crate::schedule_store::save_task(&s.store.connection, &adapter).unwrap();
    crate::work::reconcile(home.path()).unwrap();
    assert_eq!(ledger.get(&parent).unwrap().state, WorkState::Queued);
}
#[test]
fn timeout_resumes_with_event_but_does_not_consume_late_result() {
    let (home, mut s, chat, parent) = setup();
    let child = suspend(home.path(), &mut s, &chat, &parent);
    s.store
        .connection
        .execute("UPDATE work_waits SET deadline=0", [])
        .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    assert!(!crate::work::wait::consumes_notice(&s.store.connection, &child).unwrap());
    let task = claim_work(&mut s, &parent);
    let mut model = Continue {
        called: false,
        kind: "deadline",
    };
    execute(home.path(), task, &mut model, &CancellationToken::new()).unwrap();
    assert!(model.called);
    assert_eq!(
        WorkLedger::open(home.path())
            .unwrap()
            .get(&child)
            .unwrap()
            .state,
        WorkState::Queued
    );
}
#[test]
fn end_and_context_reset_do_not_resume_suspended_work() {
    for reset in [false, true] {
        let (home, mut s, chat, parent) = setup();
        suspend(home.path(), &mut s, &chat, &parent);
        if reset {
            reset_conversation_context(home.path(), &chat, "user").unwrap();
        } else {
            s.store.control(&chat, "end", false, now_ms()).unwrap();
        }
        crate::work::reconcile(home.path()).unwrap();
        assert!(matches!(
            WorkLedger::open(home.path())
                .unwrap()
                .get(&parent)
                .unwrap()
                .state,
            WorkState::Canceled { .. }
        ));
        assert_eq!(
            crate::RunStore::open(home.path())
                .unwrap()
                .list(10)
                .unwrap()
                .len(),
            1
        );
    }
}
#[test]
fn await_is_not_exposed_to_local_or_harness_hosts() {
    let (home, mut s, _, parent) = setup();
    let actor = Actor {
        principal: PrincipalId::new("a"),
        role: Role::Participant,
        automated_depth: Some(1),
    };
    let task = claim_work(&mut s, &parent);
    for control in [
        TaskControl::local(home.path(), actor.clone()),
        TaskControl::hosted(home.path(), actor.clone(), task),
    ] {
        let service = ToolService::open(home.path(), actor.clone())
            .unwrap()
            .with_work_control(Box::new(control));
        assert!(!service.schemas().iter().any(|v| v["name"] == "task_await"));
        assert!(
            service
                .call(
                    "task_await",
                    json!({"task_id":"no","deadline":now_ms()+1000})
                )
                .is_err()
        );
    }
}

#[test]
fn result_before_await_is_not_lost_including_unknown_outcome() {
    struct Early {
        script: Suspend,
        home: PathBuf,
        unknown: bool,
    }
    impl ModelClient for Early {
        fn complete(
            &mut self,
            agent: &ResolvedAgent,
            messages: &[Value],
            tools: &[Value],
        ) -> crate::Result<Value> {
            if self.script.step == 1 {
                let value: Value =
                    serde_json::from_str(messages.last().unwrap()["content"].as_str().unwrap())
                        .unwrap();
                let id = value["id"].as_str().unwrap();
                let mut ledger = WorkLedger::open(&self.home).unwrap();
                let c = ledger.get(id).unwrap();
                let c = ledger.record_running(id, c.revision, "early").unwrap();
                ledger
                    .record_outcome(
                        id,
                        c.revision,
                        "early",
                        if self.unknown {
                            crate::work::WorkOutcome::Unknown("connection lost".into())
                        } else {
                            crate::work::WorkOutcome::Succeeded("done already".into())
                        },
                    )
                    .unwrap();
            }
            self.script.complete(agent, messages, tools)
        }
    }
    for unknown in [false, true] {
        let (home, mut s, chat, parent) = setup();
        let task = claim_work(&mut s, &parent);
        let mut model = Early {
            script: Suspend {
                step: 0,
                chat,
                child: None,
            },
            home: home.path().into(),
            unknown,
        };
        execute(home.path(), task, &mut model, &CancellationToken::new()).unwrap();
        crate::work::reconcile(home.path()).unwrap();
        let item = WorkLedger::open(home.path()).unwrap().get(&parent).unwrap();
        assert_eq!(item.state, WorkState::Queued);
        let outcome =
            crate::work::wait::outcome(home.path(), item.resume_from.as_deref().unwrap()).unwrap();
        assert_eq!(
            outcome["state"],
            if unknown {
                "outcome_unknown"
            } else {
                "succeeded"
            }
        );
        assert_eq!(outcome["kind"], "resolved");
    }
}

#[test]
fn changed_configuration_fails_continuation_before_model_call() {
    let (home, mut s, chat, parent) = setup();
    suspend(home.path(), &mut s, &chat, &parent);
    s.store
        .connection
        .execute("UPDATE work_waits SET deadline=0", [])
        .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    AgentStore::new(home.path())
        .update_in(&agt_core::CategoryName::default(), "a", |c| {
            c.builtin_tools.clear();
            Ok(())
        })
        .unwrap();
    let task = claim_work(&mut s, &parent);
    let mut model = Continue {
        called: false,
        kind: "deadline",
    };
    execute(home.path(), task, &mut model, &CancellationToken::new()).unwrap();
    assert!(!model.called);
    crate::work::reconcile(home.path()).unwrap();
    assert!(matches!(
        WorkLedger::open(home.path())
            .unwrap()
            .get(&parent)
            .unwrap()
            .state,
        WorkState::Failed { .. }
    ));
}

#[test]
fn mixed_await_batch_is_rejected_before_any_side_effect() {
    let (home, mut s, chat, parent) = setup();
    let task = claim_work(&mut s, &parent);
    let request = tool(
        "task_request",
        json!({"assignee":"b","conversation":chat,"input":"must not run","request_key":"forbidden"}),
    );
    let wait = tool(
        "task_await",
        json!({"task_id":"missing","deadline":now_ms()+60_000}),
    );
    execute(home.path(),task,&mut Answer(json!({"role":"assistant","tool_calls":[request["tool_calls"][0],wait["tool_calls"][0]]})),&CancellationToken::new()).unwrap();
    let count: i64 = s
        .store
        .connection
        .query_row("SELECT count(*) FROM work_items", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    let count: i64 = s
        .store
        .connection
        .query_row("SELECT count(*) FROM work_waits", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn stopped_binding_blocks_admission_of_ready_continuation() {
    let (home, mut s, chat, parent) = setup();
    suspend(home.path(), &mut s, &chat, &parent);
    BindingStore::open(home.path())
        .unwrap()
        .set_enabled(&PrincipalId::new("a"), false)
        .unwrap();
    s.store
        .connection
        .execute("UPDATE work_waits SET deadline=0", [])
        .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    let task = s.store.get(&format!("work:{parent}")).unwrap();
    assert!(!crate::work::runnable(home.path(), &s.store, &task).unwrap());
    assert_eq!(
        WorkLedger::open(home.path())
            .unwrap()
            .get(&parent)
            .unwrap()
            .state,
        WorkState::Queued
    );
    BindingStore::open(home.path())
        .unwrap()
        .set_enabled(&PrincipalId::new("a"), true)
        .unwrap();
    assert!(crate::work::runnable(home.path(), &s.store, &task).unwrap());
}

#[test]
fn await_checks_existing_barrier_edges_and_rolls_back_cycles() {
    struct Cycle {
        script: Suspend,
        home: PathBuf,
        parent: String,
    }
    impl ModelClient for Cycle {
        fn complete(
            &mut self,
            agent: &ResolvedAgent,
            messages: &[Value],
            tools: &[Value],
        ) -> crate::Result<Value> {
            if self.script.step == 1 {
                let result: Value =
                    serde_json::from_str(messages.last().unwrap()["content"].as_str().unwrap())
                        .unwrap();
                // Represents an existing barrier path back to the current work item.
                ScheduleStore::open(&self.home)
                    .unwrap()
                    .connection
                    .execute(
                        "INSERT INTO work_dependencies VALUES(?1,?2)",
                        rusqlite::params![result["id"].as_str().unwrap(), self.parent],
                    )
                    .unwrap();
            }
            if self.script.step == 2 {
                let result: Value =
                    serde_json::from_str(messages.last().unwrap()["content"].as_str().unwrap())
                        .unwrap();
                assert_eq!(result["error"]["code"], "operation_rejected");
                return Ok(json!({"role":"assistant","content":"cycle rejected"}));
            }
            self.script.complete(agent, messages, tools)
        }
    }
    let (home, mut s, chat, parent) = setup();
    let task = claim_work(&mut s, &parent);
    let mut model = Cycle {
        script: Suspend {
            step: 0,
            chat,
            child: None,
        },
        home: home.path().into(),
        parent,
    };
    execute(home.path(), task, &mut model, &CancellationToken::new()).unwrap();
    let count: i64 = s
        .store
        .connection
        .query_row("SELECT count(*) FROM work_waits", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn checkpoint_crash_gap_recovers_but_uncheckpointed_intent_is_unknown() {
    for checkpoint_committed in [false, true] {
        let (home, mut s, chat, parent) = setup();
        let child = suspend(home.path(), &mut s, &chat, &parent);
        let mut adapter = s.store.get(&format!("work:{parent}")).unwrap();
        adapter.state = "responding".into();
        crate::schedule_store::save_task(&s.store.connection, &adapter).unwrap();
        if !checkpoint_committed {
            let runs = crate::RunStore::open(home.path()).unwrap();
            let mut record = runs.get(adapter.run_id.as_deref().unwrap()).unwrap();
            record.state = RunState::Running;
            runs.save(&record).unwrap();
            rusqlite::Connection::open(home.path().join("runtime/runs.db"))
                .unwrap()
                .execute("DELETE FROM native_checkpoints", [])
                .unwrap();
        }
        drop(s);
        let s = Scheduler::open(home.path(), 3).unwrap();
        crate::work::reconcile(home.path()).unwrap();
        let item = WorkLedger::open(home.path()).unwrap().get(&parent).unwrap();
        if checkpoint_committed {
            assert!(matches!(item.state, WorkState::WaitingDependency { .. }));
        } else {
            assert!(matches!(item.state, WorkState::OutcomeUnknown { .. }));
            assert!(!crate::work::wait::consumes_notice(&s.store.connection, &child).unwrap());
        }
        assert_eq!(
            crate::RunStore::open(home.path())
                .unwrap()
                .list(20)
                .unwrap()
                .len(),
            1
        );
    }
}
