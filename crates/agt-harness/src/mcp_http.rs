//! A loopback MCP endpoint (Streamable HTTP, JSON responses) that offers agt's
//! framework tools to an engine for the length of one turn.
//!
//! Only `127.0.0.1` is bound, and every request must carry a random bearer
//! token. Tool calls are not run on the server thread: they are handed to the
//! engine's own thread, which runs them through the same callback, and so
//! inside the same scheduler fence, as any other tool call.
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, mpsc::RecvTimeoutError};
use std::time::Duration;

const MAX_BODY: usize = 4 * 1024 * 1024;

/// A tool call waiting for the engine thread, and where to send its result.
pub(crate) struct Call {
    pub name: String,
    pub arguments: Value,
    pub reply: Sender<Value>,
}

pub(crate) struct Bridge {
    pub url: String,
    pub token: String,
    pub calls: Receiver<Call>,
    stop: Arc<AtomicBool>,
}

impl Bridge {
    /// Start serving `tools` (MCP tool descriptors) until the bridge is dropped.
    pub(crate) fn start(tools: Vec<Value>, timeout: Duration) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").context("cannot open the tool bridge")?;
        listener.set_nonblocking(true)?;
        let url = format!("http://{}/mcp", listener.local_addr()?);
        let token = uuid::Uuid::new_v4().simple().to_string();
        let (sender, calls) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Server {
            tools,
            token: token.clone(),
            sender,
            timeout,
        });
        let stopped = stop.clone();
        std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let state = state.clone();
                        std::thread::spawn(move || {
                            let _ = state.serve(stream);
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            url,
            token,
            calls,
            stop,
        })
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

struct Server {
    tools: Vec<Value>,
    token: String,
    sender: Sender<Call>,
    timeout: Duration,
}

impl Server {
    fn serve(&self, stream: TcpStream) -> Result<()> {
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut writer = stream;
        // Keep-alive: one connection may carry several requests.
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                return Ok(());
            }
            let method = line.split_whitespace().next().unwrap_or("").to_owned();
            let mut length = 0;
            let mut authorized = false;
            let mut close = false;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header)?;
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':') {
                    let value = value.trim();
                    match name.trim().to_ascii_lowercase().as_str() {
                        "content-length" => length = value.parse().unwrap_or(0),
                        "authorization" => {
                            authorized = value == format!("Bearer {}", self.token);
                        }
                        "connection" => close = value.eq_ignore_ascii_case("close"),
                        _ => {}
                    }
                }
            }
            if length > MAX_BODY {
                return respond(&mut writer, 413, None);
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body)?;
            if !authorized {
                respond(&mut writer, 401, None)?;
                if close {
                    return Ok(());
                }
                continue;
            }
            match method.as_str() {
                "POST" => {
                    let message: Value = match serde_json::from_slice(&body) {
                        Ok(message) => message,
                        Err(_) => {
                            respond(&mut writer, 400, None)?;
                            continue;
                        }
                    };
                    match self.handle(&message) {
                        Some(reply) => respond(&mut writer, 200, Some(&reply))?,
                        None => respond(&mut writer, 202, None)?,
                    }
                }
                // No server-initiated stream; sessions need no explicit end.
                "DELETE" => respond(&mut writer, 200, None)?,
                _ => respond(&mut writer, 405, None)?,
            }
            if close {
                return Ok(());
            }
        }
    }

    /// The JSON-RPC reply, or `None` for a notification.
    fn handle(&self, message: &Value) -> Option<Value> {
        let id = message.get("id")?.clone();
        let result = match message["method"].as_str() {
            Some("initialize") => json!({
                "protocolVersion": message["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "agt", "version": env!("CARGO_PKG_VERSION")},
            }),
            Some("ping") => json!({}),
            Some("tools/list") => json!({"tools": self.tools}),
            Some("tools/call") => {
                let (reply, result) = mpsc::channel();
                let call = Call {
                    name: message["params"]["name"].as_str().unwrap_or("").to_owned(),
                    arguments: message["params"]["arguments"].clone(),
                    reply,
                };
                if self.sender.send(call).is_err() {
                    return Some(error(id, "the turn has ended"));
                }
                match result.recv_timeout(self.timeout) {
                    Ok(value) => value,
                    Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                        return Some(error(id, "the turn has ended"));
                    }
                }
            }
            _ => {
                return Some(json!({"jsonrpc":"2.0","id":id,
                    "error":{"code":-32601,"message":"method not found"}}));
            }
        };
        Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
}

fn error(id: Value, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":message}})
}

fn respond(writer: &mut TcpStream, status: u16, body: Option<&Value>) -> Result<()> {
    let text = body.map(Value::to_string).unwrap_or_default();
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        405 => "Method Not Allowed",
        _ => "Payload Too Large",
    };
    write!(
        writer,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{text}",
        text.len()
    )?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(url: &str, token: &str, body: Value) -> (u16, String) {
        let address = url.trim_start_matches("http://").split('/').next().unwrap();
        let mut stream = TcpStream::connect(address).unwrap();
        let text = body.to_string();
        write!(
            stream,
            "POST /mcp HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
            text.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let status = response[9..12].parse().unwrap();
        let body = response.split("\r\n\r\n").nth(1).unwrap_or("").to_owned();
        (status, body)
    }

    #[test]
    fn tools_run_on_the_engine_thread_and_strangers_are_refused() {
        let bridge = Bridge::start(
            vec![json!({"name":"echo","inputSchema":{"type":"object"}})],
            Duration::from_secs(5),
        )
        .unwrap();
        let (url, token) = (bridge.url.clone(), bridge.token.clone());
        assert_eq!(
            post(
                &url,
                "wrong",
                json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})
            )
            .0,
            401
        );
        let (status, body) = post(
            &url,
            &token,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        );
        assert_eq!(status, 200);
        assert!(body.contains("\"echo\""));
        assert_eq!(
            post(
                &url,
                &token,
                json!({"jsonrpc":"2.0","method":"notifications/initialized"})
            )
            .0,
            202
        );
        let caller = std::thread::spawn(move || {
            post(
                &url,
                &token,
                json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"echo","arguments":{"x":1}}}),
            )
        });
        let call = bridge.calls.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(call.name, "echo");
        assert_eq!(call.arguments["x"], 1);
        call.reply
            .send(json!({"content":[{"type":"text","text":"done"}]}))
            .unwrap();
        let (status, body) = caller.join().unwrap();
        assert_eq!(status, 200);
        assert!(body.contains("done"));
    }
}
