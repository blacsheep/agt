use std::collections::VecDeque;
use std::path::Path;

use agt_agent::{AgentConfig, AgentStore};
use agt_core::CancellationToken;
use agt_im::{ImStore, MessageListOptions, PrincipalId};
use agt_mcp::{DefaultMcpClientFactory, McpDocument, McpStore};
use agt_model::{ModelConfig, ModelReference, ModelStore};
use agt_prompt::PromptStore;
use agt_provider::{ProviderConfig, ProviderStore};
use agt_runtime::{
    ModelClient, ResolvedAgent, Result, RunRequest, RunState, RunStore, Runtime, RuntimeError,
};
use serde_json::{Value, json};

fn home() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    agt_config::initialize_home(Some(temp.path().to_owned())).unwrap();
    PromptStore::new(temp.path())
        .ensure_default_prompt()
        .unwrap();
    ProviderStore::new(temp.path())
        .create(
            &ProviderConfig::openai_compatible(
                "local",
                "http://127.0.0.1:1/v1",
                "test-secret-do-not-persist",
            )
            .unwrap(),
        )
        .unwrap();
    ModelStore::new(temp.path())
        .create(
            &ModelConfig::new("fast", ModelReference::parse("local/test-model").unwrap()).unwrap(),
        )
        .unwrap();
    for name in ["a", "b"] {
        AgentStore::new(temp.path())
            .create(&AgentConfig::new(name, "fast").unwrap())
            .unwrap();
    }
    temp
}

