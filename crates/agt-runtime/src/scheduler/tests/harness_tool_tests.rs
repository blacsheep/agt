use super::*;
#[cfg(unix)]
#[test]
fn resource_agent_can_query_and_configure_only_shared_idle_sessions() {
    let (home, user, chat) = external_fixture();
    let actor = Actor {
        principal: PrincipalId::new("a"),
        role: Role::Participant,
        automated_depth: None,
    };
    let service = ToolService::open(home.path(), actor.clone())
        .unwrap()
        .with_resource_management(true)
        .with_harness_control(Box::new(crate::SessionControl::new(
            home.path(),
            actor.clone(),
        )));
    let denied = ToolService::open(home.path(), actor.clone())
        .unwrap()
        .with_harness_control(Box::new(crate::SessionControl::new(home.path(), actor)));
    assert!(
        !denied
            .schemas()
            .iter()
            .any(|s| s["name"] == "harness_models")
    );
    let models = service
        .call("harness_models", json!({"reference":"local"}))
        .unwrap();
    assert_eq!(models["data"][0]["model"], "fixture-model");
    let session = service
        .call(
            "harness_session_get",
            json!({"conversation":chat,"target":"a"}),
        )
        .unwrap();
    let args = json!({"conversation":chat,"target":"a","session":session["id"],"model":"fixture-model","effort":"xhigh"});
    let changed = service
        .call("harness_session_configure", args.clone())
        .unwrap();
    assert_eq!(changed["reasoning_effort"], "xhigh");
    assert!(changed.get("config").is_none());
    let mut stale = args.clone();
    stale["session"] = json!("stale");
    assert!(service.call("harness_session_configure", stale).is_err());
    let other = user
        .call("direct_create", json!({"principal":"b"}))
        .unwrap();
    assert!(
        service
            .call(
                "harness_session_get",
                json!({"conversation":other["id"],"target":"b"})
            )
            .is_err()
    );
    let sessions = crate::HarnessSessions::open(home.path()).unwrap();
    let _lease = sessions
        .execution_lock(session["id"].as_str().unwrap())
        .unwrap();
    assert!(service.call("harness_session_configure", args).is_err());
}

#[cfg(unix)]
#[test]
fn scheduled_harness_configuration_uses_task_fence_without_nested_scheduler_lock() {
    let (home, user, chat) = external_fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    let actor = Actor {
        principal: PrincipalId::new("a"),
        role: Role::Participant,
        automated_depth: None,
    };
    let service = ToolService::open(home.path(), actor.clone())
        .unwrap()
        .with_resource_management(true)
        .with_harness_control(Box::new(
            crate::SessionControl::new(home.path(), actor).with_task(Some(task)),
        ));
    let session = service
        .call(
            "harness_session_get",
            json!({"conversation":chat,"target":"a"}),
        )
        .unwrap();
    let args = json!({"conversation":chat,"target":"a","session":session["id"],"model":"fixture-model","effort":"xhigh"});
    assert!(
        service
            .call("harness_session_configure", args.clone())
            .is_ok()
    );
    scheduler
        .store
        .connection
        .execute("UPDATE tasks SET version=version+1", [])
        .unwrap();
    assert!(service.call("harness_session_configure", args).is_err());
}
