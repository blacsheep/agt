//! HTTP boundary of the console. Only this machine can connect, every API call
//! needs the startup token, and writes always act as the fixed operator.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::header::AsHeaderName;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::Response;
use futures_util::future::try_join_all;
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::ChildStdout;
use tokio::sync::{mpsc, watch};

use super::activity;
use super::assets;
use super::cli::{Cli, Ops, is_member, stop, strings};
use super::commands;
use super::error::ApiError;
use super::js;

const READ_TOOLS: [&str; 8] = [
    "conversation_list",
    "member_list",
    "message_history",
    "message_status",
    "message_inbox",
    "task_get",
    "task_list",
    "task_attention",
];
const WRITE_TOOLS: [&str; 18] = [
    "direct_create",
    "group_create",
    "message_send",
    "member_add",
    "member_remove",
    "member_role_set",
    "resource_write",
    "principal_create",
    "agent_bind",
    "agent_unbind",
    "agent_set_enabled",
    "task_cancel",
    "task_reply",
    "task_reprioritize",
    "task_defer",
    "task_resolve",
    "task_retry",
    "task_report",
];
const KINDS: [&str; 7] = [
    "provider", "model", "prompt", "skill", "mcp", "agent", "harness",
];
const TOKEN_HEADER: &str = "x-agt-token";
const MAX_STREAMS: usize = 8;
const MAX_BODY: usize = 512 * 1024;
const MAX_EVENT_LINE: usize = 256 * 1024;
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

pub(crate) struct App {
    pub cli: Arc<Cli>,
    pub principal: String,
    pub home: String,
    pub token: String,
    pub port: u16,
    pub streams: AtomicUsize,
    pub changing: Mutex<HashSet<String>>,
    pub shutdown: watch::Receiver<bool>,
}

impl App {
    fn ops(&self) -> Ops<'_> {
        Ops {
            backend: &*self.cli,
            operator: &self.principal,
        }
    }

    async fn tool(&self, name: &str, args: Value) -> Result<Value, ApiError> {
        self.ops().tool(name, &args, &self.principal).await
    }

    async fn run(&self, args: Vec<String>) -> Result<Value, ApiError> {
        self.ops().run(args).await
    }
}

pub(crate) async fn handle(State(app): State<Arc<App>>, request: Request) -> Response {
    let mut response = match route(&app, request).await {
        Ok(response) => response,
        Err(error) => json_response(error.status, &error.body()),
    };
    let headers = response.headers_mut();
    for (name, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("content-security-policy", CSP),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

fn json_response(status: u16, value: &Value) -> Response {
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR))
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(Body::from(value.to_string()))
        .expect("valid response")
}

fn ok(value: Value) -> Result<Response, ApiError> {
    Ok(json_response(200, &value))
}

fn bad(code: &str, message: &str) -> ApiError {
    ApiError::new(400, code, message)
}

/// `str()`: a non-empty identifier of at most 256 UTF-16 units.
fn ident(value: Option<&Value>) -> Result<String, ApiError> {
    match value.and_then(Value::as_str) {
        Some(s) if !s.trim().is_empty() && js::utf16_len(s) <= 256 => Ok(s.to_owned()),
        _ => Err(ApiError::new(
            400,
            "invalid_arguments",
            "标识符必须是非空字符串。",
        )),
    }
}

fn ident_text(value: &str) -> Result<String, ApiError> {
    ident(Some(&Value::String(value.to_owned())))
}

fn object(value: Option<&Value>) -> Result<&Map<String, Value>, ApiError> {
    value
        .and_then(Value::as_object)
        .ok_or_else(|| ApiError::new(400, "invalid_arguments", "请求必须是 JSON 对象。"))
}

fn has_any(data: &Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter().any(|k| data.contains_key(*k))
}

struct Query(Vec<(String, String)>);

impl Query {
    fn parse(query: Option<&str>) -> Self {
        Self(
            form_urlencoded::parse(query.unwrap_or("").as_bytes())
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect(),
        )
    }

    /// `URLSearchParams.get`: the first value, if any.
    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// `get(key) || fallback`
    fn or<'a>(&'a self, key: &str, fallback: &'a str) -> &'a str {
        self.get(key).filter(|v| !v.is_empty()).unwrap_or(fallback)
    }
}

/// The JSON body, read only by routes that take one.
struct Payload {
    content_type: Option<String>,
    body: Option<Body>,
}

