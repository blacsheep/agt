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
        let mut command = Command::new(&harness.executable);
        command
            .arg("app-server")
            .arg("--listen")
            .arg("stdio://")
            .current_dir(&harness.working_directory);
        let mut client = Self {
            process: Lines::spawn(command, "Codex app-server")?,
            next_id: 0,
        };
        client.request("initialize", json!({"clientInfo":{"name":"agt","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}), event)?;
        client.send(json!({"method":"initialized","params":{}}))?;
        client.process.deadline = Instant::now() + Duration::from_secs(harness.timeout_seconds);
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
