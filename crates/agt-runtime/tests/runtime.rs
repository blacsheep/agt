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
fn hosted_tool_corrections_are_audited_redacted_and_not_replayed() {
    let home = home();
    let user = hosted(home.path(), agt_service::ResponsePolicy::Mentioned);
    let direct = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap();
    let trigger = user
        .call(
            "message_send",
            json!({"conversation":direct["id"],"body":"test","request_key":"trigger"}),
        )
        .unwrap();
    let mut request = request();
    request.trigger = Some(agt_runtime::MessageTrigger {
        message_id: agt_im::MessageId::new(trigger["message"]["id"].as_str().unwrap()),
        conversation_id: agt_im::ConversationId::new(direct["id"].as_str().unwrap()),
        principal: PrincipalId::new("a"),
        depth: 1,
        allow_mcp: false,
    });
    let base = json!({"conversation":direct["id"],"body":"private-body-do-not-log","request_key":"private-request-key","reply_to":""});
    let mut unknown = base.clone();
    unknown["secret-unknown-field"] = json!("secret-value");
    let mut missing = base.clone();
    missing["reply_to"] = json!("secret-missing-message");
    let mut model = Script::new(vec![
        (
            "default/a",
            calls(vec![call("bad-shape", "framework_message_send", unknown)]),
        ),
        (
            "default/a",
            calls(vec![call("bad-target", "framework_message_send", missing)]),
        ),
        (
            "default/a",
            calls(vec![call("fixed", "framework_message_send", base)]),
        ),
        ("default/a", text("[[silent]]")),
    ]);
    let mut runtime = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory).unwrap();
    let run = runtime
        .run(request.clone(), &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Completed);
    assert_eq!(run.tool_failures, 2);
    let records = runtime.runs().calls(&run.id, 0, 200).unwrap();
    assert_eq!(records.len(), 7);
    let tools: Vec<_> = records.iter().filter(|r| r.kind == "tool").collect();
    assert_eq!(tools[0].state, agt_runtime::CallState::Failed);
    assert_eq!(tools[0].argument_summary["unknown_field_count"], 1);
    assert_eq!(tools[1].error.as_ref().unwrap()["field"], "reply_to");
    assert_eq!(tools[2].state, agt_runtime::CallState::Succeeded);
    assert_eq!(
        tools[2].argument_summary["fields"]["reply_to"]["blank"],
        true
    );
    assert!(records.iter().all(|r| r.finished_at.is_some()));
    let encoded = serde_json::to_string(&records).unwrap();
    for private in [
        "private-body",
        "private-request",
        "secret-unknown",
        "secret-value",
        "secret-missing",
        "test-secret",
    ] {
        assert!(!encoded.contains(private));
    }
    assert_eq!(
        runtime
            .runs()
            .calls(&run.id, records[2].id, 2)
            .unwrap()
            .len(),
        2
    );
    drop(runtime);
    let mut runtime = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory).unwrap();
    assert_eq!(
        runtime
            .run(request, &CancellationToken::new())
            .unwrap()
            .tool_failures,
        2
    );
    assert_eq!(runtime.runs().calls(&run.id, 0, 200).unwrap().len(), 7);
    drop(runtime);
    assert_eq!(model.seen.len(), 4);
    let error: Value =
        serde_json::from_str(model.seen[2].1.last().unwrap()["content"].as_str().unwrap()).unwrap();
    assert_eq!(error["error"]["code"], "invalid_arguments");
    assert_eq!(error["error"]["field"], "reply_to");
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
fn failed_tool_then_silent_remains_visible_on_completed_run() {
    let home = home();
    let user = hosted(home.path(), agt_service::ResponsePolicy::Mentioned);
    let direct = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap();
    let sent = user
        .call(
            "message_send",
            json!({"conversation":direct["id"],"body":"test","request_key":"start"}),
        )
        .unwrap();
    let mut model = Script::new(vec![
        (
            "default/a",
            calls(vec![call("bad", "framework_message_ack", json!({"id":-1}))]),
        ),
        ("default/a", text("[[silent]]")),
    ]);
    let mut request = RunRequest::new("a", "test");
    request.trigger = Some(agt_runtime::MessageTrigger {
        message_id: agt_im::MessageId::new(sent["message"]["id"].as_str().unwrap()),
        conversation_id: agt_im::ConversationId::new(direct["id"].as_str().unwrap()),
        principal: PrincipalId::new("a"),
        depth: 1,
        allow_mcp: false,
    });
    Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(request, &CancellationToken::new())
        .unwrap();
    let store = RunStore::open(home.path()).unwrap();
    let run = &store.list(10).unwrap()[0];
    assert_eq!(run.state, RunState::Completed);
    assert_eq!(run.tool_failures, 1);
    assert_eq!(
        store.calls(&run.id, 0, 50).unwrap()[1]
            .error
            .as_ref()
            .unwrap()["field"],
        "id"
    );
}