impl Payload {
    async fn json(&mut self) -> Result<Map<String, Value>, ApiError> {
        if self
            .content_type
            .as_deref()
            .and_then(|t| t.split(';').next())
            != Some("application/json")
        {
            return Err(ApiError::new(
                415,
                "content_type",
                "需要 application/json。",
            ));
        }
        let body = self.body.take().unwrap_or_default();
        let bytes = axum::body::to_bytes(body, MAX_BODY)
            .await
            .map_err(|_| ApiError::new(413, "too_large", "请求超过 512 KiB。"))?;
        match serde_json::from_slice::<Value>(&bytes) {
            Ok(Value::Object(map)) => Ok(map),
            Ok(_) => Err(ApiError::new(
                400,
                "invalid_arguments",
                "请求必须是 JSON 对象。",
            )),
            Err(_) => Err(ApiError::new(400, "invalid_json", "JSON 格式不正确。")),
        }
    }
}

async fn route(app: &Arc<App>, request: Request) -> Result<Response, ApiError> {
    let host = format!("127.0.0.1:{}", app.port);
    let headers = request.headers();
    let origin = header_text(headers, header::ORIGIN).filter(|o| !o.is_empty());
    if header_text(headers, header::HOST) != Some(host.as_str())
        || origin.is_some_and(|o| o != format!("http://{host}"))
    {
        return Err(ApiError::new(
            403,
            "origin_denied",
            "仅允许从本机控制台访问。",
        ));
    }
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    if !path.starts_with("/api/") {
        let asset = (method == Method::GET)
            .then(|| assets::get(&path))
            .flatten();
        let Some((bytes, kind)) = asset else {
            return Err(ApiError::new(404, "not_found", "页面不存在。"));
        };
        return Ok(Response::builder()
            .header(header::CONTENT_TYPE, format!("{kind}; charset=utf-8"))
            .body(Body::from(Bytes::from_static(bytes)))
            .expect("valid response"));
    }
    if header_text(headers, TOKEN_HEADER) != Some(app.token.as_str()) {
        return Err(ApiError::new(
            401,
            "unauthorized",
            "请使用启动终端提供的完整链接打开控制台。",
        ));
    }
    let query = Query::parse(request.uri().query());
    let content_type = header_text(headers, header::CONTENT_TYPE).map(str::to_owned);
    let mut payload = Payload {
        content_type,
        body: Some(request.into_body()),
    };
    let p = app.principal.as_str();
    let ops = app.ops();
    match (method, path.as_str()) {
        (Method::GET, "/api/events") => events(app, &query).await,
        (Method::GET, "/api/config") => {
            let config = app.run(strings(&["config", "show"])).await?;
            let schema = app.run(strings(&["config", "describe"])).await?;
            ok(json!({ "config": config, "schema": schema }))
        }
        (Method::POST, "/api/config") => {
            let data = payload.json().await?;
            if data.keys().any(|k| k != "config" && k != "expected") {
                return Err(ApiError::new(
                    400,
                    "invalid_config",
                    "配置请求包含未知字段。",
                ));
            }
            let value = Value::Object(object(data.get("config"))?.clone()).to_string();
            let expected = Value::Object(object(data.get("expected"))?.clone()).to_string();
            ok(app
                .run(strings(&[
                    "config",
                    "update",
                    "--value",
                    &value,
                    "--expected",
                    &expected,
                ]))
                .await?)
        }
        (Method::GET, "/api/bootstrap") => {
            let mut requests = vec![
                ("principal_list", json!({ "include_deleted": true })),
                ("binding_list", json!({})),
            ];
            requests.extend(KINDS.map(|kind| ("resource_list", json!({ "kind": kind }))));
            let mut results =
                try_join_all(requests.iter().map(|(name, args)| ops.tool(name, args, p))).await?;
            let resources: Map<String, Value> = KINDS
                .iter()
                .zip(results.drain(2..))
                .map(|(kind, value)| ((*kind).to_owned(), value))
                .collect();
            ok(json!({
                "home": app.home, "principal": p,
                "principals": results[0], "bindings": results[1], "resources": resources,
            }))
        }
        (Method::GET, "/api/activity") => {
            ok(activity::snapshot(ops, query.get("conversation"), query.or("viewer", p)).await?)
        }
        (Method::GET, "/api/message-runs") => {
            ok(
                activity::conversation_runs(ops, query.get("conversation"), query.or("viewer", p))
                    .await?,
            )
        }
        (Method::GET, "/api/approvals") => {
            let viewer = ident_text(query.or("viewer", p))?;
            let id = query.get("id").filter(|i| !i.is_empty());
            let mut args = strings(&["permission", if id.is_some() { "info" } else { "list" }]);
            if let Some(id) = id {
                args.push(ident_text(id)?);
            }
            args.extend(["--principal".into(), viewer]);
            if let (None, Some(before)) = (id, query.get("before").filter(|b| !b.is_empty())) {
                args.extend(["--before".into(), ident_text(before)?]);
            }
            ok(app.run(args).await?)
        }
        (Method::POST, "/api/approvals") => {
            let data = payload.json().await?;
            if data
                .keys()
                .any(|k| !["id", "expected_revision", "decision", "reason"].contains(&k.as_str()))
            {
                return Err(ApiError::new(
                    403,
                    "identity_fixed",
                    "审批操作身份由启动者固定。",
                ));
            }
            let revision = js::safe_integer(data.get("expected_revision")).filter(|r| *r >= 1);
            let Some(revision) = revision else {
                return Err(bad("invalid_revision", "审批版本无效。"));
            };
            let reason = data
                .get("reason")
                .and_then(Value::as_str)
                .filter(|r| js::utf16_len(r) <= 2000);
            let (Some(reason), Some(decision)) = (reason, data.get("decision")) else {
                return Err(bad("invalid_decision", "审批决定或理由无效。"));
            };
            let id = ident(data.get("id"))?;
            let revision = revision.to_string();
            let decision = decision.to_string();
            ok(app
                .run(strings(&[
                    "permission",
                    "decide",
                    &id,
                    "--principal",
                    p,
                    "--expected-revision",
                    &revision,
                    "--decision",
                    &decision,
                    "--reason",
                    reason,
                ]))
                .await?)
        }
        (Method::POST, "/api/harness") => harness(app, payload.json().await?).await,
        (Method::POST, "/api/chat-status") => chat_status(app, payload.json().await?).await,
        (Method::POST, "/api/read") => {
            let data = payload.json().await?;
            let name = ident(data.get("name"))?;
            if !READ_TOOLS.contains(&name.as_str()) {
                return Err(ApiError::new(
                    403,
                    "tool_denied",
                    "观察接口仅允许读取会话。",
                ));
            }
            let args = match data.get("args") {
                Some(v) if !v.is_null() => object(Some(v))?.clone(),
                _ => Map::new(),
            };
            let viewer = if js::truthy(data.get("viewer")) {
                ident(data.get("viewer"))?
            } else {
                p.to_owned()
            };
            let args = Value::Object(args);
            let mut result = ops.tool(&name, &args, &viewer).await?;
            if name == "message_history"
                && let Some(messages) = result.get("messages").and_then(Value::as_array)
            {
                let seq = |m: Option<&Value>| {
                    m.and_then(|m| m.get("seq"))
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0)
                };
                let (first, last) = (seq(messages.first()), seq(messages.last()));
                let from = if js::truthy(result.get("next_cursor")) {
                    (first - 1.0).max(0.0)
                } else {
                    0.0
                };
                let conversation = ident(args.get("conversation"))?;
                result["context_resets"] = app
                    .run(strings(&[
                        "schedule",
                        "reset-history",
                        &conversation,
                        "--actor",
                        &viewer,
                        "--from",
                        &js::number_text(&json!(from)),
                        "--through",
                        &js::number_text(&json!(last)),
                    ]))
                    .await?;
            }
            ok(result)
        }
        (Method::POST, "/api/tool") => write_tool(app, payload.json().await?).await,
        (Method::POST, "/api/harness-test") => {
            let data = payload.json().await?;
            if has_any(&data, &["viewer", "principal"]) {
                return Err(ApiError::new(
                    403,
                    "identity_fixed",
                    "连接测试仅供控制台管理员使用。",
                ));
            }
            let reference = ident(data.get("reference"))?;
            ok(app
                .run(strings(&["harness", "resource", "test", &reference]))
                .await?)
        }
        (Method::POST, "/api/provider-models") => {
            let data = payload.json().await?;
            let provider = ident(data.get("provider"))?;
            match app
                .tool("provider_models", json!({ "reference": provider }))
                .await
            {
                Ok(models) => ok(models),
                Err(_) => Err(ApiError::new(
                    502,
                    "provider_models_failed",
                    "无法获取模型列表。请检查 Provider 地址、密钥和连接，或直接填写模型名称。",
                )),
            }
        }
        (Method::POST, "/api/resource") => resource(app, payload.json().await?).await,
        (Method::POST, "/api/agent-entries") => agent_entries(app, payload.json().await?).await,
        (Method::POST, "/api/conversation") => conversation(app, payload.json().await?).await,
        (Method::POST, "/api/reply") => {
            let data = payload.json().await?;
            let conversation = ident(data.get("conversation"))?;
            let action = data.get("action").and_then(Value::as_str).unwrap_or("");
            if !["start", "pause", "end", "resume"].contains(&action) {
                return Err(bad("invalid_arguments", "未知回复操作。"));
            }
            let members = ops.members(&conversation, p).await?;
            // Strictly `left_message_seq === null`: departed members cannot control the round.
            let present = members.as_array().is_some_and(|list| {
                list.iter().any(|m| {
                    js::is(m.get("principal_id"), p)
                        && m.get("left_message_seq") == Some(&Value::Null)
                })
            });
            if !present {
                return Err(ApiError::new(403, "not_member", "退出群聊后不能控制讨论。"));
            }
            if ["start", "resume"].contains(&action) && data.get("confirm") != Some(&json!(true)) {
                return Err(bad("confirmation_required", "请确认调用模型。"));
            }
            let scheduler = app
                .run(strings(&["schedule", action, &conversation]))
                .await?;
            Ok(json_response(202, &json!({ "scheduler": scheduler })))
        }
        (Method::POST, "/api/tool-detail") => {
            let data = payload.json().await?;
            let viewer = if js::truthy(data.get("viewer")) {
                ident(data.get("viewer"))?
            } else {
                p.to_owned()
            };
            let conversation = ident(data.get("conversation"))?;
            let id = ident(data.get("id"))?;
            let activity = ident(data.get("activity"))?;
            match activity::tool_detail(ops, &viewer, &conversation, &id, &activity).await? {
                Some(detail) => ok(detail),
                None => Err(ApiError::new(
                    403,
                    "detail_denied",
                    "仅执行者或本机操作员可在所属会话查看原始工具详情。",
                )),
            }
        }
        (Method::POST, "/api/calls") => {
            let data = payload.json().await?;
            let after = match data.get("after") {
                None | Some(Value::Null) => Some(0),
                after => js::safe_integer(after).filter(|a| *a >= 0),
            };
            let Some(after) = after else {
                return Err(bad("invalid_cursor", "调用游标无效。"));
            };
            let id = ident(data.get("id"))?;
            ok(app
                .run(strings(&[
                    "run",
                    "calls",
                    &id,
                    "--after",
                    &after.to_string(),
                    "--limit",
                    "100",
                ]))
                .await?)
        }
        _ => Err(ApiError::new(404, "not_found", "接口不存在。")),
    }
}

