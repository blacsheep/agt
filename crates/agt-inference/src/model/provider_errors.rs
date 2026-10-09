use super::*;

#[test]
fn structured_http_json_errors_are_classified_without_echoing_provider_text() {
    use std::io::{Read, Write};
    for (status, body, expected) in [
        (
            400,
            json!({"error":{"code":"context_length_exceeded","message":"private-provider-input"}}),
            true,
        ),
        (413, json!({"error":{"message":"request too large"}}), false),
    ] {
        let home = tempfile::tempdir().unwrap();
        agt_config::initialize_home(Some(home.path().into())).unwrap();
        agt_prompt::PromptStore::new(home.path())
            .ensure_default_prompt()
            .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        agt_provider::ProviderStore::new(home.path())
            .create(
                &agt_provider::ProviderConfig::openai_compatible("fixture", &url, "unused")
                    .unwrap(),
            )
            .unwrap();
        agt_model::ModelStore::new(home.path())
            .create(
                &agt_model::ModelConfig::new(
                    "fixture",
                    agt_model::ModelReference::parse("fixture/model").unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
        agt_agent::AgentStore::new(home.path())
            .create(&agt_agent::AgentConfig::new("fixture", "fixture").unwrap())
            .unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = vec![];
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let headers = String::from_utf8(request).unwrap();
            let length: usize = headers
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|n| n.trim().parse().unwrap())
                })
                .unwrap();
            stream.read_exact(&mut vec![0; length]).unwrap();
            let body = body.to_string();
            write!(stream, "HTTP/1.1 {status} Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        let agent = ResolvedAgent::load(home.path(), "fixture").unwrap();
        let error = ChatCompletions
            .complete(&agent, &[json!({"role":"user","content":"hello"})], &[])
            .unwrap_err();
        assert_eq!(
            matches!(error, RuntimeError::ContextWindowRejected),
            expected
        );
        assert!(!error.to_string().contains("private-provider-input"));
        server.join().unwrap();
    }
}
