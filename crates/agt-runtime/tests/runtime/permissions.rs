use super::*;
use agt_config::runtime::PermissionMode;
use agt_runtime::approval::ApprovalStore;
use agt_runtime::approval::{ApprovalState, DecisionOrigin, Delivery};
#[test]
fn manual_denial_pairs_tool_result_without_executing_delegate() {
    let home = home();
    let mut config = agt_config::RuntimeConfig::load(home.path()).unwrap();
    config.permissions.mode = PermissionMode::Manual;
    config.save(home.path()).unwrap();
    let path = home.path().to_owned();
    let human = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let store = ApprovalStore::open(&path).unwrap();
            if let Some(a) = store.list(None, None).unwrap().first() {
                store
                    .decide(&a.id, a.revision, json!("deny"), "not authorized", None)
                    .unwrap();
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no approval appeared");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    });
    let mut model = Script::new(vec![
        ("default/a", delegate()),
        ("default/a", text("declined")),
    ]);
    let mut req = request();
    req.delegates = vec!["default/b".into()];
    let run = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(req, &CancellationToken::new())
        .unwrap();
    human.join().unwrap();
    assert_eq!(run.state, RunState::Completed);
    assert_eq!(model.seen.len(), 2);
    let transcript = &model.seen[1].1;
    assert!(transcript.iter().any(|v| v["role"] == "tool"
        && v["tool_call_id"] == "delegate-1"
        && v["content"].as_str().unwrap().contains("not executed")));
    let a = ApprovalStore::open(home.path())
        .unwrap()
        .list(None, None)
        .unwrap()
        .remove(0);
    assert_eq!(a.state, ApprovalState::Denied);
    assert_eq!(a.delivery, Delivery::Resolved);
}
#[test]
fn missing_reviewer_falls_back_to_human_and_cancellation_stops_tool() {
    let home = home();
    let mut config = agt_config::RuntimeConfig::load(home.path()).unwrap();
    config.permissions.mode = PermissionMode::ModelReview;
    config.save(home.path()).unwrap();
    let token = CancellationToken::new();
    let cancel = token.clone();
    let path = home.path().to_owned();
    let human = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let store = ApprovalStore::open(&path).unwrap();
            if store
                .list(None, None)
                .unwrap()
                .iter()
                .any(|a| a.state == ApprovalState::AwaitingHuman && !a.reason.is_empty())
            {
                cancel.cancel();
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no fallback appeared");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    });
    let mut model = Script::new(vec![("default/a", delegate())]);
    let mut req = request();
    req.delegates = vec!["default/b".into()];
    let run = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(req, &token)
        .unwrap();
    human.join().unwrap();
    assert_eq!(run.state, RunState::Canceled);
    assert_eq!(model.seen.len(), 1);
    let a = ApprovalStore::open(home.path())
        .unwrap()
        .list(None, None)
        .unwrap()
        .remove(0);
    assert_eq!(a.delivery, Delivery::Invalidated);
    assert!(a.decision.is_none());
}

#[test]
fn switching_mode_releases_waiting_native_call_and_applies_to_next_call() {
    let home = home();
    mount_resources(home.path(), "a");
    let mut config = agt_config::RuntimeConfig::load(home.path()).unwrap();
    config.permissions.mode = PermissionMode::Manual;
    config.permissions.request_timeout_seconds = 5;
    config.save(home.path()).unwrap();
    let path = home.path().to_owned();
    let changer = std::thread::spawn(move || {
        for _ in 0..200 {
            if !ApprovalStore::open(&path)
                .unwrap()
                .list(None, None)
                .unwrap()
                .is_empty()
            {
                let mut config = agt_config::RuntimeConfig::load(&path).unwrap();
                config.permissions.mode = PermissionMode::AutoAllow;
                config.save(&path).unwrap();
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("approval never appeared");
    });
    let mut model = Script::new(vec![
        (
            "default/a",
            calls(vec![call(
                "one",
                "framework_resource_write",
                json!({"kind":"prompt","reference":"one","create":true,"data":{"text":"one"}}),
            )]),
        ),
        (
            "default/a",
            calls(vec![call(
                "two",
                "framework_resource_write",
                json!({"kind":"prompt","reference":"two","create":true,"data":{"text":"two"}}),
            )]),
        ),
        ("default/a", text("done")),
    ]);
    let run = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &CancellationToken::new())
        .unwrap();
    changer.join().unwrap();
    assert_eq!(run.state, RunState::Completed, "{:?}", run.error);
    let approvals = ApprovalStore::open(home.path())
        .unwrap()
        .list(None, None)
        .unwrap();
    assert_eq!(approvals.len(), 2);
    assert!(
        approvals
            .iter()
            .all(|a| a.decision_origin == Some(DecisionOrigin::AutoAllow)
                && a.delivery == Delivery::Resolved)
    );
}

#[test]
fn framework_reads_skip_approval_even_in_manual_mode() {
    let home = home();
    mount_resources(home.path(), "a");
    let mut config = agt_config::RuntimeConfig::load(home.path()).unwrap();
    config.permissions.mode = PermissionMode::Manual;
    config.save(home.path()).unwrap();
    let mut model = Script::new(vec![
        (
            "default/a",
            calls(vec![
                call("list", "framework_resource_list", json!({"kind":"harness"})),
                call(
                    "mounts",
                    "framework_agent_mcp_list",
                    json!({"agent":"default/a"}),
                ),
            ]),
        ),
        ("default/a", text("done")),
    ]);
    let run = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Completed, "{:?}", run.error);
    let transcript = &model.seen[1].1;
    assert!(
        transcript
            .iter()
            .filter(|v| v["role"] == "tool")
            .all(|v| !v["content"].as_str().unwrap().contains("not executed"))
    );
    assert!(
        ApprovalStore::open(home.path())
            .unwrap()
            .list(None, None)
            .unwrap()
            .is_empty()
    );
}
