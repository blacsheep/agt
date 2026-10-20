use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use agt_core::CancellationToken;
use futures_util::StreamExt;
use futures_util::future::ready;
use futures_util::stream::BoxStream;
use http::{HeaderName, HeaderValue};
use reqwest::header::{ACCEPT, CONTENT_LENGTH, CONTENT_TYPE};
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientInfo, ClientJsonRpcMessage, Implementation,
    PaginatedRequestParams, ProtocolVersion, ServerJsonRpcMessage,
};
use rmcp::service::{NotificationContext, RoleClient};
use rmcp::transport::streamable_http_client::{
    AuthRequiredError, InsufficientScopeError, StreamableHttpClient, StreamableHttpClientTransport,
    StreamableHttpClientTransportConfig, StreamableHttpError, StreamableHttpPostResponse,
};
use rmcp::{ClientHandler, ServiceExt};
use serde::Serialize;
use serde_json::Value;
use sse_stream::{Sse, SseStream};
use tokio::sync::mpsc as tokio_mpsc;

use crate::client::McpClient;
use crate::config::{McpDocument, McpHttpServerConfig};
use crate::error::{McpError, McpStderrSummary, Result};
use crate::limits::McpLimits;
use crate::protocol::{McpClientEvent, McpInitializeInfo, McpTool, McpToolCallResult};

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(10);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
const JSON_MIME_TYPE: &str = "application/json";
const EVENT_STREAM_MIME_TYPE: &str = "text/event-stream";
const HEADER_SESSION_ID: &str = "mcp-session-id";
const HEADER_LAST_EVENT_ID: &str = "last-event-id";

#[derive(Debug)]
pub struct HttpMcpClient {
    name: String,
    initialize_info: McpInitializeInfo,
    commands: tokio_mpsc::Sender<HttpCommand>,
    events: Receiver<McpClientEvent>,
    event_overflowed: Arc<AtomicBool>,
    max_queued_events: usize,
    worker: Option<thread::JoinHandle<()>>,
    closed: bool,
}

impl HttpMcpClient {
    pub fn start_with_limits(document: &McpDocument, limits: McpLimits) -> Result<Self> {
        let config = resolve_http_config(document, limits)?;
        let name = document.name.to_string();
        let (commands, command_rx) = tokio_mpsc::channel(8);
        let (events_tx, events) = mpsc::sync_channel(limits.max_queued_events.max(1));
        let event_overflowed = Arc::new(AtomicBool::new(false));
        let handler = AgtHttpClientHandler {
            events: events_tx,
            event_overflowed: Arc::clone(&event_overflowed),
        };
        let (init_tx, init_rx) = mpsc::sync_channel(1);
        let worker_name = format!("agt-mcp-http-{name}");
        let worker_server_name = name.clone();
        let worker = thread::Builder::new()
            .name(worker_name)
            .spawn(move || {
                run_worker(
                    worker_server_name,
                    config,
                    limits,
                    handler,
                    command_rx,
                    init_tx,
                );
            })
            .map_err(|_| McpError::HttpTransport {
                name: name.clone(),
                operation: "start".to_string(),
                message: "无法创建 HTTP MCP worker thread".to_string(),
            })?;

        let initialize_info =
            match init_rx.recv_timeout(INITIALIZE_TIMEOUT + Duration::from_secs(1)) {
                Ok(Ok(info)) => info,
                Ok(Err(error)) => {
                    let _ = worker.join();
                    return Err(error);
                }
                Err(RecvTimeoutError::Timeout) => {
                    let (reply, _reply_rx) = mpsc::sync_channel(1);
                    let _ = commands.try_send(HttpCommand::Close { reply });
                    let _ = worker.join();
                    return Err(McpError::Timeout {
                        name,
                        operation: "initialize".to_string(),
                        stderr: McpStderrSummary::default(),
                    });
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let _ = worker.join();
                    return Err(McpError::HttpTransport {
                        name,
                        operation: "initialize".to_string(),
                        message: "HTTP MCP worker 在初始化前退出".to_string(),
                    });
                }
            };

        Ok(Self {
            name,
            initialize_info,
            commands,
            events,
            event_overflowed,
            max_queued_events: limits.max_queued_events,
            worker: Some(worker),
            closed: false,
        })
    }

    fn request<T>(
        &self,
        operation: &str,
        build: impl FnOnce(SyncSender<Result<T>>) -> HttpCommand,
    ) -> Result<T> {
        self.request_with_cancellation(operation, &CancellationToken::default(), |reply, _| {
            build(reply)
        })
    }

    fn request_with_cancellation<T>(
        &self,
        operation: &str,
        cancellation: &CancellationToken,
        build: impl FnOnce(SyncSender<Result<T>>, CancellationToken) -> HttpCommand,
    ) -> Result<T> {
        if cancellation.is_canceled() {
            return Err(McpError::Canceled {
                operation: operation.to_string(),
            });
        }
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.commands
            .blocking_send(build(reply_tx, cancellation.clone()))
            .map_err(|_| McpError::HttpTransport {
                name: self.name.clone(),
                operation: operation.to_string(),
                message: "HTTP MCP worker 已关闭".to_string(),
            })?;
        let deadline = Instant::now() + REQUEST_TIMEOUT + Duration::from_secs(1);
        loop {
            match reply_rx.recv_timeout(CANCEL_POLL_INTERVAL) {
                Ok(result) => return result,
                Err(RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
                Err(RecvTimeoutError::Timeout) => {
                    return Err(McpError::Timeout {
                        name: self.name.clone(),
                        operation: operation.to_string(),
                        stderr: McpStderrSummary::default(),
                    });
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(McpError::HttpTransport {
                        name: self.name.clone(),
                        operation: operation.to_string(),
                        message: "HTTP MCP worker 未返回结果".to_string(),
                    });
                }
            }
        }
    }
}

impl McpClient for HttpMcpClient {
    fn initialize_info(&self) -> &McpInitializeInfo {
        &self.initialize_info
    }

    fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        let result = self.request("tools/list", |reply| HttpCommand::ListTools { reply });
        if result.is_err() {
            let _ = self.close();
        }
        result
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Result<McpToolCallResult> {
        self.call_tool_with_cancellation(name, arguments, &CancellationToken::default())
    }

