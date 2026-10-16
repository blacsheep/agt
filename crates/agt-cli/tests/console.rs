//! The console called in-process, as the desktop app does: no listener, same routes.
use agt_console::{Console, ConsoleOptions};

#[test]
fn the_console_answers_in_process_without_a_listener() {
    let home = tempfile::tempdir().unwrap();
    let agt = env!("CARGO_BIN_EXE_agt");
    assert!(
        std::process::Command::new(agt)
            .arg("init")
            .arg(home.path())
            .status()
            .unwrap()
            .success()
    );
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let console = Console::start(ConsoleOptions {
                home: home.path().into(),
                principal: "desktop-operator".into(),
                binary: agt.into(),
                binary_args: Vec::new(),
            })
            .await
            .unwrap();
            let response = console.request("GET", "/api/bootstrap", Vec::new()).await;
            assert_eq!(response.status(), 200);
            let body = axum_body(response).await;
            assert_eq!(body["principal"], "desktop-operator", "{body}");
            let page = console.request("GET", "/", Vec::new()).await;
            assert_eq!(page.status(), 200);
            let missing = console.request("GET", "/api/nothing", Vec::new()).await;
            assert_eq!(missing.status(), 404);
            assert!(agt_console::asset("/app.js").is_some());
            console.close().await;
        });
}

async fn axum_body(response: agt_console::Response) -> serde_json::Value {
    use futures_util::StreamExt;
    let mut stream = response.into_body().into_data_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk.unwrap());
    }
    serde_json::from_slice(&bytes).unwrap()
}
