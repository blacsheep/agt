use std::collections::{HashSet, VecDeque};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use agt_core::CancellationToken;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::client::McpClient;
use crate::config::McpDocument;
use crate::error::{McpError, McpStderrSummary, Result};
use crate::limits::McpLimits;
use crate::protocol::{
    MCP_PROTOCOL_VERSION, McpClientEvent, McpInitializeInfo, McpTool, McpToolCallResult,
};

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug)]
pub struct MinimalStdioMcpClient {
    connection: StdioConnection,
    initialize_info: McpInitializeInfo,
}

impl MinimalStdioMcpClient {
    #[cfg(test)]
    fn start(document: &McpDocument) -> Result<Self> {
        Self::start_with_limits(document, McpLimits::default())
    }

    pub fn start_with_limits(document: &McpDocument, limits: McpLimits) -> Result<Self> {
        let config = document.resolve_stdio_server()?;
        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .current_dir(&config.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in &config.env {
            command.env(key, value);
        }

        let mut child = command.spawn().map_err(|source| McpError::Startup {
            name: document.name.to_string(),
            command: config.command.clone(),
            source,
        })?;
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                terminate_spawned_child(&mut child);
                return Err(McpError::Protocol {
                    name: document.name.to_string(),
                    message: "stdio MCP 缺少 stdout pipe".to_string(),
                    stderr: McpStderrSummary::default(),
                });
            }
        };
        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                terminate_spawned_child(&mut child);
                return Err(McpError::Protocol {
                    name: document.name.to_string(),
                    message: "stdio MCP 缺少 stdin pipe".to_string(),
                    stderr: McpStderrSummary::default(),
                });
            }
        };
        let stderr = child.stderr.take();
        let stderr_state = Arc::new(StderrState::default());
        let (sender, messages) = mpsc::channel();
        spawn_stdout_reader(stdout, sender, limits);
        spawn_stderr_reader(stderr, Arc::clone(&stderr_state), limits.max_stderr_bytes);

        let mut connection = StdioConnection {
            name: document.name.to_string(),
            child: Some(child),
            writer: Some(stdin),
            messages,
            pending_messages: VecDeque::new(),
            events: VecDeque::new(),
            stderr_state,
            limits,
            next_id: 1,
        };
        let initialize_info = connection.initialize()?;
        Ok(Self {
            connection,
            initialize_info,
        })
    }
}

impl McpClient for MinimalStdioMcpClient {
    fn initialize_info(&self) -> &McpInitializeInfo {
        &self.initialize_info
    }

    fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        let result = (|| {
            if self.initialize_info.capabilities.tools.is_none() {
                return Err(McpError::MissingCapability {
                    name: self.connection.name.clone(),
                    capability: "tools",
                    stderr: self.connection.stderr_summary(),
                });
            }

            let mut tools = Vec::new();
            let mut cursor = None;
            let mut seen_cursors = HashSet::new();
            let mut pages = 0usize;
            let mut metadata_bytes = 0usize;

            loop {
                if pages >= self.connection.limits.max_list_pages {
                    return Err(self.connection.limit_error(
                        "list_pages",
                        self.connection.limits.max_list_pages,
                        pages.saturating_add(1),
                    ));
                }
                pages += 1;
                let params = match &cursor {
                    Some(cursor) => json!({ "cursor": cursor }),
                    None => Value::Null,
                };
                let response =
                    self.connection
                        .send_request("tools/list", params, REQUEST_TIMEOUT)?;
                let page: ListToolsResultPage =
                    serde_json::from_value(response).map_err(|source| {
                        self.connection
                            .protocol_error(format!("tools/list 返回格式无效：{source}"))
                    })?;
                if page.tools.len() > self.connection.limits.max_tools_per_page {
                    return Err(self.connection.limit_error(
                        "tools_per_page",
                        self.connection.limits.max_tools_per_page,
                        page.tools.len(),
                    ));
                }

                for tool in page.tools {
                    self.connection.validate_tool(&tool)?;
                    let tool_bytes = serialized_len(&tool).map_err(|source| {
                        self.connection
                            .protocol_error(format!("序列化 tool metadata 失败：{source}"))
                    })?;
                    metadata_bytes = metadata_bytes.saturating_add(tool_bytes);
                    if metadata_bytes > self.connection.limits.max_tool_metadata_bytes {
                        return Err(self.connection.limit_error(
                            "tool_metadata_bytes",
                            self.connection.limits.max_tool_metadata_bytes,
                            metadata_bytes,
                        ));
                    }
                    if tools.len() >= self.connection.limits.max_tools_total {
                        return Err(self.connection.limit_error(
                            "tools_total",
                            self.connection.limits.max_tools_total,
                            tools.len().saturating_add(1),
                        ));
                    }
                    tools.push(tool);
                }

                match page.next_cursor {
                    Some(next_cursor) if !next_cursor.is_empty() => {
                        if !seen_cursors.insert(next_cursor.clone()) {
                            return Err(self.connection.protocol_error(
                                "tools/list 返回重复 nextCursor，已停止分页".to_string(),
                            ));
                        }
                        cursor = Some(next_cursor);
                    }
                    _ => break,
                }
            }
            Ok(tools)
        })();
        if result.is_err() {
            let _ = self.connection.close();
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
        let result = (|| {
            let response = self.connection.send_request_with_cancellation(
                "tools/call",
                json!({"name": name, "arguments": arguments}),
                REQUEST_TIMEOUT,
                cancellation,
            )?;
            let result_bytes = serialized_len(&response).map_err(|source| {
                self.connection
                    .protocol_error(format!("序列化 tools/call result 失败：{source}"))
            })?;
            if result_bytes > self.connection.limits.max_call_result_bytes {
                return Err(self.connection.limit_error(
                    "call_result_bytes",
                    self.connection.limits.max_call_result_bytes,
                    result_bytes,
                ));
            }
            serde_json::from_value(response).map_err(|source| {
                self.connection
                    .protocol_error(format!("tools/call 返回格式无效：{source}"))
            })
        })();
        if result.is_err() {
            let _ = self.connection.close();
        }
        result
    }

    fn poll_event(&mut self) -> Result<Option<McpClientEvent>> {
        let result = self.connection.poll_event();
        if result.is_err() {
            let _ = self.connection.close();
        }
        result
    }

    fn close(&mut self) -> Result<()> {
        self.connection.close()
    }
}

#[derive(Debug)]
struct StdioConnection {
    name: String,
    child: Option<Child>,
    writer: Option<ChildStdin>,
    messages: Receiver<std::result::Result<Value, TransportReadError>>,
    pending_messages: VecDeque<Value>,
    events: VecDeque<McpClientEvent>,
    stderr_state: Arc<StderrState>,
    limits: McpLimits,
    next_id: u64,
}