    fn call_tool_with_cancellation(
        &mut self,
        name: &str,
        arguments: Value,
        cancellation: &CancellationToken,
    ) -> Result<McpToolCallResult> {
        let result =
            self.request_with_cancellation("tools/call", cancellation, |reply, cancellation| {
                HttpCommand::CallTool {
                    name: name.to_string(),
                    arguments,
                    cancellation,
                    reply,
                }
            });
        if result.is_err() {
            let _ = self.close();
        }
        result
    }

    fn poll_event(&mut self) -> Result<Option<McpClientEvent>> {
        if self.event_overflowed.swap(false, Ordering::SeqCst) {
            return Err(McpError::LimitExceeded {
                name: self.name.clone(),
                limit: "queued_client_events",
                max: self.max_queued_events,
                actual: self.max_queued_events.saturating_add(1),
                stderr: McpStderrSummary::default(),
            });
        }
        match self.events.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => Ok(None),
        }
    }

    fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let result = self.request("close", |reply| HttpCommand::Close { reply });
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        result
    }
}

impl Drop for HttpMcpClient {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        let (reply, _rx) = mpsc::sync_channel(1);
        let _ = self.commands.try_send(HttpCommand::Close { reply });
    }
}

enum HttpCommand {
    ListTools {
        reply: SyncSender<Result<Vec<McpTool>>>,
    },
    CallTool {
        name: String,
        arguments: Value,
        cancellation: CancellationToken,
        reply: SyncSender<Result<McpToolCallResult>>,
    },
    Close {
        reply: SyncSender<Result<()>>,
    },
}

#[derive(Clone, Debug)]
struct AgtHttpClientHandler {
    events: SyncSender<McpClientEvent>,
    event_overflowed: Arc<AtomicBool>,
}

impl ClientHandler for AgtHttpClientHandler {
    fn on_tool_list_changed(
        &self,
        _context: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        if self
            .events
            .try_send(McpClientEvent::ToolsListChanged)
            .is_err()
        {
            self.event_overflowed.store(true, Ordering::SeqCst);
        }
        ready(())
    }

    fn get_info(&self) -> ClientInfo {
        ClientInfo::new(
            ClientCapabilities::default(),
            Implementation::new("agt", env!("CARGO_PKG_VERSION")),
        )
        .with_protocol_version(ProtocolVersion::V_2025_11_25)
    }
}

fn run_worker(
    name: String,
    config: ResolvedHttpConfig,
    limits: McpLimits,
    handler: AgtHttpClientHandler,
    mut commands: tokio_mpsc::Receiver<HttpCommand>,
    init_tx: SyncSender<Result<McpInitializeInfo>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            let _ = init_tx.send(Err(McpError::HttpTransport {
                name,
                operation: "initialize".to_string(),
                message: "无法创建 Tokio runtime".to_string(),
            }));
            return;
        }
    };
    runtime.block_on(async move {
        let backend_state = config.backend.state.clone();
        let mut transport_config = StreamableHttpClientTransportConfig::with_uri(config.url);
        transport_config.auth_header = config.bearer_token;
        transport_config.custom_headers = config.headers;
        transport_config.channel_buffer_capacity = limits.max_queued_events.max(1);
        transport_config.allow_stateless = true;
        transport_config.reinit_on_expired_session = true;
        let transport =
            StreamableHttpClientTransport::with_client(config.backend, transport_config);
        let service = tokio::time::timeout(INITIALIZE_TIMEOUT, handler.serve(transport)).await;
        let mut service = match service {
            Ok(Ok(service)) => service,
            Ok(Err(_)) => {
                let _ = init_tx.send(Err(map_backend_error(&name, "initialize", &backend_state)));
                return;
            }
            Err(_) => {
                let _ = init_tx.send(Err(McpError::Timeout {
                    name,
                    operation: "initialize".to_string(),
                    stderr: McpStderrSummary::default(),
                }));
                return;
            }
        };
        let Some(peer_info) = service.peer_info() else {
            let _ = init_tx.send(Err(McpError::HttpTransport {
                name,
                operation: "initialize".to_string(),
                message: "server 未返回 initialize info".to_string(),
            }));
            return;
        };
        let initialize_info: McpInitializeInfo =
            match convert_value(peer_info.as_ref(), &name, "initialize") {
                Ok(info) => info,
                Err(error) => {
                    let _ = init_tx.send(Err(error));
                    return;
                }
            };
        if !crate::protocol::accepts_protocol_version(&initialize_info.protocol_version) {
            let _ = init_tx.send(Err(McpError::UnsupportedProtocolVersion {
                name,
                version: initialize_info.protocol_version,
                stderr: McpStderrSummary::default(),
            }));
            return;
        }
        if init_tx.send(Ok(initialize_info.clone())).is_err() {
            let _ = service.close_with_timeout(CLOSE_TIMEOUT).await;
            return;
        }

        while let Some(command) = commands.recv().await {
            match command {
                HttpCommand::ListTools { reply } => {
                    let result = list_tools(&name, &service, limits, &backend_state).await;
                    let _ = reply.send(result);
                }
                HttpCommand::CallTool {
                    name: tool_name,
                    arguments,
                    cancellation,
                    reply,
                } => {
                    let result = call_tool(
                        &name,
                        &service,
                        limits,
                        &backend_state,
                        tool_name,
                        arguments,
                        cancellation,
                    )
                    .await;
                    let _ = reply.send(result);
                }
                HttpCommand::Close { reply } => {
                    let result = service
                        .close_with_timeout(CLOSE_TIMEOUT)
                        .await
                        .map(|_| ())
                        .map_err(|_| McpError::HttpTransport {
                            name: name.clone(),
                            operation: "close".to_string(),
                            message: "HTTP MCP worker 清理失败".to_string(),
                        });
                    let _ = reply.send(result);
                    break;
                }
            }
        }
    });
}

