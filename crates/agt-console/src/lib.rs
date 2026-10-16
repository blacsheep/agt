//! `agt web`: serve the Web console from this binary.
//!
//! The page (`web/public`) is compiled into the binary (see `build.rs`) and
//! served from memory. Every operation it requests runs as an `agt` CLI
//! command with JSON output, so the console has no write path of its own.

mod activity;
mod cli;
mod commands;
mod error;
mod http;
mod js;

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Args;
use serde_json::json;
use tokio::sync::watch;

use cli::{Backend, Cli, strings};

mod assets {
    include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));

    /// A page file and its media type; `/` is the console itself.
    pub(super) fn get(path: &str) -> Option<(&'static [u8], &'static str)> {
        let name = if path == "/" {
            "index.html"
        } else {
            path.strip_prefix('/')?
        };
        let kind = match name.rsplit_once('.')?.1 {
            "html" => "text/html",
            "js" => "text/javascript",
            "css" => "text/css",
            "svg" => "image/svg+xml",
            _ => return None,
        };
        FILES
            .iter()
            .find(|(file, _)| *file == name)
            .map(|(_, bytes)| (*bytes, kind))
    }
}

const DEFAULT_PORT: u16 = 4317;
/// The operator identity `agt web` and the desktop app use unless told otherwise.
pub const DEFAULT_PRINCIPAL: &str = "web-operator";

#[derive(Debug, Args)]
pub struct WebArgs {
    /// Port on 127.0.0.1 [default: 4317]
    #[arg(long)]
    port: Option<u16>,
    /// Your identity in conversations [default: web-operator]
    #[arg(long)]
    principal: Option<String>,
    /// Fixed access token instead of a random one (tests).
    #[arg(long, hide = true)]
    token: Option<String>,
    /// CLI executable for console operations instead of this one (tests).
    #[arg(long, hide = true)]
    binary: Option<PathBuf>,
    /// Skip preparing the operator, the workspace owner and the scheduler (tests).
    #[arg(long, hide = true)]
    no_setup: bool,
}

pub fn run(args: WebArgs, home: &Path) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("无法启动 Web 控制台运行时")?
        .block_on(serve(args, home))
}

// Terminal messages follow the system locale; the page has its own language switch.
fn say(zh: &str, en: &str) -> String {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()))
        .unwrap_or_default();
    let zh_terminal = locale
        .get(..2)
        .is_some_and(|p| p.eq_ignore_ascii_case("zh"));
    if zh_terminal { zh } else { en }.to_owned()
}

async fn serve(args: WebArgs, home: &Path) -> Result<()> {
    let principal = args
        .principal
        .unwrap_or_else(|| DEFAULT_PRINCIPAL.to_owned());
    if principal.trim().is_empty() || js::utf16_len(&principal) > 256 {
        bail!("标识符必须是非空字符串。");
    }
    let mut home = std::path::absolute(home).context("无法确定 agt home 路径")?;
    let binary = match args.binary {
        Some(binary) => std::path::absolute(binary)?,
        None => std::env::current_exe().context("无法确定 agt 可执行文件路径")?,
    };
    let cli = Cli::new(binary, home.clone());
    if !args.no_setup {
        home = prepare(&cli, &principal).await?;
    }
    let result = listen(
        &cli,
        home,
        principal,
        args.port.unwrap_or(DEFAULT_PORT),
        args.token,
    )
    .await;
    cli.close().await;
    result
}

/// Ensures a human operator identity that owns the workspace, and a scheduler
/// to run the agents' replies. Returns the canonical home for display.
async fn prepare(cli: &Arc<Cli>, principal: &str) -> Result<PathBuf> {
    let call = |args: Vec<String>| async move {
        cli.call(args, Vec::new())
            .await
            .map_err(|error| anyhow::anyhow!(error.message))
    };
    let principals = call(strings(&["im", "principal", "list"])).await?;
    let found = principals
        .as_array()
        .and_then(|list| list.iter().find(|p| js::is(p.get("id"), principal)));
    match found {
        Some(found) if !js::is(found.get("kind"), "user") => bail!(say(
            "操作身份已被非用户占用，请使用 --principal 指定新身份。",
            "That identity belongs to a non-user participant. Choose another with --principal.",
        )),
        Some(_) => {}
        None => {
            let name = say("本地操作员", "Local operator");
            call(strings(&[
                "im",
                "principal",
                "create",
                principal,
                "--name",
                &name,
                "--kind",
                "user",
            ]))
            .await?;
        }
    }
    let home = std::fs::canonicalize(cli.home()).context("无法确定 agt home 路径")?;
    // Earlier versions unpacked the console here for Node.js; it is unused now.
    let _ = std::fs::remove_dir_all(home.join(".web"));
    let owner = call(strings(&["im", "owner", "ensure", principal])).await?;
    if owner != json!(principal) {
        let owner = owner
            .as_str()
            .map_or_else(|| owner.to_string(), str::to_owned);
        bail!(say(
            &format!("此工作空间的负责人是 {owner}，请使用 --principal 指定该身份启动。"),
            &format!("The owner of this workspace is {owner}. Start with --principal {owner}."),
        ));
    }
    cli.start_scheduler()
        .await
        .map_err(|error| anyhow::anyhow!(error.message))?;
    Ok(home)
}

