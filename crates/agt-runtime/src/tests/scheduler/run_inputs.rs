use super::*;
use agt_service::InputMode;

struct Model<F>(F);
impl<F> ModelClient for Model<F>
where
    F: FnMut(&[Value], &[Value]) -> crate::Result<Value>,
{
    fn complete(
        &mut self,
        _: &ResolvedAgent,
        messages: &[Value],
        tools: &[Value],
    ) -> crate::Result<Value> {
        (self.0)(messages, tools)
    }
}
fn answer(body: &str) -> Value {
    json!({"role":"assistant","content":body})
}
fn tools() -> Value {
    json!({"role":"assistant","content":null,"tool_calls":[
        {"id":"one","type":"function","function":{"name":"framework_principal_list","arguments":"{}"}},
        {"id":"two","type":"function","function":{"name":"framework_principal_list","arguments":"{}"}}
    ]})
}
fn send(home: &Path, user: &ToolService, chat: &str, body: &str, mode: Option<&str>) -> String {
    let value = user
        .call(
            "message_send",
            json!({"conversation":chat,"body":body,"request_key":body,"input_mode":mode}),
        )
        .unwrap();
    let id = value["message"]["id"].as_str().unwrap();
    start_discussion(home, chat, Some(id)).unwrap();
    id.into()
}
fn last_input(messages: &[Value]) -> Value {
    serde_json::from_str(messages.last().unwrap()["content"].as_str().unwrap()).unwrap()
}

#[test]
fn steering_follows_complete_tool_batch_and_follow_up_follows_final_reply() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    let mut step = 0;
    let mut model = Model(|messages: &[Value], _: &[Value]| {
        let reply = match step {
            0 => {
                send(home.path(), &user, &chat, "correct this", None);
                send(home.path(), &user, &chat, "next task", Some("follow_up"));
                tools()
            }
            1 => {
                assert_eq!(messages.iter().filter(|m| m["role"] == "tool").count(), 2);
                assert_eq!(last_input(messages)["messages"][0]["body"], "correct this");
                assert!(!messages.iter().any(|m| m.to_string().contains("next task")));
                answer("corrected result")
            }
            2 => {
                assert_eq!(last_input(messages)["messages"][0]["body"], "next task");
                let history = user
                    .call("message_history", json!({"conversation":chat}))
                    .unwrap();
                assert!(
                    history["messages"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|m| m["body"] == "corrected result")
                );
                answer("next result")
            }
            _ => panic!("unexpected extra model request"),
        };
        step += 1;
        Ok(reply)
    });
    execute(
        home.path(),
        task.clone(),
        &mut model,
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(step, 3);
    let tasks = scheduler.store.tasks().unwrap();
    assert!(
        tasks
            .iter()
            .filter(|t| t.principal == "a")
            .all(|t| t.state == "completed")
    );
    let run = scheduler.store.get(&task.id).unwrap().run_id.unwrap();
    assert!(
        tasks
            .iter()
            .filter(|t| t.principal == "a")
            .all(|t| t.run_id.as_deref() == Some(&run))
    );
    assert_eq!(
        scheduler
            .store
            .connection
            .query_row("SELECT COUNT(*) FROM run_inputs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn unmentioned_peer_follow_up_retains_participation_silent_and_wait_gates() {
    for action in ["participate", "silent", "wait"] {
        let (home, user, chat) = fixture();
        let mut scheduler = begin(home.path(), &user, &chat);
        let task = claim(&mut scheduler, "a");
        let peer = Messaging::open(
            home.path(),
            Actor {
                principal: PrincipalId::new("b"),
                role: Role::Participant,
                automated_depth: Some(1),
            },
        )
        .unwrap()
        .with_discussion(Some(task.discussion.clone()));
        let mut step = 0;
        let mut model = Model(|messages: &[Value], schemas: &[Value]| {
            let reply = match step {
                0 => {
                    peer.send(MessageInput {
                        conversation: ConversationId::new(&chat),
                        body: "peer input".into(),
                        request_key: "peer-input".into(),
                        mentions: vec![],
                        reply_to: None,
                        input_mode: None,
                    })
                    .unwrap();
                    tools()
                }
                1 => {
                    assert!(
                        !messages
                            .iter()
                            .any(|m| m.to_string().contains("peer input"))
                    );
                    answer("initial result")
                }
                2 => {
                    assert_eq!(schemas[0]["function"]["name"], "participation_decision");
                    let mut d = json!({"action":action,"reason_code":"observe_discussion"});
                    if action == "wait" {
                        d["wake"] = json!({"type":"after","seconds":30});
                    }
                    decision(d)
                }
                3 if action == "participate" => {
                    assert_eq!(last_input(messages)["messages"][0]["body"], "peer input");
                    answer("peer result")
                }
                _ => panic!("unexpected execution after decision"),
            };
            step += 1;
            Ok(reply)
        });
        execute(home.path(), task, &mut model, &CancellationToken::new()).unwrap();
        assert_eq!(step, if action == "participate" { 4 } else { 3 });
        let received = scheduler
            .store
            .tasks()
            .unwrap()
            .into_iter()
            .find(|t| t.principal == "a" && t.last_decision.is_some())
            .unwrap();
        assert_eq!(
            received.state,
            match action {
                "participate" => "completed",
                "silent" => "silent",
                _ => "waiting",
            }
        );
    }
}

#[test]
fn takeover_interrupts_accepted_input_but_preserves_unconsumed_follow_up() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    let steering = send(home.path(), &user, &chat, "steer", None);
    let follow_up = send(home.path(), &user, &chat, "later", Some("follow_up"));
    let input = crate::run_inputs::take(home.path(), &task, "run-test", InputMode::Steering)
        .unwrap()
        .unwrap();
    assert_eq!(input.task.message, steering);
    assert!(
        crate::run_inputs::take(home.path(), &task, "run-test", InputMode::Steering)
            .unwrap()
            .is_none()
    );
    assert!(
        scheduler
            .store
            .acquire(
                "replacement",
                now_ms() + crate::schedule_store::LEASE_MS + 1
            )
            .unwrap()
    );
    assert_eq!(
        scheduler.store.get(&input.task.id).unwrap().state,
        "interrupted"
    );
    let remaining = scheduler
        .store
        .tasks()
        .unwrap()
        .into_iter()
        .find(|t| t.principal == "a" && t.message == follow_up)
        .unwrap();
    assert_eq!(remaining.state, "queued");
    assert!(crate::run_inputs::take(home.path(), &task, "run-test", InputMode::FollowUp).is_err());
}

#[test]
fn same_principal_other_conversation_and_manual_policy_do_not_enter_run() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    let direct = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    send(home.path(), &user, &direct, "private details", None);
    assert!(
        crate::run_inputs::take(home.path(), &task, "test", InputMode::Steering)
            .unwrap()
            .is_none()
    );
    send(home.path(), &user, &chat, "manual message", None);
    let actor = ToolService::open(
        home.path(),
        Actor {
            principal: PrincipalId::new("a"),
            role: Role::Participant,
            automated_depth: None,
        },
    )
    .unwrap();
    actor
        .call(
            "response_policy",
            json!({"conversation":chat,"policy":"manual"}),
        )
        .unwrap();
    assert!(
        crate::run_inputs::take(home.path(), &task, "test", InputMode::Steering)
            .unwrap()
            .is_none()
    );
}

