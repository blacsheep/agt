use super::*;
use crate::work::{TaskControl, WorkLedger, WorkState};
use agt_service::WorkControl;

fn delegate(scheduler: &mut Scheduler, chat: &str) -> String {
    let mut lead = claim(scheduler, "a");
    let control = TaskControl::hosted(
        &scheduler.home,
        Actor {
            principal: PrincipalId::new("a"),
            role: Role::Participant,
            automated_depth: Some(1),
        },
        lead.clone(),
    );
    let args = json!({"assignee":"b","conversation":chat,"input":"implement the requested fix","request_key":"work-1"});
    let item = control.call("task_request", args.clone()).unwrap();
    assert_eq!(item["state"], "queued");
    assert_eq!(
        control.call("task_request", args).unwrap()["id"],
        item["id"]
    );
    lead.state = "completed".into();
    scheduler.store.update(&lead, now_ms()).unwrap();
    crate::work::reconcile(&scheduler.home).unwrap();
    scheduler.scan().unwrap();
    item["id"].as_str().unwrap().into()
}
fn claim_work(scheduler: &mut Scheduler, id: &str) -> ScheduledTask {
    scheduler
        .store
        .claim(&format!("work:{id}"), &scheduler.owner, now_ms())
        .unwrap()
        .unwrap()
}
fn results(home: &Path, chat: &str) -> Vec<agt_im::Message> {
    im(home)
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
        .messages
        .into_iter()
        .filter(|m| m.metadata["work_kind"] == "result")
        .collect()
}

#[test]
fn explicit_work_is_separate_and_result_wakes_only_requester() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let id = delegate(&mut scheduler, &chat);
    let task = claim_work(&mut scheduler, &id);
    assert!(!task.decision_required);
    assert_eq!(task.triggers.len(), 1);
    // Existing ordinary chat cannot take the same principal's execution slot.
    let ordinary = scheduler
        .store
        .tasks()
        .unwrap()
        .into_iter()
        .find(|t| t.principal == "b" && t.work_id.is_none())
        .unwrap();
    assert!(
        scheduler
            .store
            .claim(&ordinary.id, &scheduler.owner, now_ms())
            .unwrap()
            .is_none()
    );
    execute(
        home.path(),
        task,
        &mut Answer(json!({"role":"assistant","content":"implemented and tested"})),
        &CancellationToken::new(),
    )
    .unwrap();
    assert!(results(home.path(), &chat).is_empty()); // Run completion precedes publication.
    crate::work::reconcile(home.path()).unwrap();
    crate::work::reconcile(home.path()).unwrap();
    assert_eq!(results(home.path(), &chat).len(), 1);
    assert!(matches!(
        WorkLedger::open(home.path())
            .unwrap()
            .get(&id)
            .unwrap()
            .state,
        WorkState::Succeeded { .. }
    ));
    scheduler.scan().unwrap();
    let notices: Vec<_> = scheduler
        .store
        .tasks()
        .unwrap()
        .into_iter()
        .filter(|t| t.work_notification)
        .collect();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].principal, "a");
    assert!(!notices[0].decision_required);
    assert!(quiet_ready(&notices[0], now_ms(), now_ms()));
}

