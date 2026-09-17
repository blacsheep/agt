use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agt"))
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env("no_proxy", "127.0.0.1,localhost")
        .arg("--home")
        .arg(home)
        .args(args)
        .output()
        .unwrap()
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn setup(home: &Path, url: &str) {
    success(run(home, &["init", home.to_str().unwrap()]));
    success(run(
        home,
        &[
            "provider",
            "create",
            "mock",
            "--base-url",
            url,
            "--api-key",
            "mock-secret",
        ],
    ));
    success(run(
        home,
        &[
            "model",
            "create",
            "test",
            "--provider",
            "mock",
            "--model",
            "test-model",
            "--max-tokens",
            "256",
        ],
    ));
    for name in ["a", "b"] {
        success(run(home, &["agent", "create", name, "--model", "test"]));
    }
}

/// Loopback only. The listener and sockets have bounded waits so a broken client
/// fails a test rather than leaving a hanging fixture.
fn server(responses: Vec<(u16, Value)>) -> (String, JoinHandle<Vec<Value>>) {
    server_at(responses, "chat/completions")
}

fn server_at(
    responses: Vec<(u16, Value)>,
    endpoint: &'static str,
) -> (String, JoinHandle<Vec<Value>>) {
    server_wire(
        responses
            .into_iter()
            .map(|(status, value)| (status, "application/json", value.to_string()))
            .collect(),
        endpoint,
    )
}
fn server_wire(
    responses: Vec<(u16, &'static str, String)>,
    endpoint: &'static str,
) -> (String, JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/v1/", listener.local_addr().unwrap());
    let thread = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, content_type, body) in responses {
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "HTTP request did not arrive");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            // Accepted sockets can inherit nonblocking mode on macOS.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(&mut stream);
            let mut first_line = String::new();
            reader.read_line(&mut first_line).unwrap();
            assert_eq!(first_line, format!("POST /v1/{endpoint} HTTP/1.1\r\n"));
            let mut length = None;
            let mut authorized = false;
            loop {
                let mut line = String::new();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                let lower = line.to_ascii_lowercase();
                if let Some(value) = lower.strip_prefix("content-length:") {
                    length = Some(value.trim().parse::<usize>().unwrap());
                }
                if lower.trim() == "authorization: bearer mock-secret" {
                    authorized = true;
                }
            }
            assert!(authorized);
            let mut request_body = vec![0; length.expect("content length")];
            reader.read_exact(&mut request_body).unwrap();
            requests.push(serde_json::from_slice(&request_body).unwrap());
            write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            if content_type == "text/event-stream" {
                let split = body.find("\n\n").unwrap() + 2;
                stream.write_all(&body.as_bytes()[..split]).unwrap();
                stream.flush().unwrap();
                thread::sleep(Duration::from_millis(700));
                stream.write_all(&body.as_bytes()[split..]).unwrap();
            } else {
                stream.write_all(body.as_bytes()).unwrap();
            }
        }
        requests
    });
    (url, thread)
}

fn completion(message: Value, reason: &str) -> (u16, Value) {
    (
        200,
        json!({"choices":[{"message":message,"finish_reason":reason}]}),
    )
}
fn answer(content: &str) -> (u16, Value) {
    completion(json!({"role":"assistant","content":content}), "stop")
}