impl StdioConnection {
    fn initialize(&mut self) -> Result<McpInitializeInfo> {
        let response = self.send_request(
            "initialize",
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "agt",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }),
            INITIALIZE_TIMEOUT,
        )?;
        let initialize: McpInitializeInfo = serde_json::from_value(response)
            .map_err(|source| self.protocol_error(format!("initialize 返回格式无效：{source}")))?;
        if initialize.protocol_version != MCP_PROTOCOL_VERSION {
            return Err(McpError::UnsupportedProtocolVersion {
                name: self.name.clone(),
                version: initialize.protocol_version,
                stderr: self.stderr_summary(),
            });
        }
        self.send_notification("notifications/initialized", Value::Null)?;
        Ok(initialize)
    }

    fn send_request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        self.send_request_with_cancellation(method, params, timeout, &CancellationToken::default())
    }

    fn send_request_with_cancellation(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<Value> {
        let id = Value::from(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| self.protocol_error("JSON-RPC request id 已耗尽".to_string()))?;
        let mut message = serde_json::Map::new();
        message.insert("jsonrpc".to_string(), Value::String("2.0".to_string()));
        message.insert("id".to_string(), id.clone());
        message.insert("method".to_string(), Value::String(method.to_string()));
        if !params.is_null() {
            message.insert("params".to_string(), params);
        }
        self.write_message(&Value::Object(message))?;

        let deadline = Instant::now() + timeout;
        loop {
            if cancellation.is_canceled() {
                let _ = self.send_notification(
                    "notifications/cancelled",
                    json!({
                        "requestId": id,
                        "reason": "agt execution canceled",
                    }),
                );
                return Err(McpError::Canceled {
                    operation: method.to_string(),
                });
            }
            let Some(message) =
                self.receive_message_with_poll(deadline, method, CANCEL_POLL_INTERVAL)?
            else {
                continue;
            };
            let Some(object) = message.as_object() else {
                return Err(self.protocol_error("收到的 JSON-RPC 消息不是对象".to_string()));
            };
            if object.get("method").is_some() {
                self.handle_server_message(&message)?;
                continue;
            }
            let Some(response_id) = object.get("id") else {
                return Err(self.protocol_error("JSON-RPC response 缺少 id".to_string()));
            };
            if response_id != &id {
                continue;
            }
            if let Some(result) = object.get("result") {
                return Ok(result.clone());
            }
            if let Some(error) = object.get("error") {
                return Err(self.protocol_error(format!(
                    "{method} 返回 JSON-RPC error code {}（server message 已省略）",
                    json_rpc_error_code(error)
                )));
            }
            return Err(self.protocol_error(format!("{method} 返回消息缺少 result/error")));
        }
    }

    fn receive_message_with_poll(
        &mut self,
        deadline: Instant,
        operation: &str,
        max_wait: Duration,
    ) -> Result<Option<Value>> {
        if let Some(message) = self.pending_messages.pop_front() {
            return Ok(Some(message));
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(self.timeout_error(operation));
        };
        match self.messages.recv_timeout(remaining.min(max_wait)) {
            Ok(Ok(message)) => Ok(Some(message)),
            Ok(Err(TransportReadError::Io(source))) => {
                Err(self.protocol_error(format!("读取 MCP 消息失败：{source}")))
            }
            Ok(Err(TransportReadError::Limit { limit, max, actual })) => {
                Err(self.limit_error(limit, max, actual))
            }
            Err(RecvTimeoutError::Timeout) if Instant::now() < deadline => Ok(None),
            Err(RecvTimeoutError::Timeout) => Err(self.timeout_error(operation)),
            Err(RecvTimeoutError::Disconnected) => {
                Err(self.protocol_error("MCP 连接已关闭".to_string()))
            }
        }
    }

    fn send_notification(&mut self, method: &str, params: Value) -> Result<()> {
        let mut message = serde_json::Map::new();
        message.insert("jsonrpc".to_string(), Value::String("2.0".to_string()));
        message.insert("method".to_string(), Value::String(method.to_string()));
        if !params.is_null() {
            message.insert("params".to_string(), params);
        }
        self.write_message(&Value::Object(message))
    }

    fn write_message(&mut self, message: &Value) -> Result<()> {
        let stderr = self.stderr_summary();
        let writer = self.writer.as_mut().ok_or_else(|| McpError::Protocol {
            name: self.name.clone(),
            message: "stdio writer 不可用".to_string(),
            stderr,
        })?;
        serde_json::to_writer(&mut *writer, message).map_err(|source| McpError::Protocol {
            name: self.name.clone(),
            message: format!("写入 MCP 请求失败：{source}"),
            stderr,
        })?;
        writer
            .write_all(b"\n")
            .and_then(|_| writer.flush())
            .map_err(|source| McpError::Protocol {
                name: self.name.clone(),
                message: format!("刷新 MCP 请求失败：{source}"),
                stderr,
            })
    }

    fn handle_server_message(&mut self, message: &Value) -> Result<()> {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Err(self.protocol_error("server message 的 method 必须是字符串".to_string()));
        };
        if let Some(id) = message.get("id") {
            let response = if method == "ping" {
                json!({"jsonrpc": "2.0", "id": id.clone(), "result": {}})
            } else {
                json!({
                    "jsonrpc": "2.0",
                    "id": id.clone(),
                    "error": {
                        "code": -32601,
                        "message": format!("agt client 不支持 server request `{method}`"),
                    }
                })
            };
            return self.write_message(&response);
        }
        if method == "notifications/tools/list_changed" {
            self.enqueue_event(McpClientEvent::ToolsListChanged)?;
        }
        Ok(())
    }

    fn enqueue_event(&mut self, event: McpClientEvent) -> Result<()> {
        if self.events.len() >= self.limits.max_queued_events {
            return Err(self.limit_error(
                "queued_client_events",
                self.limits.max_queued_events,
                self.events.len().saturating_add(1),
            ));
        }
        self.events.push_back(event);
        Ok(())
    }

    fn poll_event(&mut self) -> Result<Option<McpClientEvent>> {
        if let Some(event) = self.events.pop_front() {
            return Ok(Some(event));
        }
        loop {
            match self.messages.try_recv() {
                Ok(Ok(message)) if message.get("method").is_some() => {
                    self.handle_server_message(&message)?;
                    if let Some(event) = self.events.pop_front() {
                        return Ok(Some(event));
                    }
                }
                Ok(Ok(message)) => {
                    self.pending_messages.push_back(message);
                    return Ok(None);
                }
                Ok(Err(TransportReadError::Io(source))) => {
                    return Err(self.protocol_error(format!("读取 MCP 消息失败：{source}")));
                }
                Ok(Err(TransportReadError::Limit { limit, max, actual })) => {
                    return Err(self.limit_error(limit, max, actual));
                }
                Err(TryRecvError::Empty) => return Ok(None),
                Err(TryRecvError::Disconnected) => return Ok(None),
            }
        }
    }

    fn validate_tool(&self, tool: &McpTool) -> Result<()> {
        if tool.name.is_empty() {
            return Err(self.protocol_error("tools/list 返回空 tool name".to_string()));
        }
        check_len(
            self,
            "tool_name_bytes",
            self.limits.max_tool_name_bytes,
            tool.name.len(),
        )?;
        if let Some(description) = &tool.description {
            check_len(
                self,
                "tool_description_bytes",
                self.limits.max_description_bytes,
                description.len(),
            )?;
        }
        if !tool.input_schema.is_object() {
            return Err(self.protocol_error(format!(
                "tools/list 返回的工具 `{}` 缺少有效 inputSchema 对象",
                tool.name
            )));
        }
        check_len(
            self,
            "input_schema_bytes",
            self.limits.max_input_schema_bytes,
            serialized_len(&tool.input_schema).map_err(|source| {
                self.protocol_error(format!("序列化 inputSchema 失败：{source}"))
            })?,
        )?;
        if let Some(output_schema) = &tool.output_schema {
            if !output_schema.is_object() {
                return Err(self.protocol_error(format!(
                    "tools/list 返回的工具 `{}` 的 outputSchema 必须是对象",
                    tool.name
                )));
            }
            check_len(
                self,
                "output_schema_bytes",
                self.limits.max_output_schema_bytes,
                serialized_len(output_schema).map_err(|source| {
                    self.protocol_error(format!("序列化 outputSchema 失败：{source}"))
                })?,
            )?;
        }
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        self.writer.take();
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        match child.try_wait() {
            Ok(Some(_)) => Ok(()),
            Ok(None) => {
                child.kill().map_err(|source| McpError::ProcessCleanup {
                    name: self.name.clone(),
                    source,
                })?;
                child.wait().map_err(|source| McpError::ProcessCleanup {
                    name: self.name.clone(),
                    source,
                })?;
                Ok(())
            }
            Err(source) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(McpError::ProcessCleanup {
                    name: self.name.clone(),
                    source,
                })
            }
        }
    }

    fn stderr_summary(&self) -> McpStderrSummary {
        self.stderr_state.summary()
    }

    fn protocol_error(&self, message: String) -> McpError {
        McpError::Protocol {
            name: self.name.clone(),
            message,
            stderr: self.stderr_summary(),
        }
    }

    fn limit_error(&self, limit: &'static str, max: usize, actual: usize) -> McpError {
        McpError::LimitExceeded {
            name: self.name.clone(),
            limit,
            max,
            actual,
            stderr: self.stderr_summary(),
        }
    }

    fn timeout_error(&self, operation: &str) -> McpError {
        McpError::Timeout {
            name: self.name.clone(),
            operation: operation.to_string(),
            stderr: self.stderr_summary(),
        }
    }
}

