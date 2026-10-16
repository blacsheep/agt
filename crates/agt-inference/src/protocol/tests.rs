use super::*;
use crate::{ChatCompletions, ModelClient};
use agt_model::ReasoningEffort;
use std::io::{Read, Write};
use std::time::Duration;

/// A home with one agent whose model speaks `format`, served by a local fixture.
fn agent(
    format: &str,
    effort: Option<ReasoningEffort>,
) -> (tempfile::TempDir, std::net::TcpListener) {
    let home = tempfile::tempdir().unwrap();
    agt_config::initialize_home(Some(home.path().into())).unwrap();
    agt_prompt::PromptStore::new(home.path())
        .ensure_default_prompt()
        .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    agt_provider::ProviderStore::new(home.path())
        .create(
            &agt_provider::ProviderConfig::openai_compatible("fixture", &url, "secret-key")
                .unwrap(),
        )
        .unwrap();
    let mut model = agt_model::ModelConfig::new(
        "fixture",
        agt_model::ModelReference::parse("fixture/model-x").unwrap(),
    )
    .unwrap();
    model.api_format = format.parse().unwrap();
    model.params.reasoning_effort = effort;
    agt_model::ModelStore::new(home.path())
        .create(&model)
        .unwrap();
    agt_agent::AgentStore::new(home.path())
        .create(&agt_agent::AgentConfig::new("fixture", "fixture").unwrap())
        .unwrap();
    (home, listener)
}

/// Serve one request with an event stream; return the request line, headers and body.
fn serve(
    listener: std::net::TcpListener,
    events: String,
) -> std::thread::JoinHandle<(String, Value)> {
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut head = vec![];
        let mut byte = [0];
        while !head.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        let head = String::from_utf8(head).unwrap();
        let length: usize = head
            .lines()
            .find_map(|l| {
                l.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|n| n.trim().parse().unwrap())
            })
            .unwrap();
        let mut body = vec![0; length];
        stream.read_exact(&mut body).unwrap();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{events}", events.len()).unwrap();
        (head, serde_json::from_slice(&body).unwrap())
    })
}

fn frame(event: Value) -> String {
    format!(
        "event: {}\ndata: {event}\n\n",
        event["type"].as_str().unwrap_or("chunk")
    )
}

fn tool(name: &str) -> Value {
    json!({"type":"function","function":{"name":name,"description":"d","parameters":{"type":"object","properties":{"q":{"type":"string"}},"additionalProperties":false}}})
}

#[test]
fn anthropic_round_trip_keeps_thinking_for_replay_and_caches_the_prefix() {
    let (home, listener) = agent("anthropic-messages", Some(ReasoningEffort::High));
    let events = [
        json!({"type":"message_start","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":90}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"PRIVATE"}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"查一下"}}),
        json!({"type":"content_block_stop","index":1}),
        json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_1","name":"search","input":{}}}),
        json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"q\":"}}),
        json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"\"中文\"}"}}),
        json!({"type":"content_block_stop","index":2}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":5}}),
        json!({"type":"message_stop"}),
    ]
    .into_iter()
    .map(frame)
    .collect::<String>();
    let server = serve(listener, events);
    let agent = crate::ResolvedAgent::load(home.path(), "fixture").unwrap();
    let messages = vec![
        json!({"role":"system","content":"You are b."}),
        json!({"role":"system","content":"Decide."}),
        json!({"role":"user","content":"hello"}),
    ];
    let mut preview = String::new();
    let message = ChatCompletions
        .complete_stream(&agent, &messages, &[tool("search")], &mut |e| {
            if let crate::ModelEvent::Text(t) = e {
                preview.push_str(t);
            }
            Ok(())
        })
        .unwrap();
    let (head, body) = server.join().unwrap();
    assert!(head.starts_with("POST /v1/messages "), "{head}");
    assert!(head.to_ascii_lowercase().contains("x-api-key: secret-key"));
    assert!(
        head.to_ascii_lowercase()
            .contains("anthropic-version: 2023-06-01")
    );
    assert!(!head.to_ascii_lowercase().contains("authorization"));
    assert_eq!(body["system"][0]["text"], "You are b.\n\nDecide.");
    assert_eq!(body["thinking"]["budget_tokens"], 16_384);
    assert!(body["max_tokens"].as_u64().unwrap() > 16_384);
    assert!(body.get("temperature").is_none());
    assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    for cached in [
        &body["system"][0],
        &body["tools"][0],
        &body["messages"][0]["content"][0],
    ] {
        assert_eq!(cached["cache_control"]["type"], "ephemeral");
    }
    assert_eq!(preview, "查一下");
    assert_eq!(message["content"], "查一下");
    assert_eq!(message["tool_calls"][0]["id"], "toolu_1");
    assert_eq!(
        message["tool_calls"][0]["function"]["arguments"],
        "{\"q\":\"中文\"}"
    );
    assert_eq!(message["_agt_total_tokens"], 105);
    // The next step sends the thinking block back unchanged, then the tool result.
    let blocks = message["_anthropic_content"].clone();
    assert_eq!(blocks[0]["signature"], "sig");
    let next = vec![
        json!({"role":"user","content":"hello"}),
        message,
        json!({"role":"tool","tool_call_id":"toolu_1","content":"result"}),
        json!({"role":"user","content":"steer"}),
    ];
    let (_, turns) = anthropic::convert_for_test(&next).unwrap();
    assert_eq!(turns[1]["content"], blocks);
    assert_eq!(
        turns.len(),
        3,
        "the tool result and the next message share one user turn"
    );
    assert_eq!(turns[2]["content"][0]["type"], "tool_result");
    assert_eq!(turns[2]["content"][1]["text"], "steer");
}