#[test]
fn cli_runs_real_http_delegation_and_replays_with_server_offline() {
    let delegation = json!({"role":"assistant","content":null,"tool_calls":[{
        "id":"review-call","type":"function","function":{"name":"delegate","arguments":json!({"agent":"default/b","input":"review task"}).to_string()}
    }]});
    let (url, server) = server(vec![
        completion(delegation, "tool_calls"),
        answer("review result"),
        answer("final answer"),
    ]);
    let home = tempfile::tempdir().unwrap();
    setup(home.path(), &url);
    let args = [
        "agent",
        "run",
        "a",
        "--input",
        "solve task",
        "--delegate",
        "b",
        "--request-key",
        "cli-replay",
    ];
    let result = success(run(home.path(), &args));
    assert_eq!(result["state"], "completed");
    assert_eq!(result["output"], "final answer");
    assert_eq!(result["model_steps"], 3);
    assert!(!result.to_string().contains("mock-secret"));
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0]["model"], "test-model");
    assert_eq!(requests[0]["max_completion_tokens"], 256);
    assert_eq!(requests[0]["stream"], true);
    assert_eq!(requests[0]["tools"][0]["function"]["name"], "delegate");
    assert!(requests[1]["tools"].is_null());
    assert_eq!(requests[1]["messages"][1]["content"], "review task");
    assert_eq!(requests[2]["messages"][3]["tool_call_id"], "review-call");
    assert_eq!(requests[2]["messages"][3]["content"], "review result");
    let replay = success(run(home.path(), &args));
    assert_eq!(replay["id"], result["id"]);
    let id = result["id"].as_str().unwrap();
    assert_eq!(
        success(run(home.path(), &["run", "info", id]))["output"],
        "final answer"
    );
    let children = success(run(home.path(), &["run", "children", id]));
    assert_eq!(children.as_array().unwrap().len(), 1);
    assert_eq!(children[0]["output"], "review result");
    let listed = success(run(home.path(), &["run", "list", "--limit", "1"]));
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["id"], children[0]["id"]);
    let calls = success(run(home.path(), &["run", "calls", id]));
    assert_eq!(calls.as_array().unwrap().len(), 3);
    assert_eq!(calls[1]["tool_name"], "delegate");
    assert_eq!(calls[1]["state"], "succeeded");
    let after = calls[0]["id"].to_string();
    assert_eq!(
        success(run(
            home.path(),
            &["run", "calls", id, "--after", &after, "--limit", "1"]
        ))[0]["id"],
        calls[1]["id"]
    );
}

#[test]
fn http_failure_retries_ten_times_and_persists_redacted_status() {
    let (url, server) = server(vec![
        (
            401,
            json!({"error":{"message":"mock-secret reflected upstream"}}),
        );
        10
    ]);
    let home = tempfile::tempdir().unwrap();
    setup(home.path(), &url);
    let args = [
        "agent",
        "run",
        "a",
        "--input",
        "hello",
        "--request-key",
        "failure",
    ];
    let output = run(home.path(), &args);
    assert!(!output.status.success());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["state"], "failed");
    assert_eq!(
        result["error"],
        "model request failed after 10 attempts: model HTTP status 401"
    );
    let calls = success(run(
        home.path(),
        &["run", "calls", result["id"].as_str().unwrap()],
    ));
    assert_eq!(calls[0]["state"], "failed");
    assert_eq!(calls[0]["error"]["code"], "model_http_error");
    assert_eq!(calls[0]["error"]["http_status"], 401);
    assert_eq!(calls[0]["error"]["attempts"], 10);
    assert_eq!(calls[0]["argument_summary"]["request_attempts"], 10);
    assert!(!calls.to_string().contains("mock-secret"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("mock-secret"));
    assert_eq!(server.join().unwrap().len(), 10);
    let replay = run(home.path(), &args);
    assert!(!replay.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&replay.stdout).unwrap()["id"],
        result["id"]
    );
}

#[test]
fn cli_tool_errors_are_structured_and_do_not_echo_arguments() {
    let home = tempfile::tempdir().unwrap();
    success(run(home.path(), &["init", home.path().to_str().unwrap()]));
    success(run(home.path(), &["im", "principal", "create", "user"]));
    let output = run(
        home.path(),
        &[
            "tool",
            "call",
            "--principal",
            "user",
            "message_ack",
            "--args",
            r#"{"id":"secret-value"}"#,
        ],
    );
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["error"]["code"], "invalid_arguments");
    assert_eq!(error["error"]["field"], "id");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("secret-value"));
}

#[test]
fn truncated_model_response_is_not_success() {
    let (url, server) = server(vec![completion(
        json!({"role":"assistant","content":"incomplete text"}),
        "length",
    )]);
    let home = tempfile::tempdir().unwrap();
    setup(home.path(), &url);
    let output = run(home.path(), &["agent", "run", "a", "--input", "hello"]);
    assert!(!output.status.success());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["state"], "failed");
    assert!(result["output"].is_null());
    server.join().unwrap();
}