#[derive(Default)]
struct Script {
    answers: VecDeque<(&'static str, Value)>,
    seen: Vec<(String, Vec<Value>, Vec<Value>)>,
}

impl Script {
    fn new(answers: Vec<(&'static str, Value)>) -> Self {
        Self {
            answers: answers.into(),
            ..Self::default()
        }
    }
}

impl ModelClient for Script {
    fn complete(
        &mut self,
        agent: &ResolvedAgent,
        messages: &[Value],
        tools: &[Value],
    ) -> Result<Value> {
        self.seen
            .push((agent.name.clone(), messages.to_vec(), tools.to_vec()));
        let (expected, answer) = self
            .answers
            .pop_front()
            .expect("unexpected model execution");
        assert_eq!(agent.name, expected);
        Ok(answer)
    }
}

fn text(body: &str) -> Value {
    json!({"role":"assistant","content":body})
}
fn call(id: &str, name: &str, args: Value) -> Value {
    json!({"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}})
}
fn calls(calls: Vec<Value>) -> Value {
    json!({"role":"assistant","content":null,"tool_calls":calls})
}
fn delegate() -> Value {
    calls(vec![call(
        "delegate-1",
        "delegate",
        json!({"agent":"default/b","input":"review this"}),
    )])
}
fn request() -> RunRequest {
    let mut request = RunRequest::new("a", "solve this");
    request.request_key = "stable-request".into();
    request
}

#[test]
fn completed_run_replays_after_reopening_without_loading_changed_configuration() {
    let home = home();
    let mut model = Script::new(vec![("default/a", text("done"))]);
    let mut runtime = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory).unwrap();
    let first = runtime.run(request(), &CancellationToken::new()).unwrap();
    assert_eq!(first.state, RunState::Completed);
    assert_eq!(first.model_steps, 1);
    assert!(first.config_fingerprint.is_some());
    assert!(
        !serde_json::to_string(&first)
            .unwrap()
            .contains("test-secret")
    );
    drop(runtime);
    AgentStore::new(home.path()).delete("a").unwrap();
    let mut runtime = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory).unwrap();
    let replay = runtime.run(request(), &CancellationToken::new()).unwrap();
    assert_eq!(first.id, replay.id);
    assert_eq!(replay.output.as_deref(), Some("done"));
    let mut changed = request();
    changed.input = "different task".into();
    assert!(matches!(
        runtime.run(changed, &CancellationToken::new()),
        Err(RuntimeError::Conflict)
    ));
    drop(runtime);
    assert_eq!(model.seen.len(), 1);
}

#[test]
fn two_agents_delegate_synthesize_and_replay_without_duplicate_messages() {
    let home = home();
    let mut model = Script::new(vec![
        ("default/a", delegate()),
        ("default/b", text("review passed")),
        ("default/a", text("final synthesis")),
    ]);
    let mut runtime = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory).unwrap();
    let mut request = request();
    request.delegates = vec!["b".into()];
    let first = runtime
        .run(request.clone(), &CancellationToken::new())
        .unwrap();
    assert_eq!(first.state, RunState::Completed, "{:?}", first.error);
    assert_eq!(first.model_steps, 3);
    assert_eq!(
        runtime.run(request, &CancellationToken::new()).unwrap().id,
        first.id
    );
    let children = runtime.runs().children(&first.id).unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(
        children[0].parent_run_id.as_deref(),
        Some(first.id.as_str())
    );
    assert_eq!(children[0].model_steps, 1);
    let im = ImStore::open(home.path().join("im/im.db")).unwrap();
    let a = PrincipalId::new("runtime:default/a");
    let b = PrincipalId::new("runtime:default/b");
    let conversation = im.create_direct(&a, &b).unwrap();
    let messages = im
        .list_messages(&conversation.id, &a, MessageListOptions::default())
        .unwrap();
    assert_eq!(messages.messages.len(), 2);
    assert!(
        messages
            .messages
            .iter()
            .any(|message| message.body == "review this")
    );
    assert!(
        messages
            .messages
            .iter()
            .any(|message| message.body == "review passed" && message.reply_to_id.is_some())
    );
    assert!(
        !messages
            .messages
            .iter()
            .any(|message| message.body == "final synthesis")
    );
    drop(runtime);
    assert_eq!(model.seen.len(), 3);
    assert!(
        model.seen[1].2.is_empty(),
        "child must not see delegate tool"
    );
    assert_eq!(
        model.seen[1].1.len(),
        2,
        "child must not inherit parent transcript"
    );
    assert_eq!(model.seen[2].1.last().unwrap()["content"], "review passed");
}

#[test]
fn entire_call_batch_is_validated_before_any_delegation() {
    let home = home();
    for duplicate in [false, true] {
        let valid = call(
            "same",
            "delegate",
            json!({"agent":"default/b","input":"review"}),
        );
        let invalid = if duplicate {
            valid.clone()
        } else {
            call(
                "other",
                "delegate",
                json!({"agent":"default/outsider","input":"review"}),
            )
        };
        let mut model = Script::new(vec![("default/a", calls(vec![valid, invalid]))]);
        let mut runtime = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory).unwrap();
        let mut request = request();
        request.request_key = format!("batch-{duplicate}");
        request.delegates = vec!["b".into()];
        let record = runtime.run(request, &CancellationToken::new()).unwrap();
        assert_eq!(record.state, RunState::Failed);
        assert!(runtime.runs().children(&record.id).unwrap().is_empty());
        assert!(!home.path().join("im/im.db").exists());
    }
}

#[test]
fn child_cannot_delegate_and_failure_is_terminal_for_request_key() {
    let home = home();
    let mut model = Script::new(vec![("default/a", delegate()), ("default/b", delegate())]);
    let mut runtime = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory).unwrap();
    let mut request = request();
    request.delegates = vec!["b".into()];
    let record = runtime
        .run(request.clone(), &CancellationToken::new())
        .unwrap();
    assert_eq!(record.state, RunState::Failed);
    let children = runtime.runs().children(&record.id).unwrap();
    assert_eq!(children[0].state, RunState::Failed);
    assert_eq!(
        runtime.run(request, &CancellationToken::new()).unwrap().id,
        record.id
    );
}