async fn list_tools(
    name: &str,
    service: &rmcp::service::RunningService<RoleClient, AgtHttpClientHandler>,
    limits: McpLimits,
    backend_state: &BackendState,
) -> Result<Vec<McpTool>> {
    let Some(info) = service.peer_info() else {
        return Err(protocol_error(name, "initialize info 不可用"));
    };
    if info.capabilities.tools.is_none() {
        return Err(McpError::MissingCapability {
            name: name.to_string(),
            capability: "tools",
            stderr: McpStderrSummary::default(),
        });
    }
    let mut tools = Vec::new();
    let mut cursor = None;
    let mut seen_cursors = HashSet::new();
    let mut pages = 0usize;
    let mut metadata_bytes = 0usize;
    loop {
        if pages >= limits.max_list_pages {
            return Err(limit_error(
                name,
                "list_pages",
                limits.max_list_pages,
                pages.saturating_add(1),
            ));
        }
        pages += 1;
        let request = service
            .peer()
            .list_tools(Some(PaginatedRequestParams::default().with_cursor(cursor)));
        let page = match tokio::time::timeout(REQUEST_TIMEOUT, request).await {
            Ok(Ok(page)) => page,
            Ok(Err(_)) => return Err(map_backend_error(name, "tools/list", backend_state)),
            Err(_) => {
                return Err(McpError::Timeout {
                    name: name.to_string(),
                    operation: "tools/list".to_string(),
                    stderr: McpStderrSummary::default(),
                });
            }
        };
        if page.tools.len() > limits.max_tools_per_page {
            return Err(limit_error(
                name,
                "tools_per_page",
                limits.max_tools_per_page,
                page.tools.len(),
            ));
        }
        for tool in page.tools {
            let tool: McpTool = convert_value(&tool, name, "tools/list")?;
            validate_tool(name, &tool, limits)?;
            metadata_bytes = metadata_bytes.saturating_add(serialized_len(&tool)?);
            if metadata_bytes > limits.max_tool_metadata_bytes {
                return Err(limit_error(
                    name,
                    "tool_metadata_bytes",
                    limits.max_tool_metadata_bytes,
                    metadata_bytes,
                ));
            }
            if tools.len() >= limits.max_tools_total {
                return Err(limit_error(
                    name,
                    "tools_total",
                    limits.max_tools_total,
                    tools.len().saturating_add(1),
                ));
            }
            tools.push(tool);
        }
        match page.next_cursor {
            Some(next_cursor) if !next_cursor.is_empty() => {
                if !seen_cursors.insert(next_cursor.clone()) {
                    return Err(protocol_error(name, "tools/list 返回重复 nextCursor"));
                }
                cursor = Some(next_cursor);
            }
            _ => break,
        }
    }
    Ok(tools)
}

async fn call_tool(
    server_name: &str,
    service: &rmcp::service::RunningService<RoleClient, AgtHttpClientHandler>,
    limits: McpLimits,
    backend_state: &BackendState,
    tool_name: String,
    arguments: Value,
    cancellation: CancellationToken,
) -> Result<McpToolCallResult> {
    let arguments = arguments
        .as_object()
        .cloned()
        .ok_or_else(|| protocol_error(server_name, "tools/call arguments 必须是对象"))?;
    let request = service
        .peer()
        .call_tool(CallToolRequestParams::new(tool_name).with_arguments(arguments));
    let response = tokio::select! {
        result = tokio::time::timeout(REQUEST_TIMEOUT, request) => match result {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => return Err(map_backend_error(server_name, "tools/call", backend_state)),
            Err(_) => {
                return Err(McpError::Timeout {
                    name: server_name.to_string(),
                    operation: "tools/call".to_string(),
                    stderr: McpStderrSummary::default(),
                });
            }
        },
        () = wait_for_cancellation(cancellation) => {
            return Err(McpError::Canceled {
                operation: "tools/call".to_string(),
            });
        }
    };
    let result: McpToolCallResult = convert_value(&response, server_name, "tools/call")?;
    let bytes = serialized_len(&result)?;
    if bytes > limits.max_call_result_bytes {
        return Err(limit_error(
            server_name,
            "call_result_bytes",
            limits.max_call_result_bytes,
            bytes,
        ));
    }
    Ok(result)
}

async fn wait_for_cancellation(cancellation: CancellationToken) {
    while !cancellation.is_canceled() {
        tokio::time::sleep(CANCEL_POLL_INTERVAL).await;
    }
}

fn convert_value<T: Serialize, U: serde::de::DeserializeOwned>(
    value: &T,
    name: &str,
    operation: &str,
) -> Result<U> {
    let value = serde_json::to_value(value)
        .map_err(|_| protocol_error(name, &format!("{operation} 响应序列化失败")))?;
    serde_json::from_value(value)
        .map_err(|_| protocol_error(name, &format!("{operation} 响应格式无效")))
}

fn validate_tool(name: &str, tool: &McpTool, limits: McpLimits) -> Result<()> {
    if tool.name.is_empty() {
        return Err(protocol_error(name, "tools/list 返回空 tool name"));
    }
    check_len(
        name,
        "tool_name_bytes",
        limits.max_tool_name_bytes,
        tool.name.len(),
    )?;
    if let Some(description) = &tool.description {
        check_len(
            name,
            "tool_description_bytes",
            limits.max_description_bytes,
            description.len(),
        )?;
    }
    if !tool.input_schema.is_object() {
        return Err(protocol_error(name, "tools/list 返回无效 inputSchema"));
    }
    check_len(
        name,
        "input_schema_bytes",
        limits.max_input_schema_bytes,
        serialized_len(&tool.input_schema)?,
    )?;
    if let Some(output_schema) = &tool.output_schema {
        if !output_schema.is_object() {
            return Err(protocol_error(name, "tools/list 返回无效 outputSchema"));
        }
        check_len(
            name,
            "output_schema_bytes",
            limits.max_output_schema_bytes,
            serialized_len(output_schema)?,
        )?;
    }
    Ok(())
}

fn serialized_len(value: &impl Serialize) -> Result<usize> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|_| McpError::InvalidConfig {
            message: "MCP payload 序列化失败".to_string(),
        })
}

fn check_len(name: &str, limit: &'static str, max: usize, actual: usize) -> Result<()> {
    if actual > max {
        Err(limit_error(name, limit, max, actual))
    } else {
        Ok(())
    }
}

