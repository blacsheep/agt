use super::*;
use agt_runtime::CallState;

fn write_prompt(id: &str, reference: &str) -> Value {
    call(
        id,
        "framework_resource_write",
        json!({
            "kind":"prompt", "reference":reference, "create":true,
            "data":{"text":"created by a tool"}
        }),
    )
}

#[test]
fn business_error_is_audited_and_the_rest_of_the_batch_reaches_the_next_turn() {
    let home = home();
    mount_resources(home.path(), "a");
    let mut model = Script::new(vec![
        (
            "default/a",
            calls(vec![
                call(
                    "missing",
                    "framework_resource_read",
                    json!({"kind":"prompt","reference":"missing"}),
                ),
                write_prompt("write", "created"),
            ]),
        ),
        ("default/a", text("done")),
    ]);
    let run = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Completed, "{:?}", run.error);
    assert_eq!(run.tool_failures, 1);
    assert_eq!(model.seen.len(), 2);
    let messages = &model.seen[1].1;
    assert_eq!(messages.len(), 5);
    assert_eq!(messages[3]["tool_call_id"], "missing");
    assert_eq!(messages[4]["tool_call_id"], "write");
    assert!(PromptStore::new(home.path()).read("created").is_ok());
    let audit = RunStore::open(home.path())
        .unwrap()
        .calls(&run.id, 0, 100)
        .unwrap();
    assert_eq!(
        audit.iter().map(|c| c.state).collect::<Vec<_>>(),
        vec![
            CallState::Succeeded,
            CallState::Failed,
            CallState::Succeeded,
            CallState::Succeeded,
        ]
    );
    assert_eq!(
        audit[1].tool_name.as_deref(),
        Some("framework_resource_read")
    );
    assert_eq!(
        audit[2].tool_name.as_deref(),
        Some("framework_resource_write")
    );
    assert!(audit[3].tool_name.is_none());
    let store = RunStore::open(home.path()).unwrap();
    let detail = store
        .tool_detail(&run.id, &format!("call:{}", audit[2].id))
        .unwrap()
        .unwrap();
    assert!(
        detail.fields["arguments"]
            .text
            .contains("created by a tool")
    );
    assert!(!detail.fields["output"].text.is_empty());
    let failed = store
        .tool_detail(&run.id, &format!("call:{}", audit[1].id))
        .unwrap()
        .unwrap();
    assert!(failed.fields["output"].text.contains("error"));
    assert!(
        !serde_json::to_string(&audit)
            .unwrap()
            .contains("created by a tool")
    );
}

#[test]
fn fatal_or_canceled_tool_stops_the_batch_before_the_next_side_effect() {
    struct FailingChild {
        cancel: bool,
        token: CancellationToken,
    }
    impl ModelClient for FailingChild {
        fn complete(&mut self, agent: &ResolvedAgent, _: &[Value], _: &[Value]) -> Result<Value> {
            if agent.name == "default/a" {
                return Ok(calls(vec![
                    call(
                        "delegate",
                        "delegate",
                        json!({"agent":"default/b","input":"review"}),
                    ),
                    write_prompt("must-not-run", "forbidden"),
                ]));
            }
            if self.cancel {
                self.token.cancel();
                Ok(text("ignored after cancellation"))
            } else {
                Err(RuntimeError::HttpStatus(400))
            }
        }
    }
    for cancel in [false, true] {
        let home = home();
        mount_resources(home.path(), "a");
        let token = CancellationToken::new();
        let mut model = FailingChild {
            cancel,
            token: token.clone(),
        };
        let mut req = request();
        req.delegates = vec!["b".into()];
        let run = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
            .unwrap()
            .run(req, &token)
            .unwrap();
        assert_eq!(
            run.state,
            if cancel {
                RunState::Canceled
            } else {
                RunState::Failed
            }
        );
        assert_eq!(run.tool_failures, 1);
        assert!(PromptStore::new(home.path()).read("forbidden").is_err());
        let audit = RunStore::open(home.path())
            .unwrap()
            .calls(&run.id, 0, 100)
            .unwrap();
        assert_eq!(audit.len(), 2);
        assert_eq!(audit[1].tool_name.as_deref(), Some("delegate"));
        assert_eq!(
            audit[1].state,
            if cancel {
                CallState::Canceled
            } else {
                CallState::Failed
            }
        );
    }
}

#[test]
fn context_recovery_allowance_resets_after_a_successful_tool_turn() {
    struct Recovering {
        answers: VecDeque<Result<Value>>,
    }
    impl ModelClient for Recovering {
        fn complete(
            &mut self,
            _: &ResolvedAgent,
            messages: &[Value],
            _: &[Value],
        ) -> Result<Value> {
            assert!(
                !messages
                    .iter()
                    .any(|m| m.get("_agt_total_tokens").is_some())
            );
            self.answers
                .pop_front()
                .expect("unexpected request or replay")
        }
    }
    let home = home();
    mount_resources(home.path(), "a");
    let mut first = calls(vec![write_prompt("first", "first")]);
    first["_agt_total_tokens"] = json!(100);
    let mut model = Recovering {
        answers: VecDeque::from([
            Ok(first),
            Err(RuntimeError::ContextWindowRejected),
            Ok(text("First prompt was created; do not repeat it.")),
            Ok(calls(vec![write_prompt("second", "second")])),
            Err(RuntimeError::ContextWindowRejected),
            Ok(text("Both prompts were created; return the result.")),
            Ok(text("done")),
        ]),
    };
    let run = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(request(), &CancellationToken::new())
        .unwrap();
    assert_eq!(run.state, RunState::Completed, "{:?}", run.error);
    assert_eq!(run.context.compactions, 2);
    assert_eq!(run.model_steps, 5);
    assert_eq!(run.tool_failures, 0);
    assert!(model.answers.is_empty());
    let audit = RunStore::open(home.path())
        .unwrap()
        .calls(&run.id, 0, 100)
        .unwrap();
    assert_eq!(audit.iter().filter(|c| c.tool_name.is_some()).count(), 2);
    for name in ["first", "second"] {
        assert_eq!(
            PromptStore::new(home.path()).read(name).unwrap().content,
            "created by a tool"
        );
    }
}