#[test]
fn budget_prevents_delegation_without_capacity_for_child_and_parent() {
    let home = home();
    let mut model = Script::new(vec![("default/a", delegate())]);
    let mut runtime = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory).unwrap();
    let mut request = request();
    request.max_steps = 2;
    request.delegates = vec!["b".into()];
    let record = runtime.run(request, &CancellationToken::new()).unwrap();
    assert_eq!(record.state, RunState::Failed);
    assert!(record.error.unwrap().contains("budget"));
    assert_eq!(record.model_steps, 1);
    assert!(runtime.runs().children(&record.id).unwrap().is_empty());
}

#[test]
fn canceled_and_invalid_requests_never_call_model() {
    let home = home();
    let mut model = Script::default();
    let mut runtime = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory).unwrap();
    let token = CancellationToken::new();
    token.cancel();
    let record = runtime.run(request(), &token).unwrap();
    assert_eq!(record.state, RunState::Canceled);
    assert_eq!(record.model_steps, 0);
    let mut invalid = request();
    invalid.agent = "../escape".into();
    assert!(matches!(
        runtime.run(invalid, &token),
        Err(RuntimeError::Invalid(_))
    ));
}

#[test]
fn model_cancellation_is_observed_before_returning_success() {
    struct Cancel(CancellationToken);
    impl ModelClient for Cancel {
        fn complete(&mut self, _: &ResolvedAgent, _: &[Value], _: &[Value]) -> Result<Value> {
            self.0.cancel();
            Ok(text("must not publish"))
        }
    }
    let home = home();
    let token = CancellationToken::new();
    let mut model = Cancel(token.clone());
    let record = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &token)
        .unwrap();
    assert_eq!(record.state, RunState::Canceled);
    assert!(record.output.is_none());
}

#[test]
fn interrupted_run_is_not_automatically_executed_again() {
    struct Interrupted;
    impl ModelClient for Interrupted {
        fn complete(&mut self, _: &ResolvedAgent, _: &[Value], _: &[Value]) -> Result<Value> {
            panic!("simulate interrupted execution")
        }
    }
    let home = home();
    let path = home.path().to_owned();
    assert!(
        std::thread::spawn(move || {
            let mut model = Interrupted;
            Runtime::open(&path, &mut model, &DefaultMcpClientFactory)
                .unwrap()
                .run(request(), &CancellationToken::new())
                .unwrap();
        })
        .join()
        .is_err()
    );
    let mut model = Script::default();
    let replay = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &CancellationToken::new())
        .unwrap();
    assert_eq!(replay.state, RunState::Running);
    assert_eq!(replay.model_steps, 1);
}

#[test]
fn concurrent_runtime_observes_claim_without_duplicate_model_call() {
    use std::sync::mpsc;
    use std::time::Duration;
    struct Blocking {
        started: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }
    impl ModelClient for Blocking {
        fn complete(&mut self, _: &ResolvedAgent, _: &[Value], _: &[Value]) -> Result<Value> {
            self.started.send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(10)).unwrap();
            Ok(text("done"))
        }
    }
    let home = home();
    let path = home.path().to_owned();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let mut model = Blocking {
            started: started_tx,
            release: release_rx,
        };
        Runtime::open(&path, &mut model, &DefaultMcpClientFactory)
            .unwrap()
            .run(request(), &CancellationToken::new())
            .unwrap()
    });
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let mut model = Script::default();
    let record = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &CancellationToken::new())
        .unwrap();
    assert_eq!(record.state, RunState::Running);
    release_tx.send(()).unwrap();
    assert_eq!(thread.join().unwrap().id, record.id);
}

#[test]
fn unknown_schema_version_is_rejected_without_downgrading() {
    let home = home();
    drop(RunStore::open(home.path()).unwrap());
    let db = rusqlite::Connection::open(home.path().join("runtime/runs.db")).unwrap();
    db.pragma_update(None, "user_version", 99).unwrap();
    assert!(matches!(
        RunStore::open(home.path()),
        Err(RuntimeError::Configuration(_))
    ));
    assert_eq!(
        db.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        99
    );
}

