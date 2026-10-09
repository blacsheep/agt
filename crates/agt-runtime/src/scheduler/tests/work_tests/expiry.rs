use super::*;

#[test]
fn expiry_preserves_running_work_and_result_but_blocks_new_admission() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let id = delegate(&mut scheduler, &chat);
    let task = claim_work(&mut scheduler, &id);
    let mut request = RunRequest::new("b", "in flight");
    request.request_key = crate::work::run_key(&id);
    let mut runs = crate::RunStore::open(home.path()).unwrap();
    let (mut run, _) = runs.claim(&request, None).unwrap();
    crate::work::bind_run(home.path(), &task, &run).unwrap();
    scheduler
        .store
        .connection
        .execute("UPDATE discussions SET expires=?1", [now_ms() - 1])
        .unwrap();
    scheduler.store.expire(now_ms()).unwrap();
    assert!(scheduler.store.authorized(&task, now_ms()).unwrap());
    assert!(scheduler.store.debit(&task, now_ms()).unwrap());
    guarded(home.path(), &task, || Ok(())).unwrap();
    crate::work::reconcile(home.path()).unwrap();
    assert!(matches!(
        WorkLedger::open(home.path())
            .unwrap()
            .get(&id)
            .unwrap()
            .state,
        WorkState::Running { .. }
    ));
    assert!(results(home.path(), &chat).is_empty());
    run.state = crate::RunState::Completed;
    run.output = Some("completed after expiry".into());
    runs.save(&run).unwrap();
    crate::work::reconcile(home.path()).unwrap();
    assert!(matches!(
        WorkLedger::open(home.path())
            .unwrap()
            .get(&id)
            .unwrap()
            .state,
        WorkState::Succeeded { .. }
    ));
    assert_eq!(results(home.path(), &chat).len(), 1);
    let control = TaskControl::local(
        home.path(),
        Actor {
            principal: PrincipalId::new("user"),
            role: Role::Manager,
            automated_depth: None,
        },
    );
    assert_eq!(
        control.call("task_get", json!({"task_id":id})).unwrap()["notification_blocked_reason"],
        "discussion_expired"
    );
    scheduler.scan().unwrap();
    let notice = scheduler
        .store
        .tasks()
        .unwrap()
        .into_iter()
        .find(|t| t.work_notification)
        .unwrap();
    assert!(
        scheduler
            .store
            .claim(&notice.id, &scheduler.owner, now_ms())
            .unwrap()
            .is_none()
    );
    scheduler
        .store
        .connection
        .execute("UPDATE discussions SET state='ended'", [])
        .unwrap();
    assert!(!scheduler.store.authorized(&task, now_ms()).unwrap());
    assert!(guarded(home.path(), &task, || Ok(())).is_err());
    assert!(scheduler.store.owns_execution(&task, now_ms()).unwrap());
}

#[test]
fn explicit_stop_waits_for_live_executor_confirmation() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let id = delegate(&mut scheduler, &chat);
    let task = claim_work(&mut scheduler, &id);
    let mut request = RunRequest::new("b", "in flight");
    request.request_key = crate::work::run_key(&id);
    let mut runs = crate::RunStore::open(home.path()).unwrap();
    let (mut run, _) = runs.claim(&request, None).unwrap();
    crate::work::bind_run(home.path(), &task, &run).unwrap();
    scheduler.store.control(&chat, "end", now_ms()).unwrap();
    assert!(guarded(home.path(), &task, || Ok(())).is_err());
    crate::work::reconcile(home.path()).unwrap();
    assert!(matches!(
        WorkLedger::open(home.path())
            .unwrap()
            .get(&id)
            .unwrap()
            .state,
        WorkState::Running { .. }
    ));
    run.state = crate::RunState::Canceled;
    let mut termination = crate::Termination::new(crate::StopCode::DiscussionStopped);
    termination.confirmation = "engine_interrupted".into();
    termination.confirmed_at = Some(now_ms());
    run.termination = Some(termination);
    runs.save(&run).unwrap();
    crate::work::reconcile(home.path()).unwrap();
    assert!(matches!(
        WorkLedger::open(home.path())
            .unwrap()
            .get(&id)
            .unwrap()
            .state,
        WorkState::Canceled { .. }
    ));
    assert!(results(home.path(), &chat).is_empty()); // End still fences publication.
}