fn protocol_error(name: &str, message: &str) -> McpError {
    McpError::Protocol {
        name: name.to_string(),
        message: message.to_string(),
        stderr: McpStderrSummary::default(),
    }
}

fn limit_error(name: &str, limit: &'static str, max: usize, actual: usize) -> McpError {
    McpError::LimitExceeded {
        name: name.to_string(),
        limit,
        max,
        actual,
        stderr: McpStderrSummary::default(),
    }
}

struct ResolvedHttpConfig {
    url: String,
    bearer_token: Option<String>,
    headers: HashMap<HeaderName, HeaderValue>,
    backend: BoundedHttpClient,
}

fn resolve_http_config(document: &McpDocument, limits: McpLimits) -> Result<ResolvedHttpConfig> {
    let config = document.http_config()?;
    let parsed = url::Url::parse(&config.url).map_err(|_| McpError::InvalidConfig {
        message: "HTTP MCP url 无效".to_string(),
    })?;
    let bearer_token = match config.bearer_token_env_var.as_deref() {
        Some(variable) => Some(read_env(document, variable)?),
        None => None,
    };
    let headers = resolve_headers(document, &config, bearer_token.is_some(), limits)?;
    let credential_header_bytes = header_bytes(&headers).saturating_add(
        bearer_token
            .as_ref()
            .map_or(0, |value| "authorization".len().saturating_add(value.len())),
    );
    if credential_header_bytes > limits.max_http_header_bytes {
        return Err(limit_error(
            document.name.as_str(),
            "http_header_bytes",
            limits.max_http_header_bytes,
            credential_header_bytes,
        ));
    }
    let mut builder = reqwest::Client::builder()
        .connect_timeout(INITIALIZE_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none());
    if should_bypass_proxy(&parsed) {
        builder = builder.no_proxy();
    }
    let client = builder.build().map_err(|_| McpError::HttpTransport {
        name: document.name.to_string(),
        operation: "client".to_string(),
        message: "无法创建 HTTP client".to_string(),
    })?;
    let state = BackendState::default();
    Ok(ResolvedHttpConfig {
        url: config.url,
        bearer_token,
        headers,
        backend: BoundedHttpClient {
            client,
            limits,
            state,
        },
    })
}

fn resolve_headers(
    document: &McpDocument,
    config: &McpHttpServerConfig,
    has_bearer: bool,
    limits: McpLimits,
) -> Result<HashMap<HeaderName, HeaderValue>> {
    let mut headers = HashMap::new();
    let mut seen = BTreeSet::new();
    for (name, value) in &config.http_headers {
        insert_header(&mut headers, &mut seen, name, value, has_bearer)?;
    }
    for (name, variable) in &config.env_http_headers {
        let value = read_env(document, variable)?;
        insert_header(&mut headers, &mut seen, name, &value, has_bearer)?;
    }
    let bytes = header_bytes(&headers);
    if bytes > limits.max_http_header_bytes {
        return Err(limit_error(
            document.name.as_str(),
            "http_header_bytes",
            limits.max_http_header_bytes,
            bytes,
        ));
    }
    Ok(headers)
}

fn insert_header(
    headers: &mut HashMap<HeaderName, HeaderValue>,
    seen: &mut BTreeSet<String>,
    name: &str,
    value: &str,
    has_bearer: bool,
) -> Result<()> {
    let normalized = name.to_ascii_lowercase();
    if !seen.insert(normalized.clone()) {
        return Err(McpError::InvalidConfig {
            message: format!("HTTP MCP header `{name}` 重复"),
        });
    }
    if matches!(
        normalized.as_str(),
        "accept"
            | "connection"
            | "content-length"
            | "content-type"
            | "host"
            | "proxy-authorization"
            | "transfer-encoding"
            | HEADER_SESSION_ID
            | "mcp-protocol-version"
            | HEADER_LAST_EVENT_ID
    ) || (has_bearer && normalized == "authorization")
    {
        return Err(McpError::InvalidConfig {
            message: format!("HTTP MCP header `{name}` 与协议或 bearer 配置冲突"),
        });
    }
    let header_name =
        HeaderName::from_bytes(name.as_bytes()).map_err(|_| McpError::InvalidConfig {
            message: format!("HTTP MCP header name 无效：{name}"),
        })?;
    let header_value = HeaderValue::from_str(value).map_err(|_| McpError::InvalidConfig {
        message: format!("HTTP MCP header `{name}` value 无效"),
    })?;
    headers.insert(header_name, header_value);
    Ok(())
}

fn read_env(document: &McpDocument, variable: &str) -> Result<String> {
    std::env::var(variable).map_err(|_| McpError::MissingCredential {
        name: document.name.to_string(),
        variable: variable.to_string(),
    })
}

fn should_bypass_proxy(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => IpAddr::V4(ip).is_loopback(),
        Some(url::Host::Ipv6(ip)) => IpAddr::V6(ip).is_loopback(),
        None => false,
    }
}

#[derive(Clone)]
struct BoundedHttpClient {
    client: reqwest::Client,
    limits: McpLimits,
    state: BackendState,
}

impl fmt::Debug for BoundedHttpClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("BoundedHttpClient").finish()
    }
}

#[derive(Clone, Default)]
struct BackendState(Arc<Mutex<Option<BackendFailure>>>);

impl BackendState {
    fn record(&self, failure: BackendFailure) {
        if let Ok(mut state) = self.0.lock() {
            *state = Some(failure);
        }
    }

    fn take(&self) -> Option<BackendFailure> {
        self.0.lock().ok()?.take()
    }
}

#[derive(Clone, Debug)]
enum BackendFailure {
    Authentication(u16),
    Limit {
        limit: &'static str,
        max: usize,
        actual: usize,
    },
    Transport,
}

#[derive(Debug, thiserror::Error)]
enum HttpBackendError {
    #[error("HTTP request failed")]
    Request,
    #[error("HTTP response exceeded configured limit")]
    Limit,
}

#[derive(Debug, thiserror::Error)]
enum BoundedStreamError {
    #[error("HTTP stream failed")]
    Request,
    #[error("SSE event exceeded configured limit")]
    Limit,
}