#[test]
fn stdin_is_supported_and_input_sources_are_exclusive() {
    let (url, server) = server(vec![answer("stdin result")]);
    let home = tempfile::tempdir().unwrap();
    setup(home.path(), &url);
    let mut child = Command::new(env!("CARGO_BIN_EXE_agt"))
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env("no_proxy", "127.0.0.1,localhost")
        .arg("--home")
        .arg(home.path())
        .args(["agent", "run", "a", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"stdin task")
        .unwrap();
    assert_eq!(
        success(child.wait_with_output().unwrap())["output"],
        "stdin result"
    );
    assert_eq!(
        server.join().unwrap()[0]["messages"][1]["content"],
        "stdin task"
    );
    assert!(
        !run(
            home.path(),
            &["agent", "run", "a", "--input", "x", "--stdin"]
        )
        .status
        .success()
    );
    assert!(!run(home.path(), &["agent", "run", "a"]).status.success());
}

#[test]
fn responses_runs_tools_and_preserves_reasoning_without_cross_agent_state() {
    let reasoning =
        json!({"type":"reasoning","id":"rs_1","summary":[],"encrypted_content":"opaque-context"});
    let call = json!({"type":"function_call","id":"fc_1","call_id":"review-call","name":"delegate","arguments":json!({"agent":"default/b","input":"review task"}).to_string(),"status":"completed"});
    let text = |s: &str| json!({"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":s}]});
    let (url, server) = server_at(
        vec![
            (200, json!({"status":"completed","output":[reasoning,call]})),
            (
                200,
                json!({"status":"completed","output":[text("review result")]}),
            ),
            (
                200,
                json!({"status":"completed","output":[text("final answer")]}),
            ),
        ],
        "responses",
    );
    let home = tempfile::tempdir().unwrap();
    setup(home.path(), &url);
    success(run(
        home.path(),
        &[
            "model",
            "update",
            "test",
            "--api-format",
            "responses",
            "--reasoning-effort",
            "low",
        ],
    ));
    let args = [
        "agent",
        "run",
        "a",
        "--input",
        "hello",
        "--delegate",
        "b",
        "--request-key",
        "responses-replay",
    ];
    let result = success(run(home.path(), &args));
    assert_eq!(result["state"], "completed");
    assert_eq!(result["output"], "final answer");
    assert_eq!(result["model_steps"], 3);
    let requests = server.join().unwrap();
    assert_eq!(requests[0]["max_output_tokens"], 256);
    assert_eq!(requests[0]["reasoning"]["effort"], "low");
    assert!(requests[0]["temperature"].is_null());
    assert!(requests[0]["messages"].is_null());
    assert_eq!(requests[0]["store"], false);
    assert_eq!(
        requests[0]["include"],
        json!(["reasoning.encrypted_content"])
    );
    assert_eq!(requests[0]["tools"][0]["name"], "delegate");
    assert_eq!(requests[0]["tools"][0]["strict"], true);
    assert_eq!(requests[1]["input"].as_array().unwrap().len(), 2);
    assert_eq!(requests[1]["input"][1]["content"], "review task");
    assert_eq!(requests[2]["input"][2], reasoning);
    assert_eq!(requests[2]["input"][3], call);
    assert_eq!(
        requests[2]["input"][4],
        json!({"type":"function_call_output","call_id":"review-call","output":"review result"})
    );
    assert_eq!(success(run(home.path(), &args))["id"], result["id"]);
    let calls = success(run(
        home.path(),
        &["run", "calls", result["id"].as_str().unwrap()],
    ));
    assert_eq!(calls.as_array().unwrap().len(), 3);
    assert!(!calls.to_string().contains("opaque-context"));
}

