//! Newline-delimited stdio MCP. Stdout is exclusively JSON-RPC frames.
use crate::ToolService;
use anyhow::Result;
use serde_json::{Value, json};
use std::io::{BufRead, Write};

pub fn serve(service: &ToolService, mut input: impl BufRead, mut output: impl Write) -> Result<()> {
    let mut initialized = false;
    let mut ready = false;
    loop {
        let mut bytes = Vec::new();
        let count =
            std::io::Read::take(&mut input, 1024 * 1024 + 1).read_until(b'\n', &mut bytes)?;
        if count == 0 {
            break;
        }
        if bytes.len() > 1024 * 1024 {
            anyhow::bail!("MCP input frame exceeds 1 MiB");
        }
        let request: Value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(_) => {
                write_frame(&mut output, &error(Value::Null, -32700, "Parse error"))?;
                continue;
            }
        };
        let id = request.get("id").cloned();
        let method = request.get("method").and_then(Value::as_str);
        if !request.is_object()
            || request["jsonrpc"] != "2.0"
            || method.is_none()
            || id
                .as_ref()
                .is_some_and(|v| !v.is_string() && !v.is_i64() && !v.is_u64())
        {
            write_frame(&mut output, &error(Value::Null, -32600, "Invalid request"))?;
            continue;
        }
        let method = method.expect("checked method");
        if id.is_none() {
            if initialized && method == "notifications/initialized" {
                ready = true;
            }
            continue;
        }
        let id = id.expect("request id");
        let result = match method {
            "initialize" if !initialized => {
                if !request["params"]["protocolVersion"].is_string()
                    || !request["params"]["clientInfo"].is_object()
                {
                    Err((-32602, "Invalid initialize parameters"))
                } else {
                    initialized = true;
                    Ok(
                        json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"agt-framework","version":env!("CARGO_PKG_VERSION")},"instructions":"Your identity and role are fixed by the host. Send messages then inspect history/inbox; the host authorizes discussions and runs agt schedule run to execute hosted agents."}),
                    )
                }
            }
            "ping" => Ok(json!({})),
            _ if !ready => Err((-32002, "Server not initialized")),
            "tools/list" => {
                if request["params"].get("cursor").is_some() {
                    Err((-32602, "This tool list has no continuation cursor"))
                } else {
                    Ok(json!({"tools":service.schemas()}))
                }
            }
            "tools/call" => {
                let params = &request["params"];
                if params["name"].as_str().is_none()
                    || params.get("arguments").is_some_and(|v| !v.is_object())
                {
                    Err((-32602, "Invalid tool call parameters"))
                } else {
                    let name = params["name"].as_str().expect("name");
                    if !service.schemas().iter().any(|s| s["name"] == name) {
                        Err((-32602, "Unknown or unavailable tool"))
                    } else {
                        let result = service
                            .call(name, params.get("arguments").cloned().unwrap_or(json!({})));
                        Ok(match result {
                            Ok(value) => {
                                json!({"content":[{"type":"text","text":value.to_string()}],"isError":false})
                            }
                            Err(error) => {
                                json!({"content":[{"type":"text","text":error.as_value().to_string()}],"isError":true})
                            }
                        })
                    }
                }
            }
            _ => Err((-32601, "Method not found")),
        };
        let response = match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err((code, message)) => error(id, code, message),
        };
        write_frame(&mut output, &response)?;
    }
    Ok(())
}
fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
fn write_frame(output: &mut impl Write, value: &Value) -> Result<()> {
    let encoded = serde_json::to_vec(value)?;
    if encoded.len() > 4 * 1024 * 1024 {
        let fallback = error(
            value["id"].clone(),
            -32603,
            "Result exceeds 4 MiB; use a smaller page",
        );
        serde_json::to_writer(&mut *output, &fallback)?;
    } else {
        output.write_all(&encoded)?;
    }
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}