impl StreamableHttpClient for BoundedHttpClient {
    type Error = HttpBackendError;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> std::result::Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let mut request = self.client.post(uri.as_ref()).header(
            ACCEPT,
            format!("{JSON_MIME_TYPE}, {EVENT_STREAM_MIME_TYPE}"),
        );
        if let Some(token) = auth_header {
            request = request.bearer_auth(token);
        }
        request = apply_headers(request, custom_headers);
        let had_session = session_id.is_some();
        if let Some(session_id) = session_id {
            request = request.header(HEADER_SESSION_ID, session_id.as_ref());
        }
        let response = request.json(&message).send().await.map_err(|_| {
            self.state.record(BackendFailure::Transport);
            StreamableHttpError::Client(HttpBackendError::Request)
        })?;
        self.validate_headers(&response)?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            self.state.record(BackendFailure::Authentication(401));
            return Err(StreamableHttpError::AuthRequired(AuthRequiredError::new(
                "Bearer".to_string(),
            )));
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            self.state.record(BackendFailure::Authentication(403));
            return Err(StreamableHttpError::InsufficientScope(
                InsufficientScopeError::new("Bearer".to_string(), None),
            ));
        }
        if status == reqwest::StatusCode::NOT_FOUND && had_session {
            return Err(StreamableHttpError::SessionExpired);
        }
        if matches!(
            status,
            reqwest::StatusCode::ACCEPTED | reqwest::StatusCode::NO_CONTENT
        ) {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        if !status.is_success() {
            self.state.record(BackendFailure::Transport);
            return Err(StreamableHttpError::UnexpectedServerResponse(Cow::Owned(
                format!("HTTP {status}"),
            )));
        }
        let content_type = content_type(&response);
        let session_id = response
            .headers()
            .get(HEADER_SESSION_ID)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        match content_type.as_deref() {
            Some(value) if value.starts_with(EVENT_STREAM_MIME_TYPE) => {
                let stream = self.sse_stream(response);
                Ok(StreamableHttpPostResponse::Sse(stream, session_id))
            }
            Some(value) if value.starts_with(JSON_MIME_TYPE) => {
                let bytes = self.read_body(response).await?;
                if bytes.is_empty()
                    && matches!(
                        message,
                        ClientJsonRpcMessage::Notification(_)
                            | ClientJsonRpcMessage::Response(_)
                            | ClientJsonRpcMessage::Error(_)
                    )
                {
                    return Ok(StreamableHttpPostResponse::Accepted);
                }
                let message =
                    serde_json::from_slice::<ServerJsonRpcMessage>(&bytes).map_err(|_| {
                        self.state.record(BackendFailure::Transport);
                        StreamableHttpError::UnexpectedServerResponse(Cow::Borrowed(
                            "invalid JSON-RPC response",
                        ))
                    })?;
                Ok(StreamableHttpPostResponse::Json(message, session_id))
            }
            _ => Err(StreamableHttpError::UnexpectedContentType(content_type)),
        }
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> std::result::Result<(), StreamableHttpError<Self::Error>> {
        let mut request = self
            .client
            .delete(uri.as_ref())
            .header(HEADER_SESSION_ID, session_id.as_ref());
        if let Some(token) = auth_header {
            request = request.bearer_auth(token);
        }
        request = apply_headers(request, custom_headers);
        let response = request.send().await.map_err(|_| {
            self.state.record(BackendFailure::Transport);
            StreamableHttpError::Client(HttpBackendError::Request)
        })?;
        self.validate_headers(&response)?;
        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Ok(());
        }
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.state.record(BackendFailure::Authentication(401));
            return Err(StreamableHttpError::AuthRequired(AuthRequiredError::new(
                "Bearer".to_string(),
            )));
        }
        if !response.status().is_success() {
            self.state.record(BackendFailure::Transport);
            return Err(StreamableHttpError::UnexpectedServerResponse(
                Cow::Borrowed("HTTP session cleanup failed"),
            ));
        }
        Ok(())
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> std::result::Result<
        BoxStream<'static, std::result::Result<Sse, sse_stream::Error>>,
        StreamableHttpError<Self::Error>,
    > {
        let mut request = self
            .client
            .get(uri.as_ref())
            .header(ACCEPT, EVENT_STREAM_MIME_TYPE)
            .header(HEADER_SESSION_ID, session_id.as_ref());
        if let Some(last_event_id) = last_event_id {
            request = request.header(HEADER_LAST_EVENT_ID, last_event_id);
        }
        if let Some(token) = auth_header {
            request = request.bearer_auth(token);
        }
        request = apply_headers(request, custom_headers);
        let response = request.send().await.map_err(|_| {
            self.state.record(BackendFailure::Transport);
            StreamableHttpError::Client(HttpBackendError::Request)
        })?;
        self.validate_headers(&response)?;
        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Err(StreamableHttpError::ServerDoesNotSupportSse);
        }
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.state.record(BackendFailure::Authentication(401));
            return Err(StreamableHttpError::AuthRequired(AuthRequiredError::new(
                "Bearer".to_string(),
            )));
        }
        if !response.status().is_success() {
            self.state.record(BackendFailure::Transport);
            return Err(StreamableHttpError::UnexpectedServerResponse(
                Cow::Borrowed("HTTP SSE connection failed"),
            ));
        }
        let content_type = content_type(&response);
        if !content_type
            .as_deref()
            .is_some_and(|value| value.starts_with(EVENT_STREAM_MIME_TYPE))
        {
            return Err(StreamableHttpError::UnexpectedContentType(content_type));
        }
        Ok(self.sse_stream(response))
    }
}

