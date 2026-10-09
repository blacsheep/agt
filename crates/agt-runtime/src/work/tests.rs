use super::*;

fn request(key: &str) -> WorkRequest {
    WorkRequest {
        parent_id: None,
        retry_of: None,
        requester: "lead".into(),
        assignee: "codex".into(),
        origin_conversation: "origin".into(),
        report_to: "report".into(),
        discussion: "discussion".into(),
        context_epoch: 0,
        delivery_epoch: 0,
        input: "Implement the requested fix".into(),
        request_key: key.into(),
        priority: Priority::Normal,
    }
}

#[test]
fn request_replay_survives_reopen_and_rejects_changed_input() {
    let home = tempfile::tempdir().unwrap();
    let mut ledger = WorkLedger::open(home.path()).unwrap();
    let first = ledger.enqueue(request("a")).unwrap();
    drop(ledger);
    let mut ledger = WorkLedger::open(home.path()).unwrap();
    assert_eq!(first, ledger.enqueue(request("a")).unwrap());
    let mut conflict = request("a");
    conflict.input = "different work".into();
    assert!(ledger.enqueue(conflict).is_err());
    assert_eq!(ledger.events_after(0, 20).unwrap().len(), 1);
    assert_eq!(ledger.inbox("codex", 0, 20).unwrap().len(), 1);
    assert!(ledger.inbox("other", 0, 20).unwrap().is_empty());
    let mut other = request("a");
    other.requester = "another-lead".into();
    assert_ne!(first.id, ledger.enqueue(other).unwrap().id);
}

#[test]
fn competing_submissions_create_one_work_and_one_notification() {
    let home = tempfile::tempdir().unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let path = home.path().to_owned();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut ledger = WorkLedger::open(&path).unwrap();
                barrier.wait();
                ledger.enqueue(request("same")).unwrap().id
            })
        })
        .collect();
    let ids: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(ids[0], ids[1]);
    assert_eq!(
        WorkLedger::open(home.path())
            .unwrap()
            .events_after(0, 20)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn result_and_publication_survive_reopen_without_acknowledging_inbox() {
    let home = tempfile::tempdir().unwrap();
    let mut ledger = WorkLedger::open(home.path()).unwrap();
    let item = ledger.enqueue(request("a")).unwrap();
    let running = ledger
        .record_running(&item.id, item.revision, "run-a")
        .unwrap();
    assert!(
        ledger
            .record_outcome(
                &item.id,
                running.revision,
                "other-run",
                WorkOutcome::Succeeded("wrong".into())
            )
            .is_err()
    );
    let result = ledger
        .record_outcome(
            &item.id,
            running.revision,
            "run-a",
            WorkOutcome::Succeeded("done".into()),
        )
        .unwrap();
    assert!(
        ledger
            .record_outcome(
                &item.id,
                running.revision,
                "run-a",
                WorkOutcome::Failed("late failure".into())
            )
            .is_err()
    );
    let publication = ledger.pending_publications(20).unwrap().remove(0);
    drop(ledger);
    let ledger = WorkLedger::open(home.path()).unwrap();
    assert_eq!(ledger.get(&item.id).unwrap(), result);
    let recovered = ledger.pending_publications(20).unwrap().remove(0);
    assert_eq!(publication.request_key, recovered.request_key);
    assert_eq!(recovered.destination, "report");
    ledger
        .confirm_publication(publication.event_seq, "im-message")
        .unwrap();
    ledger
        .confirm_publication(publication.event_seq, "im-message")
        .unwrap();
    assert!(
        ledger
            .confirm_publication(publication.event_seq, "different-message")
            .is_err()
    );
    assert!(ledger.pending_publications(20).unwrap().is_empty());
    assert_eq!(ledger.inbox("lead", 0, 20).unwrap().len(), 1);
    assert_eq!(ledger.inbox("lead", 0, 20).unwrap().len(), 1);
    let events = ledger.events_after(0, 20).unwrap();
    assert_eq!(
        events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
        vec!["requested", "running", "succeeded"]
    );
    assert_eq!(
        ledger.events_after(events[1].seq, 1).unwrap()[0].seq,
        events[2].seq
    );
}

#[test]
fn failed_notification_rolls_back_result_and_outbox() {
    let home = tempfile::tempdir().unwrap();
    let mut ledger = WorkLedger::open(home.path()).unwrap();
    let item = ledger.enqueue(request("a")).unwrap();
    let running = ledger.record_running(&item.id, 1, "run-a").unwrap();
    ledger.store.connection.execute_batch(
        "CREATE TRIGGER reject_publication BEFORE INSERT ON work_publications BEGIN SELECT RAISE(ABORT,'injected write failure'); END;"
    ).unwrap();
    assert!(
        ledger
            .record_outcome(
                &item.id,
                2,
                "run-a",
                WorkOutcome::Failed("model failed".into())
            )
            .is_err()
    );
    assert_eq!(ledger.get(&item.id).unwrap(), running);
    assert_eq!(ledger.events_after(0, 20).unwrap().len(), 2);
    assert!(ledger.inbox("lead", 0, 20).unwrap().is_empty());
    ledger
        .store
        .connection
        .execute_batch("DROP TRIGGER reject_publication;")
        .unwrap();
    ledger
        .record_outcome(
            &item.id,
            2,
            "run-a",
            WorkOutcome::Failed("model failed".into()),
        )
        .unwrap();
    assert_eq!(ledger.inbox("lead", 0, 20).unwrap()[0].kind, "failed");
}

#[test]
fn unknown_outcome_cannot_be_reexecuted_or_overwritten() {
    let home = tempfile::tempdir().unwrap();
    let mut ledger = WorkLedger::open(home.path()).unwrap();
    let item = ledger.enqueue(request("a")).unwrap();
    ledger.record_running(&item.id, 1, "run-a").unwrap();
    assert!(
        ledger
            .record_outcome(&item.id, 2, "run-a", WorkOutcome::Succeeded(" ".into()))
            .is_err()
    );
    let unknown = ledger
        .record_outcome(
            &item.id,
            2,
            "run-a",
            WorkOutcome::Unknown("lost execution ownership".into()),
        )
        .unwrap();
    assert!(
        ledger
            .record_running(&item.id, unknown.revision, "run-b")
            .is_err()
    );
    assert!(
        ledger
            .record_outcome(
                &item.id,
                unknown.revision,
                "run-a",
                WorkOutcome::Succeeded("late".into())
            )
            .is_err()
    );
    assert_eq!(ledger.enqueue(request("a")).unwrap(), unknown);
    assert_eq!(
        ledger.inbox("lead", 0, 20).unwrap()[0].kind,
        "outcome_unknown"
    );
    assert_eq!(ledger.pending_publications(20).unwrap().len(), 1);
}

#[test]
fn publication_key_fits_messaging_limit_for_long_destination() {
    let home = tempfile::tempdir().unwrap();
    let mut ledger = WorkLedger::open(home.path()).unwrap();
    let mut input = request("a");
    input.report_to = "x".repeat(256);
    let item = ledger.enqueue(input).unwrap();
    ledger.record_running(&item.id, 1, "run").unwrap();
    ledger
        .record_outcome(&item.id, 2, "run", WorkOutcome::Succeeded("done".into()))
        .unwrap();
    let publication = ledger.pending_publications(1).unwrap().remove(0);
    assert!(publication.request_key.len() <= 256);
    assert_eq!(publication.destination, "x".repeat(256));
}
