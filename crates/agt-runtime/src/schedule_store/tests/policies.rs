use super::*;
fn queued_work(
    store: &mut ScheduleStore,
    n: i64,
    actor: &str,
    parallel: bool,
    resource: &str,
) -> String {
    let request: crate::work::WorkRequest=serde_json::from_value(serde_json::json!({"requester":"user","assignee":actor,"origin_conversation":"chat","report_to":"chat","discussion":"round","context_epoch":0,"input":"test","request_key":format!("request-{n}")})).unwrap();
    let work = crate::work::WorkLedger::enqueue_on(&store.connection, request).unwrap();
    let policy = crate::work::execution_policy::Policy {
        parallel,
        exclusive_resources: if resource.is_empty() {
            vec![]
        } else {
            vec![resource.into()]
        },
    };
    crate::work::execution_policy::create(&store.connection, &work.id, &policy).unwrap();
    let mut scheduled = task(n, actor, "chat", false);
    scheduled.work_id = Some(work.id);
    store.ingest(scheduled).unwrap();
    n.to_string()
}
#[test]
fn explicit_parallel_work_respects_atomic_resource_and_identity_rules() {
    let (_home, mut store) = fixture();
    store.config.scheduler.parallel_work = true;
    let a = queued_work(&mut store, 1, "a", true, "repo:a");
    let b = queued_work(&mut store, 2, "a", true, "repo:b");
    let c = queued_work(&mut store, 3, "b", true, "repo:a");
    let serial = queued_work(&mut store, 4, "a", false, "");
    let mut first = store.claim(&a, "scheduler:one", 0).unwrap().unwrap();
    assert!(store.claim(&b, "scheduler:one", 0).unwrap().is_some());
    assert!(store.claim(&c, "scheduler:one", 0).unwrap().is_none());
    assert!(store.claim(&serial, "scheduler:one", 0).unwrap().is_none());
    first.state = crate::TaskState::Completed;
    store.update(&first, 0).unwrap();
    assert!(store.claim(&c, "scheduler:one", 0).unwrap().is_some());
}
#[test]
fn workspace_gate_and_unlimited_budgets_are_enforced() {
    let (_home, mut store) = fixture();
    let a = queued_work(&mut store, 1, "a", true, "");
    let b = queued_work(&mut store, 2, "a", true, "");
    store.claim(&a, "scheduler:one", 0).unwrap().unwrap();
    assert!(store.claim(&b, "scheduler:one", 0).unwrap().is_none());
    store.config.scheduler.replies_per_agent = None;
    store.config.scheduler.discussion_seconds = None;
    store.start("unlimited", "other", 0).unwrap();
    assert_eq!(
        store.discussion("unlimited").unwrap().unwrap().max_replies,
        None
    );
    assert_eq!(
        store.discussion("unlimited").unwrap().unwrap().expires_at,
        9_007_199_254_740_991
    );
    store
        .connection
        .execute("UPDATE usage SET replies=100000 WHERE principal='a'", [])
        .unwrap();
    store.config.scheduler.parallel_work = true;
    assert!(store.claim(&b, "scheduler:one", 0).unwrap().is_some());
}
#[test]
fn resource_claims_compete_across_connections_without_double_admission() {
    let (home, mut store) = fixture();
    let a = queued_work(&mut store, 1, "a", true, "game:1");
    let b = queued_work(&mut store, 2, "b", true, "game:1");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let jobs = [a, b]
        .into_iter()
        .map(|id| {
            let path = home.path().to_owned();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut s = ScheduleStore::open(&path).unwrap();
                barrier.wait();
                s.claim(&id, "scheduler:one", 0).unwrap().is_some()
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        jobs.into_iter()
            .filter_map(|j| j.join().ok())
            .filter(|v| *v)
            .count(),
        1
    );
}

#[test]
fn unknown_results_keep_resources_until_reconciled() {
    let (_home, mut store) = fixture();
    let a = queued_work(&mut store, 1, "a", true, "game:1");
    let b = queued_work(&mut store, 2, "b", true, "game:1");
    let mut task = store.claim(&a, "scheduler:one", 0).unwrap().unwrap();
    let work = task.work_id.clone().unwrap();
    task.state = crate::TaskState::Interrupted;
    store.update(&task, 0).unwrap();
    store.connection.execute("UPDATE work_items SET record=json_set(record,'$.state','outcome_unknown','$.run_id','lost-run','$.reason','lost owner') WHERE id=?1",[&work]).unwrap();
    assert!(store.claim(&b, "scheduler:one", 0).unwrap().is_none());
    store.connection.execute("UPDATE work_items SET record=json_set(record,'$.state','failed','$.error','confirmed stopped') WHERE id=?1",[&work]).unwrap();
    assert!(store.claim(&b, "scheduler:one", 0).unwrap().is_some());
}
