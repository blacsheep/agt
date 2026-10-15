use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Python launcher of the MCP test fixtures; Windows installs it as `python`.
const PYTHON: &str = if cfg!(windows) { "python" } else { "python3" };

fn temp_home(label: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("agt-resource-cli-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    path
}

fn run(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_agt"))
        .args(["--home", home.to_str().expect("utf8")])
        .args(args)
        .output()
        .expect("run agt")
}

fn init(path: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_agt"))
        .args(["init", path.to_str().expect("utf8")])
        .output()
        .expect("run agt init")
}

#[test]
fn manages_definitions_and_masks_provider_secret() {
    let home = temp_home("crud");
    assert!(init(&home).status.success());
    assert!(
        run(
            &home,
            &[
                "provider",
                "create",
                "openai",
                "--base-url",
                "http://localhost/v1",
                "--api-key",
                "secret-value",
            ],
        )
        .status
        .success()
    );
    assert!(
        run(
            &home,
            &[
                "model",
                "create",
                "gpt",
                "--provider",
                "openai",
                "--model",
                "gpt-4o",
            ],
        )
        .status
        .success()
    );
    let output = run(&home, &["provider", "info", "openai", "--output", "json"]);
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("api_key_configured"));
    assert!(!text.contains("secret-value"));

    let _ = fs::remove_dir_all(home);
}

