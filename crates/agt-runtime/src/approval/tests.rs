use super::*;
fn fixture() -> (tempfile::TempDir, ApprovalStore, Scope) {
    let home = tempfile::tempdir().unwrap();
    agt_config::initialize_home(Some(home.path().into())).unwrap();
    let (record, _) = crate::RunStore::open(home.path())
        .unwrap()
        .claim(&crate::RunRequest::new("a", "task"), None)
        .unwrap();
    let scope = Scope::new(&record, None);
    let store = ApprovalStore::open(home.path()).unwrap();
    (home, store, scope)
}
fn request() -> Value {
    json!({"tool":"test","options":[{"value":"allow"},{"value":"deny"},{"value":"cancel"}]})
}
#[test]
fn decisions_are_conditional_and_bound_to_execution_and_options() {
    let (_home, store, scope) = fixture();
    let item = store
        .create(&scope, "native", "call", request(), 600)
        .unwrap();
    assert_eq!(
        store
            .create(&scope, "native", "call", request(), 600)
            .unwrap()
            .id,
        item.id
    );
    let mut restarted = scope.clone();
    restarted.execution_id = "different host".into();
    assert!(
        store
            .create(&restarted, "native", "call", request(), 600)
            .is_err()
    );
    assert!(
        store
            .create(&scope, "native", "call", json!({}), 600)
            .is_err()
    );
    assert!(
        store
            .decide(&item.id, 1, json!("anything"), "", None)
            .is_err()
    );
    let approved = store
        .decide(&item.id, 1, json!("allow"), "checked", None)
        .unwrap();
    assert_eq!(approved.state, "approved");
    assert!(store.decide(&item.id, 1, json!("deny"), "", None).is_err());
    store.fallback(&item.id, 1, "late model").unwrap();
    assert_eq!(store.get(&item.id, None).unwrap().reason, "checked");
    store.invalidate_run(&scope.run_id).unwrap();
    assert_eq!(store.poll(&item.id).unwrap().delivery, "invalidated");
    assert!(store.change(&item.id, None, Some("sending")).is_err());
}
#[test]
fn expiration_and_lost_lease_never_authorize_or_replay() {
    let (_home, store, scope) = fixture();
    let expired = store
        .create(&scope, "native", "expired", request(), 0)
        .unwrap();
    assert!(
        store
            .decide(&expired.id, 1, json!("allow"), "", None)
            .is_err()
    );
    assert_eq!(store.get(&expired.id, None).unwrap().state, "expired");
    let sending = store
        .create(&scope, "harness", "sending", request(), 600)
        .unwrap();
    store
        .decide(&sending.id, 1, json!("allow"), "", None)
        .unwrap();
    store.change(&sending.id, None, Some("sending")).unwrap();
    store
        .db
        .execute(
            "UPDATE approvals SET lease_until=0 WHERE id=?1",
            [&sending.id],
        )
        .unwrap();
    assert_eq!(store.poll(&sending.id).unwrap().delivery, "unknown");
}
#[test]
fn human_approval_requires_owner_membership_and_agents_are_read_only() {
    use agt_im::{PrincipalId, PrincipalInput, PrincipalKind};
    use agt_service::{Actor, Role, ToolService};
    let (home, store, mut scope) = fixture();
    let im = agt_im::ImStore::open(agt_storage::database_path(home.path(), "im", "im.db").unwrap())
        .unwrap();
    for (id, kind) in [
        ("owner", PrincipalKind::User),
        ("guest", PrincipalKind::User),
        ("a", PrincipalKind::Agent),
        ("outsider", PrincipalKind::Agent),
    ] {
        im.create_principal(PrincipalInput {
            id: PrincipalId::new(id),
            kind,
            display_name: id.into(),
            metadata: json!({}),
        })
        .unwrap();
    }
    im.ensure_workspace_owner(&PrincipalId::new("owner"))
        .unwrap();
    let service = ToolService::open(
        home.path(),
        Actor {
            principal: PrincipalId::new("owner"),
            role: Role::Manager,
            automated_depth: None,
        },
    )
    .unwrap();
    let chat = service
        .call(
            "group_create",
            json!({"title":"approval","members":["guest","a"]}),
        )
        .unwrap();
    scope.conversation = Some(chat["id"].as_str().unwrap().into());
    scope.principal = Some("a".into());
    let item = store
        .create(&scope, "native", "call", request(), 600)
        .unwrap();
    assert!(store.get(&item.id, Some("a")).is_ok());
    assert!(store.get(&item.id, Some("outsider")).is_err());
    for actor in ["a", "guest", "outsider"] {
        assert!(
            store
                .decide(&item.id, 1, json!("allow"), "", Some(actor))
                .is_err()
        );
    }
    assert!(
        store
            .decide(&item.id, 1, json!("deny"), "", Some("owner"))
            .is_ok()
    );
}
#[test]
fn concurrent_creates_share_one_record() {
    let (home, _store, scope) = fixture();
    std::thread::scope(|threads| {
        let joins: Vec<_> = (0..4)
            .map(|_| {
                threads.spawn(|| {
                    ApprovalStore::open(home.path())
                        .unwrap()
                        .create(&scope, "native", "same", request(), 600)
                        .unwrap()
                        .id
                })
            })
            .collect();
        let ids: Vec<_> = joins.into_iter().map(|j| j.join().unwrap()).collect();
        assert!(ids.iter().all(|id| id == &ids[0]));
    });
}