impl BoundedHttpClient {
    fn validate_headers(
        &self,
        response: &reqwest::Response,
    ) -> std::result::Result<(), StreamableHttpError<HttpBackendError>> {
        let actual = response
            .headers()
            .iter()
            .map(|(name, value)| name.as_str().len().saturating_add(value.as_bytes().len()))
            .sum::<usize>();
        if actual > self.limits.max_http_header_bytes {
            self.state.record(BackendFailure::Limit {
                limit: "http_header_bytes",
                max: self.limits.max_http_header_bytes,
                actual,
            });
            return Err(StreamableHttpError::Client(HttpBackendError::Limit));
        }
        if let Some(length) = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok())
            && length > self.limits.max_http_body_bytes
        {
            self.state.record(BackendFailure::Limit {
                limit: "http_body_bytes",
                max: self.limits.max_http_body_bytes,
                actual: length,
            });
            return Err(StreamableHttpError::Client(HttpBackendError::Limit));
        }
        Ok(())
    }

    async fn read_body(
        &self,
        response: reqwest::Response,
    ) -> std::result::Result<Vec<u8>, StreamableHttpError<HttpBackendError>> {
        let mut output = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| {
                self.state.record(BackendFailure::Transport);
                StreamableHttpError::Client(HttpBackendError::Request)
            })?;
            let actual = output.len().saturating_add(chunk.len());
            if actual > self.limits.max_http_body_bytes {
                self.state.record(BackendFailure::Limit {
                    limit: "http_body_bytes",
                    max: self.limits.max_http_body_bytes,
                    actual,
                });
                return Err(StreamableHttpError::Client(HttpBackendError::Limit));
            }
            output.extend_from_slice(&chunk);
        }
        Ok(output)
    }

    fn sse_stream(
        &self,
        response: reqwest::Response,
    ) -> BoxStream<'static, std::result::Result<Sse, sse_stream::Error>> {
        let state = self.state.clone();
        let max = self.limits.max_sse_event_bytes;
        let bytes = response
            .bytes_stream()
            .scan(SseByteLimit::default(), move |counter, item| {
                let result = match item {
                    Ok(bytes) => match counter.check(&bytes, max) {
                        Ok(()) => Ok(bytes),
                        Err(actual) => {
                            state.record(BackendFailure::Limit {
                                limit: "sse_event_bytes",
                                max,
                                actual,
                            });
                            Err(BoundedStreamError::Limit)
                        }
                    },
                    Err(_) => {
                        state.record(BackendFailure::Transport);
                        Err(BoundedStreamError::Request)
                    }
                };
                ready(Some(result))
            });
        SseStream::from_bytes_stream(bytes).boxed()
    }
}

#[derive(Default)]
struct SseByteLimit {
    event_bytes: usize,
    line_has_content: bool,
}

impl SseByteLimit {
    fn check(&mut self, bytes: &[u8], max: usize) -> std::result::Result<(), usize> {
        for byte in bytes {
            self.event_bytes = self.event_bytes.saturating_add(1);
            if self.event_bytes > max {
                return Err(self.event_bytes);
            }
            match byte {
                b'\n' => {
                    if !self.line_has_content {
                        self.event_bytes = 0;
                    }
                    self.line_has_content = false;
                }
                b'\r' => {}
                _ => self.line_has_content = true,
            }
        }
        Ok(())
    }
}

fn apply_headers(
    mut request: reqwest::RequestBuilder,
    headers: HashMap<HeaderName, HeaderValue>,
) -> reqwest::RequestBuilder {
    for (name, value) in headers {
        request = request.header(name, value);
    }
    request
}

fn content_type(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get(CONTENT_TYPE)
        .map(|value| String::from_utf8_lossy(value.as_bytes()).to_string())
}

fn header_bytes(headers: &HashMap<HeaderName, HeaderValue>) -> usize {
    headers
        .iter()
        .map(|(name, value)| name.as_str().len().saturating_add(value.as_bytes().len()))
        .sum()
}