#[test]
fn recursive_delete_removes_dependents_but_keeps_prompt() {
    let home = temp_home("recursive");
    assert!(init(&home).status.success());
    assert!(
        run(
            &home,
            &[
                "provider",
                "create",
                "openai",
                "--base-url",
                "http://localhost/v1",
                "--api-key",
                "secret-value",
            ],
        )
        .status
        .success()
    );
    assert!(
        run(
            &home,
            &[
                "model",
                "create",
                "gpt",
                "--provider",
                "openai",
                "--model",
                "gpt-4o",
            ],
        )
        .status
        .success()
    );
    assert!(
        run(&home, &["agent", "create", "coder", "--model", "gpt"])
            .status
            .success()
    );

    let output = run(
        &home,
        &[
            "provider",
            "delete",
            "openai",
            "--recursive",
            "--yes",
            "--output",
            "json",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!home.join("providers/default/openai").exists());
    assert!(!home.join("models/default/gpt").exists());
    assert!(!home.join("agents/default/coder").exists());
    assert!(home.join("prompts/default/default").exists());

    let _ = fs::remove_dir_all(home);
}

#[test]
fn deleting_scoped_skill_cleans_empty_scope_directory() {
    let home = temp_home("scoped-skill");
    assert!(init(&home).status.success());
    assert!(
        run(
            &home,
            &[
                "skill",
                "create",
                "@acme/code-review",
                "--description",
                "Review source code",
            ],
        )
        .status
        .success()
    );
    let scope_dir = home.join("skills/default/@acme");
    assert!(scope_dir.is_dir());

    let output = run(&home, &["skill", "delete", "@acme/code-review", "--yes"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!scope_dir.exists());

    let _ = fs::remove_dir_all(home);
}

#[test]
fn agent_update_is_atomic_when_a_later_reference_is_invalid() {
    let home = temp_home("agent-update-atomic");
    assert!(init(&home).status.success());
    assert!(
        run(
            &home,
            &[
                "provider",
                "create",
                "openai",
                "--base-url",
                "http://localhost/v1",
                "--api-key",
                "secret-value",
            ],
        )
        .status
        .success()
    );
    assert!(
        run(
            &home,
            &[
                "model",
                "create",
                "fast",
                "--provider",
                "openai",
                "--model",
                "gpt-4o",
            ],
        )
        .status
        .success()
    );
    assert!(
        run(&home, &["agent", "create", "coder", "--model", "fast"])
            .status
            .success()
    );

    let output = run(
        &home,
        &[
            "agent",
            "update",
            "coder",
            "--description",
            "should-not-persist",
            "--model",
            "missing",
        ],
    );
    assert!(!output.status.success());
    let info = run(&home, &["agent", "info", "coder", "--output", "json"]);
    assert!(info.status.success());
    let text = String::from_utf8_lossy(&info.stdout);
    assert!(text.contains("\"description\": \"\""));

    let _ = fs::remove_dir_all(home);
}

#[test]
fn init_requires_an_explicit_directory() {
    let output = Command::new(env!("CARGO_BIN_EXE_agt"))
        .arg("init")
        .output()
        .expect("run agt init without a directory");
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("required") || error.contains("需要"),
        "{error}"
    );
}

#[test]
fn default_home_is_prepared_without_explicit_init() {
    let parent = temp_home("default-home-parent");
    let default_home = parent.join(".agt");
    let output = Command::new(env!("CARGO_BIN_EXE_agt"))
        .env("HOME", &parent)
        .args(["provider", "list"])
        .output()
        .expect("run agt with default home");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(default_home.join("config.yaml").is_file());
    assert!(default_home.join("providers/default").is_dir());
    assert!(
        default_home
            .join("prompts/default/default/PROMPT.md")
            .is_file()
    );

    let _ = fs::remove_dir_all(parent);
}

#[test]
fn builtin_resource_mounts_are_configurable_from_cli() {
    let home = temp_home("builtin");
    assert!(init(&home).status.success());
    for args in [
        vec![
            "provider",
            "create",
            "p",
            "--base-url",
            "http://localhost/v1",
            "--api-key",
            "fixture",
        ],
        vec![
            "model",
            "create",
            "m",
            "--provider",
            "p",
            "--model",
            "fixture",
        ],
        vec![
            "agent",
            "create",
            "a",
            "--model",
            "m",
            "--builtin-tool",
            "resource_management",
        ],
    ] {
        assert!(run(&home, &args).status.success());
    }
    let info = || {
        serde_json::from_slice::<serde_json::Value>(
            &run(&home, &["--output", "json", "agent", "info", "a"]).stdout,
        )
        .unwrap()
    };
    assert_eq!(
        info()["builtin_tools"],
        serde_json::json!(["resource_management"])
    );
    assert!(
        run(
            &home,
            &[
                "agent",
                "update",
                "a",
                "--remove-builtin-tool",
                "resource_management"
            ]
        )
        .status
        .success()
    );
    assert_eq!(info()["builtin_tools"], serde_json::json!([]));
    assert!(
        run(
            &home,
            &[
                "agent",
                "update",
                "a",
                "--add-builtin-tool",
                "resource_management"
            ]
        )
        .status
        .success()
    );
    assert!(
        !run(
            &home,
            &[
                "agent",
                "update",
                "a",
                "--description",
                "must-not-save",
                "--add-builtin-tool",
                "unknown"
            ]
        )
        .status
        .success()
    );
    assert_eq!(info()["description"], "");
    assert!(
        !run(
            &home,
            &[
                "agent",
                "create",
                "duplicate",
                "--model",
                "m",
                "--builtin-tool",
                "resource_management",
                "--builtin-tool",
                "resource_management"
            ]
        )
        .status
        .success()
    );
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn mcp_probe_is_explicit_and_checks_tool_schemas() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    assert!(init(&home).status.success());
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../agt-runtime/tests/fixtures/contract_mcp.py");
    let log = temp.path().join("calls");
    let file = temp.path().join("mcp.json");
    let config = serde_json::json!({"mcpServers":{"counter":{"cwd":std::env::temp_dir(),"command":PYTHON,"args":[fixture,"valid",log]}}});
    fs::write(&file, config.to_string()).unwrap();
    assert!(
        run(
            &home,
            &["mcp", "create", "counter", "--file", file.to_str().unwrap()]
        )
        .status
        .success()
    );
    assert!(run(&home, &["mcp", "test", "counter"]).status.success());
    assert!(!temp.path().join("calls.starts").exists());
    let output = run(&home, &["--output", "json", "mcp", "probe", "counter"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schemas_valid"], true);
    assert!(temp.path().join("calls.starts").exists());
    assert!(!log.exists());
}

#[test]
fn tool_failure_cause_goes_to_stderr_only() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    assert!(init(&home).status.success());
    assert!(
        run(
            &home,
            &["im", "principal", "create", "owner", "--name", "Owner"]
        )
        .status
        .success()
    );
    let output = run(
        &home,
        &[
            "--output",
            "json",
            "tool",
            "call",
            "--principal",
            "owner",
            "--manager",
            "resource_read",
            "--args",
            r#"{"kind":"prompt","reference":"missing"}"#,
        ],
    );
    assert!(!output.status.success());
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(stdout["error"]["code"].is_string());
    assert!(stdout["error"].get("diagnostic").is_none());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let line = stderr
        .lines()
        .find(|l| l.starts_with("diagnostic: "))
        .expect("diagnostic on stderr");
    assert!(!String::from_utf8_lossy(&output.stdout).contains(&line["diagnostic: ".len()..]));
}