fn header_text(headers: &HeaderMap, name: impl AsHeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

async fn write_tool(app: &App, data: Map<String, Value>) -> Result<Response, ApiError> {
    let p = app.principal.as_str();
    let name = ident(data.get("name"))?;
    if !WRITE_TOOLS.contains(&name.as_str()) {
        return Err(ApiError::new(403, "tool_denied", "该操作未开放。"));
    }
    if has_any(&data, &["viewer", "principal"]) {
        return Err(ApiError::new(
            403,
            "identity_fixed",
            "写入身份由启动者固定。",
        ));
    }
    if data.get("auto_reply").is_some_and(|v| !v.is_boolean()) {
        return Err(bad("invalid_arguments", "自动回复必须为布尔值。"));
    }
    let args = match data.get("args") {
        Some(v) if !v.is_null() => object(Some(v))?.clone(),
        _ => Map::new(),
    };
    if data.get("literal_message").is_some_and(|v| !v.is_boolean()) {
        return Err(bad("invalid_arguments", "普通文本标记必须为布尔值。"));
    }
    if name == "message_send"
        && data.get("literal_message") != Some(&json!(true))
        && args
            .get("body")
            .and_then(Value::as_str)
            .is_some_and(commands::is_command)
    {
        return Err(bad(
            "chat_command_required",
            "这是控制命令，请通过 Web 命令入口执行；如需讨论命令本身，请明确按普通文本发送。",
        ));
    }
    let provider = name == "resource_write" && js::is(args.get("kind"), "provider");
    if provider && object(args.get("data"))?.contains_key("api_key_env") {
        return Err(bad(
            "invalid_arguments",
            "请直接输入 API Key，Web 不支持环境变量方式。",
        )
        .with_field(json!("data.api_key")));
    }
    let key_data = args
        .get("data")
        .and_then(Value::as_object)
        .filter(|d| provider && d.contains_key("api_key"));
    let result = if let Some(config) = key_data {
        // The key reaches the CLI through a variable of the child process only,
        // never through command arguments or this process's environment.
        let api_key = config
            .get("api_key")
            .and_then(Value::as_str)
            .filter(|k| !k.trim().is_empty());
        let Some(api_key) = api_key else {
            return Err(bad("invalid_arguments", "请填写非空的 API Key。")
                .with_field(json!("data.api_key")));
        };
        let variable = format!("AGT_WEB_CREDENTIAL_{}", uuid::Uuid::new_v4().simple());
        let mut config = config.clone();
        config.remove("api_key");
        config.insert("api_key_env".into(), json!(variable));
        let mut args = args.clone();
        args.insert("data".into(), Value::Object(config));
        let command = strings(&[
            "tool",
            "call",
            "--principal",
            p,
            "--manager",
            &name,
            "--args",
            &Value::Object(args).to_string(),
        ]);
        app.cli
            .call_with_env(command, vec![(variable, api_key.to_owned())])
            .await?
    } else {
        app.tool(&name, Value::Object(args.clone())).await?
    };
    if name == "message_send" && data.get("auto_reply") == Some(&json!(true)) {
        let conversation = ident(args.get("conversation"))?;
        let message = ident(result.get("message").and_then(|m| m.get("id")))?;
        app.run(strings(&[
            "schedule",
            "start",
            &conversation,
            "--message",
            &message,
        ]))
        .await?;
    }
    ok(result)
}

async fn harness(app: &App, data: Map<String, Value>) -> Result<Response, ApiError> {
    let p = app.principal.as_str();
    let ops = app.ops();
    let operation = ident(data.get("operation"))?;
    if has_any(&data, &["viewer", "actor", "principal"]) {
        return Err(ApiError::new(
            403,
            "identity_fixed",
            "操作身份由启动者固定。",
        ));
    }
    match operation.as_str() {
        "capabilities" => return ok(app.run(strings(&["harness", "capabilities"])).await?),
        "resource-models" => {
            let reference = ident(data.get("reference"))?;
            return ok(app
                .tool("harness_models", json!({ "reference": reference }))
                .await?);
        }
        "resource-sessions" => {
            let reference = ident(data.get("reference"))?;
            let records = app.run(strings(&["harness", "session", "list"])).await?;
            let mut visible = Vec::new();
            for session in records.as_array().into_iter().flatten() {
                let conversation = session.get("conversation");
                if !js::is(session.get("harness"), &reference) || !js::truthy(conversation) {
                    continue;
                }
                let conversation = conversation.and_then(Value::as_str).unwrap_or_default();
                let members = ops
                    .members(conversation, p)
                    .await
                    .unwrap_or_else(|_| json!([]));
                if is_member(&members, p) {
                    visible.push(session.clone());
                }
            }
            return ok(Value::Array(visible));
        }
        _ => {}
    }
    let conversation = ident(data.get("conversation"))?;
    let target = ident(data.get("target"))?;
    let members = ops.members(&conversation, p).await?;
    if !is_member(&members, p) || !is_member(&members, &target) {
        return Err(ApiError::new(
            403,
            "not_member",
            "仅当前会话成员可管理该 Agent 的执行会话。",
        ));
    }
    let session = app
        .run(strings(&[
            "harness",
            "session",
            "current",
            "--principal",
            &target,
            "--conversation",
            &conversation,
        ]))
        .await?;
    if operation == "current" {
        return ok(session);
    }
    let id = session.get("id").cloned().unwrap_or(Value::Null);
    if !js::is(Some(&id), &ident(data.get("session"))?) {
        return Err(ApiError::new(
            409,
            "session_changed",
            "执行会话已变化，请刷新后重试。",
        ));
    }
    let id_text = id.as_str().unwrap_or_default();
    if operation == "models" || operation == "configure" {
        let mut args = json!({ "conversation": conversation, "target": target, "session": id });
        let name = if operation == "models" {
            "harness_session_models"
        } else {
            args["model"] = json!(ident(data.get("model"))?);
            if js::truthy(data.get("effort")) {
                args["effort"] = json!(ident(data.get("effort"))?);
            }
            "harness_session_configure"
        };
        return ok(app.tool(name, args).await?);
    }
    let mut args = strings(&["harness", "session"]);
    if ["skills", "mcp", "compact", "cancel"].contains(&operation.as_str()) {
        args.extend([operation.clone(), id_text.to_owned()]);
    } else if operation == "rename" {
        args.extend([
            "rename".into(),
            id_text.to_owned(),
            ident(data.get("title"))?,
        ]);
    } else {
        return Err(bad("unsupported_operation", "尚未接入此 Harness 操作。"));
    }
    ok(app.run(args).await?)
}

async fn chat_status(app: &App, data: Map<String, Value>) -> Result<Response, ApiError> {
    let p = app.principal.as_str();
    let conversation = ident(data.get("conversation"))?;
    if has_any(&data, &["viewer", "principal", "actor"]) {
        return Err(ApiError::new(
            403,
            "identity_fixed",
            "查看身份由启动者固定。",
        ));
    }
    let members = app.ops().members(&conversation, p).await?;
    if !is_member(&members, p) {
        return Err(ApiError::new(
            403,
            "not_member",
            "仅当前会话成员可查看运行信息。",
        ));
    }
    let (bindings, runs) = tokio::try_join!(
        app.tool("binding_list", json!({})),
        app.run(strings(&["run", "list", "--limit", "200"])),
    )?;
    let runs = runs.as_array().cloned().unwrap_or_default();
    let present: Vec<&Value> = bindings
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| {
            b.get("principal")
                .and_then(Value::as_str)
                .is_some_and(|id| is_member(&members, id))
        })
        .collect();
    let agents = try_join_all(present.into_iter().map(|b| {
        let runs = &runs;
        let conversation = &conversation;
        async move {
            let definition = app
                .tool(
                    "resource_read",
                    json!({ "kind": "agent", "reference": b.get("agent") }),
                )
                .await?;
            let reference = |v: Option<&Value>| {
                let part = |k| {
                    v.and_then(|v| v.get(k))
                        .map(|s| s.as_str().map_or_else(|| s.to_string(), str::to_owned))
                        .unwrap_or_else(|| "undefined".into())
                };
                format!("{}/{}", part("category"), part("name"))
            };
            let harness =
                js::truthy(definition.get("harness")).then(|| reference(definition.get("harness")));
            let config = match &harness {
                Some(h) => {
                    app.tool(
                        "resource_read",
                        json!({ "kind": "harness", "reference": h }),
                    )
                    .await?
                }
                None if js::truthy(definition.get("model")) => {
                    app.tool(
                        "resource_read",
                        json!({ "kind": "model", "reference": reference(definition.get("model")) }),
                    )
                    .await?
                }
                None => return Err(ApiError::internal()),
            };
            let last = runs.iter().find(|r| {
                let request = r.get("request");
                let trigger = request.and_then(|q| q.get("trigger"));
                js::is(trigger.and_then(|t| t.get("conversation_id")), conversation)
                    && js::same(trigger.and_then(|t| t.get("principal")), b.get("principal"))
                    && js::same(request.and_then(|q| q.get("agent")), b.get("agent"))
            });
            let truthy_or_null = |v: Option<&Value>| {
                if js::truthy(v) {
                    v.cloned().unwrap_or(Value::Null)
                } else {
                    Value::Null
                }
            };
            let mut agent = Map::new();
            for key in ["principal", "agent"] {
                if let Some(v) = b.get(key) {
                    agent.insert(key.into(), v.clone());
                }
            }
            agent.insert(
                "enabled".into(),
                json!(b.get("enabled") != Some(&json!(false))),
            );
            match &harness {
                Some(_) => {
                    if let Some(engine) = config.get("engine") {
                        agent.insert("engine".into(), engine.clone());
                    }
                }
                None => {
                    agent.insert("engine".into(), json!("native"));
                }
            }
            agent.insert("harness".into(), harness.map_or(Value::Null, Value::String));
            agent.insert(
                "configured_model".into(),
                truthy_or_null(config.get("model")),
            );
            agent.insert(
                "last_run".into(),
                match last {
                    Some(last) => {
                        let execution = last.get("execution");
                        let model = execution
                            .and_then(|e| e.get("model"))
                            .filter(|m| m.is_string());
                        let mut run = Map::new();
                        for key in ["id", "state"] {
                            if let Some(v) = last.get(key) {
                                run.insert(key.into(), v.clone());
                            }
                        }
                        run.insert(
                            "reported_model".into(),
                            model.cloned().unwrap_or(Value::Null),
                        );
                        run.insert(
                            "harness".into(),
                            truthy_or_null(execution.and_then(|e| e.get("harness"))),
                        );
                        Value::Object(run)
                    }
                    None => Value::Null,
                },
            );
            Ok::<_, ApiError>(Value::Object(agent))
        }
    }))
    .await?;
    ok(json!({ "conversation": conversation, "agents": agents }))
}