async fn listen(
    cli: &Arc<Cli>,
    home: PathBuf,
    principal: String,
    port: u16,
    token: Option<String>,
) -> Result<()> {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .await
        .with_context(|| format!("无法监听 127.0.0.1:{port}"))?;
    let port = listener.local_addr()?.port();
    let token = token.unwrap_or_else(|| {
        format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        )
    });
    let (stopping, shutdown) = watch::channel(false);
    let app = Arc::new(http::App {
        cli: cli.clone(),
        principal: principal.clone(),
        home: home.display().to_string(),
        token: token.clone(),
        port,
        streams: AtomicUsize::new(0),
        changing: Default::default(),
        shutdown: shutdown.clone(),
    });
    let router = axum::Router::new().fallback(http::handle).with_state(app);
    let url = format!("http://127.0.0.1:{port}/#{token}");
    let home = home.display();
    println!(
        "{}",
        say(
            &format!(
                "\nAGT Studio · 本地控制台\n资源目录：{home}\n操作身份：{principal}\n打开：{url}\n仅本机访问。此链接拥有主机管理权限，请勿分享。按 Ctrl+C 退出。\n"
            ),
            &format!(
                "\nAGT Studio · local console\nHome:     {home}\nIdentity: {principal}\nOpen:     {url}\nOnly this machine can connect. The link gives full control of this workspace; do not share it. Press Ctrl+C to stop.\n"
            ),
        )
    );
    let mut graceful = shutdown.clone();
    let server = axum::serve(listener, router).with_graceful_shutdown(async move {
        let _ = graceful.wait_for(|stop| *stop).await;
    });
    tokio::select! {
        result = server => result.context("Web 控制台已退出")?,
        // Event streams end on shutdown; other requests get a short grace period.
        _ = async {
            stop_signal().await;
            let _ = stopping.send(true);
            tokio::time::sleep(Duration::from_secs(5)).await;
        } => {}
    }
    Ok(())
}

/// The console without an HTTP listener, for a host that calls it in-process,
/// such as the desktop app. Requests reach the same routes as `agt web`.
pub struct Console {
    app: Arc<http::App>,
    cli: Arc<Cli>,
    stopping: watch::Sender<bool>,
}

/// How an embedding host starts the console.
pub struct ConsoleOptions {
    pub home: PathBuf,
    /// Your identity in conversations; it owns the workspace.
    pub principal: String,
    /// The program that runs agt CLI commands, and arguments it needs first.
    pub binary: PathBuf,
    pub binary_args: Vec<String>,
}

pub use axum::body::Body;
pub use axum::response::Response;

impl Console {
    /// Prepare the operator, the workspace owner and the scheduler, like `agt web`.
    pub async fn start(options: ConsoleOptions) -> Result<Self> {
        if options.principal.trim().is_empty() || js::utf16_len(&options.principal) > 256 {
            bail!("标识符必须是非空字符串。");
        }
        let home = std::path::absolute(&options.home).context("无法确定 agt home 路径")?;
        let cli = Cli::with_prefix(options.binary, options.binary_args, home);
        let home = prepare(&cli, &options.principal).await?;
        let (stopping, shutdown) = watch::channel(false);
        let app = Arc::new(http::App {
            cli: cli.clone(),
            principal: options.principal,
            home: home.display().to_string(),
            token: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
            port: 0,
            streams: AtomicUsize::new(0),
            changing: Default::default(),
            shutdown,
        });
        Ok(Self { app, cli, stopping })
    }

    /// Handle one request, as if the page had sent it to `agt web`. `path`
    /// includes any query string. Streaming routes return a streaming body.
    pub async fn request(&self, method: &str, path: &str, body: Vec<u8>) -> Response {
        let request = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("host", format!("127.0.0.1:{}", self.app.port))
            .header("x-agt-token", &self.app.token)
            .header("content-type", "application/json")
            .body(Body::from(body));
        match request {
            Ok(request) => http::handle(axum::extract::State(self.app.clone()), request).await,
            Err(_) => axum::http::Response::builder()
                .status(400)
                .body(Body::empty())
                .expect("valid response"),
        }
    }

    pub fn home(&self) -> &str {
        &self.app.home
    }

    pub fn principal(&self) -> &str {
        &self.app.principal
    }

    /// End event streams and stop the scheduler this console started.
    pub async fn close(&self) {
        let _ = self.stopping.send(true);
        self.cli.close().await;
    }
}

/// A page file and its media type, for a host that serves the page itself.
pub fn asset(path: &str) -> Option<(&'static [u8], &'static str)> {
    assets::get(path)
}

async fn stop_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::assets;

    #[test]
    fn serves_the_embedded_page_and_nothing_else() {
        let names: Vec<&str> = assets::FILES.iter().map(|(name, _)| *name).collect();
        for required in [
            "index.html",
            "app.js",
            "style.css",
            "i18n.js",
            "locales/en.js",
        ] {
            assert!(names.contains(&required), "missing {required}");
        }
        assert_eq!(assets::get("/").unwrap().1, "text/html");
        assert_eq!(assets::get("/app.js").unwrap().1, "text/javascript");
        assert_eq!(assets::get("/locales/en.js").unwrap().1, "text/javascript");
        for path in [
            "/missing.js",
            "/../Cargo.toml",
            "/public/app.js",
            "app.js",
            "/locales",
        ] {
            assert!(assets::get(path).is_none(), "{path}");
        }
    }
}