#[test]
fn model_errors_and_invalid_batches_have_safe_audit_records() {
    let home = home();
    let mut model = Script::new(vec![(
        "default/a",
        calls(vec![call(
            "x",
            "secret-unknown-tool",
            json!({"secret":"private"}),
        )]),
    )]);
    let mut runtime = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory).unwrap();
    let run = runtime.run(request(), &CancellationToken::new()).unwrap();
    assert_eq!(run.state, RunState::Failed);
    let records = runtime.runs().calls(&run.id, 0, 50).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].state, agt_runtime::CallState::Failed);
    assert!(!serde_json::to_string(&records).unwrap().contains("secret"));
}

#[test]
fn runtime_rejects_unsupported_schema_without_changing_it() {
    for version in [1, 99] {
        let home = home();
        drop(RunStore::open(home.path()).unwrap());
        let db = rusqlite::Connection::open(home.path().join("runtime/runs.db")).unwrap();
        db.pragma_update(None, "user_version", version).unwrap();
        assert!(RunStore::open(home.path()).is_err());
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))
                .unwrap(),
            version
        );
    }
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
fn legacy_step_limit_does_not_stop_parent_or_delegated_child() {
    let home = home();
    let mut model = Script::new(vec![
        ("default/a", delegate()),
        ("default/b", text("review")),
        ("default/a", text("done")),
    ]);
    let mut runtime = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory).unwrap();
    let mut request = request();
    request.max_steps = 2;
    request.delegates = vec!["b".into()];
    let record = runtime.run(request, &CancellationToken::new()).unwrap();
    assert_eq!(record.state, RunState::Completed);
    assert_eq!(record.model_steps, 3);
    assert_eq!(runtime.runs().children(&record.id).unwrap().len(), 1);
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
    let calls = RunStore::open(home.path())
        .unwrap()
        .calls(&record.id, 0, 50)
        .unwrap();
    assert_eq!(calls[0].state, agt_runtime::CallState::Canceled);
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
    let calls = RunStore::open(home.path())
        .unwrap()
        .calls(&replay.id, 0, 50)
        .unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].state, agt_runtime::CallState::Started);
    assert!(calls[0].finished_at.is_none());
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
    let calls = RunStore::open(home.path())
        .unwrap()
        .calls(&record.id, 0, 50)
        .unwrap();
    assert_eq!(calls[1].tool_name.as_deref(), Some("mcp_0_0"));
    assert_eq!(calls[1].state, agt_runtime::CallState::Succeeded);
    assert_eq!(calls[1].argument_summary["unknown_field_count"], 1);
    let serialized = serde_json::to_string(&calls).unwrap();
    assert!(!serialized.contains("client-secret"));
    assert!(!serialized.contains("query"));
    assert!(!serialized.contains("pong"));
    assert!(
        model.seen[0].2[0]["function"]["description"]
            .as_str()
            .unwrap()
            .contains("读取/a__b")
    );
}

