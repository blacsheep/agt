use super::*;
#[test]
fn scheduling_policy_is_created_atomically_and_revision_fenced() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let lead = claim(&mut scheduler, "a");
    let control = TaskControl::hosted(
        home.path(),
        Actor {
            principal: PrincipalId::new("a"),
            role: Role::Participant,
            automated_depth: Some(1),
        },
        lead,
    );
    let args = json!({"assignee":"b","conversation":chat,"input":"independent work","request_key":"policy-work","execution_policy":{"parallel":true,"exclusive_resources":["game:1"]}});
    let item = control.call("task_request", args.clone()).unwrap();
    assert_eq!(item["execution_policy"]["parallel"], true);
    assert_eq!(
        control.call("task_request", args.clone()).unwrap()["id"],
        item["id"]
    );
    let mut conflict = args.clone();
    conflict["execution_policy"]["parallel"] = json!(false);
    assert!(control.call("task_request", conflict).is_err());
    let changed=control.call("task_execution_policy",json!({"task_id":item["id"],"expected_revision":item["revision"],"reason":"shared state requires ordering","execution_policy":{"parallel":false,"exclusive_resources":["game:1"]}})).unwrap();
    assert_eq!(changed["execution_policy"]["parallel"], false);
    assert!(control.call("task_execution_policy",json!({"task_id":item["id"],"expected_revision":item["revision"],"reason":"stale write","execution_policy":{"parallel":true}})).is_err());
    assert_eq!(
        control.call("task_request", args).unwrap()["execution_policy"]["parallel"],
        false
    );
}
