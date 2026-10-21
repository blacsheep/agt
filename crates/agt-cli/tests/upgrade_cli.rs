//! `agt upgrade` against a local stand-in for GitHub's release pages.
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;

use serde_json::Value;

/// Serves `releases/latest` as a redirect to `tag`, and an installer script
/// that writes a stand-in `agt` reporting that version into the install
/// directory, plus the environment it was given.
fn releases(tag: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/releases", listener.local_addr().unwrap());
    let location = format!("{base}/tag/{tag}");
    let installer = format!("/releases/download/{tag}/agt-cli-installer.sh");
    let version = tag.trim_start_matches('v').to_owned();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut line = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            reader.read_line(&mut line).unwrap();
            while reader.read_line(&mut String::new()).unwrap_or(0) > 2 {}
            let path = line.split_whitespace().nth(1).unwrap_or_default();
            let response = if path == "/releases/latest" {
                format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n")
            } else if path == installer {
                let script = format!(
                    // Like the real installer: write aside, then rename over the
                    // running binary, which cannot be written in place.
                    "set -e\n\
                     printf '#!/bin/sh\\necho \"agt {version}\"\\n' > \"$AGT_CLI_INSTALL_DIR/agt.new\"\n\
                     chmod +x \"$AGT_CLI_INSTALL_DIR/agt.new\"\n\
                     mv \"$AGT_CLI_INSTALL_DIR/agt.new\" \"$AGT_CLI_INSTALL_DIR/agt\"\n\
                     echo \"$AGT_CLI_NO_MODIFY_PATH\" > \"$AGT_CLI_INSTALL_DIR/no-modify-path\"\n"
                );
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{script}",
                    script.len()
                )
            } else {
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_owned()
            };
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("{base}/latest")
}

/// A copy of agt in `<dir>/bin`, with the installer's receipt when `receipt`.
fn installed(label: &str, receipt: bool) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("agt-upgrade-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let agt = bin.join(format!("agt{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(env!("CARGO_BIN_EXE_agt"), &agt).unwrap();
    if receipt {
        let config = dir.join("config").join("agt-cli");
        std::fs::create_dir_all(&config).unwrap();
        let receipt = serde_json::json!({"install_prefix": bin, "version": "0.0.1"});
        std::fs::write(config.join("agt-cli-receipt.json"), receipt.to_string()).unwrap();
    }
    (dir, agt)
}

fn run(dir: &Path, agt: &Path, url: &str, args: &[&str]) -> Output {
    Command::new(agt)
        .env("AGT_UPDATE_URL", url)
        .env("AGT_UPDATE_CACHE", dir.join("update-check.json"))
        // Where the installer keeps its receipt on Unix and on Windows.
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("LOCALAPPDATA", dir.join("config"))
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env("no_proxy", "127.0.0.1,localhost")
        .env("LANG", "C")
        .args(args)
        .output()
        .unwrap()
}

fn json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn check_reports_a_newer_release_without_installing() {
    let url = releases("v99.0.0");
    let (dir, agt) = installed("check", true);
    let report = json(&run(
        &dir,
        &agt,
        &url,
        &["upgrade", "--check", "--output", "json"],
    ));
    assert_eq!(report["current"], env!("CARGO_PKG_VERSION"));
    assert_eq!(report["latest"], "99.0.0");
    assert_eq!(report["update_available"], true);
    assert!(
        report["url"]
            .as_str()
            .unwrap()
            .ends_with("/releases/tag/v99.0.0")
    );
    // Nothing was installed, and the answer is cached for the notice.
    assert!(!dir.join("bin").join("no-modify-path").exists());
    let cache: Value =
        serde_json::from_slice(&std::fs::read(dir.join("update-check.json")).unwrap()).unwrap();
    assert_eq!(cache["latest"]["version"], "99.0.0");
}

#[test]
fn the_latest_release_needs_no_upgrade() {
    let url = releases(concat!("v", env!("CARGO_PKG_VERSION")));
    let (dir, agt) = installed("latest", true);
    let report = json(&run(
        &dir,
        &agt,
        &url,
        &["upgrade", "--yes", "--output", "json"],
    ));
    assert_eq!(report["update_available"], false);
}

#[test]
fn without_a_receipt_agt_is_left_alone() {
    let url = releases("v99.0.0");
    let (dir, agt) = installed("manual", false);
    let output = run(&dir, &agt, &url, &["upgrade", "--yes"]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("没有安装记录"), "{error}");
    assert!(error.contains("cargo install"), "{error}");
}

#[test]
fn a_receipt_for_another_directory_is_not_used() {
    let url = releases("v99.0.0");
    let (dir, _) = installed("elsewhere", true);
    let other = dir.join("other");
    std::fs::create_dir_all(&other).unwrap();
    let agt = other.join(format!("agt{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(env!("CARGO_BIN_EXE_agt"), &agt).unwrap();
    let output = run(&dir, &agt, &url, &["upgrade", "--yes"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("但正在运行的是"));
}

#[test]
fn upgrading_needs_a_confirmation() {
    let url = releases("v99.0.0");
    let (dir, agt) = installed("confirm", true);
    let output = run(&dir, &agt, &url, &["upgrade"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--yes"));
}

#[cfg(unix)]
#[test]
fn upgrade_runs_the_new_installer_into_the_same_directory() {
    let url = releases("v99.0.0");
    let (dir, agt) = installed("install", true);
    let report = json(&run(
        &dir,
        &agt,
        &url,
        &["upgrade", "--yes", "--output", "json"],
    ));
    assert_eq!(report["upgraded_from"], env!("CARGO_PKG_VERSION"));
    assert_eq!(report["upgraded_to"], "99.0.0");
    let version = Command::new(&agt).arg("--version").output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        "agt 99.0.0"
    );
    let no_modify_path = std::fs::read_to_string(dir.join("bin").join("no-modify-path")).unwrap();
    assert_eq!(no_modify_path.trim(), "1");
}