#[test]
fn publication_ack_failure_replays_message_without_reexecuting() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let id = delegate(&mut scheduler, &chat);
    let task = claim_work(&mut scheduler, &id);
    execute(
        home.path(),
        task,
        &mut Answer(json!({"role":"assistant","content":"\"".repeat(200_000)})),
        &CancellationToken::new(),
    )
    .unwrap();
    scheduler.store.connection.execute_batch("CREATE TRIGGER reject_ack BEFORE UPDATE ON work_publications WHEN NEW.message_id IS NOT NULL BEGIN SELECT RAISE(ABORT,'injected crash window'); END;").unwrap();
    crate::work::reconcile(home.path()).unwrap();
    assert_eq!(results(home.path(), &chat).len(), 1);
    let notice = results(home.path(), &chat).remove(0);
    assert!(serde_json::to_string(&notice.body).unwrap().len() < 120 * 1024);
    assert!(notice.body.contains("truncated"));
    assert_eq!(
        WorkLedger::open(home.path())
            .unwrap()
            .pending_publications(10)
            .unwrap()
            .len(),
        1
    );
    scheduler
        .store
        .connection
        .execute_batch("DROP TRIGGER reject_ack;")
        .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    assert_eq!(results(home.path(), &chat).len(), 1);
    assert!(
        WorkLedger::open(home.path())
            .unwrap()
            .pending_publications(10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        crate::RunStore::open(home.path())
            .unwrap()
            .list(100)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn lost_run_claim_is_unknown_and_never_requeued() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let id = delegate(&mut scheduler, &chat);
    let mut task = claim_work(&mut scheduler, &id);
    let mut request = RunRequest::new("b", "side effects may have started");
    request.request_key = crate::work::run_key(&id);
    let mut runs = crate::RunStore::open(home.path()).unwrap();
    runs.claim(&request, None).unwrap(); // Crash before the work association.
    task.state = "interrupted".into();
    scheduler.store.update(&task, now_ms()).unwrap();
    crate::work::reconcile(home.path()).unwrap();
    assert!(matches!(
        WorkLedger::open(home.path())
            .unwrap()
            .get(&id)
            .unwrap()
            .state,
        WorkState::OutcomeUnknown { .. }
    ));
    assert!(!crate::work::runnable(home.path(), &scheduler.store, &task).unwrap());
    assert_eq!(results(home.path(), &chat).len(), 1);
}

#[test]
fn manual_disabled_and_reset_do_not_start_pending_work() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let id = delegate(&mut scheduler, &chat);
    let task = scheduler.store.get(&format!("work:{id}")).unwrap();
    let bindings = BindingStore::open(home.path()).unwrap();
    bindings.set_enabled(&PrincipalId::new("b"), false).unwrap();
    assert!(!crate::work::runnable(home.path(), &scheduler.store, &task).unwrap());
    scheduler.tick(false).unwrap();
    assert_eq!(
        WorkLedger::open(home.path())
            .unwrap()
            .get(&id)
            .unwrap()
            .state,
        WorkState::Queued
    );
    bindings.set_enabled(&PrincipalId::new("b"), true).unwrap();
    bindings
        .set_policy(
            &ConversationId::new(&chat),
            &PrincipalId::new("b"),
            ResponsePolicy::Manual,
        )
        .unwrap();
    assert!(!crate::work::runnable(home.path(), &scheduler.store, &task).unwrap());
    reset_conversation_context(home.path(), &chat, "user").unwrap();
    crate::work::reconcile(home.path()).unwrap();
    assert!(matches!(
        WorkLedger::open(home.path())
            .unwrap()
            .get(&id)
            .unwrap()
            .state,
        WorkState::Canceled { .. }
    ));
    assert!(results(home.path(), &chat).is_empty());
}

#[test]
fn task_access_and_creation_require_fixed_authorization() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let id = delegate(&mut scheduler, &chat);
    let actor = |name| Actor {
        principal: PrincipalId::new(name),
        role: Role::Participant,
        automated_depth: None,
    };
    assert!(
        TaskControl::local(home.path(), actor("user"))
            .call("task_get", json!({"task_id":id}))
            .is_err()
    );
    assert!(
        TaskControl::local(home.path(), actor("a"))
            .call("task_get", json!({"task_id":id}))
            .is_ok()
    );
    assert!(
        TaskControl::local(home.path(), actor("a"))
            .call(
                "task_request",
                json!({"assignee":"b","conversation":chat,"input":"untrusted", "request_key":"new"})
            )
            .is_err()
    );
    let old = scheduler
        .store
        .tasks()
        .unwrap()
        .into_iter()
        .find(|t| t.principal == "a" && t.work_id.is_none())
        .unwrap();
    assert!(
        TaskControl::hosted(home.path(), actor("a"), old)
            .call(
                "task_request",
                json!({"assignee":"b","conversation":chat,"input":"stale", "request_key":"new"})
            )
            .is_err()
    );
}

