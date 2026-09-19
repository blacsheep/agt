use super::*;
use agt_service::ExecutionControl;
fn fixture() -> (tempfile::TempDir, LocalExecution) {
    let home = tempfile::tempdir().unwrap();
    let mut config = agt_config::RuntimeConfig::default();
    config.execution.enabled = true;
    config.execution.output_bytes = 1024;
    config.execution.timeout_seconds = Some(1);
    config.save(home.path()).unwrap();
    let tool = LocalExecution {
        home: home.path().into(),
        directory: home.path().into(),
        cancellation: CancellationToken::new(),
        task: None,
    };
    (home, tool)
}
#[test]
fn command_returns_bounded_output_and_exit_status() {
    let (_home, tool) = fixture();
    let result = tool
        .call(json!({"command":"printf hello; printf err >&2; exit 7"}))
        .unwrap();
    assert_eq!(result["exit_code"], 7);
    assert_eq!(result["success"], false);
    assert!(result["output"].as_str().unwrap().contains("hello"));
    let result = tool
        .call(json!({"command":"for ((i=0;i<2000;i++)); do printf x; done"}))
        .unwrap();
    assert_eq!(result["truncated"], true);
    assert_eq!(result["output"].as_str().unwrap().len(), 1024);
}
#[test]
fn timeout_cancel_and_global_disable_stop_execution() {
    let (home, tool) = fixture();
    assert!(
        tool.call(json!({"command":"sleep 5; touch should-not-exist"}))
            .unwrap_err()
            .to_string()
            .contains("timed out")
    );
    assert!(!home.path().join("should-not-exist").exists());
    tool.cancellation.cancel();
    assert!(tool.call(json!({"command":"touch canceled"})).is_err());
    assert!(!home.path().join("canceled").exists());
    let mut config = agt_config::RuntimeConfig::default();
    config.execution.enabled = false;
    config.save(home.path()).unwrap();
    assert!(
        tool.call(json!({"command":"true"}))
            .unwrap_err()
            .to_string()
            .contains("disabled")
    );
}