impl Drop for StdioConnection {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListToolsResultPage {
    #[serde(default)]
    tools: Vec<McpTool>,
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Debug)]
enum TransportReadError {
    Io(io::Error),
    Limit {
        limit: &'static str,
        max: usize,
        actual: usize,
    },
}

#[derive(Debug, Default)]
struct StderrState {
    had_output: AtomicBool,
    bytes_seen: AtomicUsize,
    truncated: AtomicBool,
}

impl StderrState {
    fn observe(&self, bytes: usize, max: usize) {
        if bytes == 0 {
            return;
        }
        self.had_output.store(true, Ordering::Relaxed);
        let previous = self
            .bytes_seen
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_add(bytes))
            })
            .unwrap_or_else(|value| value);
        if previous.saturating_add(bytes) > max {
            self.truncated.store(true, Ordering::Relaxed);
        }
    }

    fn summary(&self) -> McpStderrSummary {
        McpStderrSummary {
            bytes_seen: self.bytes_seen.load(Ordering::Relaxed),
            had_output: self.had_output.load(Ordering::Relaxed),
            truncated: self.truncated.load(Ordering::Relaxed),
        }
    }
}

fn spawn_stdout_reader(
    stdout: ChildStdout,
    sender: Sender<std::result::Result<Value, TransportReadError>>,
    limits: McpLimits,
) {
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            match read_next_message_bytes(&mut reader, limits) {
                Ok(Some(bytes)) => {
                    let parsed = serde_json::from_slice::<Value>(&bytes).map_err(|source| {
                        TransportReadError::Io(io::Error::new(io::ErrorKind::InvalidData, source))
                    });
                    if sender.send(parsed).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let _ = sender.send(Err(error));
                    break;
                }
            }
        }
    });
}

