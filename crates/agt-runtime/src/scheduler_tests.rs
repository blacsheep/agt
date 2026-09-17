use super::*;
use agt_agent::{AgentConfig, AgentStore};
use agt_im::PrincipalInput;
use agt_model::{ModelConfig, ModelReference, ModelStore};
use agt_provider::{ProviderConfig, ProviderStore};
use agt_service::{Binding, ToolService};

fn fixture() -> (tempfile::TempDir, ToolService, String) {
    let home = tempfile::tempdir().unwrap();
    let path = home.path();
    agt_config::initialize_home(Some(path.into())).unwrap();
    agt_prompt::PromptStore::new(path)
        .ensure_default_prompt()
        .unwrap();
    ProviderStore::new(path)
        .create(
            &ProviderConfig::openai_compatible("local", "http://127.0.0.1:1/v1", "unused").unwrap(),
        )
        .unwrap();
    ModelStore::new(path)
        .create(&ModelConfig::new("fast", ModelReference::parse("local/test").unwrap()).unwrap())
        .unwrap();
    let im = im(path).unwrap();
    for name in ["user", "a", "b"] {
        im.create_principal(PrincipalInput {
            id: PrincipalId::new(name),
            kind: if name == "user" {
                PrincipalKind::User
            } else {
                PrincipalKind::Agent
            },
            display_name: name.into(),
            metadata: json!({}),
        })
        .unwrap();
        if name != "user" {
            AgentStore::new(path)
                .create(&AgentConfig::new(name, "fast").unwrap())
                .unwrap();
            BindingStore::open(path)
                .unwrap()
                .bind(Binding {
                    principal: PrincipalId::new(name),
                    agent: name.into(),
                    working_directory: path.into(),
                    policy: ResponsePolicy::All,
                    max_steps: 4,

                    allow_mcp: false,
                })
                .unwrap();
        }
    }
    im.ensure_workspace_owner(&PrincipalId::new("user"))
        .unwrap();
    let user = ToolService::open(
        path,
        Actor {
            principal: PrincipalId::new("user"),
            role: Role::Manager,
            automated_depth: None,
        },
    )
    .unwrap();
    let chat = user
        .call("group_create", json!({"title":"test","members":["a","b"]}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    (home, user, chat)
}
fn begin(home: &Path, user: &ToolService, chat: &str) -> Scheduler {
    let sent = user
        .call(
            "message_send",
            json!({"conversation":chat,"body":"question","request_key":"root"}),
        )
        .unwrap();
    start_discussion(
        home,
        chat,
        Some(sent["message"]["id"].as_str().unwrap()),
        false,
    )
    .unwrap();
    let mut scheduler = Scheduler::open(home, 3).unwrap();
    scheduler.scan().unwrap();
    scheduler
}
fn make_quiet(scheduler: &mut Scheduler) {
    rusqlite::Connection::open(scheduler.home.join("im/im.db"))
        .unwrap()
        .execute("UPDATE messages SET created_at=?1", [now_ms() - QUIET_MS])
        .unwrap();
    for mut task in scheduler.store.candidates().unwrap() {
        task.due_at = now_ms();
        scheduler
            .store
            .connection
            .execute(
                "UPDATE tasks SET due=?2,record=?3 WHERE id=?1",
                rusqlite::params![task.id, task.due_at, serde_json::to_string(&task).unwrap()],
            )
            .unwrap();
    }
}
fn claim(scheduler: &mut Scheduler, principal: &str) -> ScheduledTask {
    make_quiet(scheduler);
    let task = scheduler
        .store
        .candidates()
        .unwrap()
        .into_iter()
        .find(|t| t.principal == principal)
        .unwrap();
    scheduler
        .store
        .claim(&task.id, &scheduler.owner, now_ms())
        .unwrap()
        .unwrap()
}
struct Answer(Value);
impl ModelClient for Answer {
    fn complete(&mut self, _: &ResolvedAgent, _: &[Value], _: &[Value]) -> crate::Result<Value> {
        Ok(self.0.clone())
    }
}
fn decision(value: Value) -> Value {
    json!({"role":"assistant","tool_calls":[{"id":"decision","type":"function","function":{"name":"participation_decision","arguments":value.to_string()}}]})
}

#[test]
fn model_calls_overlap_and_replies_inherit_one_round() {
    struct Blocking {
        started: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }
    impl ModelClient for Blocking {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            _: &[Value],
            _: &[Value],
        ) -> crate::Result<Value> {
            self.started.send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(json!({"role":"assistant","content":"useful reply"}))
        }
    }
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let tasks = [claim(&mut scheduler, "a"), claim(&mut scheduler, "b")];
    let round = tasks[0].discussion.clone();
    let (tx, rx) = mpsc::channel();
    let mut releases = Vec::new();
    let mut threads = Vec::new();
    for task in tasks {
        let path = home.path().to_owned();
        let started = tx.clone();
        let (release, wait) = mpsc::channel();
        releases.push(release);
        threads.push(thread::spawn(move || {
            execute(
                &path,
                task,
                &mut Blocking {
                    started,
                    release: wait,
                },
                &CancellationToken::new(),
            )
            .unwrap()
        }));
    }
    // Both calls must have begun before either is released.
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    for release in releases {
        release.send(()).unwrap();
    }
    for handle in threads {
        handle.join().unwrap();
    }
    let history = user
        .call("message_history", json!({"conversation":chat}))
        .unwrap();
    assert_eq!(history["messages"].as_array().unwrap().len(), 3);
    for message in history["messages"].as_array().unwrap().iter().skip(1) {
        assert_eq!(message["metadata"]["discussion_id"], round);
    }
    assert_eq!(
        scheduler.store.discussion(&round).unwrap().unwrap().calls,
        2
    );
}

fn peer_task(home: &Path, user: &ToolService, chat: &str) -> (Scheduler, ScheduledTask) {
    let mut scheduler = begin(home, user, chat);
    let mut a = claim(&mut scheduler, "a");
    let mut b = claim(&mut scheduler, "b");
    a.state = "silent".into();
    b.state = "silent".into();
    scheduler.store.update(&a, now_ms()).unwrap();
    scheduler.store.update(&b, now_ms()).unwrap();
    let sender = Messaging::open(
        home,
        Actor {
            principal: PrincipalId::new("a"),
            role: Role::Participant,
            automated_depth: Some(1),
        },
    )
    .unwrap()
    .with_discussion(Some(a.discussion));
    sender
        .send(MessageInput {
            conversation: ConversationId::new(chat),
            body: "peer idea".into(),
            request_key: "peer".into(),
            mentions: vec![],
            reply_to: None,
        })
        .unwrap();
    scheduler.scan().unwrap();
    let task = claim(&mut scheduler, "b");
    (scheduler, task)
}

#[test]
fn model_wait_is_durable_and_cannot_be_replayed_before_due() {
    let (home, user, chat) = fixture();
    let (scheduler, task) = peer_task(home.path(), &user, &chat);
    let answer = decision(
        json!({"action":"wait","reason_code":"observe_discussion","public_reason":"先等待其他成员给出评估。","wake":{"type":"after","seconds":30}}),
    );
    execute(
        home.path(),
        task.clone(),
        &mut Answer(answer),
        &CancellationToken::new(),
    )
    .unwrap();
    let waiting = scheduler.store.get(&task.id).unwrap();
    assert_eq!(waiting.state, "waiting");
    assert!(waiting.due_at > now_ms() + 29_000);
    drop(scheduler);
    let mut restarted = Scheduler::open(home.path(), 3).unwrap();
    assert_eq!(restarted.store.get(&task.id).unwrap().state, "waiting");
    let saved = serde_json::to_value(restarted.store.get(&task.id).unwrap()).unwrap();
    assert_eq!(saved["last_decision"]["action"], "wait");
    assert_eq!(
        saved["last_decision"]["public_reason"],
        "先等待其他成员给出评估。"
    );
    assert_eq!(saved["last_decision"]["wake"]["seconds"], 30);
    assert_eq!(saved["last_decision"]["through_seq"], task.through_seq);
    assert!(saved["last_decision"]["decided_at"].as_i64().unwrap() > 0);
    assert!(
        restarted
            .store
            .claim(&task.id, &restarted.owner, now_ms())
            .unwrap()
            .is_none()
    );
}

#[test]
fn malformed_participation_never_starts_business_execution() {
    for answer in [
        json!({"role":"assistant","content":"I will reply"}),
        decision(
            json!({"action":"silent","reason_code":"already_answered","public_reason":"长".repeat(241)}),
        ),
        decision(
            json!({"action":"wait","reason_code":"observe_discussion","wake":{"type":"after","seconds":0}}),
        ),
    ] {
        let (home, user, chat) = fixture();
        let (scheduler, task) = peer_task(home.path(), &user, &chat);
        execute(
            home.path(),
            task.clone(),
            &mut Answer(answer),
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(scheduler.store.get(&task.id).unwrap().state, "failed");
        assert!(
            crate::RunStore::open(home.path())
                .unwrap()
                .list(50)
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn end_during_model_call_fences_framework_send_and_final_output() {
    struct End {
        home: PathBuf,
        chat: String,
        answer: Value,
    }
    impl ModelClient for End {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            _: &[Value],
            _: &[Value],
        ) -> crate::Result<Value> {
            ScheduleStore::open(&self.home)
                .unwrap()
                .control(&self.chat, "end", false, now_ms())
                .unwrap();
            Ok(self.answer.clone())
        }
    }
    for tool in [false, true] {
        let (home, user, chat) = fixture();
        let mut scheduler = begin(home.path(), &user, &chat);
        let task = claim(&mut scheduler, "a");
        let answer = if tool {
            json!({"role":"assistant","tool_calls":[{"id":"send","type":"function","function":{"name":"framework_message_send","arguments":json!({"conversation":chat,"body":"must not send","request_key":"late"}).to_string()}}]})
        } else {
            json!({"role":"assistant","content":"must not publish"})
        };
        execute(
            home.path(),
            task.clone(),
            &mut End {
                home: home.path().into(),
                chat: chat.clone(),
                answer,
            },
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(scheduler.store.get(&task.id).unwrap().state, "canceled");
        assert_eq!(
            user.call("message_history", json!({"conversation":chat}))
                .unwrap()["messages"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn participation_continues_into_tools_without_duplicate_publication() {
    struct Participate {
        step: u32,
        chat: String,
    }
    impl ModelClient for Participate {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            messages: &[Value],
            tools: &[Value],
        ) -> crate::Result<Value> {
            self.step += 1;
            Ok(match self.step {
                1 => {
                    assert_eq!(tools.len(), 1);
                    decision(json!({"action":"participate","reason_code":"correction_needed"}))
                }
                2 => {
                    let input: Value =
                        serde_json::from_str(messages[1]["content"].as_str().unwrap()).unwrap();
                    assert_eq!(input["last_decision"], "correction_needed");
                    assert!(
                        tools
                            .iter()
                            .any(|t| t["function"]["name"] == "framework_message_send")
                    );
                    json!({"role":"assistant","tool_calls":[{"id":"send","type":"function","function":{"name":"framework_message_send","arguments":json!({"conversation":self.chat,"body":"a correction","request_key":"correction"}).to_string()}}]})
                }
                3 => json!({"role":"assistant","content":"a redundant final summary"}),
                _ => panic!("unexpected model call"),
            })
        }
    }
    let (home, user, chat) = fixture();
    let (mut scheduler, task) = peer_task(home.path(), &user, &chat);
    let mut model = Participate {
        step: 0,
        chat: chat.clone(),
    };
    execute(
        home.path(),
        task.clone(),
        &mut model,
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(model.step, 1);
    assert_eq!(
        scheduler.store.get(&task.id).unwrap().state,
        "ready_to_reply"
    );
    let reply = claim(&mut scheduler, "b");
    execute(home.path(), reply, &mut model, &CancellationToken::new()).unwrap();
    assert_eq!(model.step, 3);
    let saved = serde_json::to_value(scheduler.store.get(&task.id).unwrap()).unwrap();
    assert_eq!(saved["state"], "silent");
    assert_eq!(saved["last_decision"]["action"], "participate");
    assert_eq!(
        scheduler
            .store
            .discussion(&task.discussion)
            .unwrap()
            .unwrap()
            .calls,
        3
    );
    let history = user
        .call("message_history", json!({"conversation":chat}))
        .unwrap();
    assert_eq!(history["messages"].as_array().unwrap().len(), 3);
    assert_eq!(history["messages"][2]["body"], "a correction");
    assert_eq!(
        history["messages"][2]["metadata"]["discussion_id"],
        task.discussion
    );
}

#[test]
fn model_call_count_is_diagnostic_not_an_admission_limit() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    scheduler
        .store
        .connection
        .execute("UPDATE discussions SET calls=1000", [])
        .unwrap();
    execute(
        home.path(),
        task.clone(),
        &mut Answer(json!({"role":"assistant","content":"hello"})),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(scheduler.store.get(&task.id).unwrap().state, "completed");
    assert_eq!(
        scheduler
            .store
            .discussion(&task.discussion)
            .unwrap()
            .unwrap()
            .calls,
        1001
    );
}

#[test]
fn five_second_window_slides_and_wait_deadline_cannot_bypass_it() {
    let (home, user, chat) = fixture();
    let scheduler = begin(home.path(), &user, &chat);
    let mut task = scheduler.store.candidates().unwrap().remove(0);
    task.due_at = 0;
    assert!(!quiet_ready(&task, 0, 4999));
    assert!(quiet_ready(&task, 0, 5000));
    assert!(!quiet_ready(&task, 4000, 5000));
    assert!(!quiet_ready(&task, 4000, 8999));
    assert!(quiet_ready(&task, 4000, 9000));
    task.quiet = true;
    task.deadline = 1;
    task.quiet_ms = 30_000;
    assert!(!quiet_ready(&task, 4000, 5000));
    assert!(quiet_ready(&task, 4000, 9000));
}

#[test]
fn human_burst_merges_and_waits_before_any_model_call() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    for key in ["fragment-2", "fragment-3"] {
        let sent = user
            .call(
                "message_send",
                json!({"conversation":chat,"body":key,"request_key":key}),
            )
            .unwrap();
        start_discussion(
            home.path(),
            &chat,
            Some(sent["message"]["id"].as_str().unwrap()),
            false,
        )
        .unwrap();
    }
    scheduler.tick(true).unwrap();
    assert!(scheduler.jobs.is_empty());
    let pending = scheduler.store.candidates().unwrap();
    assert_eq!(pending.len(), 2);
    assert!(
        pending
            .iter()
            .all(|t| t.deliveries.len() == 3 && t.triggers.len() == 3)
    );
    assert_eq!(scheduler.store.snapshot().unwrap().discussions.len(), 1);
    let task = claim(&mut scheduler, "a");
    execute(
        home.path(),
        task.clone(),
        &mut Answer(json!({"role":"assistant","content":"one reply"})),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(scheduler.store.get(&task.id).unwrap().state, "completed");
    assert_eq!(scheduler.store.snapshot().unwrap().usage[0].replies, 1);
}

#[test]
fn unauthorized_delete_cannot_end_a_discussion() {
    let (home, user, _chat) = fixture();
    let owner = ToolService::open(
        home.path(),
        Actor {
            principal: PrincipalId::new("b"),
            role: Role::Participant,
            automated_depth: None,
        },
    )
    .unwrap();
    let chat = owner
        .call(
            "group_create",
            json!({"title":"owned by b","members":["user","a"]}),
        )
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let scheduler = begin(home.path(), &user, &chat);
    assert!(delete_scheduled_conversation(home.path(), &chat, "b").is_err());
    assert_eq!(
        scheduler.store.snapshot().unwrap().discussions[0].state,
        "active"
    );
    delete_scheduled_conversation(home.path(), &chat, "user").unwrap();
    assert_eq!(
        scheduler.store.snapshot().unwrap().discussions[0].state,
        "ended"
    );
}

#[test]
fn participation_model_calls_overlap_in_one_conversation() {
    struct DecideTogether {
        started: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }
    impl ModelClient for DecideTogether {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            _: &[Value],
            tools: &[Value],
        ) -> crate::Result<Value> {
            assert_eq!(tools[0]["function"]["name"], "participation_decision");
            self.started.send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(decision(
                json!({"action":"participate","reason_code":"relevant_expertise"}),
            ))
        }
    }
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    // Treat the two recipient tasks as unsolicited peer evaluations.
    for mut task in scheduler.store.candidates().unwrap() {
        task.decision_required = true;
        scheduler
            .store
            .connection
            .execute(
                "UPDATE tasks SET record=?2 WHERE id=?1",
                rusqlite::params![task.id, serde_json::to_string(&task).unwrap()],
            )
            .unwrap();
    }
    let tasks = [claim(&mut scheduler, "a"), claim(&mut scheduler, "b")];
    let (tx, rx) = mpsc::channel();
    let mut workers = Vec::new();
    let mut releases = Vec::new();
    for task in tasks {
        let path = home.path().to_owned();
        let started = tx.clone();
        let (release, wait) = mpsc::channel();
        releases.push(release);
        workers.push(thread::spawn(move || {
            execute(
                &path,
                task,
                &mut DecideTogether {
                    started,
                    release: wait,
                },
                &CancellationToken::new(),
            )
            .unwrap()
        }));
    }
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    for release in releases {
        release.send(()).unwrap();
    }
    for worker in workers {
        worker.join().unwrap();
    }
    let tasks = scheduler.store.candidates().unwrap();
    assert_eq!(tasks.len(), 2);
    assert!(tasks.iter().all(|t| t.state == "ready_to_reply"));
    assert!(
        scheduler
            .store
            .snapshot()
            .unwrap()
            .usage
            .iter()
            .all(|u| u.replies == 0)
    );
    let first = claim(&mut scheduler, "a");
    execute(
        home.path(),
        first,
        &mut Answer(json!({"role":"assistant","content":"answer"})),
        &CancellationToken::new(),
    )
    .unwrap();
    let waiting = scheduler.store.candidates().unwrap().remove(0);
    let latest = im(home.path())
        .unwrap()
        .get_conversation(&ConversationId::new(&chat))
        .unwrap()
        .last_message_seq;
    scheduler
        .store
        .invalidate_ready(&waiting.id, latest, now_ms())
        .unwrap();
    let reconsider = claim(&mut scheduler, "b");
    execute(
        home.path(),
        reconsider,
        &mut Answer(decision(
            json!({"action":"silent","reason_code":"already_answered"}),
        )),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(scheduler.store.get(&waiting.id).unwrap().state, "silent");
}

#[test]
fn new_message_after_claim_defers_execution_without_consuming_reply() {
    struct NoCall;
    impl ModelClient for NoCall {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            _: &[Value],
            _: &[Value],
        ) -> crate::Result<Value> {
            panic!("must await quiet");
        }
    }
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    user.call(
        "message_send",
        json!({"conversation":chat,"body":"more detail","request_key":"later"}),
    )
    .unwrap();
    execute(
        home.path(),
        task.clone(),
        &mut NoCall,
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(scheduler.store.get(&task.id).unwrap().state, "queued");
    assert_eq!(scheduler.store.snapshot().unwrap().usage[0].replies, 0);
}

#[test]
fn three_agents_count_zero_through_ten_without_a_depth_cutoff() {
    let (home, user, chat) = fixture();
    AgentStore::new(home.path())
        .create(&AgentConfig::new("c", "fast").unwrap())
        .unwrap();
    im(home.path())
        .unwrap()
        .create_principal(PrincipalInput {
            id: PrincipalId::new("c"),
            kind: PrincipalKind::Agent,
            display_name: "c".into(),
            metadata: json!({}),
        })
        .unwrap();
    BindingStore::open(home.path())
        .unwrap()
        .bind(Binding {
            principal: PrincipalId::new("c"),
            agent: "c".into(),
            working_directory: home.path().into(),
            policy: ResponsePolicy::All,
            max_steps: 4,
            allow_mcp: false,
        })
        .unwrap();
    user.call("member_add", json!({"conversation":chat,"principal":"c"}))
        .unwrap();
    let mut scheduler = begin(home.path(), &user, &chat);
    for number in 0..=10 {
        scheduler.scan().unwrap();
        let principal = ["a", "b", "c"][number % 3];
        let mut task = claim(&mut scheduler, principal);
        if task.state == "evaluating" {
            execute(
                home.path(),
                task.clone(),
                &mut Answer(decision(
                    json!({"action":"participate","reason_code":"direct_question"}),
                )),
                &CancellationToken::new(),
            )
            .unwrap();
            assert_eq!(
                scheduler.store.get(&task.id).unwrap().state,
                "ready_to_reply"
            );
            task = claim(&mut scheduler, principal);
        }
        execute(
            home.path(),
            task.clone(),
            &mut Answer(json!({"role":"assistant","content":number.to_string()})),
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(
            scheduler.store.get(&task.id).unwrap().state,
            "completed",
            "stopped before reporting {number}"
        );
    }
    let history = user
        .call("message_history", json!({"conversation":chat}))
        .unwrap();
    let messages = history["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 12);
    for (number, message) in messages.iter().skip(1).enumerate() {
        assert_eq!(message["body"], number.to_string());
        assert_eq!(message["sender_id"], ["a", "b", "c"][number % 3]);
    }
    assert_eq!(messages.last().unwrap()["metadata"]["automated_depth"], 11);
}

#[test]
fn participation_retries_and_debits_each_actual_request() {
    struct Flaky {
        calls: usize,
    }
    impl ModelClient for Flaky {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            _: &[Value],
            _: &[Value],
        ) -> crate::Result<Value> {
            self.calls += 1;
            if self.calls == 1 {
                return Err(RuntimeError::HttpStatus(503));
            }
            Ok(decision(
                json!({"action":"silent","reason_code":"already_answered"}),
            ))
        }
    }
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    let mut flaky = Flaky { calls: 0 };
    let agent = ResolvedAgent::load(home.path(), "a").unwrap();
    let mut model = ScheduledModel {
        inner: &mut flaky,
        home: home.path().into(),
        task: task.clone(),
        token: CancellationToken::new(),
    };
    let result = crate::participation::decide(&mut model, &agent, "test").unwrap();
    assert!(matches!(result, ParticipationDecision::Silent { .. }));
    assert_eq!(flaky.calls, 2);
    assert_eq!(
        scheduler
            .store
            .discussion(&task.discussion)
            .unwrap()
            .unwrap()
            .calls,
        2
    );
}

#[test]
fn reset_stops_inflight_reply_preserves_history_and_excludes_old_input() {
    struct Reset {
        home: PathBuf,
        chat: String,
    }
    impl ModelClient for Reset {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            _: &[Value],
            _: &[Value],
        ) -> crate::Result<Value> {
            reset_conversation_context(&self.home, &self.chat, "user").unwrap();
            Ok(json!({"role":"assistant","content":"obsolete reply"}))
        }
    }
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    assert!(reset_conversation_context(home.path(), &chat, "a").is_err());
    execute(
        home.path(),
        task.clone(),
        &mut Reset {
            home: home.path().into(),
            chat: chat.clone(),
        },
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(scheduler.store.get(&task.id).unwrap().state, "canceled");
    let history = user
        .call("message_history", json!({"conversation":chat}))
        .unwrap();
    assert_eq!(history["messages"].as_array().unwrap().len(), 1);
    start_discussion(home.path(), &chat, None, false).unwrap();
    assert!(
        scheduler
            .store
            .snapshot()
            .unwrap()
            .discussions
            .iter()
            .all(|d| d.state == "ended")
    );
    let message = user
        .call(
            "message_send",
            json!({"conversation":chat,"body":"new topic","request_key":"new"}),
        )
        .unwrap();
    start_discussion(
        home.path(),
        &chat,
        Some(message["message"]["id"].as_str().unwrap()),
        false,
    )
    .unwrap();
    scheduler.scan().unwrap();
    let next = claim(&mut scheduler, "b");
    let agent = ResolvedAgent::load(home.path(), "b").unwrap();
    let context = crate::conversation_context::prepare(
        home.path(),
        &im(home.path()).unwrap(),
        &next,
        &agent,
        &mut Answer(json!({"role":"assistant","content":"unused"})),
        &CancellationToken::new(),
    )
    .unwrap();
    assert!(!context.to_string().contains("question"));
    assert!(context.to_string().contains("new topic"));
}

#[test]
fn conversation_summary_survives_reopen_is_private_and_invalidates_deleted_sources() {
    struct Summary {
        calls: usize,
    }
    impl ModelClient for Summary {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            messages: &[Value],
            tools: &[Value],
        ) -> crate::Result<Value> {
            self.calls += 1;
            assert!(tools.is_empty());
            assert!(
                messages[0]["content"]
                    .as_str()
                    .unwrap()
                    .starts_with("You summarize")
            );
            Ok(json!({"role":"assistant","content":"The user chose the blue theme."}))
        }
    }
    let (home, user, chat) = fixture();
    ModelStore::new(home.path())
        .update(
            "fast",
            agt_model::ModelConfigUpdate {
                context_window: Some(32768),
                ..Default::default()
            },
        )
        .unwrap();
    let mut first = None;
    for i in 0..30 {
        let sent = user.call("message_send", json!({"conversation":chat,"body":"blue theme requirement ".repeat(90),"request_key":format!("history-{i}")})).unwrap();
        if i == 0 {
            first = Some(sent["message"]["id"].as_str().unwrap().to_string());
        }
    }
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    let agent = ResolvedAgent::load(home.path(), "a").unwrap();
    let mut model = Summary { calls: 0 };
    let im = im(home.path()).unwrap();
    let context = crate::conversation_context::prepare(
        home.path(),
        &im,
        &task,
        &agent,
        &mut model,
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(context["summary"], "The user chose the blue theme.");
    assert!(model.calls > 0);
    let calls = model.calls;
    let again = crate::conversation_context::prepare(
        home.path(),
        &im,
        &task,
        &agent,
        &mut model,
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(context, again);
    assert_eq!(model.calls, calls);
    assert!(
        crate::context_store::ContextStore::open(home.path())
            .unwrap()
            .load(&chat, "b")
            .unwrap()
            .is_none()
    );
    im.delete_message(&MessageId::new(first.unwrap()), &PrincipalId::new("user"))
        .unwrap();
    crate::conversation_context::prepare(
        home.path(),
        &im,
        &task,
        &agent,
        &mut model,
        &CancellationToken::new(),
    )
    .unwrap();
    assert!(model.calls > calls);
    reset_conversation_context(home.path(), &chat, "user").unwrap();
    assert!(
        crate::context_store::ContextStore::open(home.path())
            .unwrap()
            .load(&chat, "a")
            .unwrap()
            .is_none()
    );
}

#[test]
fn reset_schema_upgrade_preserves_existing_discussions() {
    let home = tempfile::tempdir().unwrap();
    let store = ScheduleStore::open(home.path()).unwrap();
    store.start("existing", "chat", false, now_ms()).unwrap();
    store
        .connection
        .execute_batch("DROP TABLE context_resets; DROP TABLE work_fairness; DROP TABLE work_waits; DROP TABLE work_dependencies; DROP TABLE work_publications; DROP TABLE work_inbox; DROP TABLE work_outbox; DROP TABLE work_items; PRAGMA user_version=3;")
        .unwrap();
    drop(store);
    let migrated = ScheduleStore::open(home.path()).unwrap();
    assert!(migrated.discussion("existing").unwrap().is_some());
    assert_eq!(migrated.context_cutoff("chat").unwrap(), 0);
    assert_eq!(
        migrated
            .connection
            .pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
            .unwrap(),
        13
    );
}

#[test]
fn reset_during_summary_generation_cannot_recreate_the_summary() {
    struct ResetSummary {
        home: PathBuf,
        chat: String,
    }
    impl ModelClient for ResetSummary {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            _: &[Value],
            _: &[Value],
        ) -> crate::Result<Value> {
            reset_conversation_context(&self.home, &self.chat, "user").unwrap();
            Ok(json!({"role":"assistant","content":"obsolete summary"}))
        }
    }
    let (home, user, chat) = fixture();
    ModelStore::new(home.path())
        .update(
            "fast",
            agt_model::ModelConfigUpdate {
                context_window: Some(32768),
                ..Default::default()
            },
        )
        .unwrap();
    user.call(
        "message_send",
        json!({"conversation":chat,"body":"old context ".repeat(4000),"request_key":"history"}),
    )
    .unwrap();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    execute(
        home.path(),
        task.clone(),
        &mut ResetSummary {
            home: home.path().into(),
            chat: chat.clone(),
        },
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(scheduler.store.get(&task.id).unwrap().state, "canceled");
    assert!(
        crate::context_store::ContextStore::open(home.path())
            .unwrap()
            .load(&chat, "a")
            .unwrap()
            .is_none()
    );
}

#[cfg(unix)]
fn external_fixture() -> (tempfile::TempDir, ToolService, String) {
    use std::os::unix::fs::PermissionsExt;
    let (home, user, group) = fixture();
    let path = home.path();
    let exe = path.join("codex-fixture");
    std::fs::write(
        &exe,
        include_str!("../../agt-harness/tests/fixtures/codex.py"),
    )
    .unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
    agt_harness::HarnessStore::new(path)
        .write_in(
            &agt_core::CategoryName::default(),
            "local",
            &agt_harness::HarnessConfig {
                engine: "codex".into(),
                executable: exe,
                working_directory: path.into(),
                model: None,
                description: String::new(),
                reasoning_effort: None,
                sandbox: agt_harness::Sandbox::ReadOnly,
                timeout_seconds: 10,
            },
            true,
        )
        .unwrap();
    AgentStore::new(path)
        .update_in(&agt_core::CategoryName::default(), "a", |a| {
            a.model = None;
            a.harness = Some(agt_core::QualifiedRef::in_default(
                agt_core::HarnessId::new("local"),
            ));
            Ok(())
        })
        .unwrap();
    (home, user, group)
}
#[cfg(unix)]
#[test]
fn external_engine_uses_private_sessions_incremental_history_and_reset() {
    let (home, user, _) = external_fixture();
    let path = home.path();
    let chat = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut scheduler = begin(path, &user, &chat);
    struct NoModel;
    impl ModelClient for NoModel {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            _: &[Value],
            _: &[Value],
        ) -> crate::Result<Value> {
            panic!("external engine must never call native model")
        }
    }
    let task = claim(&mut scheduler, "a");
    execute(path, task.clone(), &mut NoModel, &CancellationToken::new()).unwrap();
    let done = scheduler.store.get(&task.id).unwrap();
    assert_eq!(done.state, "completed", "{:?}", done.reason);
    let snapshot = crate::HarnessSessions::open(path)
        .unwrap()
        .current(path, "a", &chat)
        .unwrap();
    assert_eq!(
        snapshot.config.model.as_deref(),
        Some("fixture-effective-model")
    );
    assert_eq!(snapshot.config.reasoning_effort.as_deref(), Some("high"));
    for (key, reset) in [("follow-up", false), ("fresh", true)] {
        if reset {
            reset_conversation_context(path, &chat, "user").unwrap();
        }
        let sent = user
            .call(
                "message_send",
                json!({"conversation":chat,"body":key,"request_key":key}),
            )
            .unwrap();
        start_discussion(
            path,
            &chat,
            Some(sent["message"]["id"].as_str().unwrap()),
            false,
        )
        .unwrap();
        scheduler.scan().unwrap();
        let task = claim(&mut scheduler, "a");
        execute(path, task.clone(), &mut NoModel, &CancellationToken::new()).unwrap();
        assert_eq!(scheduler.store.get(&task.id).unwrap().state, "completed");
    }
    let log = std::fs::read_to_string(path.join("protocol.jsonl")).unwrap();
    let events: Vec<Value> = log
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(
        events
            .iter()
            .filter(|e| e["method"] == "thread/start")
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e["method"] == "thread/resume")
            .count(),
        1
    );
    let turns: Vec<&str> = events
        .iter()
        .filter(|e| e["method"] == "turn/start")
        .map(|e| e["params"]["input"][0]["text"].as_str().unwrap())
        .collect();
    assert_eq!(turns, vec!["question", "follow-up", "fresh"]);
}

#[cfg(unix)]
#[test]
fn external_group_requires_mention_and_reset_cancels_without_publication() {
    let (home, user, chat) = external_fixture();
    let path = home.path();
    let mut scheduler = begin(path, &user, &chat);
    let task = claim(&mut scheduler, "a");
    execute(
        path,
        task.clone(),
        &mut Answer(json!({})),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(scheduler.store.get(&task.id).unwrap().state, "skipped");
    assert!(!path.join("protocol.jsonl").exists());
    let sent = user
        .call(
            "message_send",
            json!({"conversation":chat,"body":"HANG","mentions":["a"],"request_key":"hang"}),
        )
        .unwrap();
    start_discussion(
        path,
        &chat,
        Some(sent["message"]["id"].as_str().unwrap()),
        false,
    )
    .unwrap();
    scheduler.scan().unwrap();
    let task = claim(&mut scheduler, "a");
    let task_id = task.id.clone();
    let work = path.to_owned();
    let handle = std::thread::spawn(move || {
        execute(
            &work,
            task,
            &mut Answer(json!({})),
            &CancellationToken::new(),
        )
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !path.join("started").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    reset_conversation_context(path, &chat, "user").unwrap();
    handle.join().unwrap().unwrap();
    let runs = crate::RunStore::open(path).unwrap().list(10).unwrap();
    assert_eq!(runs[0].state, RunState::Canceled);
    assert_ne!(scheduler.store.get(&task_id).unwrap().state, "completed");
    let messages = user
        .call("message_history", json!({"conversation":chat}))
        .unwrap();
    assert_eq!(messages["messages"].as_array().unwrap().len(), 2);
}

#[cfg(unix)]
#[test]
fn external_group_transcript_preserves_senders_order_and_each_body_once() {
    let (home, user, chat) = external_fixture();
    let path = home.path();
    let mut scheduler = begin(path, &user, &chat);
    let peer = ToolService::open(
        path,
        Actor {
            principal: PrincipalId::new("b"),
            role: Role::Participant,
            automated_depth: None,
        },
    )
    .unwrap();
    peer.call("message_send",json!({"conversation":chat,"body":"API changes are ready\nPlease review.","request_key":"peer-one"})).unwrap();
    user.call(
        "message_send",
        json!({"conversation":chat,"body":"Please coordinate","request_key":"user-two"}),
    )
    .unwrap();
    let mut task = claim(&mut scheduler, "a");
    task.through_seq = im(path)
        .unwrap()
        .get_conversation(&ConversationId::new(&chat))
        .unwrap()
        .last_message_seq;
    let history =
        crate::conversation_context::visible_history(&im(path).unwrap(), &task, 0).unwrap();
    task.triggers = history
        .iter()
        .map(|m| m["message_id"].as_str().unwrap().to_owned())
        .collect();
    let text = crate::harness_input::render(
        &im(path).unwrap(),
        &crate::RunStore::open(path).unwrap(),
        &task,
        &history,
        0,
        None,
    )
    .unwrap();
    assert_eq!(text.matches("question").count(), 1);
    assert_eq!(text.matches("API changes are ready").count(), 1);
    assert_eq!(text.matches("Please coordinate").count(), 1);
    assert!(text.contains("\"b\" [id=\"b\", Agent]:\n  API changes are ready\n  Please review."));
    assert!(text.find("question").unwrap() < text.find("API changes").unwrap());
    assert!(text.find("API changes").unwrap() < text.find("Please coordinate").unwrap());
    assert!(!text.contains("context_reset_through_seq"));
}

#[test]
fn native_scheduled_input_keeps_peer_identity_without_duplicate_trigger_body() {
    let (home, user, chat) = fixture();
    let path = home.path();
    let peer = ToolService::open(
        path,
        Actor {
            principal: PrincipalId::new("b"),
            role: Role::Participant,
            automated_depth: None,
        },
    )
    .unwrap();
    peer.call(
        "message_send",
        json!({"conversation":chat,"body":"peer background","request_key":"peer-background"}),
    )
    .unwrap();
    let mut scheduler = begin(path, &user, &chat);
    let task = claim(&mut scheduler, "a");
    struct Capture(Vec<Value>);
    impl ModelClient for Capture {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            messages: &[Value],
            _: &[Value],
        ) -> crate::Result<Value> {
            self.0 = messages.to_vec();
            Ok(json!({"role":"assistant","content":"answer"}))
        }
    }
    let mut model = Capture(vec![]);
    execute(path, task, &mut model, &CancellationToken::new()).unwrap();
    let input: Value = serde_json::from_str(model.0[1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(input["messages"][0]["sender"], "user");
    assert!(input["trigger"].get("body").is_none());
    assert_eq!(input["history"][0]["sender"], "b");
    assert_eq!(input.to_string().matches("peer background").count(), 1);
    assert_eq!(input.to_string().matches("question").count(), 1);
}

#[cfg(unix)]
#[test]
fn harness_session_snapshots_models_names_and_compaction_are_isolated() {
    let (home, user, group) = external_fixture();
    let path = home.path();
    let chat = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let store = crate::HarnessSessions::open(path).unwrap();
    let session = store.current(path, "a", &chat).unwrap();
    assert!(session.session_id.is_none());
    let other = store.current(path, "a", &group).unwrap();
    assert_ne!(session.id, other.id);
    let harnesses = agt_harness::HarnessStore::new(path);
    let mut changed = harnesses
        .read_in(&agt_core::CategoryName::default(), "local")
        .unwrap()
        .config;
    changed.model = Some("changed-default".into());
    changed.working_directory = path.join("new-project");
    std::fs::create_dir(&changed.working_directory).unwrap();
    harnesses
        .write_in(&agt_core::CategoryName::default(), "local", &changed, false)
        .unwrap();
    assert_eq!(
        store.current(path, "a", &chat).unwrap().config,
        session.config
    );
    assert!(store.configure(&session.id, "invented", None).is_err());
    assert!(!store.get(&session.id).unwrap().busy);
    assert!(
        store
            .configure(&session.id, "fixture-model", Some("invented"))
            .is_err()
    );
    let configured = store
        .configure(&session.id, "fixture-model", Some("xhigh"))
        .unwrap();
    assert_eq!(configured.config.reasoning_effort.as_deref(), Some("xhigh"));
    assert_eq!(store.get(&other.id).unwrap().config.model, None);
    store.rename(&session.id, "Backend investigation").unwrap();
    let mut scheduler = begin(path, &user, &chat);
    let task = claim(&mut scheduler, "a");
    execute(
        path,
        task.clone(),
        &mut Answer(json!({})),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(scheduler.store.get(&task.id).unwrap().state, "completed");
    let completed = store.get(&session.id).unwrap();
    assert!(
        completed
            .session_id
            .as_deref()
            .unwrap()
            .starts_with("thread-")
    );
    store.rename(&session.id, "Backend follow-up").unwrap();
    store
        .compact(&session.id, &CancellationToken::new())
        .unwrap();
    assert_eq!(
        store.get(&session.id).unwrap().session_id,
        completed.session_id
    );
    let protocol: Vec<Value> = std::fs::read_to_string(path.join("protocol.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let turn = protocol
        .iter()
        .find(|e| e["method"] == "turn/start")
        .unwrap();
    assert_eq!(turn["params"]["model"], "fixture-model");
    assert_eq!(turn["params"]["effort"], "xhigh");
    assert!(protocol.iter().any(|e|e["method"]=="thread/name/set" && e["params"]["name"]=="Backend follow-up"));
    store
        .configure(&session.id, "fixture-model", Some("high"))
        .unwrap();
    let message=user.call("message_send",json!({"conversation":chat,"body":"continue after model change","request_key":"model-change"})).unwrap();
    start_discussion(
        path,
        &chat,
        Some(message["message"]["id"].as_str().unwrap()),
        false,
    )
    .unwrap();
    scheduler.scan().unwrap();
    let task = claim(&mut scheduler, "a");
    execute(
        path,
        task.clone(),
        &mut Answer(json!({})),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(scheduler.store.get(&task.id).unwrap().state, "completed");
    assert_eq!(
        store.get(&session.id).unwrap().session_id,
        completed.session_id
    );
    let log: Vec<Value> = std::fs::read_to_string(path.join("protocol.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let turn = log
        .iter()
        .rev()
        .find(|v| v["method"] == "turn/start")
        .unwrap();
    assert_eq!(turn["params"]["effort"], "high");
    assert_eq!(
        log.iter().filter(|v| v["method"] == "thread/start").count(),
        1
    );
    store.lock(&session.id).unwrap();
    assert!(store.configure(&session.id, "fixture-model", None).is_err());
    assert!(store.rename(&session.id, "busy").is_err());
    reset_conversation_context(path, &chat, "user").unwrap();
    let fresh = store.current(path, "a", &chat).unwrap();
    assert_ne!(fresh.id, session.id);
    assert_eq!(fresh.config, changed);
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

#[path = "scheduler_work_tests.rs"]
mod work_tests;

#[path = "scheduler_queue_tests.rs"]
mod queue_tests;

#[path = "scheduler_wait_tests.rs"]
mod wait_tests;

#[path = "scheduler_action_tests.rs"]
mod action_tests;

#[path = "scheduler_triage_tests.rs"]
mod triage_tests;

#[path = "scheduler_input_tests.rs"]
mod input_tests;
