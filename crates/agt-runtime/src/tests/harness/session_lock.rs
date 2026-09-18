//! Harness session lock lifecycle and retry boundaries. Protocol fixtures never call a model.
use super::*;

#[cfg(unix)]
#[test]
fn codex_startup_failure_does_not_poison_the_chat_session() {
    let (home, user, _) = external_fixture();
    let chat = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let sessions = crate::HarnessSessions::open(home.path()).unwrap();
    let session = sessions.current(home.path(), "a", &chat).unwrap();
    std::fs::write(home.path().join("startup-fails"), "1").unwrap();
    let mut scheduler = Scheduler::open(home.path(), 2).unwrap();
    for (key, expected) in [
        ("first", "failed"),
        ("retry", "completed"),
        ("reject", "failed"),
        ("after-reject", "completed"),
        ("disconnect", "failed"),
    ] {
        if key == "reject" {
            std::fs::write(home.path().join("reject-start"), "1").unwrap();
        } else if key == "after-reject" {
            std::fs::remove_file(home.path().join("reject-start")).unwrap();
        }
        let body = if key == "disconnect" {
            "DISCONNECT"
        } else {
            "hello"
        };
        let sent = user
            .call(
                "message_send",
                json!({"conversation":chat,"body":body,"request_key":key}),
            )
            .unwrap();
        start_discussion(
            home.path(),
            &chat,
            Some(sent["message"]["id"].as_str().unwrap()),
            false,
        )
        .unwrap();
        scheduler.scan().unwrap();
        let task = claim(&mut scheduler, "a");
        execute(
            home.path(),
            task.clone(),
            &mut Answer(json!({})),
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(scheduler.store.get(&task.id).unwrap().state, expected);
        assert_eq!(sessions.get(&session.id).unwrap().busy, key == "disconnect");
        if key == "first" {
            std::fs::remove_file(home.path().join("startup-fails")).unwrap();
        }
    }
}

#[cfg(unix)]
#[test]
fn compaction_before_submission_or_rejection_does_not_poison_session() {
    let (home, user, _) = external_fixture();
    let chat = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let sessions = crate::HarnessSessions::open(home.path()).unwrap();
    let session = sessions.current(home.path(), "a", &chat).unwrap();
    sessions
        .db
        .execute(
            "UPDATE harness_sessions SET thread='test-thread' WHERE scope=?1",
            [&session.id],
        )
        .unwrap();
    for marker in ["startup-fails", "reject-start"] {
        std::fs::write(home.path().join(marker), "1").unwrap();
        assert!(
            sessions
                .compact(&session.id, &CancellationToken::new())
                .is_err()
        );
        assert!(!sessions.get(&session.id).unwrap().busy);
        std::fs::remove_file(home.path().join(marker)).unwrap();
    }
    sessions
        .compact(&session.id, &CancellationToken::new())
        .unwrap();
    assert!(!sessions.get(&session.id).unwrap().busy);
    // A rejected competing acquisition must not release the current owner's lock.
    let owner = sessions.execution_lock(&session.id).unwrap();
    assert!(sessions.execution_lock(&session.id).is_err());
    assert!(sessions.get(&session.id).unwrap().busy);
    drop(owner);
    assert!(!sessions.get(&session.id).unwrap().busy);
}

#[cfg(unix)]
#[test]
fn harness_compaction_cancellation_keeps_the_unknown_outcome_fenced() {
    let (home, user, _) = external_fixture();
    let path = home.path();
    let chat = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let store = crate::HarnessSessions::open(path).unwrap();
    let session = store.current(path, "a", &chat).unwrap();
    store
        .db
        .execute(
            "UPDATE harness_sessions SET thread='hang-compact' WHERE scope=?1",
            [&session.id],
        )
        .unwrap();
    let work = path.to_owned();
    let id = session.id.clone();
    let handle = std::thread::spawn(move || {
        crate::HarnessSessions::open(&work)
            .unwrap()
            .compact(&id, &CancellationToken::new())
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !path.join("compacting").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    store.cancel_conversation(&chat).unwrap();
    assert!(handle.join().unwrap().is_err());
    assert!(store.get(&session.id).unwrap().busy);
    assert!(store.configure(&session.id, "fixture-model", None).is_err());
}