fn mount_tasks(home: &Path, agent: &str) {
    AgentStore::new(home)
        .update_in(&agt_core::CategoryName::default(), agent, |config| {
            config
                .builtin_tools
                .push(agt_agent::BuiltinTool::TaskCollaboration);
            Ok(())
        })
        .unwrap();
}

#[test]
fn native_tools_delegate_across_chats_and_report_back_to_origin() {
    struct Script {
        step: usize,
        operation: &'static str,
        args: Value,
    }
    impl ModelClient for Script {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            messages: &[Value],
            tools: &[Value],
        ) -> crate::Result<Value> {
            assert!(
                tools
                    .iter()
                    .any(|t| t["function"]["name"] == "framework_task_request")
            );
            assert!(
                !tools
                    .iter()
                    .any(|t| t["function"]["name"] == "framework_resource_write")
            );
            self.step += 1;
            if self.step == 1 {
                Ok(
                    json!({"role":"assistant","tool_calls":[{"id":"task-tool","type":"function","function":{"name":self.operation,"arguments":self.args.to_string()}}]}),
                )
            } else {
                let result: Value =
                    serde_json::from_str(messages.last().unwrap()["content"].as_str().unwrap())
                        .unwrap();
                assert!(result.get("code").is_none(), "{result}");
                Ok(json!({"role":"assistant","content":"request processed"}))
            }
        }
    }
    let (home, user, chat) = fixture();
    mount_tasks(home.path(), "a");
    let a = ToolService::open(
        home.path(),
        Actor {
            principal: PrincipalId::new("a"),
            role: Role::Participant,
            automated_depth: None,
        },
    )
    .unwrap();
    let direct = a.call("direct_create", json!({"principal":"b"})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut scheduler = begin(home.path(), &user, &chat);
    let lead = claim(&mut scheduler, "a");
    execute(home.path(),lead,&mut Script{step:0,operation:"framework_task_request",args:json!({"assignee":"b","conversation":direct,"input":"implement fix","request_key":"native"})},&CancellationToken::new()).unwrap();
    let item = WorkLedger::open(home.path())
        .unwrap()
        .pending_execution()
        .unwrap()
        .remove(0);
    assert_eq!(item.request.origin_conversation, chat);
    crate::work::reconcile(home.path()).unwrap();
    scheduler.scan().unwrap();
    let task = claim_work(&mut scheduler, &item.id);
    execute(
        home.path(),
        task,
        &mut Answer(json!({"role":"assistant","content":"fix verified"})),
        &CancellationToken::new(),
    )
    .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    scheduler.scan().unwrap();
    let notice = scheduler
        .store
        .tasks()
        .unwrap()
        .into_iter()
        .find(|t| t.work_notification)
        .unwrap();
    let notice = scheduler
        .store
        .claim(&notice.id, &scheduler.owner, now_ms())
        .unwrap()
        .unwrap();
    execute(home.path(),notice,&mut Script{step:0,operation:"framework_message_send",args:json!({"conversation":chat,"body":"The delegated fix is verified.","request_key":"report"})},&CancellationToken::new()).unwrap();
    scheduler.tick(false).unwrap();

    let messages = im(home.path())
        .unwrap()
        .list_messages(
            &ConversationId::new(&chat),
            &PrincipalId::new("user"),
            MessageListOptions {
                limit: 100,
                before: None,
                after: None,
            },
        )
        .unwrap()
        .messages;
    assert!(
        messages
            .iter()
            .any(|m| m.body == "The delegated fix is verified.")
    );
    assert!(
        WorkLedger::open(home.path())
            .unwrap()
            .inbox("a", 0, 20)
            .unwrap()
            .is_empty()
    );
}