#[test]
fn responses_failures_are_recorded_without_exposing_upstream_content() {
    for (status, body) in [
        (401, json!({"error":{"message":"mock-secret"}})),
        (200, json!({"status":"incomplete","output":[]})),
        (
            200,
            json!({"status":"failed","error":{"message":"mock-secret"},"output":[]}),
        ),
        (
            200,
            json!({"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"refusal","refusal":"mock-secret"}]}]}),
        ),
        (200, json!({"status":"completed","output":[]})),
        (
            200,
            json!({"status":"completed","output":[{"type":"function_call","call_id":"bad","name":"delegate","arguments":"not json"}]}),
        ),
    ] {
        let count = if status != 200
            || !body["error"].is_null()
            || (body["status"] == "completed" && body["output"] == json!([]))
        {
            10
        } else {
            1
        };
        let (url, server) = server_at(vec![(status, body); count], "responses");
        let home = tempfile::tempdir().unwrap();
        setup(home.path(), &url);
        success(run(
            home.path(),
            &["model", "update", "test", "--api-format", "responses"],
        ));
        let output = run(
            home.path(),
            &["agent", "run", "a", "--input", "hello", "--delegate", "b"],
        );
        assert!(!output.status.success());
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["state"], "failed");
        assert_eq!(result["model_steps"], 1);
        assert!(result["output"].is_null());
        let calls = success(run(
            home.path(),
            &["run", "calls", result["id"].as_str().unwrap()],
        ));
        assert_eq!(calls[0]["state"], "failed");
        assert!(!calls.to_string().contains("mock-secret"));
        server.join().unwrap();
    }
}

#[test]
fn mcp_manager_provisions_agents_and_group_then_scheduler_broadcasts_replies() {
    group_scheduler_replies("chat-completions");
}

#[test]
fn responses_group_scheduler_broadcasts_replies() {
    group_scheduler_replies("responses");
}

fn group_scheduler_replies(format: &str) {
    use agt_mcp::{DefaultMcpClientFactory, McpClientFactory, McpDocument, McpLimits};
    let (url, server) = if format == "responses" {
        server_at(["A group reply", "B group reply"].map(|text| (200, json!({"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}]}))).to_vec(), "responses")
    } else {
        server(vec![answer("A group reply"), answer("B group reply")])
    };
    let home = tempfile::tempdir().unwrap();
    success(run(home.path(), &["init", home.path().to_str().unwrap()]));
    success(run(
        home.path(),
        &["im", "principal", "create", "owner", "--name", "Owner"],
    ));
    let document = McpDocument::stdio(
        "framework",
        env!("CARGO_BIN_EXE_agt"),
        vec![
            "--home".into(),
            home.path().to_string_lossy().into_owned(),
            "serve".into(),
            "--principal".into(),
            "owner".into(),
            "--manager".into(),
        ],
        vec![("AGT_ACCEPTANCE_SECRET".into(), "mock-secret".into())],
    )
    .unwrap();
    let mut client = DefaultMcpClientFactory
        .connect(&document, home.path(), McpLimits::default())
        .unwrap();
    let schemas = client.list_tools().unwrap();
    assert!(schemas.iter().any(|t| t.name == "resource_write"));
    let mut call = |name: &str, args: Value| -> Value {
        let result = client.call_tool(name, args).unwrap();
        assert!(!result.is_error, "{name}: {:?}", result.content);
        serde_json::from_str(result.content[0].text.as_deref().unwrap()).unwrap()
    };
    let provider = call(
        "resource_write",
        json!({"kind":"provider","reference":"mock","create":true,"data":{"base_url":url,"api_key_env":"AGT_ACCEPTANCE_SECRET"}}),
    );
    assert!(!provider.to_string().contains("mock-secret"));
    call(
        "resource_write",
        json!({"kind":"model","reference":"test","create":true,"data":{"provider":"mock","model":"test-model","params":{"api_format":format}}}),
    );
    call(
        "resource_write",
        json!({"kind":"prompt","reference":"group","create":true,"data":{"text":"Contribute briefly to the group discussion."}}),
    );
    for name in ["a", "b"] {
        call(
            "resource_write",
            json!({"kind":"agent","reference":name,"create":true,"data":{"model":"test","prompt":"group"}}),
        );
        call(
            "principal_create",
            json!({"id":name,"name":name,"kind":"agent"}),
        );
        call(
            "agent_bind",
            json!({"principal":name,"agent":name,"working_directory":home.path(),"policy":"mentioned","max_steps":6}),
        );
    }
    let group = call(
        "group_create",
        json!({"title":"acceptance","members":["a","b"]}),
    );
    let sent = call(
        "message_send",
        json!({"conversation":group["id"],"body":"Discuss the plan","mentions":["a","b"],"request_key":"acceptance"}),
    );
    assert_eq!(sent["message"]["sender_id"], "owner");
    let conversation = group["id"].as_str().unwrap();
    success(run(
        home.path(),
        &[
            "schedule",
            "start",
            conversation,
            "--message",
            sent["message"]["id"].as_str().unwrap(),
        ],
    ));
    std::thread::sleep(std::time::Duration::from_millis(5100));
    success(run(home.path(), &["schedule", "run"]));
    success(run(home.path(), &["schedule", "end", conversation]));
    let requests = server.join().unwrap();
    if format == "responses" {
        assert!(
            requests[0]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .all(|tool| tool["strict"] == false
                    && tool["name"].as_str().unwrap().starts_with("framework_"))
        );
    }
    success(run(home.path(), &["schedule", "run"])); // ended discussion must not infer again
    let history = call("message_history", json!({"conversation":group["id"]}));
    assert_eq!(history["messages"].as_array().unwrap().len(), 3);
    let mut senders: Vec<_> = history["messages"].as_array().unwrap()[1..]
        .iter()
        .map(|m| m["sender_id"].as_str().unwrap())
        .collect();
    senders.sort();
    assert_eq!(senders, ["a", "b"]);
    client.close().unwrap();
    let participant = McpDocument::stdio(
        "participant",
        env!("CARGO_BIN_EXE_agt"),
        vec![
            "--home".into(),
            home.path().to_string_lossy().into_owned(),
            "serve".into(),
            "--principal".into(),
            "a".into(),
        ],
        Vec::<(String, String)>::new(),
    )
    .unwrap();
    let mut client = DefaultMcpClientFactory
        .connect(&participant, home.path(), McpLimits::default())
        .unwrap();
    assert!(
        !client
            .list_tools()
            .unwrap()
            .iter()
            .any(|t| t.name == "resource_write")
    );
    assert!(client.call_tool("resource_write", json!({})).is_err());
    client.close().unwrap();
}

#[test]
fn reasoning_max_and_custom_values_reach_both_api_formats() {
    for (format, endpoint) in [
        ("responses", "responses"),
        ("chat-completions", "chat/completions"),
    ] {
        for effort in ["max", "future-level"] {
            let response = if format == "responses" {
                json!({"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}]})
            } else {
                json!({"choices":[{"message":{"role":"assistant","content":"done"},"finish_reason":"stop"}]})
            };
            let (url, server) = server_at(vec![(200, response)], endpoint);
            let home = tempfile::tempdir().unwrap();
            setup(home.path(), &url);
            success(run(
                home.path(),
                &[
                    "model",
                    "update",
                    "test",
                    "--api-format",
                    format,
                    "--reasoning-effort",
                    effort,
                ],
            ));
            let result = success(run(home.path(), &["agent", "run", "a", "--input", "hello"]));
            assert_eq!(result["state"], "completed");
            let requests = server.join().unwrap();
            let actual = if format == "responses" {
                &requests[0]["reasoning"]["effort"]
            } else {
                &requests[0]["reasoning_effort"]
            };
            assert_eq!(actual, effort);
            assert!(requests[0]["temperature"].is_null());
        }
    }
}

#[test]
fn http_and_empty_responses_retry_identical_requests_in_both_formats() {
    for (format, endpoint) in [
        ("responses", "responses"),
        ("chat-completions", "chat/completions"),
    ] {
        let (empty, success_response) = if format == "responses" {
            (
                json!({"status":"completed","output":[]}),
                json!({"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}]}),
            )
        } else {
            (
                json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"  "}}]}),
                json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"done"}}]}),
            )
        };
        let (url, server) = server_at(
            vec![
                (503, json!({"error":"private-upstream"})),
                (200, empty),
                (200, success_response),
            ],
            endpoint,
        );
        let home = tempfile::tempdir().unwrap();
        setup(home.path(), &url);
        success(run(
            home.path(),
            &["model", "update", "test", "--api-format", format],
        ));
        let result = success(run(home.path(), &["agent", "run", "a", "--input", "hello"]));
        assert_eq!(result["state"], "completed");
        assert_eq!(result["model_steps"], 1);
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests.windows(2).all(|pair| pair[0] == pair[1]));
        let calls = success(run(
            home.path(),
            &["run", "calls", result["id"].as_str().unwrap()],
        ));
        assert_eq!(calls[0]["argument_summary"]["request_attempts"], 3);
    }
}