async fn resource(app: &App, data: Map<String, Value>) -> Result<Response, ApiError> {
    if has_any(&data, &["viewer", "principal"]) {
        return Err(ApiError::new(
            403,
            "identity_fixed",
            "资源操作身份由启动者固定。",
        ));
    }
    let kind = data
        .get("kind")
        .and_then(Value::as_str)
        .filter(|k| KINDS.contains(k));
    let action = data
        .get("action")
        .and_then(Value::as_str)
        .filter(|a| ["read", "plan", "delete"].contains(a));
    let (Some(kind), Some(action)) = (kind, action) else {
        return Err(bad("invalid_arguments", "未知资源类型或操作。"));
    };
    if data.contains_key("recursive") {
        return Err(bad(
            "invalid_arguments",
            "Web 仅支持删除单个资源，请先解除引用。",
        ));
    }
    let mut args = json!({ "kind": kind, "reference": ident(data.get("reference"))? });
    if action != "read" {
        args["recursive"] = json!(false);
    }
    if action == "delete" {
        let confirm = data
            .get("confirm")
            .and_then(Value::as_array)
            .filter(|c| c.len() == 1 && c[0].is_string());
        let Some(confirm) = confirm else {
            return Err(bad("confirmation_required", "请先获取删除计划并确认。"));
        };
        args["confirm"] = json!(confirm);
    }
    let name = if action == "read" {
        "resource_read"
    } else {
        "resource_delete"
    };
    match app.tool(name, args).await {
        Ok(value) => ok(value),
        Err(error) if action != "read" && error.code == "operation_rejected" => Err(bad(
            "operation_rejected",
            "无法删除：请先解除资源引用或在运行设置中解除 Agent 的运行配置。默认 Prompt 不可删除；资源配置损坏或引用缺失也会阻止删除。刷新状态后重试。",
        )),
        Err(error) => Err(error),
    }
}

