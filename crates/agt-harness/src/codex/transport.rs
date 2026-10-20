//! Codex app-server JSON-RPC over the shared engine process.
use super::*;
use crate::process::Lines;
use std::process::Command;
pub(super) struct Transport {
    pub(super) process: Lines,
    pub(super) next_id: u64,
}
impl Transport {
    pub(super) fn diagnostics(&mut self) -> ProcessDiagnostics {
        self.process.diagnostics()
    }

    pub(super) fn connect(
        harness: &HarnessConfig,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<Self> {
        harness.validate()?;
        let mut client = Self::start(harness, &[], event)?;
        if !harness.inherit_mcp_servers {
            // Codex reports its effective config without starting any server; restart it
            // with each configured server disabled, if there are any.
            let config = client.request(
                "config/read",
                json!({"cwd": harness.working_directory}),
                event,
            )?;
            let disabled = disable_mcp_servers(&config["config"]["mcp_servers"]);
            if !disabled.is_empty() {
                drop(client);
                client = Self::start(harness, &disabled, event)?;
            }
        }
        client.process.deadline = Instant::now() + Duration::from_secs(harness.timeout_seconds);
        Ok(client)
    }

    fn start(
        harness: &HarnessConfig,
        overrides: &[String],
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<Self> {
        let mut command = Command::new(&harness.executable);
        command.arg("app-server");
        for value in overrides {
            command.arg("-c").arg(value);
        }
        command
            .arg("--listen")
            .arg("stdio://")
            .current_dir(&harness.working_directory);
        let mut client = Self {
            process: Lines::spawn(command, "Codex app-server")?,
            next_id: 0,
        };
        client.request("initialize", json!({"clientInfo":{"name":"agt","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}), event)?;
        client.send(json!({"method":"initialized","params":{}}))?;
        Ok(client)
    }
    pub(super) fn send(&mut self, value: Value) -> Result<()> {
        self.process.send(&value)
    }
    pub(super) fn receive(
        &mut self,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<Value> {
        self.process.receive(event)
    }
    pub(super) fn poll(
        &mut self,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<Option<Value>> {
        self.process.poll(event)
    }
    pub(super) fn request(
        &mut self,
        method: &str,
        params: Value,
        event: &mut dyn FnMut(Event<'_>) -> Result<()>,
    ) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"id":id,"method":method,"params":params}))?;
        loop {
            let value = self.receive(event)?;
            if value["id"] == id && value.get("method").is_none() {
                if value.get("error").is_some() {
                    bail!(
                        "Codex {method} rejected: {}",
                        value["error"]["message"]
                            .as_str()
                            .unwrap_or("protocol error")
                    );
                }
                return Ok(value["result"].clone());
            }
            if value.get("id").is_some() {
                bail!("Codex requested unsupported interaction during {method}");
            }
        }
    }
}

/// `-c` overrides that disable every enabled server in Codex's `mcp_servers` table.
/// Overriding the table itself does not work: Codex merges it with its config files.
/// `-c` splits its key on dots and takes quotes literally, so a server whose name has
/// other characters than letters, digits, `_` and `-` cannot be addressed and stays on.
fn disable_mcp_servers(servers: &Value) -> Vec<String> {
    let Some(servers) = servers.as_object() else {
        return Vec::new();
    };
    servers
        .iter()
        .filter(|(name, server)| {
            server["enabled"] != false
                && !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
        .map(|(name, _)| format!("mcp_servers.{name}.enabled=false"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disables_each_enabled_server_it_can_address() {
        let servers = json!({
            "fetch": {"command": "uvx"},
            "sequential-thinking": {"command": "npx"},
            "off": {"command": "x", "enabled": false},
            "odd.name": {"url": "https://example.com"},
        });
        assert_eq!(
            disable_mcp_servers(&servers),
            vec![
                "mcp_servers.fetch.enabled=false".to_owned(),
                "mcp_servers.sequential-thinking.enabled=false".to_owned(),
            ]
        );
        assert!(disable_mcp_servers(&Value::Null).is_empty());
    }
}
