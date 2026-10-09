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
        ("engine-failed", "failed"),
        ("after-failure", "completed"),
        ("disconnect", "outcome_unknown"),
    ] {
        if key == "reject" {
            std::fs::write(home.path().join("reject-start"), "1").unwrap();
        } else if key == "after-reject" {
            std::fs::remove_file(home.path().join("reject-start")).unwrap();
        }
        let body = if key == "disconnect" {
            "DISCONNECT"
        } else if key == "engine-failed" {
            "FAIL"
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
        let saved = scheduler.store.get(&task.id).unwrap();
        assert_eq!(saved.state, expected, "{key}: {:?}", saved.reason);
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

#[cfg(unix)]
#[test]
fn expired_lease_fences_old_owner_and_preserves_uncertain_submission() {
    let (home, user, _) = external_fixture();
    let chat = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let sessions = crate::HarnessSessions::open(home.path()).unwrap();
    let session = sessions.current(home.path(), "a", &chat).unwrap();
    let old = sessions.execution_lock(&session.id).unwrap();
    sessions
        .db
        .execute(
            "UPDATE harness_operations SET expires=0 WHERE scope=?1",
            [&session.id],
        )
        .unwrap();
    assert_eq!(
        sessions.get(&session.id).unwrap().execution.unwrap().state,
        crate::harness_sessions::ExecutionState::Failed
    );
    let replacement = sessions.execution_lock(&session.id).unwrap();
    assert!(
        old.fenced(|| {
            sessions.db.execute(
                "UPDATE harness_sessions SET title='stale' WHERE scope=?1",
                [&session.id],
            )?;
            Ok(())
        })
        .is_err()
    );
    drop(old);
    assert!(replacement.check().is_ok());
    assert_ne!(sessions.get(&session.id).unwrap().title, "stale");
    replacement.submitted().unwrap();
    replacement.running().unwrap();
    sessions
        .db
        .execute(
            "UPDATE harness_operations SET expires=0 WHERE scope=?1",
            [&session.id],
        )
        .unwrap();
    let current = sessions.get(&session.id).unwrap();
    assert_eq!(
        current.execution.unwrap().state,
        crate::harness_sessions::ExecutionState::NeedsReview
    );
    assert!(sessions.execution_lock(&session.id).is_err());
    let owner: Option<String> = sessions
        .db
        .query_row(
            "SELECT owner FROM harness_operations WHERE scope=?1",
            [&session.id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(owner.is_none());
}

#[cfg(unix)]
#[test]
fn confirmed_cancellation_releases_session_for_next_operation() {
    let (home, user, _) = external_fixture();
    let chat = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let sent = user
        .call(
            "message_send",
            json!({"conversation":chat,"body":"CANCELABLE","request_key":"cancelable"}),
        )
        .unwrap();
    start_discussion(
        home.path(),
        &chat,
        Some(sent["message"]["id"].as_str().unwrap()),
    )
    .unwrap();
    let mut scheduler = Scheduler::open(home.path(), 2).unwrap();
    scheduler.scan().unwrap();
    let task = claim(&mut scheduler, "a");
    let token = CancellationToken::new();
    let child_token = token.clone();
    let path = home.path().to_owned();
    let handle =
        std::thread::spawn(move || execute(&path, task, &mut Answer(json!({})), &child_token));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !home.path().join("started").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let sessions = crate::HarnessSessions::open(home.path()).unwrap();
    let session = sessions.current(home.path(), "a", &chat).unwrap();
    sessions.cancel(&session.id).unwrap();
    token.cancel();
    handle.join().unwrap().unwrap();
    let current = sessions.get(&session.id).unwrap();
    assert!(!current.busy);
    assert_eq!(
        current.execution.unwrap().state,
        crate::harness_sessions::ExecutionState::Canceled
    );
    assert!(sessions.execution_lock(&session.id).is_ok());
}

// Invoked only by the subprocess crash test; no model or external engine is started.
#[test]
fn lease_crash_child() {
    let Ok(home) = std::env::var("AGT_TEST_LEASE_HOME") else {
        return;
    };
    let id = std::env::var("AGT_TEST_LEASE_ID").unwrap();
    let sessions = crate::HarnessSessions::open(std::path::Path::new(&home)).unwrap();
    let lease = sessions.execution_lock(&id).unwrap();
    if std::env::var("AGT_TEST_LEASE_SUBMITTED").unwrap() == "true" {
        lease.submitted().unwrap();
    }
    std::fs::write(std::path::Path::new(&home).join("lease-ready"), "1").unwrap();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

#[cfg(unix)]
#[test]
fn killed_owner_is_recovered_without_replaying_submitted_work() {
    use std::process::{Command, Stdio};
    for submitted in [false, true] {
        let (home, user, _) = external_fixture();
        let chat = user
            .call("direct_create", json!({"principal":"a"}))
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let sessions = crate::HarnessSessions::open(home.path()).unwrap();
        let session = sessions.current(home.path(), "a", &chat).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "scheduler::tests::harness_session_lock_tests::lease_crash_child",
            ])
            .env("AGT_TEST_LEASE_HOME", home.path())
            .env("AGT_TEST_LEASE_ID", &session.id)
            .env("AGT_TEST_LEASE_SUBMITTED", submitted.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !home.path().join("lease-ready").exists() {
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("lease child did not start");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        child.kill().unwrap();
        child.wait().unwrap();
        // Advance only the persisted deadline; avoid a 30-second wall clock wait.
        sessions
            .db
            .execute(
                "UPDATE harness_operations SET expires=0 WHERE scope=?1",
                [&session.id],
            )
            .unwrap();
        let recovered = sessions.get(&session.id).unwrap();
        assert_eq!(
            recovered.execution.unwrap().state,
            if submitted {
                crate::harness_sessions::ExecutionState::NeedsReview
            } else {
                crate::harness_sessions::ExecutionState::Failed
            }
        );
        assert_eq!(sessions.execution_lock(&session.id).is_ok(), !submitted);
    }
}

#[cfg(unix)]
#[test]
fn legacy_dirty_session_is_migrated_to_review_without_active_owner() {
    let (home, user, _) = external_fixture();
    let chat = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let sessions = crate::HarnessSessions::open(home.path()).unwrap();
    let session = sessions.current(home.path(), "a", &chat).unwrap();
    sessions.db.execute_batch("DROP TABLE harness_operations; DROP TABLE change_outbox; ALTER TABLE harness_sessions ADD COLUMN dirty INTEGER NOT NULL DEFAULT 1; ALTER TABLE harness_sessions ADD COLUMN cancel INTEGER NOT NULL DEFAULT 0; PRAGMA user_version=0;").unwrap();
    drop(sessions);
    let migrated = crate::HarnessSessions::open(home.path()).unwrap();
    let current = migrated.get(&session.id).unwrap();
    assert_eq!(
        current.execution.unwrap().state,
        crate::harness_sessions::ExecutionState::NeedsReview
    );
    assert!(migrated.execution_lock(&session.id).is_err());
}