#[cfg(unix)]
#[test]
fn codex_dynamic_task_tool_uses_same_durable_contract() {
    let (home, user, chat) = external_fixture();
    mount_tasks(home.path(), "a");
    let executable = home.path().join("codex-fixture");
    let script=std::fs::read_to_string(&executable).unwrap()
        .replace("if 'TOOL' in text:","if 'question' in text:")
        .replace("'tool':'framework_resource_list','arguments':{'kind':'agent'}", &format!("'tool':'framework_task_request','arguments':{}",json!({"assignee":"b","conversation":chat,"input":"codex delegated fix","request_key":"codex-tool"})));
    std::fs::write(&executable, script).unwrap();
    let direct = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut scheduler = begin(home.path(), &user, &direct);
    let task = claim(&mut scheduler, "a");
    execute(
        home.path(),
        task,
        &mut Answer(json!({})),
        &CancellationToken::new(),
    )
    .unwrap();
    let items = WorkLedger::open(home.path())
        .unwrap()
        .pending_execution()
        .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].request.requester, "a");
    let runs = crate::RunStore::open(home.path())
        .unwrap()
        .list(10)
        .unwrap();
    assert_eq!(runs[0].state, RunState::Completed);
    assert_eq!(runs[0].tool_failures, 0);
}

#[test]
fn failure_reaches_disabled_requester_without_triggering_a_run() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let id = delegate(&mut scheduler, &chat);
    let task = claim_work(&mut scheduler, &id);
    // A silent task is a failed explicit request, not a discarded conversation turn.
    execute(
        home.path(),
        task,
        &mut Answer(json!({"role":"assistant","content":"[[silent]]"})),
        &CancellationToken::new(),
    )
    .unwrap();
    BindingStore::open(home.path())
        .unwrap()
        .set_enabled(&PrincipalId::new("a"), false)
        .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    assert!(matches!(
        WorkLedger::open(home.path())
            .unwrap()
            .get(&id)
            .unwrap()
            .state,
        WorkState::Failed { .. }
    ));
    assert_eq!(results(home.path(), &chat).len(), 1);
    assert_eq!(
        WorkLedger::open(home.path())
            .unwrap()
            .inbox("a", 0, 20)
            .unwrap()
            .len(),
        1
    );
    scheduler.scan().unwrap();
    assert!(
        !scheduler
            .store
            .candidates()
            .unwrap()
            .iter()
            .any(|t| t.work_notification)
    );
    BindingStore::open(home.path())
        .unwrap()
        .set_enabled(&PrincipalId::new("a"), true)
        .unwrap();
    scheduler.scan().unwrap();
    assert!(
        scheduler
            .store
            .candidates()
            .unwrap()
            .iter()
            .any(|t| t.work_notification)
    );
}

#[cfg(unix)]
#[test]
fn codex_assignee_completes_work_and_reports_to_human_requester() {
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
    let item=control.call("task_request",json!({"assignee":"a","conversation":chat,"input":"implement and test","request_key":"codex-assignee","source_message":source})).unwrap();
    let id = item["id"].as_str().unwrap();
    crate::work::reconcile(home.path()).unwrap();
    scheduler.scan().unwrap();
    let task = claim_work(&mut scheduler, id);
    execute(
        home.path(),
        task,
        &mut Answer(json!({})),
        &CancellationToken::new(),
    )
    .unwrap();
    crate::work::reconcile(home.path()).unwrap();
    let result = control.call("task_get", json!({"task_id":id})).unwrap();
    assert_eq!(result["state"], "succeeded");
    assert_eq!(result["output"], "fixture final answer");
    assert_eq!(result["publication"], "published");
    assert_eq!(results(home.path(), &chat).len(), 1);
    let run = crate::RunStore::open(home.path())
        .unwrap()
        .get(result["run_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(run.execution.as_ref().unwrap()["engine"], "codex");
}
