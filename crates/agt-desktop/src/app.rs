//! The window, the in-process console it talks to, the tray and updates.
use agt_console::{Console, ConsoleOptions};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(target_os = "macos")]
use tauri::RunEvent;
use tauri::ipc::Channel;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::UpdaterExt;
use tokio::sync::OnceCell;
use tokio::task::JoinHandle;

/// State shared by commands and background tasks.
pub(crate) struct Shared {
    console: OnceCell<Arc<Console>>,
    streams: Mutex<HashMap<u64, JoinHandle<()>>>,
    next: AtomicU64,
}

impl Shared {
    /// The console, started on first use: the default home, the same operator
    /// identity as `agt web`, and this executable as the CLI.
    pub(crate) async fn console(&self) -> Result<Arc<Console>, String> {
        self.console
            .get_or_try_init(|| async {
                let home = agt_cli::ensure_default_home_ready().map_err(|e| format!("{e:#}"))?;
                let binary = std::env::current_exe().map_err(|e| e.to_string())?;
                Console::start(ConsoleOptions {
                    home,
                    principal: agt_console::DEFAULT_PRINCIPAL.into(),
                    binary,
                    binary_args: vec![crate::CLI_FLAG.into()],
                })
                .await
                .map(Arc::new)
                .map_err(|e| format!("{e:#}"))
            })
            .await
            .cloned()
    }
}

/// One console API request from the page. The status comes back at once; the
/// body follows on `channel` as `{chunk}` messages and ends with `{end}` or
/// `{error}`, so event streams work the same way as ordinary replies.
#[tauri::command]
async fn agt_request(
    state: tauri::State<'_, Shared>,
    method: String,
    path: String,
    body: String,
    channel: Channel<Value>,
) -> Result<Value, String> {
    if !path.starts_with("/api/") {
        return Err("only console API paths are accepted".into());
    }
    let console = state.console().await?;
    let response = console.request(&method, &path, body.into_bytes()).await;
    let status = response.status().as_u16();
    let id = state.next.fetch_add(1, Ordering::Relaxed);
    let task = tokio::spawn(pump(response, channel));
    let mut streams = state.streams.lock().expect("streams lock");
    streams.retain(|_, task| !task.is_finished());
    streams.insert(id, task);
    Ok(json!({"id": id, "status": status}))
}

/// Stop delivering a response body, such as an event stream the page closed.
#[tauri::command]
fn agt_cancel(state: tauri::State<'_, Shared>, id: u64) {
    if let Some(task) = state.streams.lock().expect("streams lock").remove(&id) {
        task.abort();
    }
}

/// Forward a body as text, never splitting a UTF-8 character across chunks.
async fn pump(response: agt_console::Response, channel: Channel<Value>) {
    let mut stream = response.into_body().into_data_stream();
    let mut pending = Vec::new();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => {
                pending.extend_from_slice(&bytes);
                let valid = match std::str::from_utf8(&pending) {
                    Ok(_) => pending.len(),
                    Err(error) => error.valid_up_to(),
                };
                if valid > 0 {
                    let text = String::from_utf8(pending.drain(..valid).collect())
                        .expect("validated UTF-8");
                    if channel.send(json!({"chunk": text})).is_err() {
                        return;
                    }
                }
            }
            Err(error) => {
                let _ = channel.send(json!({"error": error.to_string()}));
                return;
            }
        }
    }
    let _ = channel.send(json!({"end": true}));
}

pub(crate) fn run() -> ExitCode {
    let built = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Shared {
            console: OnceCell::new(),
            streams: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
        })
        .invoke_handler(tauri::generate_handler![agt_request, agt_cancel])
        .setup(|app| {
            tray(app.handle())?;
            tauri::async_runtime::spawn(crate::notify::watch(app.handle().clone()));
            tauri::async_runtime::spawn(update(app.handle().clone()));
            Ok(())
        })
        // Closing the window keeps the app, and so the scheduler, in the tray.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!());
    match built {
        Ok(app) => {
            app.run(|_app, _event| {
                // macOS: clicking the Dock icon brings the hidden window back.
                #[cfg(target_os = "macos")]
                if let RunEvent::Reopen { .. } = _event {
                    show(_app);
                }
            });
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("AGT Studio could not start: {error}");
            ExitCode::from(1)
        }
    }
}

fn tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(
        app,
        "show",
        crate::say("打开 AGT Studio", "Open AGT Studio"),
        true,
        None::<&str>,
    )?;
    let exit = MenuItem::with_id(app, "quit", crate::say("退出", "Quit"), true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &exit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("AGT Studio")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show(app),
            "quit" => quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

pub(crate) fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Stop the scheduler this app started, then exit.
fn quit(app: &AppHandle) {
    if let Some(console) = app.state::<Shared>().console.get().cloned() {
        tauri::async_runtime::block_on(console.close());
    }
    app.exit(0);
}

/// Install an update when the build was configured with an updater key.
async fn update(app: AppHandle) {
    let configured = app
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u["pubkey"].as_str())
        .is_some_and(|key| !key.trim().is_empty());
    if !configured {
        return;
    }
    let Ok(updater) = app.updater() else {
        return;
    };
    let Ok(Some(update)) = updater.check().await else {
        return;
    };
    let version = update.version.clone();
    if update.download_and_install(|_, _| {}, || {}).await.is_ok() {
        let _ = app
            .notification()
            .builder()
            .title("AGT Studio")
            .body(crate::say(
                &format!("已更新到 {version}，重新打开后生效。"),
                &format!("Updated to {version}; it takes effect the next time you open the app."),
            ))
            .show();
    }
}