fn map_backend_error(name: &str, operation: &str, state: &BackendState) -> McpError {
    match state.take() {
        Some(BackendFailure::Authentication(status)) => McpError::AuthenticationRequired {
            name: name.to_string(),
            status,
        },
        Some(BackendFailure::Limit { limit, max, actual }) => limit_error(name, limit, max, actual),
        Some(BackendFailure::Transport) | None => McpError::HttpTransport {
            name: name.to_string(),
            operation: operation.to_string(),
            message: "请求失败，远端响应内容已省略".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::OnceLock;
    use std::sync::atomic::AtomicUsize;

    use serde_json::json;

    use super::*;
    use crate::protocol::MCP_PROTOCOL_VERSION;

    #[derive(Clone, Copy)]
    enum ResponseMode {
        Json,
        Sse,
    }

    struct TestServer {
        address: SocketAddr,
        stop: Arc<AtomicBool>,
        state: Arc<TestServerState>,
        thread: Option<thread::JoinHandle<()>>,
    }

    #[derive(Default)]
    struct TestServerState {
        initialize_count: AtomicUsize,
        list_count: AtomicUsize,
        delete_count: AtomicUsize,
        missing_session_count: AtomicUsize,
        requests: Mutex<Vec<String>>,
    }

    impl TestServerState {
        fn record(&self, http_method: &str, rpc_method: &str, status: u16) {
            self.requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(format!("{http_method} {rpc_method} -> {status}"));
        }

        fn snapshot(&self) -> String {
            let requests = self
                .requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            format!(
                "initialize={}, list={}, delete={}, missing_session={}, requests={requests:?}",
                self.initialize_count.load(Ordering::SeqCst),
                self.list_count.load(Ordering::SeqCst),
                self.delete_count.load(Ordering::SeqCst),
                self.missing_session_count.load(Ordering::SeqCst),
            )
        }
    }

    impl TestServer {
        fn start(
            mode: ResponseMode,
            stateful: bool,
            expire_once: bool,
            require_auth: bool,
        ) -> Self {
            Self::start_with_call_delay(mode, stateful, expire_once, require_auth, None)
        }

        fn start_with_call_delay(
            mode: ResponseMode,
            stateful: bool,
            expire_once: bool,
            require_auth: bool,
            call_delay: Option<Duration>,
        ) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
            let address = listener.local_addr().expect("local addr");
            listener.set_nonblocking(true).expect("nonblocking");
            let stop = Arc::new(AtomicBool::new(false));
            let state = Arc::new(TestServerState::default());
            let worker_stop = Arc::clone(&stop);
            let worker_state = Arc::clone(&state);
            let thread = thread::spawn(move || {
                let mut handlers = Vec::new();
                while !worker_stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let state = Arc::clone(&worker_state);
                            handlers.push(thread::spawn(move || {
                                handle_test_request(
                                    stream,
                                    mode,
                                    stateful,
                                    expire_once,
                                    require_auth,
                                    call_delay,
                                    &state,
                                );
                            }));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                for handler in handlers {
                    let _ = handler.join();
                }
            });
            Self {
                address,
                stop,
                state,
                thread: Some(thread),
            }
        }

        fn url(&self) -> String {
            format!("http://{}/mcp", self.address)
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    struct TestRequest {
        method: String,
        headers: HashMap<String, String>,
        body: Vec<u8>,
    }

    fn http_test_guard() -> std::sync::MutexGuard<'static, ()> {
        static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
        GUARD
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn handle_test_request(
        mut stream: TcpStream,
        mode: ResponseMode,
        stateful: bool,
        expire_once: bool,
        require_auth: bool,
        call_delay: Option<Duration>,
        state: &TestServerState,
    ) {
        let Some(request) = read_test_request(&mut stream) else {
            return;
        };
        let value: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let method = value.get("method").and_then(Value::as_str).unwrap_or("");
        if require_auth
            && (request.headers.get("authorization").map(String::as_str) != Some("Bearer token")
                || request.headers.get("x-account").map(String::as_str) != Some("account"))
        {
            state.record(&request.method, method, 401);
            write_test_response(&mut stream, 401, "application/json", &[], None);
            return;
        }
        if request.method == "GET" {
            state.record(&request.method, method, 405);
            write_test_response(&mut stream, 405, "text/plain", &[], None);
            return;
        }
        if request.method == "DELETE" {
            state.delete_count.fetch_add(1, Ordering::SeqCst);
            state.record(&request.method, method, 204);
            write_test_response(&mut stream, 204, "application/json", &[], None);
            return;
        }
        if stateful && method != "initialize" && method != "notifications/initialized" {
            let expected = format!("session-{}", state.initialize_count.load(Ordering::SeqCst));
            if request.headers.get(HEADER_SESSION_ID).map(String::as_str) != Some(expected.as_str())
            {
                state.missing_session_count.fetch_add(1, Ordering::SeqCst);
            }
        }
        match method {
            "initialize" => {
                let count = state.initialize_count.fetch_add(1, Ordering::SeqCst) + 1;
                state.record(&request.method, method, 200);
                let body = json!({
                    "jsonrpc": "2.0",
                    "id": value["id"],
                    "result": {
                        "protocolVersion": MCP_PROTOCOL_VERSION,
                        "capabilities": {"tools": {"listChanged": true}},
                        "serverInfo": {"name": "test-http", "version": "1.0.0"}
                    }
                });
                write_protocol_response(
                    &mut stream,
                    mode,
                    &body,
                    stateful.then(|| format!("session-{count}")),
                );
            }
            "notifications/initialized" => {
                state.record(&request.method, method, 202);
                write_test_response(&mut stream, 202, "application/json", &[], None);
            }
            "tools/list" => {
                let count = state.list_count.fetch_add(1, Ordering::SeqCst) + 1;
                if expire_once && count == 1 {
                    state.record(&request.method, method, 404);
                    write_test_response(&mut stream, 404, "application/json", &[], None);
                    return;
                }
                state.record(&request.method, method, 200);
                let body = json!({
                    "jsonrpc": "2.0",
                    "id": value["id"],
                    "result": {"tools": [{
                        "name": "lookup",
                        "description": "Lookup",
                        "inputSchema": {"type": "object"}
                    }]}
                });
                match mode {
                    ResponseMode::Json => write_protocol_response(&mut stream, mode, &body, None),
                    ResponseMode::Sse => {
                        let notification = json!({
                            "jsonrpc": "2.0",
                            "method": "notifications/tools/list_changed"
                        });
                        let response = format!("data: {}\n\ndata: {}\n\n", notification, body);
                        write_test_response(
                            &mut stream,
                            200,
                            EVENT_STREAM_MIME_TYPE,
                            response.as_bytes(),
                            None,
                        );
                    }
                }
            }
            "tools/call" => {
                if let Some(delay) = call_delay {
                    thread::sleep(delay);
                }
                state.record(&request.method, method, 200);
                let body = json!({
                    "jsonrpc": "2.0",
                    "id": value["id"],
                    "result": {
                        "content": [{"type": "text", "text": "pong"}],
                        "structuredContent": {"ok": true},
                        "isError": false
                    }
                });
                write_protocol_response(&mut stream, mode, &body, None);
            }
            _ => {
                state.record(&request.method, method, 400);
                write_test_response(&mut stream, 400, "application/json", &[], None);
            }
        }
    }

    fn read_test_request(stream: &mut TcpStream) -> Option<TestRequest> {
        loop {
            match stream.set_nonblocking(false) {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return None,
            }
        }
        loop {
            match stream.set_read_timeout(Some(Duration::from_secs(2))) {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return None,
            }
        }
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 4096];
        let header_end = loop {
            let read = read_test_chunk(stream, &mut buffer)?;
            if read == 0 {
                return None;
            }
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let header_text = std::str::from_utf8(&bytes[..header_end]).ok()?;
        let mut lines = header_text.split("\r\n");
        let method = lines.next()?.split_whitespace().next()?.to_string();
        let mut headers = HashMap::new();
        for line in lines {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
        let content_length = headers
            .get("content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        while bytes.len().saturating_sub(header_end) < content_length {
            let read = read_test_chunk(stream, &mut buffer)?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        Some(TestRequest {
            method,
            headers,
            body: bytes[header_end..header_end + content_length].to_vec(),
        })
    }

    fn read_test_chunk(stream: &mut TcpStream, buffer: &mut [u8]) -> Option<usize> {
        loop {
            match stream.read(buffer) {
                Ok(read) => return Some(read),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return None,
            }
        }
    }

    fn write_protocol_response(
        stream: &mut TcpStream,
        mode: ResponseMode,
        value: &Value,
        session_id: Option<String>,
    ) {
        match mode {
            ResponseMode::Json => write_test_response(
                stream,
                200,
                JSON_MIME_TYPE,
                value.to_string().as_bytes(),
                session_id.as_deref(),
            ),
            ResponseMode::Sse => {
                let body = format!("data: {}\n\n", value);
                write_test_response(
                    stream,
                    200,
                    EVENT_STREAM_MIME_TYPE,
                    body.as_bytes(),
                    session_id.as_deref(),
                );
            }
        }
    }

    fn write_test_response(
        stream: &mut TcpStream,
        status: u16,
        content_type: &str,
        body: &[u8],
        session_id: Option<&str>,
    ) {
        let reason = match status {
            200 => "OK",
            202 => "Accepted",
            204 => "No Content",
            400 => "Bad Request",
            401 => "Unauthorized",
            404 => "Not Found",
            405 => "Method Not Allowed",
            _ => "Error",
        };
        let session = session_id
            .map(|value| format!("Mcp-Session-Id: {value}\r\n"))
            .unwrap_or_default();
        let headers = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{session}Connection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(headers.as_bytes()).expect("headers");
        stream.write_all(body).expect("body");
        stream.flush().expect("flush");
    }

    fn http_document(url: &str, extra: Value) -> McpDocument {
        let mut server = extra.as_object().cloned().unwrap_or_default();
        server.insert("url".to_string(), Value::String(url.to_string()));
        McpDocument::from_value(json!({"mcpServers": {"remote": server}})).unwrap()
    }

    #[test]
    fn sse_byte_limit_resets_at_event_boundary() {
        let mut limit = SseByteLimit::default();
        assert_eq!(limit.check(b"data: one\n\n", 12), Ok(()));
        assert_eq!(limit.check(b"data: two\n\n", 12), Ok(()));
        assert_eq!(limit.check(b"data: too-large\n", 8), Err(9));
    }

    #[test]
    fn loopback_urls_bypass_proxy() {
        assert!(should_bypass_proxy(
            &url::Url::parse("http://localhost:1/mcp").unwrap()
        ));
        assert!(should_bypass_proxy(
            &url::Url::parse("http://127.0.0.1:1/mcp").unwrap()
        ));
        assert!(should_bypass_proxy(
            &url::Url::parse("http://[::1]:1/mcp").unwrap()
        ));
        assert!(!should_bypass_proxy(
            &url::Url::parse("https://example.com/mcp").unwrap()
        ));
    }

    #[test]
    fn http_client_supports_json_stateless_tools_and_calls() {
        let _guard = http_test_guard();
        let server = TestServer::start(ResponseMode::Json, false, false, false);
        let document = http_document(&server.url(), json!({}));
        let mut client =
            HttpMcpClient::start_with_limits(&document, McpLimits::default()).expect("connect");

        let tools = client.list_tools().expect("list tools");
        assert_eq!(tools[0].name, "lookup");
        let result = client.call_tool("lookup", json!({})).expect("call tool");
        assert_eq!(result.content[0].text.as_deref(), Some("pong"));
        client.close().expect("close");
        assert_eq!(server.state.delete_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn http_tool_call_cancellation_interrupts_wait_and_closes_client() {
        let _guard = http_test_guard();
        let server = TestServer::start_with_call_delay(
            ResponseMode::Json,
            false,
            false,
            false,
            Some(Duration::from_millis(300)),
        );
        let document = http_document(&server.url(), json!({}));
        let mut client =
            HttpMcpClient::start_with_limits(&document, McpLimits::default()).expect("connect");
        let cancellation = CancellationToken::new();
        let cancel = cancellation.clone();
        let canceler = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            cancel.cancel();
        });
        let started = Instant::now();

        let error = client
            .call_tool_with_cancellation("lookup", json!({}), &cancellation)
            .expect_err("call must be canceled");

        assert!(matches!(error, McpError::Canceled { .. }));
        assert!(started.elapsed() < Duration::from_secs(1));
        canceler.join().unwrap();
    }

    #[test]
    fn http_client_supports_sse_session_cleanup_and_expiry_recovery() {
        let _guard = http_test_guard();
        let server = TestServer::start(ResponseMode::Sse, true, true, false);
        let document = http_document(&server.url(), json!({}));
        let mut client =
            HttpMcpClient::start_with_limits(&document, McpLimits::default()).expect("connect");

        let tools = client
            .list_tools()
            .unwrap_or_else(|error| panic!("list: {error:?}; {}", server.state.snapshot()));
        assert_eq!(tools.len(), 1);
        assert_eq!(
            client.poll_event().expect("event"),
            Some(McpClientEvent::ToolsListChanged)
        );
        client.close().expect("close");
        assert_eq!(server.state.initialize_count.load(Ordering::SeqCst), 2);
        assert_eq!(server.state.list_count.load(Ordering::SeqCst), 2);
        assert_eq!(server.state.missing_session_count.load(Ordering::SeqCst), 0);
        assert_eq!(server.state.delete_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn http_credentials_are_resolved_and_auth_errors_are_typed() {
        let _guard = http_test_guard();
        let server = TestServer::start(ResponseMode::Json, false, false, true);
        unsafe {
            std::env::set_var("AGT_HTTP_TEST_TOKEN", "token");
            std::env::set_var("AGT_HTTP_TEST_ACCOUNT", "account");
        }
        let document = http_document(
            &server.url(),
            json!({
                "bearer_token_env_var": "AGT_HTTP_TEST_TOKEN",
                "env_http_headers": {"X-Account": "AGT_HTTP_TEST_ACCOUNT"}
            }),
        );
        let mut client = HttpMcpClient::start_with_limits(&document, McpLimits::default())
            .expect("authenticated connect");
        assert_eq!(client.list_tools().expect("list").len(), 1);
        client.close().expect("close");
        unsafe {
            std::env::remove_var("AGT_HTTP_TEST_TOKEN");
            std::env::remove_var("AGT_HTTP_TEST_ACCOUNT");
        }

        let unauthenticated = http_document(&server.url(), json!({}));
        let error = HttpMcpClient::start_with_limits(&unauthenticated, McpLimits::default())
            .expect_err("auth required");
        assert!(
            matches!(error, McpError::AuthenticationRequired { status: 401, .. }),
            "unexpected error: {error:?}; {}",
            server.state.snapshot()
        );
        assert!(!error.to_string().contains("token"));
        assert!(!error.to_string().contains("account"));
    }
}