#[test]
fn mcp_business_and_transport_failures_are_audited_without_raw_results() {
    for mode in ["tool_error", "transport_error"] {
        let home = home();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp.py");
        McpStore::new(home.path())
            .add(
                McpDocument::stdio(
                    "helper",
                    "python3",
                    vec![fixture.to_string_lossy().into_owned()],
                    [("AGT_AUDIT_TEST_MODE".into(), mode.into())],
                )
                .unwrap(),
            )
            .unwrap();
        AgentStore::new(home.path()).add_mcp("a", "helper").unwrap();
        let mut answers = vec![(
            "default/a",
            calls(vec![call("lookup", "mcp_0_0", json!({"query":"test"}))]),
        )];
        if mode == "tool_error" {
            answers.push(("default/a", text("[[silent]]")));
        }
        let mut model = Script::new(answers);
        let mut runtime = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory).unwrap();
        let run = runtime.run(request(), &CancellationToken::new()).unwrap();
        assert_eq!(run.tool_failures, 1);
        assert_eq!(
            run.state,
            if mode == "tool_error" {
                RunState::Completed
            } else {
                RunState::Failed
            }
        );
        let records = runtime.runs().calls(&run.id, 0, 50).unwrap();
        assert_eq!(records[1].state, agt_runtime::CallState::Failed);
        assert_eq!(
            records[1].error.as_ref().unwrap()["code"],
            if mode == "tool_error" {
                "mcp_tool_error"
            } else {
                "tool_error"
            }
        );
        assert!(
            !serde_json::to_string(&records)
                .unwrap()
                .contains("client-secret")
        );
        assert!(!serde_json::to_string(&records).unwrap().contains("pong"));
    }
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
        let count = if answer["content"] == "" { 10 } else { 1 };
        let mut model = Script::new(vec![("default/a", answer); count]);
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

                    allow_mcp: false,
                })
                .unwrap();
        }
    }
    im.ensure_workspace_owner(&PrincipalId::new("user"))
        .unwrap();
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
fn hosted_agent_creates_group_and_contacts_peer_without_parent_child_delegation() {
    struct Organize {
        step: usize,
        group: Option<String>,
    }
    impl ModelClient for Organize {
        fn complete(
            &mut self,
            agent: &ResolvedAgent,
            messages: &[Value],
            tools: &[Value],
        ) -> Result<Value> {
            assert_eq!(agent.name, "default/a");
            assert!(
                !tools
                    .iter()
                    .any(|t| t["function"]["name"] == "framework_resource_write")
            );
            self.step += 1;
            Ok(match self.step {
                1 => calls(vec![call(
                    "group",
                    "framework_group_create",
                    json!({"title":"agent-created","members":["b"]}),
                )]),
                2 => {
                    let group: Value =
                        serde_json::from_str(messages.last().unwrap()["content"].as_str().unwrap())
                            .unwrap();
                    self.group = Some(group["id"].as_str().unwrap().into());
                    calls(vec![call(
                        "send",
                        "framework_message_send",
                        json!({"conversation":group["id"],"body":"Please review","request_key":"peer-task","mentions":["b"]}),
                    )])
                }
                3 => text("[[silent]]"),
                _ => panic!("unexpected model call"),
            })
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
            json!({"conversation":direct["id"],"body":"organize review","request_key":"organize"}),
        )
        .unwrap();
    let mut model = Organize {
        step: 0,
        group: None,
    };
    let mut request = RunRequest::new("a", "organize review");
    request.trigger = Some(agt_runtime::MessageTrigger {
        message_id: agt_im::MessageId::new(sent["message"]["id"].as_str().unwrap()),
        conversation_id: agt_im::ConversationId::new(direct["id"].as_str().unwrap()),
        principal: PrincipalId::new("a"),
        depth: 1,
        allow_mcp: false,
    });
    let run = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(request, &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Completed);
    assert!(run.parent_run_id.is_none());
    let im = ImStore::open(home.path().join("im/im.db")).unwrap();
    let group = agt_im::ConversationId::new(model.group.unwrap());
    let history = im
        .list_messages(
            &group,
            &PrincipalId::new("b"),
            MessageListOptions::default(),
        )
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].sender_id.as_str(), "a");
    assert_eq!(history.messages[0].body, "Please review");
    assert_eq!(history.messages[0].metadata["automated_depth"], 1);
    assert!(
        im.deliveries(&PrincipalId::new("b"), true, 50)
            .unwrap()
            .iter()
            .any(|d| d.message_id == history.messages[0].id)
    );
    assert!(
        RunStore::open(home.path())
            .unwrap()
            .children(&run.id)
            .unwrap()
            .is_empty()
    );
}

fn mount_resources(home: &Path, name: &str) {
    AgentStore::new(home)
        .update_in(&agt_core::CategoryName::default(), name, |config| {
            config.builtin_tools = vec![agt_agent::BuiltinTool::ResourceManagement];
            Ok(())
        })
        .unwrap();
}

