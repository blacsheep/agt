//! System notifications while the window is in the background: new replies,
//! timers that fire, and approvals waiting for you. Everything is read through
//! the same console API the page uses.
use crate::app::Shared;
use agt_console::Console;
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

pub(crate) async fn watch(app: AppHandle) {
    let Ok(console) = app.state::<Shared>().console().await else {
        return;
    };
    tauri::async_runtime::spawn(approvals(app.clone(), console.clone()));
    let principal = console.principal().to_owned();
    let started = now();
    let mut latest: HashMap<String, String> = HashMap::new();
    loop {
        let path = format!("/api/events?viewer={}", encode(&principal));
        let response = console.request("GET", &path, Vec::new()).await;
        if response.status() == 200 {
            let mut stream = response.into_body().into_data_stream();
            let mut line = Vec::new();
            while let Some(Ok(bytes)) = stream.next().await {
                for byte in bytes {
                    if byte != b'\n' {
                        line.push(byte);
                        continue;
                    }
                    if let Ok(event) = serde_json::from_slice::<Value>(&line) {
                        for change in event["changes"].as_array().into_iter().flatten() {
                            if change["kind"] == "im"
                                && let Some(chat) = change["conversation"].as_str()
                            {
                                message(&app, &console, &principal, chat, started, &mut latest)
                                    .await;
                            }
                        }
                    }
                    line.clear();
                }
            }
        }
        // The stream ends when the console restarts it; reconnect after a pause.
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

/// Notify about the newest message in a conversation if it is new to us.
async fn message(
    app: &AppHandle,
    console: &Console,
    principal: &str,
    chat: &str,
    started: i64,
    latest: &mut HashMap<String, String>,
) {
    let body =
        json!({"name":"message_history","args":{"conversation":chat,"limit":1},"viewer":principal});
    let Some(page) = call(console, "POST", "/api/read", Some(body)).await else {
        return;
    };
    let Some(message) = page["messages"].as_array().and_then(|m| m.last()) else {
        return;
    };
    let id = message["id"].as_str().unwrap_or_default().to_owned();
    if latest.get(chat) == Some(&id) || message["created_at"].as_i64().unwrap_or(0) < started {
        return;
    }
    latest.insert(chat.to_owned(), id);
    let timer = message["metadata"]["timer"].is_object();
    let sender = message["sender_id"].as_str().unwrap_or("");
    if (sender == principal && !timer) || focused(app) {
        return;
    }
    let title = if timer {
        crate::say("定时任务", "Timer")
    } else {
        sender.to_owned()
    };
    let text: String = message["body"]
        .as_str()
        .unwrap_or("")
        .chars()
        .take(160)
        .collect();
    let _ = app.notification().builder().title(title).body(text).show();
}

/// Approvals are not conversation changes, so check them on a short interval.
async fn approvals(app: AppHandle, console: std::sync::Arc<Console>) {
    let principal = console.principal().to_owned();
    let mut known = waiting(&console, &principal).await;
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let count = waiting(&console, &principal).await;
        if count > known && !focused(&app) {
            let _ = app
                .notification()
                .builder()
                .title("AGT Studio")
                .body(crate::say(
                    &format!("有 {count} 个请求等待你审批"),
                    &format!("{count} request(s) waiting for your approval"),
                ))
                .show();
        }
        known = count;
    }
}

async fn waiting(console: &Console, principal: &str) -> usize {
    let path = format!("/api/approvals?viewer={}", encode(principal));
    call(console, "GET", &path, None)
        .await
        .and_then(|list| {
            list.as_array().map(|items| {
                items
                    .iter()
                    .filter(|a| a["state"] == "awaiting_human" && a["decision"].is_null())
                    .count()
            })
        })
        .unwrap_or(0)
}

async fn call(console: &Console, method: &str, path: &str, body: Option<Value>) -> Option<Value> {
    let body = body.map(|b| b.to_string().into_bytes()).unwrap_or_default();
    let response = console.request(method, path, body).await;
    if response.status() != 200 {
        return None;
    }
    let mut stream = response.into_body().into_data_stream();
    let mut bytes = Vec::new();
    while let Some(Ok(chunk)) = stream.next().await {
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).ok()
}

fn focused(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Percent-encode a query value.
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