#[test]
fn anthropic_forced_tool_turns_thinking_off_and_errors_are_classified() {
    let protocol = select(&agt_model::ModelApiFormat::AnthropicMessages);
    let (home, _listener) = agent("anthropic-messages", Some(ReasoningEffort::High));
    let agent = crate::ResolvedAgent::load(home.path(), "fixture").unwrap();
    let mut required = tool("participation_decision");
    required["_agt_required"] = json!(true);
    let request = protocol
        .request(
            &"http://x/v1".parse().unwrap(),
            &agent,
            &[json!({"role":"user","content":"hi"})],
            &[required],
        )
        .unwrap();
    assert!(request.body.get("thinking").is_none());
    assert_eq!(
        request.body["tool_choice"]["name"],
        "participation_decision"
    );
    assert!(!request.body.to_string().contains("_agt_required"));
    assert!(protocol.context_rejected(
        &json!({"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 210000 tokens > 200000 maximum"}})
    ));
    assert!(!protocol.context_rejected(
        &json!({"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}})
    ));
    let mut decoder = protocol.decoder();
    for event in [
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        json!({"type":"message_delta","delta":{"stop_reason":"max_tokens"}}),
    ] {
        decoder.event(&event, &mut |_| Ok(())).unwrap();
    }
    assert!(
        decoder
            .event(&json!({"type":"message_stop"}), &mut |_| Ok(()))
            .is_err()
    );
}

#[test]
fn gemini_round_trip_names_tool_results_and_keeps_thought_signatures() {
    let (home, listener) = agent("gemini", Some(ReasoningEffort::Low));
    let chunks = [
        json!({"candidates":[{"content":{"role":"model","parts":[{"text":"PRIVATE","thought":true}]}}]}),
        json!({"candidates":[{"content":{"role":"model","parts":[{"text":"查一下"}]}}]}),
        json!({"candidates":[{"content":{"role":"model","parts":[{"functionCall":{"name":"search","args":{"q":"中文"}},"thoughtSignature":"sig"}]},"finishReason":"STOP"}],
               "usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":5,"totalTokenCount":120}}),
    ]
    .into_iter()
    .map(|c| format!("data: {c}\r\n\r\n"))
    .collect::<String>();
    let server = serve(listener, chunks);
    let agent = crate::ResolvedAgent::load(home.path(), "fixture").unwrap();
    let mut preview = String::new();
    let message = ChatCompletions
        .complete_stream(
            &agent,
            &[
                json!({"role":"system","content":"You are b."}),
                json!({"role":"user","content":"hello"}),
            ],
            &[tool("search")],
            &mut |e| {
                if let crate::ModelEvent::Text(t) = e {
                    preview.push_str(t);
                }
                Ok(())
            },
        )
        .unwrap();
    let (head, body) = server.join().unwrap();
    assert!(
        head.starts_with("POST /v1/models/model-x:streamGenerateContent?alt=sse "),
        "{head}"
    );
    assert!(
        head.to_ascii_lowercase()
            .contains("x-goog-api-key: secret-key")
    );
    assert!(!head.to_ascii_lowercase().contains("authorization"));
    assert_eq!(body["systemInstruction"]["parts"][0]["text"], "You are b.");
    assert_eq!(
        body["generationConfig"]["thinkingConfig"]["thinkingBudget"],
        2048
    );
    assert_eq!(
        body["tools"][0]["functionDeclarations"][0]["parametersJsonSchema"]["additionalProperties"],
        false
    );
    assert_eq!(preview, "查一下");
    assert_eq!(message["content"], "查一下");
    assert!(!message["content"].as_str().unwrap().contains("PRIVATE"));
    let id = message["tool_calls"][0]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        message["tool_calls"][0]["function"]["arguments"],
        "{\"q\":\"中文\"}"
    );
    assert_eq!(message["_agt_total_tokens"], 120);
    let parts = message["_gemini_parts"].clone();
    let next = vec![
        json!({"role":"user","content":"hello"}),
        message,
        json!({"role":"tool","tool_call_id":id,"content":"result"}),
    ];
    let (_, contents) = gemini::convert_for_test(&next).unwrap();
    assert_eq!(contents[1]["role"], "model");
    assert_eq!(contents[1]["parts"], parts);
    assert_eq!(
        contents[2]["parts"][0]["functionResponse"]["name"],
        "search"
    );
}