#[test]
fn mcp_tool_loop_preserves_call_id_and_excludes_client_only_metadata() {
    let home = home();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp.py");
    let document = McpDocument::stdio(
        "helper",
        "python3",
        vec![fixture.to_string_lossy().into_owned()],
        Vec::<(String, String)>::new(),
    )
    .unwrap();
    McpStore::new(home.path()).add(document).unwrap();
    AgentStore::new(home.path()).add_mcp("a", "helper").unwrap();
    let mut model = Script::new(vec![
        (
            "default/a",
            calls(vec![call("lookup-1", "mcp_0_0", json!({"query":"test"}))]),
        ),
        ("default/a", text("done")),
    ]);
    let record = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &CancellationToken::new())
        .unwrap();
    assert_eq!(record.state, RunState::Completed, "{:?}", record.error);
    let output = model.seen[1].1.last().unwrap();
    assert_eq!(output["tool_call_id"], "lookup-1");
    assert!(output["content"].as_str().unwrap().contains("pong"));
    assert!(!output.to_string().contains("client-secret"));
    assert!(
        model.seen[0].2[0]["function"]["description"]
            .as_str()
            .unwrap()
            .contains("读取/a__b")
    );
}

#[test]
fn malformed_model_output_is_failed_not_published_as_success() {
    let home = home();
    for (index, answer) in [
        text(""),
        json!({"role":"user","content":"oops"}),
        calls(vec![call("id", "unknown", json!({}))]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut model = Script::new(vec![("default/a", answer)]);
        let mut request = request();
        request.request_key = format!("malformed-{index}");
        let record = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
            .unwrap()
            .run(request, &CancellationToken::new())
            .unwrap();
        assert_eq!(record.state, RunState::Failed);
        assert!(record.output.is_none());
    }
}

#[cfg(unix)]
#[test]
fn runtime_rejects_database_sidecar_symlinks() {
    let home = home();
    let outside = tempfile::NamedTempFile::new().unwrap();
    std::fs::create_dir(home.path().join("runtime")).unwrap();
    std::os::unix::fs::symlink(outside.path(), home.path().join("runtime/runs.db-wal")).unwrap();
    assert!(matches!(
        RunStore::open(home.path()),
        Err(RuntimeError::Configuration(_))
    ));
}

#[test]
fn delegates_use_configuration_resolved_before_the_root_model_call() {
    struct MutatesConfig {
        home: std::path::PathBuf,
        script: Script,
    }
    impl ModelClient for MutatesConfig {
        fn complete(
            &mut self,
            agent: &ResolvedAgent,
            messages: &[Value],
            tools: &[Value],
        ) -> Result<Value> {
            assert_eq!(agent.system_prompt, "original prompt");
            PromptStore::new(&self.home)
                .update("default", "changed during execution")
                .unwrap();
            self.script.complete(agent, messages, tools)
        }
    }
    let home = home();
    PromptStore::new(home.path())
        .update("default", "original prompt")
        .unwrap();
    let mut model = MutatesConfig {
        home: home.path().to_owned(),
        script: Script::new(vec![
            ("default/a", delegate()),
            ("default/b", text("review")),
            ("default/a", text("done")),
        ]),
    };
    let mut request = request();
    request.delegates = vec!["b".into()];
    let record = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(request, &CancellationToken::new())
        .unwrap();
    assert_eq!(record.state, RunState::Completed);
    assert_eq!(
        ResolvedAgent::load(home.path(), "a").unwrap().system_prompt,
        "changed during execution"
    );
}

#[test]
fn canceled_child_cancels_parent_without_publishing_a_reply() {
    struct CancelChild(CancellationToken);
    impl ModelClient for CancelChild {
        fn complete(&mut self, agent: &ResolvedAgent, _: &[Value], _: &[Value]) -> Result<Value> {
            if agent.name == "default/a" {
                return Ok(delegate());
            }
            self.0.cancel();
            Ok(text("do not publish"))
        }
    }
    let home = home();
    let token = CancellationToken::new();
    let mut model = CancelChild(token.clone());
    let mut runtime = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory).unwrap();
    let mut request = request();
    request.delegates = vec!["b".into()];
    let record = runtime.run(request, &token).unwrap();
    assert_eq!(record.state, RunState::Canceled);
    assert_eq!(
        runtime.runs().children(&record.id).unwrap()[0].state,
        RunState::Canceled
    );
    let im = ImStore::open(home.path().join("im/im.db")).unwrap();
    let a = PrincipalId::new("runtime:default/a");
    let conversation = im
        .create_direct(&a, &PrincipalId::new("runtime:default/b"))
        .unwrap();
    assert_eq!(
        im.list_messages(&conversation.id, &a, MessageListOptions::default())
            .unwrap()
            .messages
            .len(),
        1
    );
}

