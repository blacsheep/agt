use super::guard::ScheduledModel;
use super::*;

/// Python launcher of the MCP test fixtures; Windows installs it as `python`.
const PYTHON: &str = if cfg!(windows) { "python" } else { "python3" };
use crate::RunRequest;
use crate::participation::ParticipationDecision;
use crate::{ModelClient, ResolvedAgent, RunState, RuntimeError, ScheduledTask};
use agt_agent::{AgentConfig, AgentStore};
use agt_im::PrincipalInput;
use agt_im::{MessageId, PrincipalKind};
use agt_model::{ModelConfig, ModelReference, ModelStore};
use agt_provider::{ProviderConfig, ProviderStore};
use agt_service::{Actor, MessageInput, Messaging, ResponsePolicy, Role};
use agt_service::{Binding, ToolService};
use serde_json::{Value, json};
use std::sync::mpsc;

fn fixture() -> (tempfile::TempDir, ToolService, String) {
    let home = tempfile::tempdir().unwrap();
    let path = home.path();
    agt_config::initialize_home(Some(path.into())).unwrap();
    let mut policy = agt_config::RuntimeConfig::load(path).unwrap();
    policy.permissions.mode = agt_config::runtime::PermissionMode::AutoAllow;
    policy.save(path).unwrap();
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
    start_discussion(home, chat, Some(sent["message"]["id"].as_str().unwrap())).unwrap();
    let mut scheduler = Scheduler::open(home, 3).unwrap();
    scheduler.scan().unwrap();
    scheduler
}
fn make_quiet(scheduler: &mut Scheduler) {
    rusqlite::Connection::open(scheduler.home.join("im/im.db"))
        .unwrap()
        .execute("UPDATE messages SET created_at=?1", [now_ms() - 5_000])
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
/// Renew the coordinator lease before a claim, as a running scheduler's tick does.
/// Without it, a slow runner can spend the whole lease on setup and the claimed
/// task's settlement is then rejected as unowned.
fn renew_lease(scheduler: &Scheduler) {
    assert!(
        scheduler
            .store
            .heartbeat(&scheduler.owner, now_ms())
            .unwrap(),
        "lease lost"
    );
}
fn claim(scheduler: &mut Scheduler, principal: &str) -> ScheduledTask {
    renew_lease(scheduler);
    make_quiet(scheduler);
    let task = scheduler
        .store
        .candidates()
        .unwrap()
        .into_iter()
        .find(|t| t.principal == principal)
        .unwrap();
    let now = now_ms();
    scheduler
        .store
        .claim(&task.id, &scheduler.owner, now)
        .unwrap()
        .unwrap_or_else(|| {
            panic!(
                "{}",
                scheduler.store.claim_state(&task.id, &scheduler.owner, now)
            )
        })
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
        decisions: std::sync::Arc<std::sync::atomic::AtomicU32>,
    }
    impl ModelClient for Blocking {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            _: &[Value],
            tools: &[Value],
        ) -> crate::Result<Value> {
            if tools
                .first()
                .is_some_and(|t| t["function"]["name"] == "participation_decision")
            {
                self.decisions
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Ok(decision(
                    json!({"action":"silent","reason_code":"already_answered"}),
                ));
            }
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
    let decisions = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let mut releases = Vec::new();
    let mut threads = Vec::new();
    for task in tasks {
        let path = home.path().to_owned();
        let started = tx.clone();
        let decisions = decisions.clone();
        let (release, wait) = mpsc::channel();
        releases.push(release);
        threads.push(thread::spawn(move || {
            execute(
                &path,
                task,
                &mut Blocking {
                    started,
                    release: wait,
                    decisions,
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
    let runs = crate::RunStore::open(home.path())
        .unwrap()
        .list_for_conversation(&chat, 50)
        .unwrap();
    assert_eq!(runs.len(), 2);
    for message in history["messages"].as_array().unwrap().iter().skip(1) {
        assert_eq!(message["metadata"]["discussion_id"], round);
        // Final replies name the Run that produced them.
        assert!(
            runs.iter()
                .any(|r| message["metadata"]["run_id"] == r.id.as_str())
        );
    }
    assert_eq!(
        scheduler.store.discussion(&round).unwrap().unwrap().calls,
        2 + decisions.load(std::sync::atomic::Ordering::Relaxed)
    );
}

fn peer_task(home: &Path, user: &ToolService, chat: &str) -> (Scheduler, ScheduledTask) {
    let mut scheduler = begin(home, user, chat);
    let mut a = claim(&mut scheduler, "a");
    let mut b = claim(&mut scheduler, "b");
    a.state = TaskState::Silent;
    b.state = TaskState::Silent;
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
            input_mode: None,
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

/// Give `principal` a reply it will run without deciding first, so it counts as working.
fn keep_busy(scheduler: &Scheduler, like: &ScheduledTask, principal: &str) -> ScheduledTask {
    let mut busy = like.clone();
    busy.id = format!("busy-{principal}");
    busy.principal = principal.into();
    busy.deliveries = vec![900_000];
    busy.state = TaskState::Queued;
    busy.decision_required = false;
    busy.owner = None;
    busy.due_at = now_ms() + 3_600_000;
    scheduler
        .store
        .connection
        .execute(
            "INSERT INTO tasks VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            rusqlite::params![
                busy.id,
                busy.discussion,
                busy.principal,
                busy.conversation,
                busy.state,
                busy.due_at,
                busy.quiet,
                busy.version,
                busy.owner,
                serde_json::to_string(&busy).unwrap()
            ],
        )
        .unwrap();
    busy
}

struct NoModel;
impl ModelClient for NoModel {
    fn complete(&mut self, _: &ResolvedAgent, _: &[Value], _: &[Value]) -> crate::Result<Value> {
        panic!("a wait that ends with nothing new must not call the model")
    }
}

#[test]
fn model_wait_blocks_on_a_working_member_until_it_settles() {
    let (home, user, chat) = fixture();
    let (scheduler, task) = peer_task(home.path(), &user, &chat);
    let mut busy = keep_busy(&scheduler, &task, "a");
    let answer = decision(
        json!({"action":"wait","reason_code":"observe_discussion","public_reason":"先等待其他成员给出评估。"}),
    );
    execute(
        home.path(),
        task.clone(),
        &mut Answer(answer),
        &CancellationToken::new(),
    )
    .unwrap();
    let waiting = scheduler.store.get(&task.id).unwrap();
    assert_eq!(waiting.state, TaskState::Waiting);
    assert_eq!(waiting.blocked_on, vec!["a"]);
    assert!(waiting.due_at > now_ms() + 290_000);
    drop(scheduler);
    let mut restarted = Scheduler::open(home.path(), 3).unwrap();
    let saved = serde_json::to_value(restarted.store.get(&task.id).unwrap()).unwrap();
    assert_eq!(saved["state"], "waiting");
    assert_eq!(saved["last_decision"]["action"], "wait");
    assert_eq!(
        saved["last_decision"]["public_reason"],
        "先等待其他成员给出评估。"
    );
    assert!(saved["last_decision"].get("wake").is_none());
    assert_eq!(saved["last_decision"]["through_seq"], task.through_seq);
    assert!(saved["last_decision"]["decided_at"].as_i64().unwrap() > 0);
    assert!(
        restarted
            .store
            .claim(&task.id, &restarted.owner, now_ms())
            .unwrap()
            .is_none()
    );
    // a settles without saying anything: b wakes, sees nothing new, and stops waiting.
    busy.state = TaskState::Silent;
    crate::schedule_store::save_task(&restarted.store.connection, &busy).unwrap();
    let woken = restarted
        .store
        .claim(&task.id, &restarted.owner, now_ms())
        .unwrap()
        .expect("settled member wakes the wait");
    assert_eq!(woken.state, TaskState::Evaluating);
    execute(home.path(), woken, &mut NoModel, &CancellationToken::new()).unwrap();
    let ended = restarted.store.get(&task.id).unwrap();
    assert_eq!(ended.state, TaskState::Silent);
    assert_eq!(
        ended.reason.as_deref(),
        Some("wait ended with no new messages")
    );
}

#[test]
fn wait_is_unavailable_when_nobody_is_working() {
    struct Offered(Vec<Value>);
    impl ModelClient for Offered {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            _: &[Value],
            tools: &[Value],
        ) -> crate::Result<Value> {
            self.0 = tools.to_vec();
            Ok(decision(
                json!({"action":"wait","reason_code":"observe_discussion"}),
            ))
        }
    }
    let (home, user, chat) = fixture();
    let (scheduler, task) = peer_task(home.path(), &user, &chat);
    let mut model = Offered(Vec::new());
    execute(
        home.path(),
        task.clone(),
        &mut model,
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        model.0[0]["function"]["parameters"]["properties"]["action"]["enum"],
        json!(["participate", "silent"])
    );
    let failed = scheduler.store.get(&task.id).unwrap();
    assert_eq!(failed.state, TaskState::Failed);
    assert!(failed.reason.unwrap().contains("wait is unavailable"));
}

#[test]
fn a_new_peer_message_joins_the_waiting_decision() {
    let (home, user, chat) = fixture();
    let (mut scheduler, task) = peer_task(home.path(), &user, &chat);
    keep_busy(&scheduler, &task, "a");
    execute(
        home.path(),
        task.clone(),
        &mut Answer(decision(
            json!({"action":"wait","reason_code":"observe_discussion"}),
        )),
        &CancellationToken::new(),
    )
    .unwrap();
    Messaging::open(
        home.path(),
        Actor {
            principal: PrincipalId::new("a"),
            role: Role::Participant,
            automated_depth: Some(1),
        },
    )
    .unwrap()
    .with_discussion(Some(task.discussion.clone()))
    .send(MessageInput {
        input_mode: None,
        conversation: ConversationId::new(&chat),
        body: "second idea".into(),
        request_key: "peer-2".into(),
        mentions: vec![],
        reply_to: None,
    })
    .unwrap();
    scheduler.scan().unwrap();
    let merged = scheduler.store.get(&task.id).unwrap();
    assert_eq!(merged.state, TaskState::Queued);
    assert_eq!(merged.triggers.len(), 2);
    assert!(merged.blocked_on.is_empty());
    assert_eq!(
        scheduler
            .store
            .tasks()
            .unwrap()
            .iter()
            .filter(|t| t.principal == "b" && !t.is_terminal())
            .count(),
        1
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
        assert_eq!(
            scheduler.store.get(&task.id).unwrap().state,
            TaskState::Failed
        );
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
                .control(&self.chat, "end", now_ms())
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
        assert_eq!(
            scheduler.store.get(&task.id).unwrap().state,
            TaskState::Canceled
        );
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
        TaskState::ReadyToReply
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
    // A tool-sent message carries the provenance of the Run that sent it.
    let runs = crate::RunStore::open(home.path())
        .unwrap()
        .list_for_conversation(&chat, 50)
        .unwrap();
    assert_eq!(
        history["messages"][2]["metadata"]["run_id"],
        runs[0].id.as_str()
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
    assert_eq!(
        scheduler.store.get(&task.id).unwrap().state,
        TaskState::Completed
    );
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
fn ordinary_inputs_are_immediate_but_explicit_quiet_waits_remain() {
    let (home, user, chat) = fixture();
    let scheduler = begin(home.path(), &user, &chat);
    let mut task = scheduler.store.candidates().unwrap().remove(0);
    task.due_at = 0;
    assert!(quiet_ready(&task, 4000, 4000));
    task.quiet = true;
    task.deadline = 50_000;
    task.quiet_ms = 30_000;
    assert!(!quiet_ready(&task, 4000, 5000));
    assert!(quiet_ready(&task, 4000, 34_000));
    assert!(quiet_ready(&task, 49_999, 50_000));
}

#[test]
fn human_burst_queues_independent_inputs_without_fixed_delay() {
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
        )
        .unwrap();
    }
    scheduler.scan().unwrap();
    let pending = scheduler.store.candidates().unwrap();
    assert_eq!(pending.len(), 6);
    assert!(
        pending
            .iter()
            .all(|t| t.deliveries.len() == 1 && t.triggers.len() == 1 && t.due_at <= now_ms())
    );
    let task = claim(&mut scheduler, "a");
    execute(
        home.path(),
        task.clone(),
        &mut Answer(json!({"role":"assistant","content":"one reply"})),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        scheduler.store.get(&task.id).unwrap().state,
        TaskState::Completed
    );
    assert!(
        scheduler
            .store
            .tasks()
            .unwrap()
            .iter()
            .filter(|t| t.principal == "a")
            .all(|t| t.state == TaskState::Completed)
    );
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
        crate::DiscussionState::Active
    );
    delete_scheduled_conversation(home.path(), &chat, "user").unwrap();
    assert_eq!(
        scheduler.store.snapshot().unwrap().discussions[0].state,
        crate::DiscussionState::Ended
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
    assert!(tasks.iter().all(|t| t.state == TaskState::ReadyToReply));
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
    assert_eq!(
        scheduler.store.get(&waiting.id).unwrap().state,
        TaskState::Silent
    );
}

#[test]
fn unauthorized_new_message_does_not_delay_or_steer_active_execution() {
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
        &mut Answer(json!({"role":"assistant","content":"done"})),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        scheduler.store.get(&task.id).unwrap().state,
        TaskState::Completed
    );
    assert_eq!(scheduler.store.snapshot().unwrap().usage[0].replies, 1);
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
        })
        .unwrap();
    user.call("member_add", json!({"conversation":chat,"principal":"c"}))
        .unwrap();
    let mut scheduler = begin(home.path(), &user, &chat);
    for number in 0..=10 {
        scheduler.scan().unwrap();
        let principal = ["a", "b", "c"][number % 3];
        // This test exercises a single causal chain. Older independent inputs
        // are deliberately skipped instead of implicitly merged into the newest.
        let pending = scheduler.store.candidates().unwrap();
        let newest = pending
            .iter()
            .filter(|t| t.principal == principal)
            .max_by_key(|t| t.through_seq)
            .unwrap()
            .id
            .clone();
        for old in pending
            .iter()
            .filter(|t| t.principal == principal && t.id != newest)
        {
            scheduler.store.cancel_pending(&old.id).unwrap();
        }
        let mut task = claim(&mut scheduler, principal);
        if task.state == TaskState::Evaluating {
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
                TaskState::ReadyToReply
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
            TaskState::Completed,
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
    let result = crate::participation::decide(&mut model, &agent, "test", true).unwrap();
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
    assert_eq!(
        scheduler.store.get(&task.id).unwrap().state,
        TaskState::Canceled
    );
    let history = user
        .call("message_history", json!({"conversation":chat}))
        .unwrap();
    assert_eq!(history["messages"].as_array().unwrap().len(), 1);
    start_discussion(home.path(), &chat, None).unwrap();
    assert!(
        scheduler
            .store
            .snapshot()
            .unwrap()
            .discussions
            .iter()
            .all(|d| d.state == DiscussionState::Ended)
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
    assert_eq!(
        scheduler.store.get(&task.id).unwrap().state,
        TaskState::Canceled
    );
    assert!(
        crate::context_store::ContextStore::open(home.path())
            .unwrap()
            .load(&chat, "a")
            .unwrap()
            .is_none()
    );
}

/// Writes an executable script from a child process. Were this process to hold
/// the file open for writing, a child forked meanwhile by a parallel test could
/// inherit the descriptor, and running the script would fail with ETXTBSY.
#[cfg(unix)]
fn write_executable(path: &std::path::Path, script: &str) {
    use std::io::Write;
    let mut child = std::process::Command::new("sh")
        .args(["-c", "cat > \"$0\" && chmod 700 \"$0\""])
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
}

#[cfg(unix)]
fn external_fixture() -> (tempfile::TempDir, ToolService, String) {
    let (home, user, group) = fixture();
    let path = home.path();
    let exe = path.join("codex-fixture");
    write_executable(
        &exe,
        include_str!("../../../agt-harness/tests/fixtures/codex.py"),
    );
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
                args: Vec::new(),
                auth_method: None,
                inherit_mcp_servers: true,
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
fn deepseek_harness_agent_replies_through_acp_and_resumes_its_session() {
    let (home, user, _) = fixture();
    let path = home.path();
    let exe = path.join("dsh-fixture");
    write_executable(
        &exe,
        include_str!("../../../agt-harness/tests/fixtures/acp.py"),
    );
    agt_harness::HarnessStore::new(path)
        .write_in(
            &agt_core::CategoryName::default(),
            "dsh",
            &agt_harness::HarnessConfig {
                engine: "deepseek-harness".into(),
                executable: exe,
                working_directory: path.into(),
                model: None,
                description: String::new(),
                reasoning_effort: None,
                sandbox: agt_harness::Sandbox::WorkspaceWrite,
                timeout_seconds: 10,
                args: Vec::new(),
                auth_method: None,
                inherit_mcp_servers: true,
            },
            true,
        )
        .unwrap();
    AgentStore::new(path)
        .update_in(&agt_core::CategoryName::default(), "a", |a| {
            a.model = None;
            a.harness = Some(agt_core::QualifiedRef::in_default(
                agt_core::HarnessId::new("dsh"),
            ));
            Ok(())
        })
        .unwrap();
    let chat = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut scheduler = begin(path, &user, &chat);
    for (round, key) in ["first", "follow-up"].into_iter().enumerate() {
        if round > 0 {
            let sent = user
                .call(
                    "message_send",
                    json!({"conversation":chat,"body":key,"request_key":key}),
                )
                .unwrap();
            start_discussion(path, &chat, Some(sent["message"]["id"].as_str().unwrap())).unwrap();
            scheduler.scan().unwrap();
        }
        let task = claim(&mut scheduler, "a");
        execute(
            path,
            task.clone(),
            &mut Answer(json!({})),
            &CancellationToken::new(),
        )
        .unwrap();
        let done = scheduler.store.get(&task.id).unwrap();
        assert_eq!(done.state, TaskState::Completed, "{:?}", done.reason);
    }
    let replies: Vec<String> = im(path)
        .unwrap()
        .list_messages(
            &ConversationId::new(&chat),
            &PrincipalId::new("user"),
            agt_im::MessageListOptions {
                limit: 50,
                before: None,
                after: None,
            },
        )
        .unwrap()
        .messages
        .into_iter()
        .filter(|m| m.sender_id.as_str() == "a")
        .map(|m| m.body)
        .collect();
    assert_eq!(replies.len(), 2);
    assert!(
        replies.iter().all(|r| r.ends_with("session:acp-session-1")),
        "{replies:?}"
    );
    let session = crate::HarnessSessions::open(path)
        .unwrap()
        .current(path, "a", &chat)
        .unwrap();
    assert_eq!(session.session_id.as_deref(), Some("acp-session-1"));
    assert_eq!(session.config.model.as_deref(), Some("deepseek-v4-flash"));
    let frames: Vec<Value> = std::fs::read_to_string(path.join("protocol.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        frames
            .iter()
            .filter(|f| f["method"] == "session/new")
            .count(),
        1
    );
    assert_eq!(
        frames
            .iter()
            .filter(|f| f["method"] == "session/resume")
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn claude_code_agent_replies_and_resumes_its_session() {
    let (home, user, _) = fixture();
    let path = home.path();
    let exe = path.join("claude-fixture");
    write_executable(
        &exe,
        include_str!("../../../agt-harness/tests/fixtures/claude.py"),
    );
    agt_harness::HarnessStore::new(path)
        .write_in(
            &agt_core::CategoryName::default(),
            "claude",
            &agt_harness::HarnessConfig {
                engine: "claude-code".into(),
                executable: exe,
                working_directory: path.into(),
                model: None,
                description: String::new(),
                reasoning_effort: None,
                sandbox: agt_harness::Sandbox::WorkspaceWrite,
                timeout_seconds: 10,
                args: Vec::new(),
                auth_method: None,
                inherit_mcp_servers: true,
            },
            true,
        )
        .unwrap();
    AgentStore::new(path)
        .update_in(&agt_core::CategoryName::default(), "a", |a| {
            a.model = None;
            a.harness = Some(agt_core::QualifiedRef::in_default(
                agt_core::HarnessId::new("claude"),
            ));
            Ok(())
        })
        .unwrap();
    let chat = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut scheduler = begin(path, &user, &chat);
    let replies = |chat: &str| -> Vec<String> {
        im(path)
            .unwrap()
            .list_messages(
                &ConversationId::new(chat),
                &PrincipalId::new("user"),
                agt_im::MessageListOptions {
                    limit: 50,
                    before: None,
                    after: None,
                },
            )
            .unwrap()
            .messages
            .into_iter()
            .filter(|m| m.sender_id.as_str() == "a")
            .map(|m| m.body)
            .collect()
    };
    for (round, key) in ["first", "follow-up"].into_iter().enumerate() {
        if round > 0 {
            let sent = user
                .call(
                    "message_send",
                    json!({"conversation":chat,"body":key,"request_key":key}),
                )
                .unwrap();
            start_discussion(path, &chat, Some(sent["message"]["id"].as_str().unwrap())).unwrap();
            scheduler.scan().unwrap();
        }
        let task = claim(&mut scheduler, "a");
        execute(
            path,
            task.clone(),
            &mut Answer(json!({})),
            &CancellationToken::new(),
        )
        .unwrap();
        let done = scheduler.store.get(&task.id).unwrap();
        assert_eq!(done.state, TaskState::Completed, "{:?}", done.reason);
        let args: Vec<String> =
            serde_json::from_str(&std::fs::read_to_string(path.join("args.json")).unwrap())
                .unwrap();
        assert_eq!(args.contains(&"--resume".to_owned()), round > 0);
    }
    assert_eq!(
        replies(&chat),
        ["done  session:session-new", "done  session:session-new"]
    );
    let session = crate::HarnessSessions::open(path)
        .unwrap()
        .current(path, "a", &chat)
        .unwrap();
    assert_eq!(session.session_id.as_deref(), Some("session-new"));
    assert_eq!(session.config.model.as_deref(), Some("claude-fixture"));
    let turns: Vec<String> = std::fs::read_to_string(path.join("protocol.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|v| v["type"] == "user")
        .map(|v| {
            v["message"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(turns.len(), 2);
    assert!(
        turns[1].contains("follow-up") && !turns[1].contains("question"),
        "{turns:?}"
    );
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
    assert_eq!(done.state, TaskState::Completed, "{:?}", done.reason);
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
        start_discussion(path, &chat, Some(sent["message"]["id"].as_str().unwrap())).unwrap();
        scheduler.scan().unwrap();
        let task = claim(&mut scheduler, "a");
        execute(path, task.clone(), &mut NoModel, &CancellationToken::new()).unwrap();
        assert_eq!(
            scheduler.store.get(&task.id).unwrap().state,
            TaskState::Completed
        );
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
    assert_eq!(
        scheduler.store.get(&task.id).unwrap().state,
        TaskState::Skipped
    );
    assert!(!path.join("protocol.jsonl").exists());
    let sent = user
        .call(
            "message_send",
            json!({"conversation":chat,"body":"HANG","mentions":["a"],"request_key":"hang"}),
        )
        .unwrap();
    start_discussion(path, &chat, Some(sent["message"]["id"].as_str().unwrap())).unwrap();
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
    assert_eq!(runs[0].state, RunState::OutcomeUnknown);
    assert_eq!(
        runs[0].termination.as_ref().unwrap().code,
        crate::StopCode::ContextReset
    );
    assert_ne!(
        scheduler.store.get(&task_id).unwrap().state,
        TaskState::Completed
    );
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
    let history = crate::conversation_context::visible_history(
        &im(path).unwrap(),
        &task,
        0,
        &jiff::tz::TimeZone::UTC,
    )
    .unwrap();
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
fn mentioned_policy_answers_only_mentions_and_reply_mentions_wake_members() {
    let (home, user, chat) = fixture();
    let path = home.path();
    BindingStore::open(path)
        .unwrap()
        .set_policy(
            &ConversationId::new(&chat),
            &PrincipalId::new("b"),
            ResponsePolicy::Mentioned,
        )
        .unwrap();
    let mut scheduler = begin(path, &user, &chat);
    let reply =
        json!({"role":"assistant","content":"Drafted it. @b please review, and ask @all later."});
    execute(
        path,
        claim(&mut scheduler, "a"),
        &mut Answer(reply),
        &CancellationToken::new(),
    )
    .unwrap();
    let a = ToolService::open(
        path,
        Actor {
            principal: PrincipalId::new("a"),
            role: Role::Participant,
            automated_depth: Some(1),
        },
    )
    .unwrap();
    a.call(
        "message_send",
        json!({"conversation":chat,"body":"an aside nobody was asked about","request_key":"aside"}),
    )
    .unwrap();
    let published = im(path)
        .unwrap()
        .list_messages(
            &ConversationId::new(&chat),
            &PrincipalId::new("user"),
            MessageListOptions {
                limit: 10,
                before: None,
                after: None,
            },
        )
        .unwrap()
        .messages;
    let draft = published
        .iter()
        .find(|m| m.body.starts_with("Drafted"))
        .unwrap();
    assert_eq!(draft.metadata["mentions"], json!(["b"]));
    struct Count(u32, Value);
    impl ModelClient for Count {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            messages: &[Value],
            _: &[Value],
        ) -> crate::Result<Value> {
            self.0 += 1;
            assert!(
                messages
                    .to_vec()
                    .iter()
                    .any(|m| m.to_string().contains("please review"))
            );
            Ok(self.1.clone())
        }
    }
    let mut model = Count(0, json!({"role":"assistant","content":"Reviewed."}));
    scheduler.scan().unwrap();
    let pending = |s: &Scheduler| {
        s.store
            .candidates()
            .unwrap()
            .iter()
            .filter(|t| t.principal == "b")
            .count()
    };
    // b's pending work: the question merged with the draft that mentions b, then the aside.
    while pending(&scheduler) > 0 {
        execute(
            path,
            claim(&mut scheduler, "b"),
            &mut model,
            &CancellationToken::new(),
        )
        .unwrap();
    }
    assert_eq!(model.0, 1, "only the mention reaches the model");
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
    assert_eq!(
        scheduler.store.get(&task.id).unwrap().state,
        TaskState::Completed
    );
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
    assert_eq!(
        scheduler.store.get(&task.id).unwrap().state,
        TaskState::Completed
    );
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
    let _lease = store.execution_lock(&session.id).unwrap();
    assert!(store.configure(&session.id, "fixture-model", None).is_err());
    assert!(store.rename(&session.id, "busy").is_err());
    reset_conversation_context(path, &chat, "user").unwrap();
    let fresh = store.current(path, "a", &chat).unwrap();
    assert_ne!(fresh.id, session.id);
    assert_eq!(fresh.config, changed);
}

mod work_tests;

mod queue_tests;

mod wait_tests;

mod action_tests;

mod triage_tests;

mod input_tests;

#[cfg(unix)]
mod harness_session_lock_tests;

mod reset_history_tests;

#[cfg(unix)]
mod harness_tool_tests;

mod run_input_tests;

mod timer_tests;

mod goal_tests;

#[test]
fn scheduled_native_uses_mounted_mcp_without_an_opt_in() {
    let (home, user, _) = fixture();
    let server = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/contract_mcp.py");
    user.call("resource_write", json!({"kind":"mcp","reference":"counter","create":true,
        "data":{"mcpServers":{"counter":{"cwd":std::env::temp_dir(),"command":PYTHON,"args":[server,"valid",home.path().join("calls")]}}}})).unwrap();
    user.call(
        "resource_write",
        json!({"kind":"agent","reference":"a","create":false,
        "data":{"mcps":["counter"],"builtin_tools":[]}}),
    )
    .unwrap();
    let chat = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    let id = task.id.clone();
    struct UseMcp(bool);
    impl ModelClient for UseMcp {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            _: &[Value],
            tools: &[Value],
        ) -> crate::Result<Value> {
            if self.0 {
                return Ok(json!({"role":"assistant","content":"counted"}));
            }
            self.0 = true;
            let tool = tools
                .iter()
                .find(|t| t["function"]["name"].as_str().unwrap().starts_with("mcp_"))
                .expect("mounted tool is available");
            Ok(
                json!({"role":"assistant","tool_calls":[{"id":"count","type":"function",
                "function":{"name":tool["function"]["name"],"arguments":"{\"count\":1}"}}]}),
            )
        }
    }
    execute(
        home.path(),
        task,
        &mut UseMcp(false),
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        scheduler.store.get(&id).unwrap().state,
        TaskState::Completed
    );
    assert!(home.path().join("calls").exists());
    // Activity names the MCP tool, not its hashed alias.
    let store = crate::RunStore::open(home.path()).unwrap();
    let run = store.list_for_conversation(&chat, 1).unwrap().remove(0);
    let call = store
        .calls(&run.id, 0, 50)
        .unwrap()
        .into_iter()
        .find(|c| {
            c.tool_name
                .as_deref()
                .is_some_and(|n| n.starts_with("mcp_"))
        })
        .unwrap();
    assert_eq!(call.public_label.as_deref(), Some("counter / count"));
}

#[cfg(unix)]
#[test]
fn unavailable_hints_do_not_stop_scheduler_or_cancellation() {
    let (home, _, _) = fixture();
    std::fs::create_dir_all(home.path().join("runtime")).unwrap();
    std::fs::write(home.path().join("runtime/wake"), b"blocked").unwrap();
    let mut scheduler = Scheduler::open(home.path(), 1).unwrap();
    let token = CancellationToken::new();
    let cancel = token.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        cancel.cancel();
    });
    scheduler.run(true, &token).unwrap();
    assert!(token.is_canceled());
    thread.join().unwrap();
}

/// Records the input of each model call and answers with a fixed message.
struct Capture(Value, Vec<Vec<Value>>);
impl ModelClient for Capture {
    fn complete(
        &mut self,
        _: &ResolvedAgent,
        messages: &[Value],
        _: &[Value],
    ) -> crate::Result<Value> {
        self.1.push(messages.to_vec());
        Ok(self.0.clone())
    }
}

#[test]
fn model_input_puts_what_changes_every_call_last() {
    let (home, user, chat) = fixture();
    let mut scheduler = begin(home.path(), &user, &chat);
    let task = claim(&mut scheduler, "a");
    let mut model = Capture(json!({"role":"assistant","content":"ok"}), Vec::new());
    execute(home.path(), task, &mut model, &CancellationToken::new()).unwrap();
    let input = model.1[0]
        .iter()
        .find(|m| m["role"] == "user")
        .and_then(|m| m["content"].as_str())
        .expect("scheduled input")
        .to_owned();
    let position = |key: &str| {
        input
            .find(&format!("\"{key}\":"))
            .unwrap_or_else(|| panic!("{key} missing from {input}"))
    };
    let stable = [
        "instruction",
        "your_principal",
        "conversation",
        "timezone",
        "history",
    ];
    let volatile = ["presence", "trigger", "messages", "now"];
    for s in stable {
        for v in volatile {
            assert!(position(s) < position(v), "{s} must come before {v}");
        }
    }
    assert!(!input.contains("now_unix_ms"));
    let parsed: Value = serde_json::from_str(&input).unwrap();
    assert!(parsed["now"].as_str().unwrap().contains('T'));
}
