//! `agt upgrade`: replace this `agt` with the latest release, after asking.
//!
//! The installer script that put `agt` here left a receipt with the install
//! directory. Upgrading runs the new release's installer script into that same
//! directory, so it downloads and checks the binary exactly as a first install
//! does. An `agt` built from source or installed with Cargo has no receipt and
//! is left alone.

use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::Command;

use agt_console::update::{self, CURRENT, Release, say};
use anyhow::{Context, Result, bail};
use clap::Args;
use serde::Deserialize;
use serde_json::json;

use crate::{OutputMode, emit_value};

#[derive(Debug, Args)]
pub(crate) struct UpgradeArgs {
    /// only report whether a newer release exists
    #[arg(long)]
    check: bool,
}

#[cfg(windows)]
const INSTALLER: &str = "agt-cli-installer.ps1";
#[cfg(not(windows))]
const INSTALLER: &str = "agt-cli-installer.sh";

pub(crate) fn run(args: UpgradeArgs, output: OutputMode, yes: bool) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let release = runtime.block_on(update::latest())?;
    let available = release.is_newer();
    if args.check || !available {
        if !available && !args.check {
            eprintln!(
                "{}",
                say(
                    &format!("已是最新版本 {CURRENT}"),
                    &format!("agt {CURRENT} is the latest version")
                )
            );
        }
        return emit_value(
            json!({
                "current": CURRENT,
                "latest": release.version,
                "update_available": available,
                "url": release.url,
            }),
            output,
        );
    }

    let install = Install::find()?;
    if !yes {
        if !io::stdin().is_terminal() {
            bail!("升级需要交互确认，请传入 --yes");
        }
        eprint!(
            "{}",
            say(
                &format!(
                    "把 {} 从 {CURRENT} 升级到 {}？[y/N] ",
                    install.binary.display(),
                    release.version
                ),
                &format!(
                    "Upgrade {} from {CURRENT} to {}? [y/N] ",
                    install.binary.display(),
                    release.version
                ),
            )
        );
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            bail!("已取消升级");
        }
    }

    let script = runtime.block_on(download(&release))?;
    install.run(&script, output)?;
    let installed = installed_version(&install.binary)?;
    if installed != release.version {
        bail!(
            "安装脚本已运行，但 {} 报告的版本是 {installed}，不是 {}",
            install.binary.display(),
            release.version
        );
    }
    eprintln!(
        "{}",
        say(
            &format!("已升级到 {installed}。正在运行的 agt web 要重启才会用上新版本。"),
            &format!(
                "Upgraded to {installed}. Restart agt web if it is running to use the new version."
            ),
        )
    );
    emit_value(
        json!({ "upgraded_from": CURRENT, "upgraded_to": installed, "binary": install.binary }),
        output,
    )
}

/// The installer receipt, as the installer script writes it.
#[derive(Deserialize)]
struct Receipt {
    install_prefix: PathBuf,
}

/// Where the installer put this `agt`.
struct Install {
    prefix: PathBuf,
    binary: PathBuf,
}

impl Install {
    /// The receipt's directory, if it holds the `agt` that is running.
    fn find() -> Result<Self> {
        let current = std::env::current_exe()
            .and_then(|path| path.canonicalize())
            .context("无法确定当前 agt 的路径")?;
        let manual = say(
            "请用原来的方式升级：用 Cargo 安装的重新运行 cargo install；从源码构建的拉取最新代码后重新构建；也可以重新运行安装脚本，见 https://blacsheep.github.io/agt/guide/installation",
            "Upgrade it the way you installed it: run cargo install again, or pull and rebuild a clone, or run the install script again; see https://blacsheep.github.io/agt/guide/installation",
        );
        let path = receipt_path();
        let receipt: Receipt = match path.as_deref().map(std::fs::read) {
            Some(Ok(bytes)) => serde_json::from_slice(&bytes).with_context(|| {
                format!("安装记录无法解析：{}", path.as_deref().unwrap().display())
            })?,
            _ => bail!(
                "{} 不是用安装脚本安装的，没有安装记录。{manual}",
                current.display()
            ),
        };
        let prefix = receipt
            .install_prefix
            .canonicalize()
            .unwrap_or(receipt.install_prefix);
        if current.parent() != Some(prefix.as_path()) {
            bail!(
                "安装脚本把 agt 装在 {}，但正在运行的是 {}。{manual}",
                prefix.display(),
                current.display()
            );
        }
        Ok(Self {
            prefix,
            binary: current,
        })
    }

    /// Run the release's installer into the same directory. It leaves `PATH`
    /// alone, since the first install already set it up.
    fn run(&self, script: &Path, output: OutputMode) -> Result<()> {
        #[cfg(windows)]
        let (mut command, aside) = {
            // A running .exe cannot be replaced, but it can be renamed.
            let aside = self.binary.with_extension("old.exe");
            let _ = std::fs::remove_file(&aside);
            std::fs::rename(&self.binary, &aside).context("无法让出正在运行的 agt.exe")?;
            let mut command = Command::new("powershell");
            command
                .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
                .arg(script);
            (command, aside)
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut command = Command::new("sh");
            command.arg(script);
            command
        };
        command
            .env("AGT_CLI_INSTALL_DIR", &self.prefix)
            .env("AGT_CLI_NO_MODIFY_PATH", "1");
        if output != OutputMode::Human {
            // Keep stdout for the JSON result.
            command
                .env("AGT_CLI_PRINT_QUIET", "1")
                .stdout(std::process::Stdio::null());
        }
        let status = command.status().context("无法运行安装脚本")?;
        if !status.success() {
            #[cfg(windows)]
            if !self.binary.exists() {
                let _ = std::fs::rename(&aside, &self.binary);
            }
            bail!("安装脚本失败（{status}），agt 没有改动");
        }
        Ok(())
    }
}

/// Where the installer script keeps its receipt.
fn receipt_path() -> Option<PathBuf> {
    #[cfg(windows)]
    let dir = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    #[cfg(not(windows))]
    let dir = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => dirs::home_dir()?.join(".config"),
    };
    Some(dir.join("agt-cli").join("agt-cli-receipt.json"))
}

/// Download the release's installer script to a temporary file.
async fn download(release: &Release) -> Result<tempfile::TempPath> {
    let url = release.download_url(INSTALLER);
    let response = reqwest::get(&url)
        .await
        .and_then(reqwest::Response::error_for_status)
        .with_context(|| format!("无法下载安装脚本 {url}"))?;
    let script = response.bytes().await?;
    let file = tempfile::Builder::new()
        .prefix("agt-installer-")
        .suffix(if cfg!(windows) { ".ps1" } else { ".sh" })
        .tempfile()?;
    std::fs::write(file.path(), &script)?;
    Ok(file.into_temp_path())
}

/// The version the binary at `path` reports.
fn installed_version(path: &Path) -> Result<String> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .with_context(|| format!("无法运行 {}", path.display()))?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .trim()
        .rsplit(' ')
        .next()
        .unwrap_or_default()
        .to_owned())
}