#[test]
fn completed_child_remains_inspectable_when_reply_publication_fails() {
    struct ArchiveDuringChild(std::path::PathBuf);
    impl ModelClient for ArchiveDuringChild {
        fn complete(&mut self, agent: &ResolvedAgent, _: &[Value], _: &[Value]) -> Result<Value> {
            if agent.name == "default/a" {
                return Ok(delegate());
            }
            let im = ImStore::open(self.0.join("im/im.db")).unwrap();
            let a = PrincipalId::new("runtime:default/a");
            let conversation = im
                .create_direct(&a, &PrincipalId::new("runtime:default/b"))
                .unwrap();
            im.archive_conversation(&conversation.id, &a).unwrap();
            Ok(text("retained result"))
        }
    }
    let home = home();
    let mut model = ArchiveDuringChild(home.path().to_owned());
    let mut runtime = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory).unwrap();
    let mut request = request();
    request.delegates = vec!["b".into()];
    let record = runtime
        .run(request.clone(), &CancellationToken::new())
        .unwrap();
    assert_eq!(record.state, RunState::Failed);
    assert!(
        record
            .error
            .as_deref()
            .unwrap()
            .contains("reply publication failed")
    );
    let children = runtime.runs().children(&record.id).unwrap();
    assert_eq!(children[0].state, RunState::Completed);
    assert_eq!(children[0].output.as_deref(), Some("retained result"));
    assert_eq!(
        runtime.run(request, &CancellationToken::new()).unwrap().id,
        record.id
    );
}

fn hosted(home: &Path, policy: agt_service::ResponsePolicy) -> agt_service::ToolService {
    use agt_im::{PrincipalInput, PrincipalKind};
    use agt_service::{Actor, Binding, BindingStore, Role, ToolService};
    let im = ImStore::open(home.join("im/im.db")).unwrap();
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
            BindingStore::open(home)
                .unwrap()
                .bind(Binding {
                    principal: PrincipalId::new(name),
                    agent: name.into(),
                    working_directory: home.to_owned(),
                    policy,
                    max_steps: 6,
                    max_depth: 1,
                    allow_mcp: false,
                })
                .unwrap();
        }
    }
    ToolService::open(
        home,
        Actor {
            principal: PrincipalId::new("user"),
            role: Role::Manager,
            automated_depth: None,
        },
    )
    .unwrap()
}