#[test]
fn compaction_uses_normal_model_protocol_without_tool_or_output_replay() {
    for responses in [false, true] {
        let text = |s: &str| json!({"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":s}]});
        let args = json!({"agent":"default/b","input":"review task"}).to_string();
        let mut replies = if responses {
            vec![
                (
                    200,
                    json!({"status":"completed","output":[{"type":"reasoning","encrypted_content":"opaque-secret","summary":[]},{"type":"function_call","call_id":"review","name":"delegate","arguments":args}]}),
                ),
                (
                    200,
                    json!({"status":"completed","output":[text("review result")]}),
                ),
                (
                    200,
                    json!({"status":"completed","output":[text("Review completed; report result without rerunning tools.")]}),
                ),
                (
                    200,
                    json!({"status":"completed","output":[text("final answer")]}),
                ),
            ]
        } else {
            vec![
                completion(
                    json!({"role":"assistant","content":null,"reasoning_content":"opaque-secret","tool_calls":[{"id":"review","type":"function","function":{"name":"delegate","arguments":args}}]}),
                    "tool_calls",
                ),
                answer("review result"),
                answer("Review completed; report result without rerunning tools."),
                answer("final answer"),
            ]
        };
        // Small local payload, but reported usage exceeds the window budget.
        // This must trigger compaction without relying on a byte-size trigger.
        replies[0].1["usage"] = if responses {
            json!({"input_tokens":29000,"output_tokens":1000})
        } else {
            json!({"prompt_tokens":29000,"completion_tokens":1000,"total_tokens":30000})
        };
        let (url, server) = server_at(
            replies,
            if responses {
                "responses"
            } else {
                "chat/completions"
            },
        );
        let home = tempfile::tempdir().unwrap();
        setup(home.path(), &url);
        success(run(
            home.path(),
            &[
                "model",
                "update",
                "test",
                "--api-format",
                if responses {
                    "responses"
                } else {
                    "chat-completions"
                },
                "--clear-max-tokens",
                "--context-window",
                "32768",
            ],
        ));
        let result = success(run(
            home.path(),
            &["agent", "run", "a", "--input", "hello", "--delegate", "b"],
        ));
        assert_eq!(result["state"], "completed");
        assert_eq!(result["context"]["compactions"], 1);
        let requests = server.join().unwrap();
        let field = if responses { "input" } else { "messages" };
        assert!(
            requests
                .iter()
                .all(|r| !r.to_string().contains("_agt_total_tokens"))
        );
        let audit = success(run(
            home.path(),
            &["run", "calls", result["id"].as_str().unwrap()],
        ));
        let compact = audit
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["kind"] == "compaction")
            .unwrap();
        assert_eq!(
            compact["argument_summary"]["count_source"],
            "provider_usage_plus_local_estimate"
        );
        assert!(
            compact["argument_summary"]["tokens_before"]
                .as_u64()
                .unwrap()
                >= 30000
        );
        assert!(requests[2]["tools"].is_null());
        assert!(!requests[2].to_string().contains("opaque-secret"));
        assert_eq!(requests[3][field][1]["content"], "hello");
        assert_eq!(requests[3][field].as_array().unwrap().len(), 3);
        assert!(!requests[3].to_string().contains("function_call_output"));
        assert!(
            requests
                .iter()
                .all(|r| r["max_output_tokens"].is_null() && r["max_completion_tokens"].is_null())
        );
    }
}