#[test]
fn gemini_incomplete_streams_and_oversized_prompts_are_errors() {
    let protocol = select(&agt_model::ModelApiFormat::Gemini);
    let mut decoder = protocol.decoder();
    decoder
        .event(
            &json!({"candidates":[{"content":{"parts":[{"text":"partial"}]}}]}),
            &mut |_| Ok(()),
        )
        .unwrap();
    assert!(matches!(
        decoder.finish(false),
        Err(RuntimeError::ModelTransient(_))
    ));
    let mut decoder = protocol.decoder();
    decoder
        .event(&json!({"candidates":[{"content":{"parts":[{"text":"cut"}]},"finishReason":"MAX_TOKENS"}]}), &mut |_| Ok(()))
        .unwrap();
    assert!(matches!(decoder.finish(false), Err(RuntimeError::Model(_))));
    assert!(protocol.context_rejected(&json!({"error":{"code":400,"status":"INVALID_ARGUMENT",
        "message":"The input token count (1200000) exceeds the maximum number of tokens allowed (1048576)."}})));
    assert!(
        !protocol
            .context_rejected(&json!({"error":{"status":"RESOURCE_EXHAUSTED","message":"quota"}}))
    );
}

#[test]
fn forced_tool_is_encoded_for_openai_apis_without_private_metadata() {
    let (home, _listener) = agent("chat-completions", None);
    let agent = crate::ResolvedAgent::load(home.path(), "fixture").unwrap();
    let mut required = tool("participation_decision");
    required["_agt_required"] = json!(true);
    for format in [
        agt_model::ModelApiFormat::ChatCompletions,
        agt_model::ModelApiFormat::Responses,
    ] {
        let responses = format == agt_model::ModelApiFormat::Responses;
        let request = select(&format)
            .request(
                &"http://x/v1".parse().unwrap(),
                &agent,
                &[],
                std::slice::from_ref(&required),
            )
            .unwrap();
        assert_eq!(request.body["parallel_tool_calls"], false);
        assert_eq!(
            request.body["tool_choice"],
            if responses {
                json!({"type":"function","name":"participation_decision"})
            } else {
                json!({"type":"function","function":{"name":"participation_decision"}})
            }
        );
        assert!(!request.body.to_string().contains("_agt_required"));
        assert_eq!(request.headers[0].1, "Bearer secret-key");
    }
    let ordinary = select(&agt_model::ModelApiFormat::ChatCompletions)
        .request(
            &"http://x/v1".parse().unwrap(),
            &agent,
            &[],
            &[tool("search")],
        )
        .unwrap();
    assert!(ordinary.body.get("tool_choice").is_none());
}