#[test]
fn deleting_accepted_input_fences_further_execution() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    let id = send(home.path(), &user, &chat, "to delete", None);
    crate::run_inputs::take(home.path(), &task, "test", InputMode::Steering)
        .unwrap()
        .unwrap();
    im(home.path())
        .unwrap()
        .delete_message(&MessageId::new(id), &PrincipalId::new("user"))
        .unwrap();
    assert!(guarded(home.path(), &task, || Ok(())).is_err());
}

#[test]
fn retry_keeps_its_snapshot_and_only_then_accepts_steering() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    let mut step = 0;
    let mut model = Model(|messages: &[Value], _: &[Value]| {
        step += 1;
        match step {
            1 => {
                send(home.path(), &user, &chat, "new instruction", None);
                Err(RuntimeError::HttpStatus(503))
            }
            2 => {
                assert_eq!(messages.len(), 2);
                assert!(
                    !messages
                        .iter()
                        .any(|m| m.to_string().contains("new instruction"))
                );
                Ok(answer("obsolete draft"))
            }
            3 => {
                assert_eq!(
                    last_input(messages)["messages"][0]["body"],
                    "new instruction"
                );
                Ok(answer("updated reply"))
            }
            _ => panic!("unexpected retry or replay"),
        }
    });
    execute(home.path(), task, &mut model, &CancellationToken::new()).unwrap();
    assert_eq!(step, 3);
    let history = user
        .call("message_history", json!({"conversation":chat}))
        .unwrap();
    assert!(
        !history["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["body"] == "obsolete draft")
    );
    assert!(
        history["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["body"] == "updated reply")
    );
}

#[test]
fn failed_follow_up_decision_does_not_discard_the_completed_reply() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    let peer = Messaging::open(
        home.path(),
        Actor {
            principal: PrincipalId::new("b"),
            role: Role::Participant,
            automated_depth: None,
        },
    )
    .unwrap()
    .with_discussion(Some(task.discussion.clone()));
    let mut step = 0;
    let mut model = Model(|_: &[Value], schemas: &[Value]| {
        step += 1;
        if schemas
            .first()
            .is_some_and(|t| t["function"]["name"] == "participation_decision")
        {
            return Err(RuntimeError::Configuration("participation unavailable"));
        }
        peer.send(MessageInput {
            conversation: ConversationId::new(&chat),
            body: "follow up".into(),
            request_key: "follow up".into(),
            mentions: vec![],
            reply_to: None,
            input_mode: None,
        })
        .unwrap();
        Ok(answer("completed result"))
    });
    execute(
        home.path(),
        task.clone(),
        &mut model,
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(step, 2);
    assert_eq!(scheduler.store.get(&task.id).unwrap().state, "completed");
    assert!(
        scheduler
            .store
            .tasks()
            .unwrap()
            .iter()
            .any(|t| t.principal == "a" && t.state == "failed")
    );
    let history = user
        .call("message_history", json!({"conversation":chat}))
        .unwrap();
    assert!(
        history["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["body"] == "completed result")
    );
}

#[test]
fn pause_and_reply_allowance_leave_inputs_pending() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    send(home.path(), &user, &chat, "queued correction", None);
    scheduler.store.control(&chat, "pause", now_ms()).unwrap();
    assert!(
        crate::run_inputs::take(home.path(), &task, "test", InputMode::Steering)
            .unwrap()
            .is_none()
    );
    scheduler.store.control(&chat, "resume", now_ms()).unwrap();
    scheduler.store.connection.execute("INSERT INTO usage(discussion,principal,replies) VALUES(?1,'a',100) ON CONFLICT(discussion,principal) DO UPDATE SET replies=100",[&task.discussion]).unwrap();
    assert!(
        crate::run_inputs::take(home.path(), &task, "test", InputMode::Steering)
            .unwrap()
            .is_none()
    );
    assert!(
        scheduler
            .store
            .tasks()
            .unwrap()
            .iter()
            .any(|t| t.principal == "a" && t.state == "limited")
    );
    assert_eq!(
        scheduler
            .store
            .connection
            .query_row("SELECT COUNT(*) FROM run_inputs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn steering_survives_context_recovery_verbatim() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    let instruction = "Keep exactly this identifier: 7ab9-corrected";
    let mut step = 0;
    let mut model = Model(|messages: &[Value], schemas: &[Value]| {
        step += 1;
        match step {
            1 => {
                send(home.path(), &user, &chat, instruction, None);
                Ok(tools())
            }
            2 => Err(RuntimeError::ContextWindowRejected),
            3 => {
                assert!(schemas.is_empty());
                Ok(answer("Two lookups finished. Do not repeat them."))
            }
            4 => {
                assert_eq!(last_input(messages)["messages"][0]["body"], instruction);
                assert!(!messages.iter().any(|m| m["role"] == "tool"));
                Ok(answer("done"))
            }
            _ => panic!("unexpected request"),
        }
    });
    execute(
        home.path(),
        task.clone(),
        &mut model,
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(step, 4);
    let record = crate::RunStore::open(home.path())
        .unwrap()
        .get(&scheduler.store.get(&task.id).unwrap().run_id.unwrap())
        .unwrap();
    assert_eq!(record.context.compactions, 1);
    assert_eq!(record.state, RunState::Completed);
}

#[test]
fn later_failure_preserves_completed_input_and_published_reply() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    let correction = send(home.path(), &user, &chat, "first correction", None);
    let next = send(
        home.path(),
        &user,
        &chat,
        "later failure",
        Some("follow_up"),
    );
    let mut step = 0;
    let mut model = Model(|messages: &[Value], _: &[Value]| {
        step += 1;
        if step == 1 {
            assert_eq!(
                last_input(messages)["messages"][0]["body"],
                "first correction"
            );
            return Ok(answer("finished correction"));
        }
        assert_eq!(last_input(messages)["messages"][0]["body"], "later failure");
        Err(RuntimeError::Configuration("later request failed"))
    });
    let _ = execute(home.path(), task, &mut model, &CancellationToken::new());
    assert_eq!(step, 2);
    let tasks = scheduler.store.tasks().unwrap();
    assert_eq!(
        tasks
            .iter()
            .find(|t| t.principal == "a" && t.message == correction)
            .unwrap()
            .state,
        "completed"
    );
    assert_eq!(
        tasks
            .iter()
            .find(|t| t.principal == "a" && t.message == next)
            .unwrap()
            .state,
        "failed"
    );
    let history = user
        .call("message_history", json!({"conversation":chat}))
        .unwrap();
    let reply = history["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["body"] == "finished correction")
        .unwrap();
    assert_eq!(reply["reply_to_id"], correction);
    assert_eq!(reply["metadata"]["automated_depth"], 1);
}