#[test]
fn http_stream_preview_is_visible_before_completion_for_both_protocols() {
    for responses in [false, true] {
        let event = |v: Value| format!("data: {v}\n\n");
        let body = if responses {
            event(json!({"type":"response.output_text.delta","delta":"正在生成"}))
                + &event(
                    json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"正在生成完成"}]}]}}),
                )
        } else {
            event(json!({"choices":[{"delta":{"content":"正在生成"}}]}))
                + &event(json!({"choices":[{"delta":{"content":"完成"},"finish_reason":"stop"}]}))
                + "data: [DONE]\n\n"
        };
        let (url, server) = server_wire(
            vec![(200, "text/event-stream", body)],
            if responses {
                "responses"
            } else {
                "chat/completions"
            },
        );
        let home = tempfile::tempdir().unwrap();
        setup(home.path(), &url);
        if responses {
            success(run(
                home.path(),
                &["model", "update", "test", "--api-format", "responses"],
            ));
        }
        let task_home = home.path().to_owned();
        let job = thread::spawn(move || {
            success(run(&task_home, &["agent", "run", "a", "--input", "hello"]))
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let runs = success(run(home.path(), &["run", "list"]));
            if runs[0]["progress"]["preview"] == "正在生成" {
                assert_eq!(runs[0]["state"], "running");
                assert!(runs[0]["output"].is_null());
                break;
            }
            assert!(
                !job.is_finished() && Instant::now() < deadline,
                "no live preview before completion"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let result = job.join().unwrap();
        assert_eq!(result["output"], "正在生成完成");
        assert!(result["progress"].is_null());
        assert_eq!(server.join().unwrap()[0]["stream"], true);
    }
}

#[test]
fn cli_and_stdio_mcp_share_durable_task_queries_and_authorization() {
    use agt_mcp::{DefaultMcpClientFactory, McpClientFactory, McpDocument, McpLimits};
    let home = tempfile::tempdir().unwrap();
    setup(home.path(), "http://127.0.0.1:1/v1");
    success(run(
        home.path(),
        &["im", "principal", "create", "owner", "--name", "Owner"],
    ));
    let call = |name: &str, args: Value| {
        success(run(
            home.path(),
            &[
                "tool",
                "call",
                "--principal",
                "owner",
                "--manager",
                name,
                "--args",
                &args.to_string(),
            ],
        ))
    };
    call(
        "principal_create",
        json!({"id":"a","name":"A","kind":"agent"}),
    );
    call("agent_bind", json!({"principal":"a","agent":"a"}));
    let chat = call("direct_create", json!({"principal":"a"}))["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let sent = call(
        "message_send",
        json!({"conversation":chat,"body":"authorize task","request_key":"source"}),
    );
    let source = sent["message"]["id"].as_str().unwrap();
    let args = json!({"assignee":"a","conversation":chat,"input":"explicit task","request_key":"task-1","source_message":source});
    assert!(
        !run(
            home.path(),
            &[
                "tool",
                "call",
                "--principal",
                "owner",
                "--manager",
                "task_request",
                "--args",
                &args.to_string()
            ]
        )
        .status
        .success()
    );
    success(run(
        home.path(),
        &["schedule", "start", &chat, "--message", source],
    ));
    let item = call("task_request", args.clone());
    assert_eq!(item["state"], "queued");
    assert_eq!(call("task_request", args)["id"], item["id"]);
    let document = McpDocument::stdio(
        "tasks",
        env!("CARGO_BIN_EXE_agt"),
        vec![
            "--home".into(),
            home.path().to_string_lossy().into_owned(),
            "serve".into(),
            "--principal".into(),
            "a".into(),
        ],
        vec![],
    )
    .unwrap();
    let mut client = DefaultMcpClientFactory
        .connect(&document, home.path(), McpLimits::default())
        .unwrap();
    assert!(
        client
            .list_tools()
            .unwrap()
            .iter()
            .any(|t| t.name == "task_get")
    );
    let read = client
        .call_tool("task_get", json!({"task_id":item["id"]}))
        .unwrap();
    assert!(!read.is_error);
    let result: Value = serde_json::from_str(read.content[0].text.as_deref().unwrap()).unwrap();
    assert_eq!(result["id"], item["id"]);
    assert_eq!(result["request"]["requester"], "owner");
    assert!(client.call_tool("task_request",json!({"assignee":"owner","conversation":chat,"input":"untrusted","request_key":"denied"})).unwrap().is_error);
}