fn spawn_stderr_reader(stderr: Option<ChildStderr>, state: Arc<StderrState>, max: usize) {
    let Some(mut stderr) = stderr else {
        return;
    };
    thread::spawn(move || {
        let mut buffer = [0_u8; 8 * 1024];
        loop {
            match stderr.read(&mut buffer) {
                Ok(0) => break,
                Ok(bytes) => state.observe(bytes, max),
                Err(_) => {
                    state.had_output.store(true, Ordering::Relaxed);
                    break;
                }
            }
        }
    });
}

fn read_next_message_bytes(
    reader: &mut impl BufRead,
    limits: McpLimits,
) -> std::result::Result<Option<Vec<u8>>, TransportReadError> {
    loop {
        let Some(mut line) = read_bounded_line(reader, limits.max_frame_bytes, "frame_bytes")?
        else {
            return Ok(None);
        };
        trim_line_ending(&mut line);
        if line.is_empty() {
            continue;
        }
        if line.first() == Some(&b'{') {
            if line.len() > limits.max_frame_bytes {
                return Err(TransportReadError::Limit {
                    limit: "frame_bytes",
                    max: limits.max_frame_bytes,
                    actual: line.len(),
                });
            }
            return Ok(Some(line));
        }
        return Err(TransportReadError::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            "不支持的 MCP stdout framing（内容已省略）",
        )));
    }
}

