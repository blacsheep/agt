use super::*;
use crate::work::{TaskControl, WorkLedger, WorkState};
use agt_service::WorkControl;

pub(super) fn setup() -> (tempfile::TempDir, Scheduler, String, String) {
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
pub(super) fn claim_work(s: &mut Scheduler, id: &str) -> ScheduledTask {
    s.store
        .claim(&format!("work:{id}"), &s.owner, now_ms())
        .unwrap()
        .unwrap()
}
pub(super) fn tool(name: &str, args: Value) -> Value {
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
pub(super) fn suspend(home: &Path, s: &mut Scheduler, chat: &str, parent: &str) -> String {
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
        TaskState::Suspended
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
    done.state = TaskState::Completed;
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
    adapter.state = TaskState::Responding;
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
    adapter.state = TaskState::Suspended;
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
            s.store.control(&chat, "end", now_ms()).unwrap();
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
fn await_is_not_exposed_without_resumable_execution_host() {
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
        adapter.state = TaskState::Responding;
        crate::schedule_store::save_task(&s.store.connection, &adapter).unwrap();
        if !checkpoint_committed {
            let runs = crate::RunStore::open(home.path()).unwrap();
            let mut record = runs.get(adapter.run_id.as_deref().unwrap()).unwrap();
            record.state = RunState::Running;
            runs.save(&record).unwrap();
            rusqlite::Connection::open(home.path().join("runtime/runs.db"))
                .unwrap()
                .execute("DELETE FROM run_checkpoints", [])
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

#[test]
fn multiple_dependencies_wait_for_all_and_unknown_wakes_early() {
    struct Many {
        step: usize,
        chat: String,
        ids: Vec<String>,
    }
    impl ModelClient for Many {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            m: &[Value],
            _: &[Value],
        ) -> crate::Result<Value> {
            self.step += 1;
            if self.step == 1 {
                let mut a = tool(
                    "task_request",
                    json!({"assignee":"b","conversation":self.chat,"input":"first","request_key":"first"}),
                );
                let mut b = tool(
                    "task_request",
                    json!({"assignee":"b","conversation":self.chat,"input":"second","request_key":"second"}),
                );
                a["tool_calls"][0]["id"] = json!("first");
                b["tool_calls"][0]["id"] = json!("second");
                return Ok(
                    json!({"role":"assistant","tool_calls":[a["tool_calls"][0],b["tool_calls"][0]]}),
                );
            }
            assert_eq!(self.step, 2);
            self.ids = m
                .iter()
                .filter(|x| x["role"] == "tool")
                .map(|x| {
                    serde_json::from_str::<Value>(x["content"].as_str().unwrap()).unwrap()["id"]
                        .as_str()
                        .unwrap()
                        .into()
                })
                .collect();
            Ok(tool(
                "task_await",
                json!({"task_ids":self.ids,"deadline":now_ms()+60000}),
            ))
        }
    }
    for unknown in [false, true] {
        let (home, mut s, chat, parent) = setup();
        let task = claim_work(&mut s, &parent);
        let mut model = Many {
            step: 0,
            chat,
            ids: vec![],
        };
        execute(home.path(), task, &mut model, &CancellationToken::new()).unwrap();
        crate::work::reconcile(home.path()).unwrap();
        let mut ledger = WorkLedger::open(home.path()).unwrap();
        let first = &model.ids[0];
        let c = ledger.get(first).unwrap();
        let c = ledger
            .record_running(first, c.revision, "first-run")
            .unwrap();
        ledger
            .record_outcome(
                first,
                c.revision,
                "first-run",
                if unknown {
                    crate::work::WorkOutcome::Unknown("lost".into())
                } else {
                    crate::work::WorkOutcome::Succeeded("done".into())
                },
            )
            .unwrap();
        crate::work::reconcile(home.path()).unwrap();
        if unknown {
            let parent = ledger.get(&parent).unwrap();
            assert_eq!(parent.state, WorkState::Queued);
            assert_eq!(
                crate::work::wait::outcome(home.path(), parent.resume_from.as_deref().unwrap())
                    .unwrap()["kind"],
                "attention"
            );
            assert!(
                !crate::work::wait::consumes_notice(&s.store.connection, &model.ids[1]).unwrap()
            );
        } else {
            assert!(matches!(
                ledger.get(&parent).unwrap().state,
                WorkState::WaitingDependency { .. }
            ));
            let second = &model.ids[1];
            let c = ledger.get(second).unwrap();
            let c = ledger
                .record_running(second, c.revision, "second-run")
                .unwrap();
            ledger
                .record_outcome(
                    second,
                    c.revision,
                    "second-run",
                    crate::work::WorkOutcome::Failed("failed".into()),
                )
                .unwrap();
            crate::work::reconcile(home.path()).unwrap();
            assert_eq!(ledger.get(&parent).unwrap().state, WorkState::Queued);
        }
    }
}

#[cfg(unix)]
#[test]
fn codex_wait_handoffs_only_completed_turn_and_resumes_same_thread() {
    for failed in [false, true] {
        let (home, user, chat) = external_fixture();
        AgentStore::new(home.path())
            .update_in(&agt_core::CategoryName::default(), "a", |c| {
                c.builtin_tools
                    .push(agt_agent::BuiltinTool::TaskCollaboration);
                Ok(())
            })
            .unwrap();
        let executable = home.path().join("codex-fixture");
        let script=std::fs::read_to_string(&executable).unwrap()
            .replace("if 'TOOL' in text:","if 'AGT task continuation.' not in text:")
            .replace("'tool':'framework_resource_list','arguments':{'kind':'agent'}",&format!("'tool':'framework_task_request','arguments':{}",json!({"assignee":"b","conversation":chat,"input":"child work","request_key":"codex-child"})))
            .replace(" elif id=='tool':finish(thread,turn,'tool completed')",&format!(" elif id=='tool':\n  child=json.loads(v['result']['contentItems'][0]['text'])['id']\n  emit({{'id':'await','method':'item/tool/call','params':{{'threadId':thread,'turnId':turn,'tool':'framework_task_await','arguments':{{'task_id':child,'deadline':int(time.time()*1000)+60000}}}}}})\n elif id=='await':\n  assert v['result']['success'], v\n  note('turn/completed',{{'threadId':thread,'turn':{{'id':turn,'status':'{}'}}}})",if failed {"failed"} else {"completed"}));
        std::fs::write(&executable, script).unwrap();
        let mut s = begin(home.path(), &user, &chat);
        let source = s.store.tasks().unwrap()[0].message.clone();
        let control = TaskControl::local(
            home.path(),
            Actor {
                principal: PrincipalId::new("user"),
                role: Role::Manager,
                automated_depth: None,
            },
        );
        let item=control.call("task_request",json!({"assignee":"a","conversation":chat,"input":"coordinate work","request_key":"codex-parent","source_message":source})).unwrap();
        let parent = item["id"].as_str().unwrap();
        crate::work::reconcile(home.path()).unwrap();
        s.scan().unwrap();
        let task = claim_work(&mut s, parent);
        execute(
            home.path(),
            task,
            &mut Answer(json!({})),
            &CancellationToken::new(),
        )
        .unwrap();
        crate::work::reconcile(home.path()).unwrap();
        let ledger = WorkLedger::open(home.path()).unwrap();
        if failed {
            assert!(matches!(
                ledger.get(parent).unwrap().state,
                WorkState::Failed { .. }
            ));
            continue;
        }
        assert!(matches!(
            ledger.get(parent).unwrap().state,
            WorkState::WaitingDependency { .. }
        ));
        let child = ledger
            .pending_execution()
            .unwrap()
            .into_iter()
            .find(|i| i.request.assignee == "b")
            .unwrap();
        s.scan().unwrap();
        let task = claim_work(&mut s, &child.id);
        execute(
            home.path(),
            task,
            &mut Answer(json!({"role":"assistant","content":"child complete"})),
            &CancellationToken::new(),
        )
        .unwrap();
        crate::work::reconcile(home.path()).unwrap();
        crate::work::reconcile(home.path()).unwrap();
        let task = claim_work(&mut s, parent);
        execute(
            home.path(),
            task,
            &mut Answer(json!({})),
            &CancellationToken::new(),
        )
        .unwrap();
        crate::work::reconcile(home.path()).unwrap();
        assert!(
            matches!(
                ledger.get(parent).unwrap().state,
                WorkState::Succeeded { .. }
            ),
            "{:?}",
            ledger.get(parent).unwrap()
        );
        let log = std::fs::read_to_string(home.path().join("protocol.jsonl")).unwrap();
        let frames: Vec<Value> = log
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(
            frames
                .iter()
                .filter(|f| f["method"] == "thread/start")
                .count(),
            1
        );
        assert_eq!(
            frames
                .iter()
                .filter(|f| f["method"] == "thread/resume")
                .count(),
            1
        );
    }
}

#[test]
fn refreshed_skills_survive_checkpoint_resume_without_replaying_configuration_write() {
    let (home, mut scheduler, chat, parent) = setup();
    agt_service::Resources::new(home.path())
        .write(
            "skill",
            "live-skill",
            true,
            json!({"description":"checkpoint skill marker"}),
        )
        .unwrap();
    AgentStore::new(home.path())
        .update_in(&agt_core::CategoryName::default(), "a", |config| {
            config
                .builtin_tools
                .push(agt_agent::BuiltinTool::ResourceManagement);
            Ok(())
        })
        .unwrap();
    struct MountThenSuspend {
        mounted: bool,
        inner: Suspend,
    }
    impl ModelClient for MountThenSuspend {
        fn complete(
            &mut self,
            agent: &ResolvedAgent,
            messages: &[Value],
            schemas: &[Value],
        ) -> crate::Result<Value> {
            if !self.mounted {
                self.mounted = true;
                return Ok(tool(
                    "resource_write",
                    json!({"kind":"agent","reference":"a","create":false,
                    "data":{},"add":{"skills":["live-skill"]}}),
                ));
            }
            assert!(
                messages[0]["content"]
                    .as_str()
                    .unwrap()
                    .contains("checkpoint skill marker")
            );
            self.inner.complete(agent, messages, schemas)
        }
    }
    let mut model = MountThenSuspend {
        mounted: false,
        inner: Suspend {
            step: 0,
            chat,
            child: None,
        },
    };
    let task = claim_work(&mut scheduler, &parent);
    execute(home.path(), task, &mut model, &CancellationToken::new()).unwrap();
    let child = model.inner.child.unwrap();
    crate::work::reconcile(home.path()).unwrap();
    scheduler.scan().unwrap();
    let task = claim_work(&mut scheduler, &child);
    execute(
        home.path(),
        task,
        &mut Answer(json!({"role":"assistant","content":"child done"})),
        &CancellationToken::new(),
    )
    .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    crate::work::reconcile(home.path()).unwrap();
    scheduler.scan().unwrap();
    struct ResumeSkill {
        called: bool,
    }
    impl ModelClient for ResumeSkill {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            messages: &[Value],
            _: &[Value],
        ) -> crate::Result<Value> {
            assert!(!self.called);
            self.called = true;
            assert!(
                messages[0]["content"]
                    .as_str()
                    .unwrap()
                    .contains("checkpoint skill marker")
            );
            assert_eq!(
                messages
                    .iter()
                    .filter(|m| m["role"] == "tool" && m["tool_call_id"] == "resource_write")
                    .count(),
                1
            );
            Ok(json!({"role":"assistant","content":"resumed with loaded skill"}))
        }
    }
    let mut model = ResumeSkill { called: false };
    let task = claim_work(&mut scheduler, &parent);
    execute(home.path(), task, &mut model, &CancellationToken::new()).unwrap();
    assert!(model.called);
    crate::work::reconcile(home.path()).unwrap();
    assert!(matches!(
        WorkLedger::open(home.path())
            .unwrap()
            .get(&parent)
            .unwrap()
            .state,
        WorkState::Succeeded { .. }
    ));
}
