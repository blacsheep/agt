//! The window, the in-process console it talks to, the tray and updates.
use agt_console::{Console, ConsoleOptions};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::Emitter;
#[cfg(target_os = "macos")]
use tauri::RunEvent;
use tauri::ipc::Channel;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent, Wry};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::OnceCell;
use tokio::task::JoinHandle;

/// State shared by commands and background tasks.
pub(crate) struct Shared {
    console: OnceCell<Arc<Console>>,
    streams: Mutex<HashMap<u64, JoinHandle<()>>>,
    next: AtomicU64,
    /// A newer version found by the last check, installed only when the person
    /// confirms.
    update: Mutex<Option<Update>>,
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
            update: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            agt_request,
            agt_cancel,
            update_status,
            update_install
        ])
        .setup(|app| {
            tray(app.handle())?;
            tauri::async_runtime::spawn(crate::notify::watch(app.handle().clone()));
            tauri::async_runtime::spawn(watch_updates(app.handle().clone()));
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
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("AGT Studio")
        .menu(&tray_menu(app, None)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show(app),
            "check" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { check(&app, true).await });
            }
            "update" => {
                show(app);
                let _ = app.emit("agt-update-open", ());
            }
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
    // The macOS menu bar takes a monochrome template image that the system tints for
    // light and dark bars; the full-color app icon looks oversized there.
    #[cfg(target_os = "macos")]
    {
        tray = tray
            .icon(tauri::image::Image::from_bytes(include_bytes!(
                "../icons/tray-macos.png"
            ))?)
            .icon_as_template(true);
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// The tray menu; with a newer version found, an item that opens its dialog.
fn tray_menu(app: &AppHandle, available: Option<&str>) -> tauri::Result<Menu<Wry>> {
    let item = |id: &str, text: String| MenuItem::with_id(app, id, text, true, None::<&str>);
    let open = item("show", crate::say("打开 AGT Studio", "Open AGT Studio"))?;
    let check = item("check", crate::say("检查更新", "Check for updates"))?;
    let exit = item("quit", crate::say("退出", "Quit"))?;
    match available {
        Some(version) => {
            let update = item(
                "update",
                crate::say(
                    &format!("更新到 {version}…"),
                    &format!("Update to {version}…"),
                ),
            )?;
            Menu::with_items(app, &[&open, &update, &check, &exit])
        }
        None => Menu::with_items(app, &[&open, &check, &exit]),
    }
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

/// Builds made without an updater key, such as local ones, never check.
fn updates_configured(app: &AppHandle) -> bool {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u["pubkey"].as_str())
        .is_some_and(|key| !key.trim().is_empty())
}

/// Look for a newer version at startup and every six hours. Nothing is
/// downloaded until the person confirms in the window.
async fn watch_updates(app: AppHandle) {
    if !updates_configured(&app) {
        return;
    }
    loop {
        check(&app, false).await;
        tokio::time::sleep(Duration::from_secs(6 * 60 * 60)).await;
    }
}

/// Check once. A newer version shows in the tray menu and the window, with one
/// notification per version; a check from the menu always says how it went.
async fn check(app: &AppHandle, asked: bool) {
    let notify = |zh: &str, en: &str| {
        let _ = app
            .notification()
            .builder()
            .title("AGT Studio")
            .body(crate::say(zh, en))
            .show();
    };
    if !updates_configured(app) {
        if asked {
            notify(
                "这个版本不能自动更新，请到发布页下载新版本。",
                "This build cannot update itself; download new versions from the releases page.",
            );
        }
        return;
    }
    let found = match app.updater() {
        Ok(updater) => updater.check().await.map_err(|e| e.to_string()),
        Err(error) => Err(error.to_string()),
    };
    let state = app.state::<Shared>();
    match found {
        Ok(Some(update)) => {
            let version = update.version.clone();
            let known = state
                .update
                .lock()
                .expect("update lock")
                .replace(update)
                .map(|u| u.version);
            if let Some(tray) = app.tray_by_id("main") {
                let _ = tray.set_menu(tray_menu(app, Some(&version)).ok());
            }
            let _ = app.emit("agt-update", ());
            if asked || known.as_deref() != Some(version.as_str()) {
                notify(
                    &format!("AGT Studio {version} 已发布。在窗口或托盘菜单里选择更新。"),
                    &format!(
                        "AGT Studio {version} is available. Choose Update in the window or the tray menu."
                    ),
                );
            }
        }
        Ok(None) => {
            *state.update.lock().expect("update lock") = None;
            if let Some(tray) = app.tray_by_id("main") {
                let _ = tray.set_menu(tray_menu(app, None).ok());
            }
            if asked {
                let current = app.package_info().version.to_string();
                notify(
                    &format!("已是最新版本 {current}。"),
                    &format!("{current} is the latest version."),
                );
            }
        }
        Err(error) => {
            if asked {
                notify(
                    &format!("检查更新失败：{error}"),
                    &format!("Could not check for updates: {error}"),
                );
            }
        }
    }
}

/// The new-version button: the version found by the last check, if any.
#[tauri::command]
fn update_status(app: AppHandle, state: tauri::State<'_, Shared>) -> Value {
    let current = app.package_info().version.to_string();
    if !updates_configured(&app) {
        return json!({ "current": current, "enabled": false, "desktop": true });
    }
    let latest = state
        .update
        .lock()
        .expect("update lock")
        .as_ref()
        .map(|u| u.version.clone());
    json!({
        "current": current,
        "enabled": true,
        "desktop": true,
        "update_available": latest.is_some(),
        "url": latest.as_ref().map(|v| format!("https://github.com/blacsheep/agt/releases/tag/v{v}")),
        "latest": latest,
    })
}

/// Download the version the person confirmed, reporting progress on
/// `channel` as `{downloaded, total}`, then stop the scheduler, install and
/// restart.
#[tauri::command]
async fn update_install(
    app: AppHandle,
    state: tauri::State<'_, Shared>,
    channel: Channel<Value>,
) -> Result<(), String> {
    let update = state
        .update
        .lock()
        .expect("update lock")
        .clone()
        .ok_or_else(|| {
            crate::say(
                "没有可安装的新版本。",
                "There is no new version to install.",
            )
        })?;
    let mut downloaded = 0usize;
    let bytes = update
        .download(
            |chunk, total| {
                downloaded += chunk;
                let _ = channel.send(json!({ "downloaded": downloaded, "total": total }));
            },
            || {},
        )
        .await
        .map_err(|e| crate::say(&format!("下载失败：{e}"), &format!("Download failed: {e}")))?;
    let console = state.console.get().cloned();
    // On Windows the installer takes over and closes the app, so the scheduler
    // stops first; elsewhere the files are replaced and the app restarts.
    #[cfg(windows)]
    if let Some(console) = &console {
        console.close().await;
    }
    update
        .install(bytes)
        .map_err(|e| crate::say(&format!("安装失败：{e}"), &format!("Install failed: {e}")))?;
    if let Some(console) = console {
        console.close().await;
    }
    app.restart();
}