async fn agent_entries(app: &App, data: Map<String, Value>) -> Result<Response, ApiError> {
    let agent = ident(data.get("agent"))?;
    let action = data
        .get("action")
        .and_then(Value::as_str)
        .filter(|a| *a == "hide" || *a == "show");
    let (true, Some(action)) = (data.get("confirm") == Some(&json!(true)), action) else {
        return Err(bad("invalid_arguments", "请确认移除或恢复会话入口。"));
    };
    let list = data
        .get("conversations")
        .and_then(Value::as_array)
        .filter(|c| (1..=100).contains(&c.len()));
    let Some(list) = list else {
        return Err(bad("invalid_arguments", "每次请选择 1–100 个会话。"));
    };
    let mut ids: Vec<String> = Vec::new();
    for id in list {
        let id = ident(Some(id))?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let principals = app.tool("principal_list", json!({})).await?;
    let is_agent = principals.as_array().is_some_and(|list| {
        list.iter()
            .any(|p| js::is(p.get("id"), &agent) && js::is(p.get("kind"), "agent"))
    });
    if !is_agent {
        return Err(ApiError::new(
            403,
            "agent_required",
            "仅可整理 Agent 的会话入口。",
        ));
    }
    let name = if action == "hide" {
        "conversation_hide"
    } else {
        "conversation_show"
    };
    let mut results = Vec::new();
    for conversation in ids {
        let outcome = app
            .ops()
            .tool(name, &json!({ "conversation": conversation }), &agent)
            .await;
        results.push(match outcome {
            Ok(_) => json!({ "conversation": conversation, "ok": true }),
            Err(_) => json!({ "conversation": conversation, "ok": false, "error": "无法整理该入口，请刷新后检查成员关系。" }),
        });
    }
    ok(json!({ "results": results }))
}

/// Releases a conversation from the in-progress set when the operation ends.
struct Busy<'a>(&'a Mutex<HashSet<String>>, String);
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.lock().expect("busy lock").remove(&self.1);
    }
}

