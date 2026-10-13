//! `agt web`: run the embedded Web console with Node.js.
//!
//! The console sources are compiled into the binary (see `build.rs`) and written
//! to `<home>/.web/<hash>/` on first use, so an installed `agt` needs no checkout.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Args;

mod assets {
    include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));
}

const MIN_NODE_MAJOR: u32 = 22;

#[derive(Debug, Args)]
pub(crate) struct WebArgs {
    /// Port on 127.0.0.1 [default: 4317]
    #[arg(long)]
    port: Option<u16>,
    /// Your identity in conversations [default: web-operator]
    #[arg(long)]
    principal: Option<String>,
    /// Node.js executable [default: node on PATH]
    #[arg(long, value_name = "PATH", default_value = "node")]
    node: PathBuf,
}

pub(crate) fn run(args: WebArgs, home: &Path) -> Result<()> {
    check_node(&args.node)?;
    let dir = extract(home)?;
    let binary = std::env::current_exe().context("无法确定 agt 可执行文件路径")?;

    let mut command = Command::new(&args.node);
    command
        .arg(dir.join("server.mjs"))
        .arg("--home")
        .arg(home)
        .arg("--binary")
        .arg(binary);
    if let Some(port) = args.port {
        command.arg("--port").arg(port.to_string());
    }
    if let Some(principal) = &args.principal {
        command.arg("--principal").arg(principal);
    }
    let mut child = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("无法启动 Node.js：{}", args.node.display()))?;

    // Ctrl-C reaches Node directly through the terminal; wait for it to shut down
    // and forward termination requests aimed at agt alone.
    let stop = crate::signals::cancellation();
    let mut forwarded = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if stop.is_canceled() && !forwarded {
            forwarded = true;
            terminate(&mut child);
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    if !status.success() && !forwarded {
        bail!("Web 控制台已退出：{status}");
    }
    Ok(())
}

fn check_node(node: &Path) -> Result<()> {
    let output = Command::new(node).arg("--version").output().with_context(|| {
        format!(
            "找不到 Node.js（{}）。Web 控制台需要 Node.js {MIN_NODE_MAJOR} 或更新版本：https://nodejs.org/",
            node.display()
        )
    })?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let major = version
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|major| major.parse::<u32>().ok());
    match major {
        Some(major) if major >= MIN_NODE_MAJOR => Ok(()),
        _ => bail!(
            "Web 控制台需要 Node.js {MIN_NODE_MAJOR} 或更新版本，当前为 {}：https://nodejs.org/",
            if version.is_empty() {
                "未知"
            } else {
                &version
            }
        ),
    }
}

/// Write the embedded console to `<home>/.web/<hash>/` unless it is already there,
/// and remove copies left by other versions.
fn extract(home: &Path) -> Result<PathBuf> {
    let root = home.join(".web");
    let dir = root.join(assets::HASH);
    if !dir.join("server.mjs").is_file() {
        fs::create_dir_all(&root).with_context(|| format!("无法创建 {}", root.display()))?;
        let staging = root.join(format!(".{}-{}", assets::HASH, std::process::id()));
        let _ = fs::remove_dir_all(&staging);
        for (name, bytes) in assets::FILES {
            let path = staging.join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, bytes).with_context(|| format!("无法写入 {}", path.display()))?;
        }
        if let Err(error) = fs::rename(&staging, &dir) {
            // Another `agt web` may have extracted the same version concurrently.
            let _ = fs::remove_dir_all(&staging);
            if !dir.join("server.mjs").is_file() {
                return Err(error).with_context(|| format!("无法写入 {}", dir.display()));
            }
        }
    }
    if let Ok(entries) = fs::read_dir(&root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name != assets::HASH && !name.to_string_lossy().starts_with('.') {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
    Ok(dir)
}

#[cfg(unix)]
fn terminate(child: &mut std::process::Child) {
    if let Ok(pid) = i32::try_from(child.id()) {
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
    }
}

#[cfg(not(unix))]
fn terminate(child: &mut std::process::Child) {
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_the_console_runtime_files() {
        let names: Vec<&str> = assets::FILES.iter().map(|(name, _)| *name).collect();
        for required in [
            "server.mjs",
            "package.json",
            "public/index.html",
            "public/app.js",
        ] {
            assert!(names.contains(&required), "missing {required}");
        }
        assert!(names.iter().all(|name| !name.starts_with("test/")));
    }

    #[test]
    fn extracts_once_and_removes_other_versions() {
        let home = tempfile::tempdir().unwrap();
        let stale = home.path().join(".web/0000000000000000");
        fs::create_dir_all(&stale).unwrap();
        let dir = extract(home.path()).unwrap();
        assert_eq!(dir, home.path().join(".web").join(assets::HASH));
        assert!(dir.join("public/index.html").is_file());
        assert!(!stale.exists());
        // A second call reuses the extracted files.
        assert_eq!(extract(home.path()).unwrap(), dir);
    }
}