#[test]
fn builtin_resources_require_mount_and_keep_run_snapshot_and_redacted_audit() {
    let home = home();
    let write = calls(vec![call(
        "write",
        "framework_resource_write",
        json!({"kind":"prompt","reference":"created","create":true,"data":{"text":"private-prompt-body"}}),
    )]);
    let mut denied = Script::new(vec![("default/a", write.clone())]);
    let denied_run = Runtime::open(home.path(), &mut denied, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &CancellationToken::new())
        .unwrap();
    assert_eq!(denied_run.state, RunState::Failed);
    assert!(PromptStore::new(home.path()).read("created").is_err());
    mount_resources(home.path(), "a");
    let mut script = Script::new(vec![
        (
            "default/a",
            calls(vec![call(
                "unmount",
                "framework_resource_write",
                json!({"kind":"agent","reference":"a","create":false,"data":{"builtin_tools":[]}}),
            )]),
        ),
        ("default/a", write),
        ("default/a", text("done")),
    ]);
    let mut request = request();
    request.request_key = "mounted".into();
    let mut runtime = Runtime::open(home.path(), &mut script, &DefaultMcpClientFactory).unwrap();
    let run = runtime
        .run(request.clone(), &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Completed);
    assert_eq!(
        PromptStore::new(home.path())
            .read("created")
            .unwrap()
            .content,
        "private-prompt-body"
    );
    let audit = runtime.runs().calls(&run.id, 0, 100).unwrap();
    assert!(
        !serde_json::to_string(&audit)
            .unwrap()
            .contains("private-prompt-body")
    );
    assert_eq!(
        runtime.run(request, &CancellationToken::new()).unwrap().id,
        run.id
    );
    drop(runtime);
    assert!(script.seen.iter().all(|(_, _, tools)| {
        tools
            .iter()
            .any(|s| s["function"]["name"] == "framework_resource_write")
    }));
    assert!(
        script.seen[0]
            .2
            .iter()
            .any(|s| s["function"]["name"] == "framework_agent_bind")
    );
    assert!(
        ResolvedAgent::load(home.path(), "a")
            .unwrap()
            .builtin_tools
            .is_empty()
    );
    let mut next = Script::new(vec![("default/a", text("next"))]);
    let mut next_request = RunRequest::new("a", "next");
    next_request.request_key = "next".into();
    Runtime::open(home.path(), &mut next, &DefaultMcpClientFactory)
        .unwrap()
        .run(next_request, &CancellationToken::new())
        .unwrap();
    assert!(next.seen[0].2.is_empty());
}

#[test]
fn builtin_resources_do_not_grant_delegates_permissions_or_bypass_model_capability() {
    let home = home();
    mount_resources(home.path(), "a");
    let mut script = Script::new(vec![
        ("default/a", delegate()),
        ("default/b", text("review")),
        ("default/a", text("done")),
    ]);
    let mut req = request();
    req.delegates = vec!["b".into()];
    let run = Runtime::open(home.path(), &mut script, &DefaultMcpClientFactory)
        .unwrap()
        .run(req, &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Completed);
    assert!(script.seen[1].2.is_empty());
    ModelStore::new(home.path())
        .update(
            "fast",
            agt_model::ModelConfigUpdate {
                tool_call: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
    let mut script = Script::default();
    let mut req = request();
    req.request_key = "disabled".into();
    let run = Runtime::open(home.path(), &mut script, &DefaultMcpClientFactory)
        .unwrap()
        .run(req, &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Failed);
    assert!(script.seen.is_empty());
}

#[test]
fn hosted_builtin_resources_work_without_external_mcp_and_keep_fixed_identity() {
    let home = home();
    mount_resources(home.path(), "a");
    let user = hosted(home.path(), agt_service::ResponsePolicy::Mentioned);
    let direct = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap();
    let message = user
        .call(
            "message_send",
            json!({"conversation":direct["id"],"body":"manage","request_key":"manage"}),
        )
        .unwrap();
    let mut req = request();
    req.trigger = Some(agt_runtime::MessageTrigger {
        message_id: agt_im::MessageId::new(message["message"]["id"].as_str().unwrap()),
        conversation_id: agt_im::ConversationId::new(direct["id"].as_str().unwrap()),
        principal: PrincipalId::new("a"),
        depth: 1,
        allow_mcp: false,
    });
    let mut script = Script::new(vec![
        (
            "default/a",
            calls(vec![call(
                "create",
                "framework_resource_write",
                json!({"kind":"prompt","reference":"hosted","create":true,"data":{"text":"created by hosted agent"}}),
            )]),
        ),
        ("default/a", text("done")),
    ]);
    let run = Runtime::open(home.path(), &mut script, &DefaultMcpClientFactory)
        .unwrap()
        .run(req, &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Completed);
    assert!(PromptStore::new(home.path()).read("hosted").is_ok());
    let tools = &script.seen[0].2;
    assert!(
        tools
            .iter()
            .any(|s| s["function"]["name"] == "framework_message_send")
    );
    assert!(
        tools
            .iter()
            .any(|s| s["function"]["name"] == "framework_agent_bind")
    );
    let im = ImStore::open(home.path().join("im/im.db")).unwrap();
    assert!(
        im.get_principal(&PrincipalId::new("runtime:default/a"))
            .is_err()
    );
}

#[test]
fn empty_response_retry_preserves_completed_tools_and_step_budget() {
    let home = home();
    let mut model = Script::new(vec![
        ("default/a", delegate()),
        ("default/b", text("review passed")),
        ("default/a", text(" ")),
        ("default/a", text("final synthesis")),
    ]);
    let mut runtime = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory).unwrap();
    let mut request = request();
    request.delegates = vec!["b".into()];
    request.max_steps = 3;
    let record = runtime.run(request, &CancellationToken::new()).unwrap();
    assert_eq!(record.state, RunState::Completed);
    assert_eq!(record.model_steps, 3);
    assert_eq!(runtime.runs().children(&record.id).unwrap().len(), 1);
    drop(runtime);
    assert_eq!(model.seen.len(), 4);
    assert_eq!(model.seen[2].1, model.seen[3].1);
}

#[test]
fn cancellation_during_retry_wait_prevents_next_model_request() {
    struct Failing {
        calls: usize,
        token: CancellationToken,
    }
    impl ModelClient for Failing {
        fn check_canceled(&self) -> Result<()> {
            if self.token.is_canceled() {
                Err(RuntimeError::Canceled)
            } else {
                Ok(())
            }
        }
        fn complete(&mut self, _: &ResolvedAgent, _: &[Value], _: &[Value]) -> Result<Value> {
            self.calls += 1;
            let token = self.token.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(75));
                token.cancel();
            });
            Err(RuntimeError::HttpStatus(503))
        }
    }
    let home = home();
    let mut model = Failing {
        calls: 0,
        token: CancellationToken::new(),
    };
    let record = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &CancellationToken::new())
        .unwrap();
    assert_eq!(record.state, RunState::Canceled);
    assert_eq!(model.calls, 1);
}