async fn conversation(app: &App, data: Map<String, Value>) -> Result<Response, ApiError> {
    let p = app.principal.as_str();
    let conversation = ident(data.get("conversation"))?;
    const ACTIONS: [&str; 9] = [
        "hide",
        "show",
        "visit",
        "read",
        "leave",
        "dissolve",
        "delete",
        "reactivate",
        "reset",
    ];
    let Some(action) = data
        .get("action")
        .and_then(Value::as_str)
        .filter(|a| ACTIONS.contains(a))
    else {
        return Err(bad("invalid_arguments", "未知会话操作。"));
    };
    let through = js::safe_integer(data.get("through_seq")).filter(|s| *s > 0);
    if action == "read" && through.is_none() {
        return Err(bad("invalid_arguments", "已读位置必须是正整数。"));
    }
    if ["delete", "leave", "dissolve", "reset"].contains(&action)
        && data.get("confirm") != Some(&json!(true))
    {
        return Err(bad("confirmation_required", "请确认会话操作。"));
    }
    if !app
        .changing
        .lock()
        .expect("busy lock")
        .insert(conversation.clone())
    {
        return Err(ApiError::new(
            409,
            "conversation_busy",
            "会话操作进行中，请稍后重试。",
        ));
    }
    let _busy = Busy(&app.changing, conversation.clone());
    app.ops().members(&conversation, p).await?;
    if has_any(&data, &["viewer", "principal", "actor"]) {
        return Err(ApiError::new(
            403,
            "identity_fixed",
            "操作身份由启动者固定。",
        ));
    }
    let c = conversation.as_str();
    let through = through.map(|s| s.to_string()).unwrap_or_default();
    let args = match action {
        "reset" => strings(&["schedule", "reset", c, "--actor", p]),
        "hide" => strings(&["im", "conversation", "hide", c, "--actor", p]),
        "show" => strings(&["im", "conversation", "unhide", c, "--actor", p]),
        "visit" => strings(&["im", "conversation", "visit", c, "--actor", p]),
        "read" => strings(&[
            "im",
            "message",
            "read",
            c,
            "--principal",
            p,
            "--through-seq",
            &through,
        ]),
        "leave" => strings(&["im", "group", "members", "leave", c, p]),
        "dissolve" => strings(&["schedule", "dissolve", c, "--actor", p]),
        "delete" => strings(&["schedule", "delete", c, "--actor", p]),
        _ => strings(&["im", "conversation", "reactivate", c, "--actor", p]),
    };
    ok(app.run(args).await?)
}

