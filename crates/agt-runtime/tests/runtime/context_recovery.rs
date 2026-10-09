use super::*;

struct Responses {
    answers: VecDeque<Result<Value>>,
    requests: Vec<Vec<Value>>,
}
impl ModelClient for Responses {
    fn complete(&mut self, _: &ResolvedAgent, messages: &[Value], _: &[Value]) -> Result<Value> {
        self.requests.push(messages.to_vec());
        self.answers.pop_front().expect("unexpected request/retry")
    }
}

#[test]
fn reported_usage_below_threshold_is_not_rejected_by_serialized_bytes() {
    let home = home();
    let mut delegation = delegate();
    delegation["content"] = json!("x".repeat(1_100_000));
    delegation["_agt_total_tokens"] = json!(100_000);
    let mut model = Script::new(vec![
        ("default/a", delegation),
        ("default/b", text("done")),
        ("default/a", text("finished")),
    ]);
    let mut request = RunRequest::new("a", "task");
    request.delegates = vec!["b".into()];
    let record = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(request, &CancellationToken::new())
        .unwrap();
    assert_eq!(record.state, RunState::Completed);
    assert_eq!(record.context.compactions, 0);
    assert!(
        model.seen.last().unwrap().1[2]["content"]
            .as_str()
            .unwrap()
            .len()
            > 1_048_576
    );
}

#[test]
fn provider_rejection_compacts_without_replaying_completed_tools_and_is_bounded() {
    for second_rejection in [false, true] {
        let home = home();
        let mut model = Responses {
            answers: VecDeque::from([
                Ok(delegate()),
                Ok(text("review complete")),
                Err(RuntimeError::ContextWindowRejected),
                Ok(text(
                    "Delegated review finished; report it without repeating.",
                )),
                if second_rejection {
                    Err(RuntimeError::ContextWindowRejected)
                } else {
                    Ok(text("done"))
                },
            ]),
            requests: vec![],
        };
        let mut request = RunRequest::new("a", "original task");
        request.delegates = vec!["b".into()];
        let record = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
            .unwrap()
            .run(request, &CancellationToken::new())
            .unwrap();
        assert_eq!(
            record.state,
            if second_rejection {
                RunState::Failed
            } else {
                RunState::Completed
            }
        );
        assert_eq!(record.context.compactions, 1);
        assert_eq!(model.requests.len(), 5);
        assert_eq!(model.requests[4][1]["content"], "original task");
        assert!(!model.requests[4].iter().any(|v| v["role"] == "tool"));
        let store = RunStore::open(home.path()).unwrap();
        assert_eq!(store.children(&record.id).unwrap().len(), 1);
        let calls = store.calls(&record.id, 0, 100).unwrap();
        let compaction = calls.iter().find(|c| c.kind == "compaction").unwrap();
        assert_eq!(
            compaction.argument_summary["trigger"],
            "provider_context_rejected"
        );
        assert!(calls.iter().any(|c| {
            c.error
                .as_ref()
                .is_some_and(|e| e["code"] == "provider_context_exceeded")
        }));
    }
}

#[test]
fn oversized_fixed_input_fails_without_compaction_or_blind_retries() {
    let home = home();
    let mut model = Responses {
        answers: VecDeque::from([Err(RuntimeError::ContextWindowRejected)]),
        requests: vec![],
    };
    let record = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(
            RunRequest::new("a", "fixed input"),
            &CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(record.state, RunState::Failed);
    assert_eq!(record.context.compactions, 0);
    assert_eq!(model.requests.len(), 1);
}

#[test]
fn summary_rejection_shrinks_chunks_without_losing_source_or_replaying_tools() {
    #[derive(Default)]
    struct SummaryModel {
        rejected: Option<String>,
        accepted: String,
        main_calls: usize,
    }
    impl ModelClient for SummaryModel {
        fn complete(
            &mut self,
            agent: &ResolvedAgent,
            messages: &[Value],
            _: &[Value],
        ) -> Result<Value> {
            if messages[0]["content"]
                .as_str()
                .unwrap_or("")
                .starts_with("You summarize")
            {
                let data: Value =
                    serde_json::from_str(messages[1]["content"].as_str().unwrap()).unwrap();
                let chunk = data["next_data_chunk"].as_str().unwrap();
                if self.rejected.is_none() {
                    self.rejected = Some(chunk.into());
                    return Err(RuntimeError::ContextWindowRejected);
                }
                assert!(chunk.len() < self.rejected.as_ref().unwrap().len());
                self.accepted.push_str(chunk);
                return Ok(text("Review completed. Report it."));
            }
            self.main_calls += 1;
            if self.main_calls == 1 {
                let mut message = delegate();
                message["content"] = json!("执行历史🙂".repeat(500));
                message["_agt_total_tokens"] = json!(245_000);
                Ok(message)
            } else if agent.name == "default/b" {
                Ok(text("review complete"))
            } else {
                Ok(text("done"))
            }
        }
    }
    let home = home();
    let mut model = SummaryModel::default();
    let mut request = RunRequest::new("a", "task");
    request.delegates = vec!["b".into()];
    let record = Runtime::open(home.path(), &mut model, &DefaultMcpClientFactory)
        .unwrap()
        .run(request, &CancellationToken::new())
        .unwrap();
    assert_eq!(record.state, RunState::Completed);
    assert_eq!(record.context.compactions, 1);
    assert_eq!(model.accepted, model.rejected.unwrap());
    assert_eq!(model.main_calls, 3);
    assert_eq!(
        RunStore::open(home.path())
            .unwrap()
            .children(&record.id)
            .unwrap()
            .len(),
        1
    );
}