#[test]
fn resource_agent_can_create_enable_disable_and_resume_a_teammate() {
    let home = home();
    mount_resources(home.path(), "a");
    let mut script = Script::new(vec![
        (
            "default/a",
            calls(vec![
                call(
                    "create",
                    "framework_resource_write",
                    json!({"kind":"agent","reference":"teammate","create":true,"data":{"model":"fast"}}),
                ),
                call(
                    "identity",
                    "framework_principal_create",
                    json!({"id":"teammate","name":"Teammate","kind":"agent"}),
                ),
                call(
                    "bind",
                    "framework_agent_bind",
                    json!({"principal":"teammate","agent":"teammate"}),
                ),
            ]),
        ),
        (
            "default/a",
            calls(vec![
                call(
                    "disable",
                    "framework_agent_set_enabled",
                    json!({"principal":"teammate","enabled":false}),
                ),
                call("state", "framework_binding_list", json!({})),
            ]),
        ),
        (
            "default/a",
            calls(vec![call(
                "enable",
                "framework_agent_set_enabled",
                json!({"principal":"teammate","enabled":true}),
            )]),
        ),
        ("default/a", text("teammate ready")),
    ]);
    let run = Runtime::open(home.path(), &mut script, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Completed);
    assert_eq!(run.tool_failures, 0);
    let binding = agt_service::BindingStore::open(home.path())
        .unwrap()
        .get(&PrincipalId::new("teammate"))
        .unwrap()
        .unwrap();
    let creation: Value = serde_json::from_str(
        script.seen[1]
            .1
            .iter()
            .find(|m| m["tool_call_id"] == "create")
            .unwrap()["content"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(creation["activation"]["enabled"], false);
    assert_eq!(creation["activation"]["configured"], false);
    assert!(
        creation["activation"]["next_steps"]
            .to_string()
            .contains("agent_bind")
    );
    let bound: Value = serde_json::from_str(
        script.seen[1]
            .1
            .iter()
            .find(|m| m["tool_call_id"] == "bind")
            .unwrap()["content"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(bound["enabled"], true);
    assert_eq!(binding.agent, "default/teammate");
    assert_eq!(binding.max_steps, 0);
    assert!(!binding.allow_mcp);
    assert_eq!(binding.policy, agt_service::ResponsePolicy::Mentioned);
    let state = script.seen[2]
        .1
        .iter()
        .find(|m| m["tool_call_id"] == "state")
        .unwrap();
    let saved: Value = serde_json::from_str(state["content"].as_str().unwrap()).unwrap();
    assert_eq!(saved[0]["principal"], "teammate");
    assert_eq!(saved[0]["enabled"], false);
}

#[test]
fn model_loop_continues_beyond_old_hundred_step_ceiling() {
    let home = home();
    mount_resources(home.path(), "a");
    let mut answers = (0..120)
        .map(|i| {
            (
                "default/a",
                calls(vec![call(
                    &format!("list-{i}"),
                    "framework_binding_list",
                    json!({}),
                )]),
            )
        })
        .collect::<Vec<_>>();
    answers.push(("default/a", text("done")));
    let mut model = Script::new(answers);
    let mut req = request();
    req.max_steps = 1;
    let run = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(req, &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Completed);
    assert_eq!(run.model_steps, 121);
    assert_eq!(run.tool_failures, 0);
}

#[test]
fn identical_sends_with_new_keys_publish_once_per_run() {
    let home = home();
    let user = hosted(home.path(), agt_service::ResponsePolicy::Mentioned);
    let direct = user
        .call("direct_create", json!({"principal":"a"}))
        .unwrap();
    let trigger = user
        .call(
            "message_send",
            json!({"conversation":direct["id"],"body":"test","request_key":"trigger"}),
        )
        .unwrap();
    let mut req = request();
    req.trigger = Some(agt_runtime::MessageTrigger {
        message_id: agt_im::MessageId::new(trigger["message"]["id"].as_str().unwrap()),
        conversation_id: agt_im::ConversationId::new(direct["id"].as_str().unwrap()),
        principal: PrincipalId::new("a"),
        depth: 1,
        allow_mcp: false,
    });
    let send = |key: &str, reply: Value| json!({"conversation":direct["id"],"body":"same reply","request_key":key,"reply_to":reply});
    let mut model = Script::new(vec![
        (
            "default/a",
            calls(vec![
                call("one", "framework_message_send", send("one", Value::Null)),
                call("two", "framework_message_send", send("two", json!(""))),
            ]),
        ),
        (
            "default/a",
            calls(vec![
                call(
                    "three",
                    "framework_message_send",
                    send("three", Value::Null),
                ),
                call(
                    "reply-target",
                    "framework_message_send",
                    send("target", trigger["message"]["id"].clone()),
                ),
            ]),
        ),
        ("default/a", text("same reply")),
    ]);
    let record = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(req.clone(), &CancellationToken::new())
        .unwrap();
    assert_eq!(record.state, RunState::Completed);
    assert_eq!(record.output.as_deref(), Some("[[silent]]"));
    assert_eq!(record.tool_failures, 0);
    let tool_result = |index: usize, id: &str| -> Value {
        serde_json::from_str(
            model.seen[index]
                .1
                .iter()
                .find(|m| m["tool_call_id"] == id)
                .unwrap()["content"]
                .as_str()
                .unwrap(),
        )
        .unwrap()
    };
    let first = tool_result(1, "one");
    for (index, id) in [(1, "two"), (2, "three")] {
        let duplicate = tool_result(index, id);
        assert_eq!(duplicate["duplicate"], true);
        assert_eq!(duplicate["message"]["id"], first["message"]["id"]);
    }
    assert_eq!(tool_result(2, "reply-target")["duplicate"], false);
    req.request_key = "another-run".into();
    let mut next = Script::new(vec![
        (
            "default/a",
            calls(vec![call(
                "next",
                "framework_message_send",
                send("new-run", Value::Null),
            )]),
        ),
        ("default/a", text("done")),
    ]);
    assert_eq!(
        Runtime::open(home.path(), &mut next, &DefaultMcpClientFactory)
            .unwrap()
            .run(req, &CancellationToken::new())
            .unwrap()
            .state,
        RunState::Completed
    );
    let im = ImStore::open(home.path().join("im/im.db")).unwrap();
    let history = im
        .list_messages(
            &agt_im::ConversationId::new(direct["id"].as_str().unwrap()),
            &PrincipalId::new("a"),
            MessageListOptions::default(),
        )
        .unwrap();
    assert_eq!(
        history
            .messages
            .iter()
            .filter(|m| m.body == "same reply")
            .count(),
        3
    );
}

#[test]
fn auto_compaction_preserves_request_and_never_replays_delegation() {
    struct CompactModel {
        parent: usize,
        children: usize,
        summaries: usize,
        fail: bool,
    }
    impl ModelClient for CompactModel {
        fn complete(
            &mut self,
            agent: &ResolvedAgent,
            messages: &[Value],
            tools: &[Value],
        ) -> Result<Value> {
            if messages[0]["content"]
                .as_str()
                .unwrap()
                .starts_with("You summarize")
            {
                self.summaries += 1;
                assert!(tools.is_empty());
                assert!(!messages[1].to_string().contains("opaque-private-reasoning"));
                if self.fail {
                    return Ok(text(&"too long ".repeat(20000)));
                }
                // A valid Chinese summary exceeding the former 2048-token cap.
                return Ok(text(&format!(
                    "Child b has completed the review. {}",
                    "保留已完成的任务。".repeat(500)
                )));
            }
            if agent.name == "default/b" {
                self.children += 1;
                return Ok(text("review complete"));
            }
            self.parent += 1;
            if self.parent == 1 {
                let mut response = delegate();
                response["content"] = json!("execution detail ".repeat(9000));
                response["_responses_output"] = json!([
                    {"type":"reasoning","encrypted_content":"opaque-private-reasoning"},
                    {"type":"message","content":[{"type":"output_text","text":response["content"]}]},
                    {"type":"function_call","name":"delegate","call_id":"delegate-1","arguments":response["tool_calls"][0]["function"]["arguments"]}
                ]);
                return Ok(response);
            }
            assert_eq!(messages[1]["content"], "original task");
            assert_eq!(messages.len(), 3);
            assert!(
                messages[2]["content"]
                    .as_str()
                    .unwrap()
                    .contains("Child b has completed")
            );
            assert!(
                messages
                    .iter()
                    .all(|m| m.get("_responses_output").is_none() && m["role"] != "tool")
            );
            Ok(text("done"))
        }
    }
    for fail in [false, true] {
        let home = home();
        ModelStore::new(home.path())
            .update(
                "fast",
                agt_model::ModelConfigUpdate {
                    context_window: Some(32768),
                    ..Default::default()
                },
            )
            .unwrap();
        let mut model = CompactModel {
            parent: 0,
            children: 0,
            summaries: 0,
            fail,
        };
        let mut request = RunRequest::new("a", "original task");
        request.delegates = vec!["b".into()];
        let record = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
            .unwrap()
            .run(request, &CancellationToken::new())
            .unwrap();
        assert_eq!(model.children, 1);
        assert!(model.summaries > 0);
        assert_eq!(
            record.state,
            if fail {
                RunState::Failed
            } else {
                RunState::Completed
            }
        );
        assert_eq!(record.context.compactions, if fail { 0 } else { 1 });
        let saved = RunStore::open(home.path())
            .unwrap()
            .get(&record.id)
            .unwrap();
        assert_eq!(saved.context.compactions, record.context.compactions);
        assert!(!saved.context.summary.contains("opaque-private-reasoning"));
        let calls = RunStore::open(home.path())
            .unwrap()
            .calls(&record.id, 0, 100)
            .unwrap();
        assert!(calls.iter().any(|c| c.kind == "compaction"));
    }
}

#[test]
fn oversized_replacement_is_rejected_before_checkpoint_and_reports_actual_budget() {
    let home = home();
    ModelStore::new(home.path())
        .update(
            "fast",
            agt_model::ModelConfigUpdate {
                context_window: Some(32768),
                ..Default::default()
            },
        )
        .unwrap();
    let mut delegation = delegate();
    delegation["_agt_total_tokens"] = json!(30000);
    let mut model = Script::new(vec![
        ("default/a", delegation),
        ("default/b", text("review complete")),
        ("default/a", text(&"summary ".repeat(20000))),
    ]);
    let mut request = RunRequest::new("a", "original task");
    request.delegates = vec!["b".into()];
    let record = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(request, &CancellationToken::new())
        .unwrap();
    assert_eq!(record.state, RunState::Failed);
    assert_eq!(record.context.compactions, 0);
    assert!(record.context.summary.is_empty());
    let store = RunStore::open(home.path()).unwrap();
    let calls = store.calls(&record.id, 0, 100).unwrap();
    let compact = calls.iter().find(|c| c.kind == "compaction").unwrap();
    assert_eq!(compact.state, agt_runtime::CallState::Failed);
    let error = compact.error.as_ref().unwrap();
    assert_eq!(error["code"], "context_overflow");
    assert_eq!(error["stage"], "compacted run");
    assert!(
        error["estimated_tokens"].as_u64().unwrap() > error["available_tokens"].as_u64().unwrap()
    );
    assert_eq!(compact.argument_summary["request_attempts"], 1);
    assert_eq!(store.children(&record.id).unwrap().len(), 1);
}

#[test]
fn codex_default_compacts_at_ninety_percent_instead_of_the_old_cap() {
    for compact in [false, true] {
        let home = home();
        let agent = ResolvedAgent::load(home.path(), "a").unwrap();
        assert_eq!(agent.model.capabilities.context_window, 272_000);
        let mut delegation = delegate();
        // Both exceed the old 180k ceiling; only the second reaches 90% of 272k.
        delegation["_agt_total_tokens"] = json!(if compact { 245_000 } else { 244_000 });
        let mut answers = vec![
            ("default/a", delegation),
            ("default/b", text("review complete")),
        ];
        if compact {
            answers.push(("default/a", text("Review completed. Report result.")));
        }
        answers.push(("default/a", text("done")));
        let mut model = Script::new(answers);
        let mut request = RunRequest::new("a", "original task");
        request.delegates = vec!["b".into()];
        let record = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
            .unwrap()
            .run(request, &CancellationToken::new())
            .unwrap();
        assert_eq!(record.state, RunState::Completed);
        assert_eq!(record.context.compactions, u32::from(compact));
        if compact {
            let calls = RunStore::open(home.path())
                .unwrap()
                .calls(&record.id, 0, 100)
                .unwrap();
            let call = calls.iter().find(|c| c.kind == "compaction").unwrap();
            assert_eq!(call.argument_summary["auto_compact_tokens"], 244_800);
            assert_eq!(call.argument_summary["available_tokens"], 258_400);
        }
    }
}

#[test]
fn explicitly_large_window_is_not_blocked_by_the_old_one_mib_request_guard() {
    let home = home();
    ModelStore::new(home.path())
        .update(
            "fast",
            agt_model::ModelConfigUpdate {
                context_window: Some(1_000_000),
                ..Default::default()
            },
        )
        .unwrap();
    let mut delegation = delegate();
    delegation["content"] = json!("x".repeat(1_100_000));
    delegation["_agt_total_tokens"] = json!(300_000);
    let mut model = Script::new(vec![
        ("default/a", delegation),
        ("default/b", text("review complete")),
        ("default/a", text("done")),
    ]);
    let mut request = RunRequest::new("a", "original task");
    request.delegates = vec!["b".into()];
    let record = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(request, &CancellationToken::new())
        .unwrap();
    assert_eq!(record.state, RunState::Completed);
    assert_eq!(record.context.compactions, 0);
    assert_eq!(
        model.seen.last().unwrap().1[2]["content"]
            .as_str()
            .unwrap()
            .len(),
        1_100_000
    );
}

#[test]
fn live_progress_is_bounded_retry_replaces_draft_and_completion_clears_it() {
    struct Streaming {
        home: std::path::PathBuf,
        attempts: u32,
    }
    impl ModelClient for Streaming {
        fn complete(&mut self, _: &ResolvedAgent, _: &[Value], _: &[Value]) -> Result<Value> {
            unreachable!()
        }
        fn complete_stream(
            &mut self,
            _: &ResolvedAgent,
            _: &[Value],
            _: &[Value],
            progress: &mut dyn FnMut(agt_runtime::ModelEvent<'_>) -> Result<()>,
        ) -> Result<Value> {
            self.attempts += 1;
            let store = RunStore::open(&self.home).unwrap();
            let before = store.list(1).unwrap().remove(0).progress.unwrap();
            assert_eq!(before.attempt, self.attempts);
            assert!(before.preview.is_empty());
            progress(agt_runtime::ModelEvent::Text(&"草稿".repeat(2000)))?;
            let record = store.list(1).unwrap().remove(0);
            let live = record.progress.unwrap();
            assert!(live.preview.len() <= 4096);
            assert!(!live.preview.is_empty());
            assert!(record.output.is_none());
            assert_eq!(record.state, RunState::Running);
            if self.attempts == 1 {
                return Err(RuntimeError::Model("incomplete event stream"));
            }
            Ok(text("final"))
        }
    }
    let home = home();
    let mut model = Streaming {
        home: home.path().into(),
        attempts: 0,
    };
    let factory = DefaultMcpClientFactory;
    let mut runtime = Runtime::open(home.path(), &mut model, &factory).unwrap();
    let result = runtime
        .run(RunRequest::new("a", "test"), &CancellationToken::new())
        .unwrap();
    assert_eq!(result.state, RunState::Completed);
    assert!(result.progress.is_none());
    assert_eq!(result.output.as_deref(), Some("final"));
    assert_eq!(
        runtime.runs().calls(&result.id, 0, 10).unwrap()[0].argument_summary["request_attempts"],
        2
    );
}

#[path = "context/recovery.rs"]
mod context_recovery;