/// Releases a subscription slot when its stream ends.
struct Slot(Arc<App>);
impl Drop for Slot {
    fn drop(&mut self) {
        self.0.streams.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Server-sent events from `agt events watch`. The CLI checks membership on
/// every delivery; no raw stdout or stderr reaches the page.
async fn events(app: &Arc<App>, query: &Query) -> Result<Response, ApiError> {
    let too_many = || ApiError::new(429, "too_many_streams", "订阅连接过多，请关闭多余窗口。");
    if app.streams.load(Ordering::SeqCst) >= MAX_STREAMS {
        return Err(too_many());
    }
    let viewer = ident_text(query.or("viewer", &app.principal))?;
    let after = query.get("after").map(ident_text).transpose()?;
    let identities = app.tool("principal_list", json!({})).await?;
    let allowed = identities.as_array().is_some_and(|list| {
        list.iter().any(|p| {
            js::is(p.get("id"), &viewer)
                && !js::truthy(p.get("deleted"))
                && (viewer == app.principal || js::is(p.get("kind"), "agent"))
        })
    });
    if !allowed {
        return Err(ApiError::new(
            403,
            "viewer_denied",
            "观察身份不存在或不可用。",
        ));
    }
    if app.streams.fetch_add(1, Ordering::SeqCst) >= MAX_STREAMS {
        app.streams.fetch_sub(1, Ordering::SeqCst);
        return Err(too_many());
    }
    let slot = Slot(app.clone());
    let mut child = app
        .cli
        .watch_events(&viewer, after.as_deref())
        .map_err(|_| ApiError::internal())?;
    let stdout = child.stdout.take().ok_or_else(ApiError::internal)?;
    let (sender, mut receiver) = mpsc::channel::<Bytes>(16);
    let mut shutdown = app.shutdown.clone();
    tokio::spawn(async move {
        let _slot = slot;
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();
        loop {
            line.clear();
            let page = tokio::select! {
                _ = sender.closed() => break,
                _ = shutdown.changed() => break,
                read = next_line(&mut reader, &mut line) => match read {
                    Ok(true) => serde_json::from_slice::<Value>(&line).ok(),
                    _ => break,
                },
            };
            let Some(page) = page.filter(valid_page) else {
                break;
            };
            let event = page["event"].as_str().unwrap_or_default().to_owned();
            if sender
                .send(Bytes::from(format!("event: {event}\ndata: {page}\n\n")))
                .await
                .is_err()
            {
                break;
            }
        }
        stop(&mut child).await;
    });
    let stream = futures_util::stream::poll_fn(move |cx| {
        receiver
            .poll_recv(cx)
            .map(|item| item.map(Ok::<_, std::io::Error>))
    });
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
        .header("x-accel-buffering", "no")
        .body(Body::from_stream(stream))
        .expect("valid response"))
}

fn valid_page(page: &Value) -> bool {
    ["ready", "change", "resync_required"].contains(&page["event"].as_str().unwrap_or_default())
        && page["cursor"].is_string()
        && page["changes"].is_array()
}

/// Reads one line without its newline; `false` at end of output. Errors on a
/// line longer than the subscription allows.
async fn next_line(
    reader: &mut BufReader<ChildStdout>,
    line: &mut Vec<u8>,
) -> std::io::Result<bool> {
    loop {
        let buffer = reader.fill_buf().await?;
        if buffer.is_empty() {
            return Ok(false);
        }
        if let Some(end) = buffer.iter().position(|b| *b == b'\n') {
            line.extend_from_slice(&buffer[..end]);
            reader.consume(end + 1);
            return Ok(true);
        }
        let read = buffer.len();
        line.extend_from_slice(buffer);
        reader.consume(read);
        if line.len() > MAX_EVENT_LINE {
            return Err(std::io::Error::other("event line too long"));
        }
    }
}
