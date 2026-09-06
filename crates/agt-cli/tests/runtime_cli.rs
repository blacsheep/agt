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
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/v1/", listener.local_addr().unwrap());
    let thread = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, response) in responses {
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
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(&mut stream);
            let mut first_line = String::new();
            reader.read_line(&mut first_line).unwrap();
            assert_eq!(first_line, "POST /v1/chat/completions HTTP/1.1\r\n");
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
            let mut body = vec![0; length.expect("content length")];
            reader.read_exact(&mut body).unwrap();
            requests.push(serde_json::from_slice(&body).unwrap());
            let body = response.to_string();
            write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
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
    assert_eq!(requests[0]["stream"], false);
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
fn http_failure_is_redacted_persisted_and_not_retried_implicitly() {
    let (url, server) = server(vec![(
        401,
        json!({"error":{"message":"mock-secret reflected upstream"}}),
    )]);
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
    assert_eq!(result["error"], "model HTTP status 401");
    let calls = success(run(
        home.path(),
        &["run", "calls", result["id"].as_str().unwrap()],
    ));
    assert_eq!(calls[0]["state"], "failed");
    assert_eq!(calls[0]["error"]["code"], "model_http_error");
    assert!(!calls.to_string().contains("mock-secret"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("mock-secret"));
    server.join().unwrap();
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
fn unsupported_api_format_fails_before_model_execution() {
    let home = tempfile::tempdir().unwrap();
    setup(home.path(), "http://127.0.0.1:1/v1");
    success(run(
        home.path(),
        &["model", "update", "test", "--api-format", "responses"],
    ));
    let output = run(home.path(), &["agent", "run", "a", "--input", "hello"]);
    assert!(!output.status.success());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["model_steps"], 0);
    assert!(
        result["error"]
            .as_str()
            .unwrap()
            .contains("chat-completions only")
    );
}

#[test]
fn mcp_manager_provisions_agents_and_group_then_worker_broadcasts_replies() {
    use agt_mcp::{DefaultMcpClientFactory, McpClientFactory, McpDocument, McpLimits};
    let (url, server) = server(vec![answer("A group reply"), answer("B group reply")]);
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
        json!({"kind":"model","reference":"test","create":true,"data":{"provider":"mock","model":"test-model"}}),
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
            json!({"principal":name,"agent":name,"working_directory":home.path(),"policy":"all","max_steps":6,"max_depth":1}),
        );
    }
    let group = call(
        "group_create",
        json!({"title":"acceptance","members":["a","b"]}),
    );
    let sent = call(
        "message_send",
        json!({"conversation":group["id"],"body":"Discuss the plan","request_key":"acceptance"}),
    );
    assert_eq!(sent["message"]["sender_id"], "owner");
    let outcomes = success(run(home.path(), &["worker"]));
    assert_eq!(
        outcomes
            .as_array()
            .unwrap()
            .iter()
            .filter(|o| o["state"] == "completed")
            .count(),
        2
    );
    server.join().unwrap();
    success(run(home.path(), &["worker"])); // no model server remains; must not infer again
    let history = call("message_history", json!({"conversation":group["id"]}));
    assert_eq!(history["messages"].as_array().unwrap().len(), 3);
    assert_eq!(history["messages"][1]["sender_id"], "a");
    assert_eq!(history["messages"][2]["sender_id"], "b");
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
