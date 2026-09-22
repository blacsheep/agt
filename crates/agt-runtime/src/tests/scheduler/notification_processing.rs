use super::*;

fn completed_work() -> (tempfile::TempDir, Scheduler, String, ScheduledTask) {
    let (home, user, chat) = fixture();
    mount_tasks(home.path(), "a");
    let mut scheduler = begin(home.path(), &user, &chat);
    let id = delegate(&mut scheduler, &chat);
    let task = claim_work(&mut scheduler, &id);
    execute(
        home.path(),
        task,
        &mut Answer(json!({"role":"assistant","content":"done"})),
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
    (home, scheduler, id, notice)
}

#[test]
fn prose_and_silence_do_not_discharge_report_obligation() {
    for output in ["I processed the result.", "[[silent]]"] {
        let (home, mut scheduler, id, notice) = completed_work();
        execute(
            home.path(),
            notice.clone(),
            &mut Answer(json!({"role":"assistant","content":output})),
            &CancellationToken::new(),
        )
        .unwrap();
        let saved = scheduler.store.get(&notice.id).unwrap();
        assert_eq!(saved.state, "failed");
        assert!(saved.reason.unwrap().starts_with("task_report_required:"));
        let run = crate::RunStore::open(home.path())
            .unwrap()
            .get(saved.run_id.as_ref().unwrap())
            .unwrap();
        assert_eq!(run.state, RunState::Completed); // Execution succeeded; responsibility did not.
        scheduler.tick(false).unwrap();
        let ledger = WorkLedger::open(home.path()).unwrap();
        assert!(
            ledger
                .inbox("a", 0, 200)
                .unwrap()
                .iter()
                .any(|e| e.work_id == id)
        );
        assert!(
            scheduler
                .store
                .candidates()
                .unwrap()
                .iter()
                .all(|t| t.id != notice.id)
        );
        let obligation =
            crate::work::notification_obligation(home.path(), &scheduler.store, &notice)
                .unwrap()
                .unwrap();
        assert_eq!(obligation["report_required"], true);
    }
}

struct ReportThenSilent {
    step: usize,
}
impl ModelClient for ReportThenSilent {
    fn complete(
        &mut self,
        _: &ResolvedAgent,
        messages: &[Value],
        _: &[Value],
    ) -> crate::Result<Value> {
        self.step += 1;
        if self.step == 1 {
            let input: Value = messages
                .iter()
                .filter(|m| m["role"] == "user")
                .find_map(|m| {
                    serde_json::from_str::<Value>(m["content"].as_str()?)
                        .ok()
                        .filter(|v| v.get("report_obligation").is_some())
                })
                .unwrap();
            let obligation = &input["report_obligation"];
            assert_eq!(obligation["report_required"], true);
            Ok(
                json!({"role":"assistant","tool_calls":[{"id":"report","type":"function","function":{
                    "name":"framework_task_report",
                    "arguments":json!({"task_id":obligation["task_id"],"expected_revision":obligation["expected_revision"],"body":"Result reviewed and reported."}).to_string()
                }}]}),
            )
        } else {
            let result: Value =
                serde_json::from_str(messages.last().unwrap()["content"].as_str().unwrap())
                    .unwrap();
            assert!(result.get("code").is_none(), "{result}");
            Ok(json!({"role":"assistant","content":"[[silent]]"}))
        }
    }
}

#[test]
fn committed_report_allows_silent_completion_and_survives_reconcile() {
    let (home, mut scheduler, id, notice) = completed_work();
    execute(
        home.path(),
        notice.clone(),
        &mut ReportThenSilent { step: 0 },
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(scheduler.store.get(&notice.id).unwrap().state, "completed");
    scheduler.tick(false).unwrap();
    crate::work::reconcile(home.path()).unwrap();
    crate::work::reconcile(home.path()).unwrap();
    let ledger = WorkLedger::open(home.path()).unwrap();
    assert!(
        !ledger
            .inbox("a", 0, 200)
            .unwrap()
            .iter()
            .any(|e| e.work_id == id)
    );
    let messages = im(home.path())
        .unwrap()
        .list_messages(
            &ConversationId::new(&notice.conversation),
            &PrincipalId::new("user"),
            MessageListOptions {
                limit: 200,
                before: None,
                after: None,
            },
        )
        .unwrap()
        .messages;
    assert_eq!(
        messages
            .iter()
            .filter(|m| m.body == "Result reviewed and reported.")
            .count(),
        1
    );
    assert!(!messages.iter().any(|m| m.body == "[[silent]]"));
}

#[test]
fn report_context_rejects_wrong_requester_or_conversation() {
    let (home, scheduler, _, mut notice) = completed_work();
    notice.principal = "b".into();
    assert!(crate::work::notification_obligation(home.path(), &scheduler.store, &notice).is_err());
    notice.principal = "a".into();
    notice.conversation = "another-conversation".into();
    assert!(crate::work::notification_obligation(home.path(), &scheduler.store, &notice).is_err());
}

#[test]
fn report_after_failed_notification_clears_pending_duty_without_model_retry() {
    let (home, mut scheduler, id, notice) = completed_work();
    execute(
        home.path(),
        notice.clone(),
        &mut Answer(json!({"role":"assistant","content":"[[silent]]"})),
        &CancellationToken::new(),
    )
    .unwrap();
    scheduler.tick(false).unwrap();
    let control = TaskControl::local(
        home.path(),
        Actor {
            principal: PrincipalId::new("user"),
            role: Role::Manager,
            automated_depth: None,
        },
    );
    let detail = control.call("task_get", json!({"task_id":id})).unwrap();
    assert_eq!(
        detail["notification_blocked_reason"],
        "task_report_required"
    );
    control.call("task_report", json!({"task_id":id,"expected_revision":detail["revision"],"body":"Reviewed after notification stopped."})).unwrap();
    crate::work::reconcile(home.path()).unwrap();
    let detail = control.call("task_get", json!({"task_id":id})).unwrap();
    assert_eq!(detail["requester_notification"], "handled");
    assert_eq!(detail["report"]["state"], "published");
    assert_eq!(scheduler.store.get(&notice.id).unwrap().state, "failed"); // Preserve execution history.
}
