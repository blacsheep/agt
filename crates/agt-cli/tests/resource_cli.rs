use std::fs;
use std::path::PathBuf;
use std::process::Command;

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
