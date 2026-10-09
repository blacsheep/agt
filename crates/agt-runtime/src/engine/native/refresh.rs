//! Capability changes are committed between tool batches, never during a call.
use super::*;
use crate::audit::{CallRecord, CallState, runtime_error};
use crate::fingerprint;

pub(super) fn execution_fingerprint(
    agent: &ResolvedAgent,
    agents: &BTreeMap<String, ResolvedAgent>,
    root: bool,
    permissions: &crate::permission::PermissionLoop,
    schemas: &[Value],
) -> Result<String> {
    let snapshot = if root {
        agents
            .iter()
            .map(|(name, value)| {
                (
                    name,
                    if value.name == agent.name {
                        &agent.fingerprint
                    } else {
                        &value.fingerprint
                    },
                )
            })
            .collect::<Vec<_>>()
    } else {
        vec![(&agent.name, &agent.fingerprint)]
    };
    let base = Some(fingerprint(&serde_json::to_vec(&(
        snapshot,
        permissions.config(),
        permissions.reviewer_fingerprint(),
    ))?));
    Ok(fingerprint(&serde_json::to_vec(&(base, schemas))?))
}

impl NativeRun<'_, '_> {
    pub(super) fn affects_capabilities(&self, call: &ToolCall, args: &Value) -> bool {
        if call.function.name != "framework_resource_write" {
            return false;
        }
        let Some(reference) = args["reference"].as_str() else {
            return false;
        };
        let kind = args["kind"].as_str().unwrap_or("");
        let Ok((category, name)) = agt_service::resource_ref(kind, reference) else {
            return false;
        };
        let reference = format!("{category}/{name}");
        let definition = self.agent.definition.as_ref().expect("native definition");
        match kind {
            "agent" => {
                reference == self.agent.name
                    && (["skills", "mcps"].iter().any(|field| {
                        args["data"].get(field).is_some_and(|v| !v.is_null())
                            || args["add"]
                                .get(field)
                                .is_some_and(|v| v.as_array().is_some_and(|a| !a.is_empty()))
                            || args["remove"]
                                .get(field)
                                .is_some_and(|v| v.as_array().is_some_and(|a| !a.is_empty()))
                    }) || args["clear"]
                        .as_array()
                        .is_some_and(|fields| fields.iter().any(|f| f == "skills" || f == "mcps")))
            }
            "mcp" => definition.mcps.iter().any(|r| r.to_string() == reference),
            "skill" => definition.skills.iter().any(|r| r.to_string() == reference),
            _ => false,
        }
    }

    pub(super) fn refresh_capabilities(&mut self) -> Result<()> {
        check_canceled(self.cancellation)?;
        self.runtime.check_schedule()?;
        let mut audit = self.runtime.store.start_call(CallRecord::new(
            self.record,
            Some("capability_refresh"),
            json!({}),
        ))?;
        let result = self.prepare_capabilities();
        let notice = match result {
            Ok((agent, tools, fingerprint)) => {
                self.record.config_fingerprint = Some(fingerprint.clone());
                self.runtime.store.save(self.record)?;
                self.messages[0] = json!({"role":"system","content":agent.system_prompt});
                self.agent = agent;
                if let Some(tools) = tools {
                    self.tools = tools; // Close the old session only after the candidate is ready.
                }
                self.usage.observe(None, 0);
                audit.finish(CallState::Succeeded, None);
                json!({"state":"applied","config_fingerprint":fingerprint,
                    "message":"MCP and Skill capabilities are loaded for this Run. Use the tools in the next request; other configuration changes apply to new Runs."})
            }
            Err(error @ (RuntimeError::Canceled | RuntimeError::Stopped(_))) => {
                audit.finish(CallState::Canceled, Some(runtime_error(&error)));
                self.runtime.store.finish_call(&audit)?;
                return Err(error);
            }
            Err(error) => {
                let error = match &error {
                    RuntimeError::Configuration(message) => {
                        json!({"code":"configuration_error","message":message.message()})
                    }
                    _ => runtime_error(&error),
                };
                audit.finish(CallState::Failed, Some(error.clone()));
                json!({"state":"failed","error":error,
                    "message":"Configuration was saved, but capability loading failed. This Run retains its previous tools and Skills. Do not assume the new capability is available; correct the configuration before retrying."})
            }
        };
        self.runtime.store.finish_call(&audit)?;
        self.runtime.store.save_tool_detail(
            &self.record.id,
            &format!("call:{}", audit.id),
            &[("output", &notice)],
        )?;
        self.messages.push(
            json!({"role":"user","content":format!("AGT capability refresh result: {notice}")}),
        );
        Ok(())
    }

    fn prepare_capabilities(&self) -> Result<(ResolvedAgent, Option<ToolSession>, String)> {
        let agent = self.agent.reload_capabilities(&self.runtime.home)?;
        if !agent.mcps.is_empty() && !agent.model.capabilities.tool_call {
            return Err(RuntimeError::Configuration(
                "model tool_call capability is disabled".into(),
            ));
        }
        if agent.mcps == self.agent.mcps
            && agent.definition.as_ref().unwrap().mcps
                == self.agent.definition.as_ref().unwrap().mcps
        {
            // Skill-only changes must not restart stateful MCP servers.
            let fingerprint = execution_fingerprint(
                &agent,
                self.agents,
                self.record.parent_run_id.is_none(),
                &self.permissions,
                &self.tools.schemas,
            )?;
            check_canceled(self.cancellation)?;
            self.runtime.check_schedule()?;
            return Ok((agent, None, fingerprint));
        }
        let mut tools = ToolSession::connect(
            &agent,
            &self.record.request,
            self.runtime.factory,
            self.cancellation,
        )?;
        // Preserve the exact framework grant/schema snapshot. Never reload permissions here.
        tools.schemas.extend(
            self.tools
                .schemas
                .iter()
                .filter(|schema| {
                    schema["function"]["name"]
                        .as_str()
                        .is_some_and(|name| self.communication_names.contains(name))
                })
                .cloned(),
        );
        if tools.schemas.len() > 128 {
            return Err(RuntimeError::Tool("more than 128 tools attached".into()));
        }
        tools.compile_schemas()?;
        let fingerprint = execution_fingerprint(
            &agent,
            self.agents,
            self.record.parent_run_id.is_none(),
            &self.permissions,
            &tools.schemas,
        )?;
        check_canceled(self.cancellation)?;
        self.runtime.check_schedule()?;
        Ok((agent, Some(tools), fingerprint))
    }
}