#[test]
fn dispatcher_broadcasts_to_every_bound_member_and_stops_reply_storms() {
    let home = home();
    let user = hosted(home.path(), agt_service::ResponsePolicy::All);
    let group = user
        .call(
            "group_create",
            json!({"title":"review","members":["a","b"]}),
        )
        .unwrap();
    user.call(
        "message_send",
        json!({"conversation":group["id"],"body":"discuss","request_key":"group-1"}),
    )
    .unwrap();
    let mut model = Script::new(vec![
        ("default/a", text("A opinion")),
        ("default/b", text("B opinion")),
    ]);
    let dispatcher = agt_runtime::Dispatcher::new(home.path());
    let result = dispatcher
        .drain(
            &mut model,
            &DefaultMcpClientFactory,
            50,
            &CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(result.iter().filter(|r| r.state == "completed").count(), 2);
    dispatcher
        .drain(
            &mut model,
            &DefaultMcpClientFactory,
            50,
            &CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(model.seen.len(), 2);
    let history = user
        .call("message_history", json!({"conversation":group["id"]}))
        .unwrap();
    assert_eq!(history["messages"].as_array().unwrap().len(), 3);
    assert!(
        model.seen[0]
            .2
            .iter()
            .any(|t| t["function"]["name"] == "framework_group_create")
    );
    assert!(
        !model.seen[0]
            .2
            .iter()
            .any(|t| t["function"]["name"] == "framework_resource_write")
    );
}

#[test]
fn dispatcher_mentions_direct_chats_and_silent_responses() {
    let home = home();
    let user = hosted(home.path(), agt_service::ResponsePolicy::Mentioned);
    let group = user
        .call(
            "group_create",
            json!({"title":"review","members":["a","b"]}),
        )
        .unwrap();
    user.call(
        "message_send",
        json!({"conversation":group["id"],"body":"review","request_key":"1","mentions":["b"]}),
    )
    .unwrap();
    let direct = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap();
    user.call(
        "message_send",
        json!({"conversation":direct["id"],"body":"hello","request_key":"2"}),
    )
    .unwrap();
    let mut model = Script::new(vec![
        ("default/a", text("private reply")),
        ("default/b", text("[[silent]]")),
    ]);
    agt_runtime::Dispatcher::new(home.path())
        .drain(
            &mut model,
            &DefaultMcpClientFactory,
            50,
            &CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(
        user.call("message_history", json!({"conversation":group["id"]}))
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        user.call("message_history", json!({"conversation":direct["id"]}))
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn dispatcher_republishes_completed_output_after_permission_failure_without_inference() {
    struct Archive {
        home: std::path::PathBuf,
        conversation: agt_im::ConversationId,
        calls: usize,
    }
    impl ModelClient for Archive {
        fn complete(&mut self, _: &ResolvedAgent, _: &[Value], _: &[Value]) -> Result<Value> {
            self.calls += 1;
            assert_eq!(self.calls, 1);
            ImStore::open(self.home.join("im/im.db"))
                .unwrap()
                .archive_conversation(&self.conversation, &PrincipalId::new("user"))
                .unwrap();
            Ok(text("saved result"))
        }
    }
    let home = home();
    let user = hosted(home.path(), agt_service::ResponsePolicy::All);
    let direct = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap();
    user.call(
        "message_send",
        json!({"conversation":direct["id"],"body":"hello","request_key":"1"}),
    )
    .unwrap();
    let cid = agt_im::ConversationId::new(direct["id"].as_str().unwrap());
    let mut model = Archive {
        home: home.path().to_owned(),
        conversation: cid.clone(),
        calls: 0,
    };
    let dispatcher = agt_runtime::Dispatcher::new(home.path());
    assert_eq!(
        dispatcher
            .drain(
                &mut model,
                &DefaultMcpClientFactory,
                50,
                &CancellationToken::new()
            )
            .unwrap()[0]
            .state,
        "error"
    );
    ImStore::open(home.path().join("im/im.db"))
        .unwrap()
        .reactivate_conversation(&cid, &PrincipalId::new("user"))
        .unwrap();
    assert_eq!(
        dispatcher
            .drain(
                &mut model,
                &DefaultMcpClientFactory,
                50,
                &CancellationToken::new()
            )
            .unwrap()[0]
            .state,
        "completed"
    );
    assert!(
        dispatcher
            .drain(
                &mut model,
                &DefaultMcpClientFactory,
                50,
                &CancellationToken::new()
            )
            .unwrap()
            .is_empty()
    );
    assert_eq!(model.calls, 1);
    assert_eq!(
        user.call("message_history", json!({"conversation":direct["id"]}))
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn failed_group_member_does_not_block_other_members() {
    let home = home();
    let user = hosted(home.path(), agt_service::ResponsePolicy::All);
    let group = user
        .call(
            "group_create",
            json!({"title":"review","members":["a","b"]}),
        )
        .unwrap();
    user.call(
        "message_send",
        json!({"conversation":group["id"],"body":"review","request_key":"1"}),
    )
    .unwrap();
    let mut model = Script::new(vec![
        ("default/a", text("")),
        ("default/b", text("B succeeds")),
    ]);
    let results = agt_runtime::Dispatcher::new(home.path())
        .drain(
            &mut model,
            &DefaultMcpClientFactory,
            50,
            &CancellationToken::new(),
        )
        .unwrap();
    assert!(results.iter().any(|r| r.state == "failed"));
    assert!(results.iter().any(|r| r.state == "completed"));
}

#[test]
fn hosted_agent_can_send_peer_message_with_fixed_identity_and_inherited_depth() {
    let home = home();
    let user = hosted(home.path(), agt_service::ResponsePolicy::All);
    let group = user
        .call("group_create", json!({"title":"group","members":["a"]}))
        .unwrap();
    user.call(
        "message_send",
        json!({"conversation":group["id"],"body":"send a message","request_key":"1"}),
    )
    .unwrap();
    let mut model = Script::new(vec![
        (
            "default/a",
            calls(vec![call(
                "send-1",
                "framework_message_send",
                json!({"conversation":group["id"],"body":"tool reply","request_key":"tool-1"}),
            )]),
        ),
        ("default/a", text("[[silent]]")),
    ]);
    agt_runtime::Dispatcher::new(home.path())
        .drain(
            &mut model,
            &DefaultMcpClientFactory,
            50,
            &CancellationToken::new(),
        )
        .unwrap();
    let history = user
        .call("message_history", json!({"conversation":group["id"]}))
        .unwrap();
    assert_eq!(history["messages"].as_array().unwrap().len(), 2);
    assert_eq!(history["messages"][1]["sender_id"], "a");
    assert_eq!(history["messages"][1]["metadata"]["automated_depth"], 1);
}

#[test]
fn deleting_prepared_trigger_skips_delivery_and_archiving_pauses_it() {
    let home = home();
    let user = hosted(home.path(), agt_service::ResponsePolicy::All);
    let direct = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap();
    let cid = agt_im::ConversationId::new(direct["id"].as_str().unwrap());
    let sent = user
        .call(
            "message_send",
            json!({"conversation":direct["id"],"body":"hello","request_key":"1"}),
        )
        .unwrap();
    let im = ImStore::open(home.path().join("im/im.db")).unwrap();
    let delivery = im
        .deliveries(&PrincipalId::new("a"), true, 1)
        .unwrap()
        .remove(0);
    im.prepare_delivery(
        delivery.id,
        &json!({"placeholder":"must never be executed"}),
    )
    .unwrap();
    im.archive_conversation(&cid, &PrincipalId::new("user"))
        .unwrap();
    let mut model = Script::default();
    let dispatcher = agt_runtime::Dispatcher::new(home.path());
    assert_eq!(
        dispatcher
            .drain(
                &mut model,
                &DefaultMcpClientFactory,
                50,
                &CancellationToken::new()
            )
            .unwrap()[0]
            .state,
        "conversation_inactive"
    );
    im.reactivate_conversation(&cid, &PrincipalId::new("user"))
        .unwrap();
    im.delete_message(
        &agt_im::MessageId::new(sent["message"]["id"].as_str().unwrap()),
        &PrincipalId::new("user"),
    )
    .unwrap();
    assert_eq!(
        dispatcher
            .drain(
                &mut model,
                &DefaultMcpClientFactory,
                50,
                &CancellationToken::new()
            )
            .unwrap()[0]
            .state,
        "skipped"
    );
    assert!(
        dispatcher
            .drain(
                &mut model,
                &DefaultMcpClientFactory,
                50,
                &CancellationToken::new()
            )
            .unwrap()
            .is_empty()
    );
}

#[test]
fn concurrent_dispatchers_share_one_inference_and_one_reply() {
    use std::sync::mpsc;
    use std::time::Duration;
    struct Blocking {
        started: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }
    impl ModelClient for Blocking {
        fn complete(&mut self, _: &ResolvedAgent, _: &[Value], _: &[Value]) -> Result<Value> {
            self.started.send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(10)).unwrap();
            Ok(text("one reply"))
        }
    }
    let home = home();
    let user = hosted(home.path(), agt_service::ResponsePolicy::All);
    let direct = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap();
    let sent = user
        .call(
            "message_send",
            json!({"conversation":direct["id"],"body":"hello","request_key":"1"}),
        )
        .unwrap();
    let (start_tx, start_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let path = home.path().to_owned();
    let thread = std::thread::spawn(move || {
        let mut model = Blocking {
            started: start_tx,
            release: release_rx,
        };
        agt_runtime::Dispatcher::new(&path)
            .drain(
                &mut model,
                &DefaultMcpClientFactory,
                50,
                &CancellationToken::new(),
            )
            .unwrap()
    });
    start_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let mut model = Script::default();
    let dispatcher = agt_runtime::Dispatcher::new(home.path());
    assert_eq!(
        dispatcher
            .drain(
                &mut model,
                &DefaultMcpClientFactory,
                50,
                &CancellationToken::new()
            )
            .unwrap()[0]
            .state,
        "running_or_interrupted"
    );
    release_tx.send(()).unwrap();
    thread.join().unwrap();
    assert!(
        dispatcher
            .drain(
                &mut model,
                &DefaultMcpClientFactory,
                50,
                &CancellationToken::new()
            )
            .unwrap()
            .is_empty()
    );
    let status = user
        .call(
            "message_status",
            json!({"message_id":sent["message"]["id"]}),
        )
        .unwrap();
    assert_eq!(status[0]["state"], "completed");
    assert!(status[0].get("request").is_none());
    assert_eq!(
        user.call("message_history", json!({"conversation":direct["id"]}))
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn hosted_agent_creates_group_and_contacts_peer_without_parent_child_delegation() {
    struct Organize {
        step: usize,
    }
    impl ModelClient for Organize {
        fn complete(
            &mut self,
            agent: &ResolvedAgent,
            messages: &[Value],
            _: &[Value],
        ) -> Result<Value> {
            self.step += 1;
            Ok(match self.step {
                1 => {
                    assert_eq!(agent.name, "default/a");
                    calls(vec![call(
                        "group",
                        "framework_group_create",
                        json!({"title":"agent-created","members":["b"]}),
                    )])
                }
                2 => {
                    let group: Value =
                        serde_json::from_str(messages.last().unwrap()["content"].as_str().unwrap())
                            .unwrap();
                    calls(vec![call(
                        "send",
                        "framework_message_send",
                        json!({"conversation":group["id"],"body":"Please review","request_key":"peer-task","mentions":["b"]}),
                    )])
                }
                3 => text("[[silent]]"),
                4 => {
                    assert_eq!(agent.name, "default/b");
                    text("peer review")
                }
                _ => panic!("unexpected extra response"),
            })
        }
    }
    let home = home();
    let user = hosted(home.path(), agt_service::ResponsePolicy::All);
    let store = agt_service::BindingStore::open(home.path()).unwrap();
    let mut b = store.get(&PrincipalId::new("b")).unwrap().unwrap();
    b.max_depth = 2;
    store.bind(b).unwrap();
    let direct = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap();
    user.call(
        "message_send",
        json!({"conversation":direct["id"],"body":"organize review","request_key":"1"}),
    )
    .unwrap();
    let mut model = Organize { step: 0 };
    let dispatcher = agt_runtime::Dispatcher::new(home.path());
    dispatcher
        .drain(
            &mut model,
            &DefaultMcpClientFactory,
            50,
            &CancellationToken::new(),
        )
        .unwrap();
    dispatcher
        .drain(
            &mut model,
            &DefaultMcpClientFactory,
            50,
            &CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(model.step, 4);
    let records = RunStore::open(home.path()).unwrap().list(10).unwrap();
    assert_eq!(records.len(), 2);
    assert!(
        records
            .iter()
            .all(|r| r.parent_run_id.is_none() && r.request.trigger.is_some())
    );
}