fn read_bounded_line(
    reader: &mut impl BufRead,
    max: usize,
    limit: &'static str,
) -> std::result::Result<Option<Vec<u8>>, TransportReadError> {
    let mut output = Vec::new();
    loop {
        let buffer = reader.fill_buf().map_err(TransportReadError::Io)?;
        if buffer.is_empty() {
            return if output.is_empty() {
                Ok(None)
            } else {
                Ok(Some(output))
            };
        }
        let consumed = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |index| index + 1);
        let actual = output.len().saturating_add(consumed);
        if actual > max {
            return Err(TransportReadError::Limit { limit, max, actual });
        }
        output.extend_from_slice(&buffer[..consumed]);
        let found_newline = buffer.get(consumed.saturating_sub(1)) == Some(&b'\n');
        reader.consume(consumed);
        if found_newline {
            return Ok(Some(output));
        }
    }
}

fn trim_line_ending(line: &mut Vec<u8>) {
    if line.last() == Some(&b'\n') {
        line.pop();
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
}

fn serialized_len(value: &impl serde::Serialize) -> serde_json::Result<usize> {
    serde_json::to_vec(value).map(|bytes| bytes.len())
}

fn check_len(
    connection: &StdioConnection,
    limit: &'static str,
    max: usize,
    actual: usize,
) -> Result<()> {
    if actual > max {
        Err(connection.limit_error(limit, max, actual))
    } else {
        Ok(())
    }
}

fn json_rpc_error_code(error: &Value) -> String {
    error
        .get("code")
        .map(ToString::to_string)
        .unwrap_or_else(|| "<unknown>".to_string())
}

fn terminate_spawned_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Cursor;
    use std::path::{Path, PathBuf};
    use std::thread;

    use serde_json::json;

    use super::*;

    fn test_home(name: &str) -> PathBuf {
        let home =
            std::env::temp_dir().join(format!("agt-mcp-stdio-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        home
    }

    fn helper_document(workspace: &Path, mode: &str) -> McpDocument {
        let helper = workspace.join("helper.py");
        fs::write(&helper, helper_script_source()).expect("write helper");
        McpDocument::from_value(json!({
            "mcpServers": {
                "helper": {
                    "command": "python3",
                    "args": [helper.display().to_string()],
                    "cwd": workspace.display().to_string(),
                    "env": {
                        "AGT_MCP_TEST_MODE": mode,
                        "AGT_MCP_PID_FILE": "${workspace}/server.pid"
                    }
                }
            }
        }))
        .expect("document")
    }

    #[test]
    fn initializes_preserves_metadata_lists_calls_and_observes_event() {
        let workspace = test_home("roundtrip");
        fs::create_dir_all(&workspace).expect("workspace");
        let document = helper_document(&workspace, "normal");
        let mut client = MinimalStdioMcpClient::start(&document).expect("start");
        assert_eq!(
            client.initialize_info().protocol_version,
            MCP_PROTOCOL_VERSION
        );
        assert_eq!(
            client.initialize_info().server_info.title.as_deref(),
            Some("Helper")
        );
        assert_eq!(
            client.initialize_info().instructions.as_deref(),
            Some("Use carefully")
        );

        let tools = client.list_tools().expect("tools");
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].output_schema, Some(json!({"type": "object"})));
        assert_eq!(
            tools[0]
                .annotations
                .as_ref()
                .and_then(|value| value.read_only_hint),
            Some(true)
        );
        assert_eq!(tools[0].icons[0].src, "data:image/png;base64,AA==");
        assert_eq!(
            client.poll_event().expect("event"),
            Some(McpClientEvent::ToolsListChanged)
        );

        let result = client
            .call_tool("lookup", json!({"query": "ping"}))
            .expect("call");
        assert_eq!(result.content[0].kind, "text");
        assert_eq!(result.content[0].text.as_deref(), Some("pong"));
        assert_eq!(result.structured_content, Some(json!({"echo": "ping"})));
        client.close().expect("close");
        client.close().expect("idempotent close");
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn stdio_tool_call_cancellation_interrupts_wait_and_closes_child() {
        let workspace = test_home("cancel-call");
        fs::create_dir_all(&workspace).expect("workspace");
        let helper = workspace.join("cancel_helper.py");
        fs::write(
            &helper,
            r#"import json
import sys
import time

for raw_line in sys.stdin:
    message = json.loads(raw_line)
    method = message.get("method", "")
    if method == "initialize":
        response = {"jsonrpc":"2.0","id":message["id"],"result":{
            "protocolVersion":"2025-11-25","capabilities":{"tools":{}},
            "serverInfo":{"name":"cancel-helper","version":"1"}}}
        print(json.dumps(response), flush=True)
    elif method == "notifications/initialized":
        continue
    elif method == "tools/call":
        time.sleep(10)
"#,
        )
        .expect("write helper");
        let document = McpDocument::from_value(json!({
            "mcpServers": {
                "helper": {
                    "command": "python3",
                    "args": [helper.display().to_string()],
                    "cwd": workspace.display().to_string()
                }
            }
        }))
        .expect("document");
        let mut client = MinimalStdioMcpClient::start(&document).expect("start");
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
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn starts_process_in_configured_absolute_cwd() {
        let workspace = test_home("cwd");
        let server_cwd = workspace.join("nested/server");
        fs::create_dir_all(&server_cwd).expect("server cwd");
        let helper = workspace.join("helper.py");
        fs::write(&helper, helper_script_source()).expect("write helper");
        let document = McpDocument::from_value(json!({
            "mcpServers": {
                "helper": {
                    "command": "python3",
                    "args": [helper.display().to_string()],
                    "cwd": server_cwd.display().to_string(),
                    "env": {
                        "AGT_MCP_TEST_MODE": "normal",
                        "AGT_MCP_PID_FILE": "${workspace}/server.pid"
                    }
                }
            }
        }))
        .expect("document");

        let mut client = MinimalStdioMcpClient::start(&document).expect("start");
        assert!(server_cwd.join("server.pid").is_file());
        client.close().expect("close");
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn rejects_unsupported_version_and_missing_tools_capability() {
        for (mode, expected) in [
            ("bad-version", "协议版本不受支持"),
            ("missing-tools", "capability 缺失"),
        ] {
            let workspace = test_home(mode);
            fs::create_dir_all(&workspace).expect("workspace");
            let document = helper_document(&workspace, mode);
            if mode == "bad-version" {
                let error =
                    MinimalStdioMcpClient::start(&document).expect_err("version should fail");
                assert!(error.to_string().contains(expected));
            } else {
                let mut client = MinimalStdioMcpClient::start(&document).expect("start");
                let error = client.list_tools().expect_err("capability should fail");
                assert!(error.to_string().contains(expected));
            }
            let _ = fs::remove_dir_all(workspace);
        }
    }

    #[test]
    fn rejects_repeated_cursor_page_tool_schema_and_result_limits() {
        let cases = [
            ("repeat-cursor", "重复 nextCursor"),
            ("pages", "list_pages"),
            ("many-tools", "tools_per_page"),
            ("tools-total", "tools_total"),
            ("large-name", "tool_name_bytes"),
            ("large-description", "tool_description_bytes"),
            ("large-schema", "input_schema_bytes"),
            ("large-output-schema", "output_schema_bytes"),
            ("metadata-total", "tool_metadata_bytes"),
            ("event-flood", "queued_client_events"),
        ];
        for (mode, expected) in cases {
            let workspace = test_home(mode);
            fs::create_dir_all(&workspace).expect("workspace");
            let document = helper_document(&workspace, mode);
            let mut limits = McpLimits {
                max_list_pages: 2,
                ..McpLimits::default()
            };
            if mode == "many-tools" {
                limits.max_tools_per_page = 1;
            }
            if mode == "tools-total" {
                limits.max_tools_total = 1;
            }
            if mode == "large-name" {
                limits.max_tool_name_bytes = 8;
            }
            if mode == "large-description" {
                limits.max_description_bytes = 8;
            }
            if mode == "large-schema" {
                limits.max_input_schema_bytes = 24;
            }
            if mode == "large-output-schema" {
                limits.max_output_schema_bytes = 24;
            }
            if mode == "metadata-total" {
                limits.max_tool_metadata_bytes = 32;
            }
            if mode == "event-flood" {
                limits.max_queued_events = 1;
            }
            let mut client =
                MinimalStdioMcpClient::start_with_limits(&document, limits).expect("start");
            let error = client.list_tools().expect_err("limit should fail");
            assert!(error.to_string().contains(expected), "{mode}: {error}");
            let _ = fs::remove_dir_all(workspace);
        }

        let workspace = test_home("large-result");
        fs::create_dir_all(&workspace).expect("workspace");
        let document = helper_document(&workspace, "large-result");
        let limits = McpLimits {
            max_call_result_bytes: 32,
            ..McpLimits::default()
        };
        let mut client =
            MinimalStdioMcpClient::start_with_limits(&document, limits).expect("start");
        let error = client
            .call_tool("lookup", json!({}))
            .expect_err("result limit");
        assert!(error.to_string().contains("call_result_bytes"));
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn bounded_reader_rejects_oversized_newline_frames() {
        let limits = McpLimits {
            max_frame_bytes: 16,
            ..McpLimits::default()
        };

        let mut oversized_json = vec![b'{'];
        oversized_json.extend(std::iter::repeat_n(b'x', 16));
        let mut newline = Cursor::new(oversized_json);
        let error = read_next_message_bytes(&mut newline, limits).expect_err("newline limit");
        assert!(matches!(
            error,
            TransportReadError::Limit {
                limit: "frame_bytes",
                ..
            }
        ));
    }

    #[test]
    fn stderr_summary_is_bounded_and_never_contains_raw_secret() {
        let workspace = test_home("stderr");
        fs::create_dir_all(&workspace).expect("workspace");
        let document = helper_document(&workspace, "stderr-secret");
        let limits = McpLimits {
            max_stderr_bytes: 8,
            ..McpLimits::default()
        };
        let error =
            MinimalStdioMcpClient::start_with_limits(&document, limits).expect_err("bad version");
        let message = error.to_string();
        assert!(message.contains("had_output=true"), "{message}");
        assert!(message.contains("truncated=true"), "{message}");
        assert!(!message.contains("TOP_SECRET"));
        assert!(!message.contains("do-not-leak"));
        let _ = fs::remove_dir_all(workspace);
    }

    #[cfg(unix)]
    #[test]
    fn initialize_list_and_call_failures_close_child_processes() {
        for phase in ["initialize", "list", "call"] {
            let workspace = test_home(&format!("cleanup-{phase}"));
            fs::create_dir_all(&workspace).expect("workspace");
            let mode = match phase {
                "initialize" => "bad-version",
                "list" => "missing-tools",
                "call" => "large-result",
                _ => unreachable!("fixed test phase"),
            };
            let document = helper_document(&workspace, mode);
            match phase {
                "initialize" => {
                    MinimalStdioMcpClient::start(&document).expect_err("initialize failure");
                }
                "list" => {
                    let mut client = MinimalStdioMcpClient::start(&document).expect("start");
                    client.list_tools().expect_err("list failure");
                }
                "call" => {
                    let limits = McpLimits {
                        max_call_result_bytes: 32,
                        ..McpLimits::default()
                    };
                    let mut client =
                        MinimalStdioMcpClient::start_with_limits(&document, limits).expect("start");
                    client
                        .call_tool("lookup", json!({}))
                        .expect_err("call failure");
                }
                _ => unreachable!("fixed test phase"),
            }
            assert_process_stopped(&workspace.join("server.pid"));
            let _ = fs::remove_dir_all(workspace);
        }
    }

    #[cfg(unix)]
    fn assert_process_stopped(pid_file: &Path) {
        let pid = fs::read_to_string(pid_file).expect("pid file");
        for _ in 0..50 {
            let running = Command::new("kill")
                .args(["-0", pid.trim()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("probe process")
                .success();
            if !running {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("MCP child process {pid} is still running");
    }

    fn helper_script_source() -> &'static str {
        include_str!("../tests/fixtures/stdio_conformance_server.py")
    }
}