#[test]
fn harness_mode_applies_to_waiting_and_future_requests_and_can_be_disabled() {
    let (home, store, scope) = fixture();
    let request = json!({"method":"item/commandExecution/requestApproval","options":[{"value":"accept"},{"value":"acceptForSession"},{"value":"decline"}]});
    let waiting = store
        .create(&scope, "harness", "waiting", request.clone(), 600)
        .unwrap();
    assert!(
        store
            .apply_harness_mode(waiting.clone())
            .unwrap()
            .decision
            .is_none()
    );
    let mut config = agt_config::RuntimeConfig::load(home.path()).unwrap();
    config.permissions.harness_mode = agt_config::runtime::HarnessPermissionMode::AutoAllow;
    config.save(home.path()).unwrap();
    let approved = store.apply_harness_mode(waiting).unwrap();
    assert_eq!(approved.decision, Some(json!("accept")));
    assert_eq!(approved.decision_origin.as_deref(), Some("auto_allow"));
    let future = store
        .create(&scope, "harness", "future", request.clone(), 600)
        .unwrap();
    assert_eq!(
        store.apply_harness_mode(future).unwrap().decision,
        Some(json!("accept"))
    );
    let permissions = json!({"method":"item/permissions/requestApproval","params":{"permissions":{"network":{"enabled":true}}},"options":[{"value":{"permissions":{},"scope":"turn"}},{"value":{"permissions":{"network":{"enabled":true}},"scope":"turn"}}]});
    let item = store
        .create(&scope, "harness", "permissions", permissions, 600)
        .unwrap();
    assert_eq!(
        store.apply_harness_mode(item).unwrap().decision.unwrap()["permissions"]["network"]["enabled"],
        true
    );
    let item = store
        .create(
            &scope,
            "harness",
            "session-only",
            json!({"options":[{"value":"acceptForSession"}]}),
            600,
        )
        .unwrap();
    assert!(store.apply_harness_mode(item).unwrap().decision.is_none());
    config.permissions.harness_mode = agt_config::runtime::HarnessPermissionMode::Manual;
    config.save(home.path()).unwrap();
    let item = store
        .create(&scope, "harness", "manual-again", request, 600)
        .unwrap();
    assert!(store.apply_harness_mode(item).unwrap().decision.is_none());
}
